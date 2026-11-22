use std::path::PathBuf;

use pas_core::client::Client;
use pas_core::protocol::{
    CreateParams, EVENT_SESSION_CREATED, EVENT_SESSION_OUTPUT, EVENT_SESSION_REMOVED,
    EVENT_SESSION_STATE, InputParams, METHOD_DAEMON_PING, METHOD_PROJECT_ADD, METHOD_PROJECT_LIST,
    METHOD_PROJECT_REMOVE, METHOD_SESSION_INPUT, METHOD_SESSION_RESIZE, METHOD_SESSION_SEND,
    METHOD_SESSION_SNAPSHOT, METHOD_SESSION_SUBSCRIBE, METHOD_SESSION_UNSUBSCRIBE, Notification,
    PingResult, ProjectAddParams, ProjectInfo, Request, ResizeParams, Response, RpcError,
    SendParams, SessionCreatedEvent, SessionInfo, SessionOutputEvent, SessionRemovedEvent,
    SessionState, SessionStateEvent, SnapshotResult,
};
use pas_core::socket::{read_msg, write_msg};
use serde_json::{Value, json};
use tempfile::tempdir;
use tokio::io::BufReader;
use tokio::net::UnixListener;

#[test]
fn request_roundtrip() {
    let req = Request {
        jsonrpc: "2.0".into(),
        id: 1,
        method: METHOD_SESSION_SEND.into(),
        params: serde_json::to_value(SendParams {
            id: "abc".into(),
            text: "hola".into(),
            enter: true,
        })
        .unwrap(),
    };

    let serialized = serde_json::to_string(&req).unwrap();
    let deserialized: Request = serde_json::from_str(&serialized).unwrap();
    assert_eq!(deserialized, req);
}

#[test]
fn state_json_shape() {
    assert_eq!(
        serde_json::to_string(&SessionState::Running).unwrap(),
        r#"{"state":"running"}"#
    );
    assert_eq!(
        serde_json::to_string(&SessionState::Idle).unwrap(),
        r#"{"state":"idle"}"#
    );
    assert_eq!(
        serde_json::to_string(&SessionState::Exited { code: Some(0) }).unwrap(),
        r#"{"state":"exited","code":0}"#
    );
}

#[test]
fn create_params_defaults() {
    let params: CreateParams =
        serde_json::from_str(r#"{"command":["sh"],"cwd":"/tmp"}"#).unwrap();

    assert_eq!(params.command, vec!["sh"]);
    assert_eq!(params.cwd, PathBuf::from("/tmp"));
    assert_eq!(params.cols, 120);
    assert_eq!(params.rows, 40);
    assert!(params.env.is_empty());
    assert_eq!(params.title, None);
    assert_eq!(params.project_id, None);
    assert_eq!(params.mode, pas_core::protocol::Mode::Terminal);
}

#[test]
fn chat_types_roundtrip() {
    use pas_core::protocol::{ChatEvent, ChatEventNotification, Mode, PermissionChoice};

    let create: CreateParams =
        serde_json::from_str(r#"{"command":["claude"],"cwd":"/tmp","mode":"chat"}"#).unwrap();
    assert_eq!(create.mode, Mode::Chat);

    let events = vec![
        ChatEvent::UserMessage {
            text: "hola".into(),
        },
        ChatEvent::AgentChunk {
            text: "mundo".into(),
        },
        ChatEvent::AgentMessageEnd,
        ChatEvent::ToolCall {
            id: "tc-1".into(),
            title: "Leyó archivo".into(),
            status: "completed".into(),
        },
        ChatEvent::Diff {
            path: "src/lib.rs".into(),
            unified: "--- a/src/lib.rs\n+++ b/src/lib.rs\n".into(),
        },
        ChatEvent::PermissionRequest {
            request_id: "p1".into(),
            title: "Ejecutar".into(),
            detail: Some("cargo test".into()),
        },
        ChatEvent::PermissionResolved {
            request_id: "p1".into(),
            choice: PermissionChoice::AllowSession,
        },
        ChatEvent::Visual {
            file: "mockup.html".into(),
        },
        ChatEvent::Error {
            message: "falló".into(),
        },
    ];
    for ev in events {
        let notif = ChatEventNotification {
            id: "s1".into(),
            event: ev,
        };
        let raw = serde_json::to_string(&notif).unwrap();
        let parsed: ChatEventNotification = serde_json::from_str(&raw).unwrap();
        assert_eq!(parsed, notif);
    }
}

#[test]
fn send_params_default_enter() {
    let params: SendParams =
        serde_json::from_str(r#"{"id":"s1","text":"ls"}"#).unwrap();
    assert!(params.enter);
}

#[test]
fn response_ok_and_err() {
    let ok = Response::ok(7, json!({"version": "0.1.0"}));
    assert_eq!(ok.jsonrpc, "2.0");
    assert_eq!(ok.id, 7);
    assert!(ok.result.is_some());
    assert!(ok.error.is_none());

    let err = Response::err(
        8,
        RpcError {
            code: 1001,
            message: "no existe".into(),
        },
    );
    assert_eq!(err.jsonrpc, "2.0");
    assert_eq!(err.id, 8);
    assert!(err.result.is_none());
    assert_eq!(err.error.unwrap().code, 1001);
}

#[test]
fn notification_has_no_id() {
    let notif = Notification::new(
        EVENT_SESSION_OUTPUT,
        serde_json::to_value(SessionOutputEvent {
            id: "s1".into(),
            data_b64: "aG9sYQ==".into(),
        })
        .unwrap(),
    );

    let value: Value = serde_json::to_value(&notif).unwrap();
    let obj = value.as_object().unwrap();
    assert!(!obj.contains_key("id"));
    assert_eq!(obj.get("jsonrpc").unwrap(), "2.0");
    assert_eq!(obj.get("method").unwrap(), EVENT_SESSION_OUTPUT);

    let serialized = serde_json::to_string(&notif).unwrap();
    let deserialized: Notification = serde_json::from_str(&serialized).unwrap();
    assert_eq!(deserialized, notif);
}

#[test]
fn input_params_roundtrip() {
    let params = InputParams {
        id: "s1".into(),
        data_b64: "Aw==".into(),
    };
    let serialized = serde_json::to_string(&params).unwrap();
    let deserialized: InputParams = serde_json::from_str(&serialized).unwrap();
    assert_eq!(deserialized, params);

    assert_eq!(METHOD_SESSION_INPUT, "session.input");
    assert_eq!(METHOD_SESSION_RESIZE, "session.resize");
    assert_eq!(METHOD_SESSION_SNAPSHOT, "session.snapshot");
    assert_eq!(METHOD_SESSION_SUBSCRIBE, "session.subscribe");
    assert_eq!(METHOD_SESSION_UNSUBSCRIBE, "session.unsubscribe");
    assert_eq!(METHOD_PROJECT_ADD, "project.add");
    assert_eq!(METHOD_PROJECT_LIST, "project.list");
    assert_eq!(METHOD_PROJECT_REMOVE, "project.remove");
    assert_eq!(EVENT_SESSION_OUTPUT, "session.output");
    assert_eq!(EVENT_SESSION_STATE, "session.state");
    assert_eq!(EVENT_SESSION_CREATED, "session.created");
    assert_eq!(EVENT_SESSION_REMOVED, "session.removed");
}

#[test]
fn stage2_types_roundtrip() {
    let resize = ResizeParams {
        id: "s1".into(),
        cols: 100,
        rows: 30,
    };
    assert_eq!(
        serde_json::from_str::<ResizeParams>(&serde_json::to_string(&resize).unwrap()).unwrap(),
        resize
    );

    let snap = SnapshotResult {
        data_b64: "G1szMW1ob2xh".into(),
    };
    assert_eq!(
        serde_json::from_str::<SnapshotResult>(&serde_json::to_string(&snap).unwrap()).unwrap(),
        snap
    );

    let proj_add: ProjectAddParams = serde_json::from_str(r#"{"path":"/tmp/repo"}"#).unwrap();
    assert_eq!(proj_add.path, PathBuf::from("/tmp/repo"));
    assert_eq!(proj_add.name, None);

    let proj = ProjectInfo {
        id: "p1".into(),
        name: "pas".into(),
        path: PathBuf::from("/tmp/repo"),
        group_id: "g1".into(),
        group_pinned: false,
    };
    assert_eq!(
        serde_json::from_str::<ProjectInfo>(&serde_json::to_string(&proj).unwrap()).unwrap(),
        proj
    );

    let create: CreateParams =
        serde_json::from_str(r#"{"command":["sh"],"cwd":"/tmp","project_id":"p1"}"#).unwrap();
    assert_eq!(create.project_id.as_deref(), Some("p1"));

    let info = SessionInfo {
        id: "s1".into(),
        title: "shell".into(),
        cwd: PathBuf::from("/tmp"),
        command: vec!["sh".into()],
        state: SessionState::Running,
        created_at: 123,
        project_id: Some("p1".into()),
        account_id: None,
        mode: pas_core::protocol::Mode::Terminal,
    };
    assert_eq!(
        serde_json::from_str::<SessionInfo>(&serde_json::to_string(&info).unwrap()).unwrap(),
        info
    );

    let state_ev = SessionStateEvent {
        id: "s1".into(),
        state: SessionState::Idle,
    };
    assert_eq!(
        serde_json::from_str::<SessionStateEvent>(&serde_json::to_string(&state_ev).unwrap())
            .unwrap(),
        state_ev
    );

    let created_ev = SessionCreatedEvent { info: info.clone() };
    assert_eq!(
        serde_json::from_str::<SessionCreatedEvent>(&serde_json::to_string(&created_ev).unwrap())
            .unwrap(),
        created_ev
    );

    let removed_ev = SessionRemovedEvent { id: "s1".into() };
    assert_eq!(
        serde_json::from_str::<SessionRemovedEvent>(&serde_json::to_string(&removed_ev).unwrap())
            .unwrap(),
        removed_ev
    );
}

#[tokio::test]
async fn client_separates_events_from_responses() {
    let dir = tempdir().unwrap();
    let sock = dir.path().join("pasd.sock");
    let listener = UnixListener::bind(&sock).unwrap();

    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut stream = BufReader::new(stream);

        let req: Request = read_msg(&mut stream).await.unwrap().unwrap();
        let notif = Notification::new(
            EVENT_SESSION_OUTPUT,
            serde_json::to_value(SessionOutputEvent {
                id: "s1".into(),
                data_b64: "aG9sYQ==".into(),
            })
            .unwrap(),
        );
        write_msg(&mut stream, &notif).await.unwrap();

        let resp = Response::ok(req.id, json!({ "version": "0.1.0" }));
        write_msg(&mut stream, &resp).await.unwrap();
    });

    let mut client = Client::connect(&sock).await.unwrap();
    let ping: PingResult = client.call(METHOD_DAEMON_PING, json!({})).await.unwrap();
    assert_eq!(ping.version, "0.1.0");

    let ev = client.next_event().await.unwrap().unwrap();
    assert_eq!(ev.method, EVENT_SESSION_OUTPUT);
    let payload: SessionOutputEvent = serde_json::from_value(ev.params).unwrap();
    assert_eq!(payload.id, "s1");
    assert_eq!(payload.data_b64, "aG9sYQ==");

    let eof = client.next_event().await.unwrap();
    assert_eq!(eof, None);

    server.await.unwrap();
}

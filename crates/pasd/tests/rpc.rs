use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

use pas_core::protocol::{
    CreateResult, ERR_INVALID_PARAMS, ERR_METHOD_NOT_FOUND, ERR_SESSION_EXITED,
    ERR_SESSION_RUNNING, EVENT_SESSION_REMOVED, METHOD_SESSION_CREATE, METHOD_SESSION_KILL,
    METHOD_SESSION_LIST, METHOD_SESSION_READ, METHOD_SESSION_REMOVE, METHOD_SESSION_SEND,
    METHOD_SESSION_WAIT, ReadResult, Request, SessionInfo, SessionRemovedEvent, SessionState,
};
use pasd::manager::Manager;
use pasd::rpc::dispatch;
use serde_json::{Value, json};

fn fake_agent_bin() -> String {
    static BIN: OnceLock<String> = OnceLock::new();
    BIN.get_or_init(|| {
        let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();

        let status = Command::new("cargo")
            .args(["build", "-p", "fake-agent", "--quiet"])
            .current_dir(&workspace_root)
            .status()
            .expect("no se pudo compilar fake-agent");
        assert!(status.success(), "falló cargo build -p fake-agent");

        let target_dir = std::env::var_os("CARGO_TARGET_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| workspace_root.join("target"));
        target_dir
            .join("debug")
            .join("fake-agent")
            .to_string_lossy()
            .into_owned()
    })
    .clone()
}

fn req(method: &str, params: Value) -> Request {
    Request {
        jsonrpc: "2.0".into(),
        id: 1,
        method: method.into(),
        params,
    }
}

fn create_params(title: &str) -> Value {
    json!({
        "command": [fake_agent_bin()],
        "cwd": std::env::temp_dir(),
        "title": title,
    })
}

#[tokio::test]
async fn create_list_kill() {
    let mgr = Manager::new(200);

    let r = dispatch(&mgr, req(METHOD_SESSION_CREATE, create_params("a"))).await;
    let created: CreateResult = serde_json::from_value(r.result.unwrap()).unwrap();
    assert_eq!(created.id.len(), 8);

    let r = dispatch(&mgr, req(METHOD_SESSION_LIST, json!({}))).await;
    let list: Vec<SessionInfo> = serde_json::from_value(r.result.unwrap()).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].title, "a");

    let r = dispatch(&mgr, req(METHOD_SESSION_KILL, json!({ "id": created.id }))).await;
    assert!(r.error.is_none());

    let r = dispatch(
        &mgr,
        req(
            METHOD_SESSION_WAIT,
            json!({ "id": created.id, "until": "exited", "timeout_ms": 5000 }),
        ),
    )
    .await;
    let st: SessionState = serde_json::from_value(r.result.unwrap()).unwrap();
    assert!(matches!(st, SessionState::Exited { .. }));
}

#[tokio::test]
async fn duplicate_title_suffix() {
    let mgr = Manager::new(200);

    let r1 = dispatch(&mgr, req(METHOD_SESSION_CREATE, create_params("a"))).await;
    let c1: CreateResult = serde_json::from_value(r1.result.unwrap()).unwrap();
    let r2 = dispatch(&mgr, req(METHOD_SESSION_CREATE, create_params("a"))).await;
    let c2: CreateResult = serde_json::from_value(r2.result.unwrap()).unwrap();

    let r = dispatch(&mgr, req(METHOD_SESSION_LIST, json!({}))).await;
    let mut titles: Vec<String> = serde_json::from_value::<Vec<SessionInfo>>(r.result.unwrap())
        .unwrap()
        .into_iter()
        .map(|s| s.title)
        .collect();
    titles.sort();
    assert_eq!(titles, vec!["a", "a-2"]);

    dispatch(&mgr, req(METHOD_SESSION_KILL, json!({ "id": c1.id }))).await;
    dispatch(&mgr, req(METHOD_SESSION_KILL, json!({ "id": c2.id }))).await;
}

#[tokio::test]
async fn unknown_method() {
    let mgr = Manager::new(200);
    let r = dispatch(&mgr, req("session.foo", json!({}))).await;
    assert_eq!(r.error.unwrap().code, ERR_METHOD_NOT_FOUND);
}

#[tokio::test]
async fn bad_params() {
    let mgr = Manager::new(200);
    let r = dispatch(&mgr, req(METHOD_SESSION_SEND, json!({ "text": "hola" }))).await;
    assert_eq!(r.error.unwrap().code, ERR_INVALID_PARAMS);
}

#[tokio::test]
async fn get_by_title() {
    let mgr = Manager::new(200);
    dispatch(&mgr, req(METHOD_SESSION_CREATE, create_params("a"))).await;
    dispatch(
        &mgr,
        req(
            METHOD_SESSION_WAIT,
            json!({ "id": "a", "until": "idle", "timeout_ms": 5000 }),
        ),
    )
    .await;

    let r = dispatch(
        &mgr,
        req(METHOD_SESSION_SEND, json!({ "id": "a", "text": "echo ping" })),
    )
    .await;
    assert!(r.error.is_none());

    dispatch(
        &mgr,
        req(
            METHOD_SESSION_WAIT,
            json!({ "id": "a", "until": "idle", "timeout_ms": 5000 }),
        ),
    )
    .await;

    let r = dispatch(&mgr, req(METHOD_SESSION_READ, json!({ "id": "a", "lines": 3 }))).await;
    let read: ReadResult = serde_json::from_value(r.result.unwrap()).unwrap();
    assert!(read.text.contains("ping"), "salida: {:?}", read.text);

    dispatch(&mgr, req(METHOD_SESSION_KILL, json!({ "id": "a" }))).await;
}

#[tokio::test]
async fn send_to_exited() {
    let mgr = Manager::new(200);
    dispatch(&mgr, req(METHOD_SESSION_CREATE, create_params("bye"))).await;
    dispatch(
        &mgr,
        req(
            METHOD_SESSION_WAIT,
            json!({ "id": "bye", "until": "idle", "timeout_ms": 5000 }),
        ),
    )
    .await;

    dispatch(
        &mgr,
        req(METHOD_SESSION_SEND, json!({ "id": "bye", "text": "exit 0" })),
    )
    .await;
    dispatch(
        &mgr,
        req(
            METHOD_SESSION_WAIT,
            json!({ "id": "bye", "until": "exited", "timeout_ms": 5000 }),
        ),
    )
    .await;

    let r = dispatch(
        &mgr,
        req(METHOD_SESSION_SEND, json!({ "id": "bye", "text": "hola" })),
    )
    .await;
    assert_eq!(r.error.unwrap().code, ERR_SESSION_EXITED);
}

#[tokio::test]
async fn session_remove_exited_ok() {
    let mgr = Manager::new(200);
    let mut events = mgr.subscribe_events();

    let created: CreateResult = serde_json::from_value(
        dispatch(&mgr, req(METHOD_SESSION_CREATE, create_params("done")))
            .await
            .result
            .unwrap(),
    )
    .unwrap();

    dispatch(
        &mgr,
        req(
            METHOD_SESSION_WAIT,
            json!({ "id": created.id, "until": "idle", "timeout_ms": 5000 }),
        ),
    )
    .await;
    dispatch(
        &mgr,
        req(
            METHOD_SESSION_SEND,
            json!({ "id": created.id, "text": "exit 0" }),
        ),
    )
    .await;
    dispatch(
        &mgr,
        req(
            METHOD_SESSION_WAIT,
            json!({ "id": created.id, "until": "exited", "timeout_ms": 5000 }),
        ),
    )
    .await;

    let r = dispatch(&mgr, req(METHOD_SESSION_REMOVE, json!({ "id": created.id }))).await;
    assert!(r.error.is_none(), "error inesperado: {:?}", r.error);

    let list: Vec<SessionInfo> = serde_json::from_value(
        dispatch(&mgr, req(METHOD_SESSION_LIST, json!({})))
            .await
            .result
            .unwrap(),
    )
    .unwrap();
    assert!(list.is_empty(), "la sesión debería haber salido de list");

    let mut saw_removed = false;
    while let Ok(notif) = events.try_recv() {
        if notif.method == EVENT_SESSION_REMOVED {
            let ev: SessionRemovedEvent = serde_json::from_value(notif.params).unwrap();
            if ev.id == created.id {
                saw_removed = true;
            }
        }
    }
    assert!(saw_removed, "debería emitir session.removed");
}

#[tokio::test]
async fn session_remove_running_is_error() {
    let mgr = Manager::new(200);
    let created: CreateResult = serde_json::from_value(
        dispatch(&mgr, req(METHOD_SESSION_CREATE, create_params("alive")))
            .await
            .result
            .unwrap(),
    )
    .unwrap();

    let r = dispatch(&mgr, req(METHOD_SESSION_REMOVE, json!({ "id": created.id }))).await;
    let err = r.error.expect("debería fallar si la sesión sigue corriendo");
    assert_eq!(err.code, ERR_SESSION_RUNNING);
    assert_eq!(
        err.message,
        "La sesión sigue corriendo. Cerrala primero con Ctrl+C o matala."
    );

    let list: Vec<SessionInfo> = serde_json::from_value(
        dispatch(&mgr, req(METHOD_SESSION_LIST, json!({})))
            .await
            .result
            .unwrap(),
    )
    .unwrap();
    assert_eq!(list.len(), 1);

    dispatch(&mgr, req(METHOD_SESSION_KILL, json!({ "id": created.id }))).await;
}

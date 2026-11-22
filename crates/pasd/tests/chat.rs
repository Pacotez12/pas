use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use pas_core::client::Client;
use pas_core::protocol::{
    CreateResult, METHOD_SESSION_CREATE, METHOD_SESSION_KILL, METHOD_SESSION_LIST,
    METHOD_SESSION_SUBSCRIBE_ALL, METHOD_SESSION_WAIT, Request, SessionState,
};
use pasd::config::Config;
use pasd::manager::Manager;
use pasd::notify::{Notifier, WaitingNotice};
use pasd::rpc::dispatch;
use pasd::server;
use pasd::store::Store;
use serde_json::{Value, json};
use tempfile::tempdir;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn target_debug_dir() -> PathBuf {
    std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| workspace_root().join("target"))
        .join("debug")
}

fn fake_acp_agent_bin() -> String {
    static BIN: OnceLock<String> = OnceLock::new();
    BIN.get_or_init(|| {
        let status = Command::new("cargo")
            .args(["build", "-p", "fake-acp-agent", "--quiet"])
            .current_dir(workspace_root())
            .status()
            .expect("no se pudo compilar fake-acp-agent");
        assert!(status.success(), "falló cargo build -p fake-acp-agent");
        target_debug_dir()
            .join("fake-acp-agent")
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

#[derive(Default)]
struct FakeNotifier {
    notices: Mutex<Vec<WaitingNotice>>,
}

impl FakeNotifier {
    fn notices(&self) -> Vec<WaitingNotice> {
        self.notices.lock().unwrap().clone()
    }
}

impl Notifier for FakeNotifier {
    fn notify_waiting(&self, notice: WaitingNotice, _on_open: Box<dyn FnOnce() + Send>) {
        self.notices.lock().unwrap().push(notice);
    }
}

async fn wait_for_state(mgr: &Manager, id: &str, expected: SessionState) {
    let session = mgr.get(id).unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        if session.state() == expected {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "esperaba estado {expected:?}, quedó en {:?}",
            session.state()
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn chat_streams_chunks() {
    let dir = tempdir().unwrap();
    let mgr = Manager::new(200);
    let mut events_rx = mgr.subscribe_events();

    let script = json!([
        { "type": "text", "chunks": ["Hola ", "desde ", "chat"] }
    ])
    .to_string();

    let r = dispatch(
        &mgr,
        req(
            METHOD_SESSION_CREATE,
            json!({
                "mode": "chat",
                "command": [fake_acp_agent_bin()],
                "cwd": dir.path(),
                "title": "chat-test",
                "env": { "FAKE_ACP_SCRIPT": script },
            }),
        ),
    )
    .await;
    assert!(r.error.is_none(), "error creando chat: {:?}", r.error);
    let created: CreateResult = serde_json::from_value(r.result.unwrap()).unwrap();

    // La lista debe reportar mode: "chat".
    let list_res = dispatch(&mgr, req(METHOD_SESSION_LIST, json!({}))).await;
    let list: Vec<Value> = serde_json::from_value(list_res.result.unwrap()).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["mode"], "chat");

    let prompt_res = dispatch(
        &mgr,
        req(
            "chat.prompt",
            json!({ "id": created.id, "text": "saludame" }),
        ),
    )
    .await;
    assert!(
        prompt_res.error.is_none(),
        "error en chat.prompt: {:?}",
        prompt_res.error
    );

    let mut chunks = Vec::new();
    let mut saw_user = false;
    let mut saw_end = false;
    tokio::time::timeout(Duration::from_secs(5), async {
        while let Ok(notif) = events_rx.recv().await {
            if notif.method != "chat.event" {
                continue;
            }
            if notif.params["id"] != created.id {
                continue;
            }
            let ev = &notif.params["event"];
            match ev["type"].as_str() {
                Some("user_message") => {
                    assert_eq!(ev["text"], "saludame");
                    saw_user = true;
                }
                Some("agent_chunk") => {
                    chunks.push(ev["text"].as_str().unwrap().to_string());
                }
                Some("agent_message_end") => {
                    saw_end = true;
                    break;
                }
                _ => {}
            }
        }
    })
    .await
    .expect("timeout esperando eventos de chat");

    assert!(saw_user);
    assert!(saw_end);
    assert_eq!(chunks, vec!["Hola ", "desde ", "chat"]);

    wait_for_state(&mgr, &created.id, SessionState::Idle).await;
    dispatch(&mgr, req(METHOD_SESSION_KILL, json!({ "id": created.id }))).await;
}

#[tokio::test]
async fn permission_sets_waiting_and_resolves() {
    let dir = tempdir().unwrap();
    let notifier = Arc::new(FakeNotifier::default());
    let store = Store::open_in_memory().unwrap();
    let mgr = Manager::with_config(150, store, None, None, Config::default(), notifier.clone());
    let mut events_rx = mgr.subscribe_events();

    let script = json!([
        {
            "type": "permission",
            "id": "perm-1",
            "title": "Ejecutar cargo test",
            "name": "bash",
            "detail": "cargo test",
            "after_allow": "Permiso aprobado"
        }
    ])
    .to_string();

    let created: CreateResult = serde_json::from_value(
        dispatch(
            &mgr,
            req(
                METHOD_SESSION_CREATE,
                json!({
                    "mode": "chat",
                    "command": [fake_acp_agent_bin()],
                    "cwd": dir.path(),
                    "title": "claude · chat",
                    "env": { "FAKE_ACP_SCRIPT": script },
                }),
            ),
        )
        .await
        .result
        .unwrap(),
    )
    .unwrap();

    let prompt_res = dispatch(
        &mgr,
        req(
            "chat.prompt",
            json!({ "id": created.id, "text": "corré tests" }),
        ),
    )
    .await;
    assert!(prompt_res.error.is_none(), "{:?}", prompt_res.error);

    let request_id = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let notif = events_rx.recv().await.unwrap();
            if notif.method == "chat.event"
                && notif.params["id"] == created.id
                && notif.params["event"]["type"] == "permission_request"
            {
                assert_eq!(notif.params["event"]["title"], "Ejecutar cargo test");
                assert_eq!(notif.params["event"]["detail"], "cargo test");
                return notif.params["event"]["request_id"]
                    .as_str()
                    .unwrap()
                    .to_string();
            }
        }
    })
    .await
    .expect("timeout esperando permission_request");

    wait_for_state(&mgr, &created.id, SessionState::Waiting).await;

    // Debe disparar el notificador de espera de la etapa 3.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while notifier.notices().is_empty() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "no llegó aviso de escritorio para permiso pendiente"
        );
        tokio::time::sleep(Duration::from_millis(15)).await;
    }
    let notices = notifier.notices();
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0].session_id, created.id);
    assert!(
        notices[0].body.contains("Ejecutar cargo test"),
        "body inesperado: {:?}",
        notices[0].body
    );

    let perm_res = dispatch(
        &mgr,
        req(
            "chat.permission",
            json!({
                "id": created.id,
                "request_id": request_id,
                "choice": "allow",
            }),
        ),
    )
    .await;
    assert!(perm_res.error.is_none(), "{:?}", perm_res.error);

    let mut saw_resolved = false;
    let mut chunks = Vec::new();
    tokio::time::timeout(Duration::from_secs(5), async {
        while let Ok(notif) = events_rx.recv().await {
            if notif.method != "chat.event" || notif.params["id"] != created.id {
                continue;
            }
            let ev = &notif.params["event"];
            match ev["type"].as_str() {
                Some("permission_resolved") => {
                    assert_eq!(ev["request_id"], request_id);
                    assert_eq!(ev["choice"], "allow");
                    saw_resolved = true;
                }
                Some("agent_chunk") => {
                    chunks.push(ev["text"].as_str().unwrap().to_string());
                }
                Some("agent_message_end") => break,
                _ => {}
            }
        }
    })
    .await
    .expect("timeout esperando resolución de permiso");

    assert!(saw_resolved);
    assert_eq!(chunks, vec!["Permiso aprobado"]);
    wait_for_state(&mgr, &created.id, SessionState::Idle).await;

    dispatch(&mgr, req(METHOD_SESSION_KILL, json!({ "id": created.id }))).await;
}

#[tokio::test]
async fn history_survives_reconnect() {
    let dir = tempdir().unwrap();
    let sock = dir.path().join("pas.sock");
    let chats_dir = dir.path().join("chats");
    let store = Store::open_in_memory().unwrap();
    let mgr = Manager::with_config(
        150,
        store,
        Some(dir.path().join("accounts")),
        Some(sock.clone()),
        Config::default(),
        Arc::new(FakeNotifier::default()),
    );

    let listener = server::bind(&sock).unwrap();
    let server_task = tokio::spawn(server::serve(listener, Arc::clone(&mgr)));

    let script = json!([
        { "type": "tool", "id": "tc-1", "title": "Leyó lib.rs", "name": "read_file" },
        {
            "type": "diff",
            "id": "tc-2",
            "path": "src/lib.rs",
            "old_text": "fn a() {}\n",
            "new_text": "fn a() { 1 }\n"
        },
        {
            "type": "visual",
            "file": "mockup.html",
            "html": "<h1>Hola</h1>",
            "message": "Te dejo el visual"
        }
    ])
    .to_string();

    // Cliente 1 crea la sesión, manda un prompt y se desconecta.
    let session_id = {
        let mut c1 = Client::connect(&sock).await.unwrap();
        let _: Value = c1
            .call(METHOD_SESSION_SUBSCRIBE_ALL, json!({}))
            .await
            .unwrap();
        let created: CreateResult = c1
            .call(
                METHOD_SESSION_CREATE,
                json!({
                    "mode": "chat",
                    "command": [fake_acp_agent_bin()],
                    "cwd": dir.path(),
                    "title": "hist-test",
                    "env": { "FAKE_ACP_SCRIPT": script },
                }),
            )
            .await
            .unwrap();
        let _: Value = c1
            .call("chat.prompt", json!({ "id": created.id, "text": "hacelo" }))
            .await
            .unwrap();

        tokio::time::timeout(Duration::from_secs(5), async {
            while let Some(ev) = c1.next_event().await.unwrap() {
                if ev.method == "chat.event"
                    && ev.params["id"] == created.id
                    && ev.params["event"]["type"] == "agent_message_end"
                {
                    break;
                }
            }
        })
        .await
        .expect("timeout esperando fin de turno");

        created.id
    };

    // Cliente 2 reconecta desde cero y pide chat.history.
    let mut c2 = Client::connect(&sock).await.unwrap();
    let history: Vec<Value> = c2
        .call("chat.history", json!({ "id": session_id }))
        .await
        .unwrap();

    assert!(
        history
            .iter()
            .any(|e| e["type"] == "user_message" && e["text"] == "hacelo")
    );
    assert!(
        history
            .iter()
            .any(|e| e["type"] == "tool_call" && e["title"] == "Leyó lib.rs")
    );
    assert!(
        history
            .iter()
            .any(|e| e["type"] == "diff" && e["path"] == "src/lib.rs")
    );
    assert!(
        history
            .iter()
            .any(|e| e["type"] == "visual" && e["file"] == "mockup.html")
    );
    assert!(history.iter().any(|e| e["type"] == "agent_message_end"));

    // El historial debe persistirse como JSONL dentro de chats_dir.
    let jsonl_path = chats_dir.join(format!("{session_id}.jsonl"));
    let raw = std::fs::read_to_string(&jsonl_path)
        .unwrap_or_else(|e| panic!("no se encontró {}: {e}", jsonl_path.display()));
    let disk_events: Vec<Value> = raw
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(disk_events, history);

    let _: Value = c2
        .call(METHOD_SESSION_KILL, json!({ "id": session_id }))
        .await
        .unwrap();
    server_task.abort();
}

#[tokio::test]
async fn adapter_crash_is_exited_with_error() {
    let dir = tempdir().unwrap();
    let mgr = Manager::new(200);

    let script = json!([
        { "type": "crash", "code": 42, "message": "boom desde fake" }
    ])
    .to_string();

    let created: CreateResult = serde_json::from_value(
        dispatch(
            &mgr,
            req(
                METHOD_SESSION_CREATE,
                json!({
                    "mode": "chat",
                    "command": [fake_acp_agent_bin()],
                    "cwd": dir.path(),
                    "title": "crash-test",
                    "env": { "FAKE_ACP_SCRIPT": script },
                }),
            ),
        )
        .await
        .result
        .unwrap(),
    )
    .unwrap();

    let prompt_res = dispatch(
        &mgr,
        req("chat.prompt", json!({ "id": created.id, "text": "rompé" })),
    )
    .await;
    assert!(prompt_res.error.is_none(), "{:?}", prompt_res.error);

    let wait_res = dispatch(
        &mgr,
        req(
            METHOD_SESSION_WAIT,
            json!({ "id": created.id, "until": "exited", "timeout_ms": 5000 }),
        ),
    )
    .await;
    let state: SessionState = serde_json::from_value(wait_res.result.unwrap()).unwrap();
    assert_eq!(state, SessionState::Exited { code: Some(42) });

    let hist_res = dispatch(&mgr, req("chat.history", json!({ "id": created.id }))).await;
    let history: Vec<Value> = serde_json::from_value(hist_res.result.unwrap()).unwrap();
    assert!(
        history.iter().any(|e| {
            e["type"] == "error"
                && e["message"]
                    .as_str()
                    .is_some_and(|m| m.contains("boom desde fake"))
        }),
        "esperaba evento error con stderr del adaptador, historial: {history:?}"
    );
}

#[tokio::test]
async fn allow_session_skips_same_tool_next_time() {
    let dir = tempdir().unwrap();
    let mgr = Manager::new(200);
    let mut events_rx = mgr.subscribe_events();

    let script = json!([
        {
            "type": "permission",
            "id": "perm-1",
            "title": "Ejecutar cargo test",
            "name": "bash",
            "after_allow": "primero "
        },
        {
            "type": "permission",
            "id": "perm-2",
            "title": "Ejecutar cargo clippy",
            "name": "bash",
            "after_allow": "segundo"
        }
    ])
    .to_string();

    let created: CreateResult = serde_json::from_value(
        dispatch(
            &mgr,
            req(
                METHOD_SESSION_CREATE,
                json!({
                    "mode": "chat",
                    "command": [fake_acp_agent_bin()],
                    "cwd": dir.path(),
                    "title": "allow-session-test",
                    "env": { "FAKE_ACP_SCRIPT": script },
                }),
            ),
        )
        .await
        .result
        .unwrap(),
    )
    .unwrap();

    let prompt_res = dispatch(
        &mgr,
        req("chat.prompt", json!({ "id": created.id, "text": "corré" })),
    )
    .await;
    assert!(prompt_res.error.is_none(), "{:?}", prompt_res.error);

    let first_req_id = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let notif = events_rx.recv().await.unwrap();
            if notif.method == "chat.event"
                && notif.params["id"] == created.id
                && notif.params["event"]["type"] == "permission_request"
            {
                return notif.params["event"]["request_id"]
                    .as_str()
                    .unwrap()
                    .to_string();
            }
        }
    })
    .await
    .expect("timeout esperando primer permiso");

    let perm_res = dispatch(
        &mgr,
        req(
            "chat.permission",
            json!({
                "id": created.id,
                "request_id": first_req_id,
                "choice": "allow_session",
            }),
        ),
    )
    .await;
    assert!(perm_res.error.is_none(), "{:?}", perm_res.error);

    let mut extra_perm_requests = 0usize;
    let mut chunks = Vec::new();
    tokio::time::timeout(Duration::from_secs(5), async {
        while let Ok(notif) = events_rx.recv().await {
            if notif.method != "chat.event" || notif.params["id"] != created.id {
                continue;
            }
            let ev = &notif.params["event"];
            match ev["type"].as_str() {
                Some("permission_request") => extra_perm_requests += 1,
                Some("agent_chunk") => chunks.push(ev["text"].as_str().unwrap().to_string()),
                Some("agent_message_end") => break,
                _ => {}
            }
        }
    })
    .await
    .expect("timeout esperando fin de turno tras allow_session");

    assert_eq!(
        extra_perm_requests, 0,
        "el segundo permiso de la misma herramienta no debería volver a pedir confirmación"
    );
    assert_eq!(chunks, vec!["primero ", "segundo"]);
    wait_for_state(&mgr, &created.id, SessionState::Idle).await;

    dispatch(&mgr, req(METHOD_SESSION_KILL, json!({ "id": created.id }))).await;
}

#[tokio::test]
async fn chat_cancel_interrupts_sleep() {
    let dir = tempdir().unwrap();
    let mgr = Manager::new(200);
    let mut events_rx = mgr.subscribe_events();

    let script = json!([
        { "type": "sleep", "ms": 10000 }
    ])
    .to_string();

    let created: CreateResult = serde_json::from_value(
        dispatch(
            &mgr,
            req(
                METHOD_SESSION_CREATE,
                json!({
                    "mode": "chat",
                    "command": [fake_acp_agent_bin()],
                    "cwd": dir.path(),
                    "title": "cancel-test",
                    "env": { "FAKE_ACP_SCRIPT": script },
                }),
            ),
        )
        .await
        .result
        .unwrap(),
    )
    .unwrap();

    let started = std::time::Instant::now();
    let prompt_res = dispatch(
        &mgr,
        req("chat.prompt", json!({ "id": created.id, "text": "dormí" })),
    )
    .await;
    assert!(prompt_res.error.is_none(), "{:?}", prompt_res.error);

    tokio::time::sleep(Duration::from_millis(80)).await;
    let cancel_res = dispatch(&mgr, req("chat.cancel", json!({ "id": created.id }))).await;
    assert!(cancel_res.error.is_none(), "{:?}", cancel_res.error);

    tokio::time::timeout(Duration::from_secs(3), async {
        while let Ok(notif) = events_rx.recv().await {
            if notif.method == "chat.event"
                && notif.params["id"] == created.id
                && notif.params["event"]["type"] == "agent_message_end"
            {
                break;
            }
        }
    })
    .await
    .expect("timeout esperando fin de turno cancelado");

    assert!(
        started.elapsed() < Duration::from_secs(3),
        "la cancelación tardó demasiado: {:?}",
        started.elapsed()
    );
    wait_for_state(&mgr, &created.id, SessionState::Idle).await;

    dispatch(&mgr, req(METHOD_SESSION_KILL, json!({ "id": created.id }))).await;
}

#[tokio::test]
async fn wrong_mode_returns_voseo_error_and_adapter_command_resolves() {
    use pas_core::protocol::{AgentKind, ERR_WRONG_SESSION_MODE, METHOD_SESSION_SEND};
    use pasd::chat::resolve_adapter_command;

    assert_eq!(
        resolve_adapter_command(&["claude".into()], Some(AgentKind::Claude)),
        vec!["claude-agent-acp"]
    );
    assert_eq!(
        resolve_adapter_command(&["codex".into()], Some(AgentKind::Codex)),
        vec!["codex-acp"]
    );
    assert_eq!(
        resolve_adapter_command(&["agy".into()], Some(AgentKind::Agy)),
        vec!["agy", "acp"]
    );
    assert_eq!(
        resolve_adapter_command(&[], Some(AgentKind::Codex)),
        vec!["codex-acp"]
    );
    assert_eq!(
        resolve_adapter_command(&["/tmp/fake-acp-agent".into()], None),
        vec!["/tmp/fake-acp-agent"]
    );

    let dir = tempdir().unwrap();
    let mgr = Manager::new(200);
    let created: CreateResult = serde_json::from_value(
        dispatch(
            &mgr,
            req(
                METHOD_SESSION_CREATE,
                json!({
                    "mode": "chat",
                    "command": [fake_acp_agent_bin()],
                    "cwd": dir.path(),
                    "title": "mode-check",
                }),
            ),
        )
        .await
        .result
        .unwrap(),
    )
    .unwrap();

    let send_err = dispatch(
        &mgr,
        req(
            METHOD_SESSION_SEND,
            json!({ "id": created.id, "text": "hola" }),
        ),
    )
    .await
    .error
    .expect("session.send debe fallar en modo chat");
    assert_eq!(send_err.code, ERR_WRONG_SESSION_MODE);
    assert!(send_err.message.contains("Usá"));

    dispatch(&mgr, req(METHOD_SESSION_KILL, json!({ "id": created.id }))).await;
}

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

#[tokio::test]
async fn status_reports_missing_adapter() {
    use pasd::acp::AcpEnv;
    use std::ffi::OsString;

    let dir = tempdir().unwrap();
    let store = Store::open_in_memory().unwrap();
    let mgr = Manager::with_acp_env(
        200,
        store,
        Some(dir.path().join("accounts")),
        None,
        Config::default(),
        Arc::new(FakeNotifier::default()),
        AcpEnv {
            data_dir: Some(dir.path().to_path_buf()),
            path_env: Some(OsString::new()),
            npm_bin: None,
        },
    );

    let res = dispatch(&mgr, req("acp.status", json!({}))).await;
    assert!(res.error.is_none(), "error en acp.status: {:?}", res.error);
    let statuses: Vec<Value> = serde_json::from_value(res.result.unwrap()).unwrap();
    assert_eq!(statuses.len(), 3);

    let claude = statuses
        .iter()
        .find(|s| s["agent"] == "claude")
        .expect("falta claude en acp.status");
    assert_eq!(claude["bin"], "claude-agent-acp");
    assert_eq!(claude["installed"], false);
    assert!(claude["path"].is_null());
    assert_eq!(claude["package"], "@agentclientprotocol/claude-agent-acp");

    let codex = statuses
        .iter()
        .find(|s| s["agent"] == "codex")
        .expect("falta codex en acp.status");
    assert_eq!(codex["bin"], "codex-acp");
    assert_eq!(codex["installed"], false);
    assert!(codex["path"].is_null());
    assert_eq!(codex["package"], "@agentclientprotocol/codex-acp");

    let agy = statuses
        .iter()
        .find(|s| s["agent"] == "agy")
        .expect("falta agy en acp.status");
    assert_eq!(agy["bin"], "agy");
    assert_eq!(agy["installed"], false);
    assert!(agy["path"].is_null());
    assert!(agy["package"].is_null());

    // Al crear el binario en <data_dir>/acp/node_modules/.bin/codex-acp, debe detectarlo.
    let bin_dir = dir.path().join("acp").join("node_modules").join(".bin");
    std::fs::create_dir_all(&bin_dir).unwrap();
    let fake_codex = bin_dir.join("codex-acp");
    std::fs::write(&fake_codex, "#!/bin/sh\nexit 0\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&fake_codex, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    let res2 = dispatch(&mgr, req("acp.status", json!({}))).await;
    assert!(res2.error.is_none(), "{:?}", res2.error);
    let statuses2: Vec<Value> = serde_json::from_value(res2.result.unwrap()).unwrap();
    let codex2 = statuses2.iter().find(|s| s["agent"] == "codex").unwrap();
    assert_eq!(codex2["installed"], true);
    assert_eq!(codex2["path"], fake_codex.to_string_lossy().as_ref());
}

#[tokio::test]
async fn install_uses_npm_prefix() {
    use pasd::acp::AcpEnv;
    use std::ffi::OsString;

    let dir = tempdir().unwrap();
    let log_file = dir.path().join("npm-args.txt");
    let fake_npm = dir.path().join("fake-npm");
    let expected_prefix = dir.path().join("acp");

    // Script falso de npm que registra los argumentos y crea el binario en node_modules/.bin.
    let script = format!(
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"{}\"\nmkdir -p \"{}/node_modules/.bin\"\nprintf '#!/bin/sh\\nexit 0\\n' > \"{}/node_modules/.bin/codex-acp\"\nchmod +x \"{}/node_modules/.bin/codex-acp\"\n",
        log_file.display(),
        expected_prefix.display(),
        expected_prefix.display(),
        expected_prefix.display(),
    );
    std::fs::write(&fake_npm, script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&fake_npm, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    let store = Store::open_in_memory().unwrap();
    let mgr = Manager::with_acp_env(
        200,
        store,
        Some(dir.path().join("accounts")),
        None,
        Config::default(),
        Arc::new(FakeNotifier::default()),
        AcpEnv {
            data_dir: Some(dir.path().to_path_buf()),
            path_env: Some(OsString::new()),
            npm_bin: Some(fake_npm.into_os_string()),
        },
    );

    // agy debe rechazar con "agy ya trae su adaptador".
    let agy_err = dispatch(&mgr, req("acp.install", json!({ "agent": "agy" })))
        .await
        .error
        .expect("acp.install para agy debe devolver error");
    assert!(
        agy_err.message.contains("agy ya trae su adaptador"),
        "mensaje inesperado para agy: {}",
        agy_err.message
    );

    let res = dispatch(&mgr, req("acp.install", json!({ "agent": "codex" }))).await;
    assert!(res.error.is_none(), "error en acp.install: {:?}", res.error);
    let status: Value = res.result.unwrap();
    assert_eq!(status["agent"], "codex");
    assert_eq!(status["installed"], true);

    let recorded = std::fs::read_to_string(&log_file).unwrap();
    let args: Vec<&str> = recorded.lines().collect();
    assert_eq!(
        args,
        vec![
            "install",
            "--prefix",
            expected_prefix.to_string_lossy().as_ref(),
            "@agentclientprotocol/codex-acp",
        ]
    );

    // Si npm falla, debe devolver error claro con stderr resumido y qué hacer.
    let failing_npm = dir.path().join("failing-npm");
    std::fs::write(
        &failing_npm,
        "#!/bin/sh\necho 'EAI_AGAIN registry.npmjs.org' >&2\nexit 1\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&failing_npm, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let mgr_fail = Manager::with_acp_env(
        200,
        Store::open_in_memory().unwrap(),
        Some(dir.path().join("accounts")),
        None,
        Config::default(),
        Arc::new(FakeNotifier::default()),
        AcpEnv {
            data_dir: Some(dir.path().to_path_buf()),
            path_env: Some(OsString::new()),
            npm_bin: Some(failing_npm.into_os_string()),
        },
    );
    let fail_err = dispatch(&mgr_fail, req("acp.install", json!({ "agent": "claude" })))
        .await
        .error
        .expect("acp.install debe fallar si npm devuelve error");
    assert!(
        fail_err.message.contains("EAI_AGAIN registry.npmjs.org"),
        "faltó stderr resumido: {}",
        fail_err.message
    );
    assert!(
        fail_err.message.contains("npm install --prefix"),
        "faltó sugerencia de qué hacer: {}",
        fail_err.message
    );
}

#[tokio::test]
async fn terminal_command_for_claude_uses_resume() {
    use pasd::acp::AcpEnv;
    use std::ffi::OsString;

    let dir = tempdir().unwrap();
    let bin_dir = dir.path().join("acp").join("node_modules").join(".bin");
    std::fs::create_dir_all(&bin_dir).unwrap();

    // Adaptadores falsos para claude y codex apuntando a fake-acp-agent.
    let fake_bin = fake_acp_agent_bin();
    for name in ["claude-agent-acp", "codex-acp"] {
        let target = bin_dir.join(name);
        std::fs::write(&target, format!("#!/bin/sh\nexec \"{fake_bin}\" \"$@\"\n")).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    let store = Store::open_in_memory().unwrap();
    let mgr = Manager::with_acp_env(
        200,
        store,
        Some(dir.path().join("accounts")),
        None,
        Config::default(),
        Arc::new(FakeNotifier::default()),
        AcpEnv {
            data_dir: Some(dir.path().to_path_buf()),
            path_env: Some(OsString::new()),
            npm_bin: None,
        },
    );

    let claude_created: CreateResult = serde_json::from_value(
        dispatch(
            &mgr,
            req(
                METHOD_SESSION_CREATE,
                json!({
                    "mode": "chat",
                    "command": ["claude"],
                    "cwd": dir.path(),
                    "title": "claude-chat",
                }),
            ),
        )
        .await
        .result
        .unwrap(),
    )
    .unwrap();
    wait_for_state(&mgr, &claude_created.id, SessionState::Idle).await;

    let cmd_res = dispatch(
        &mgr,
        req("chat.terminal_command", json!({ "id": claude_created.id })),
    )
    .await;
    assert!(
        cmd_res.error.is_none(),
        "error en chat.terminal_command: {:?}",
        cmd_res.error
    );
    let cmd: Option<Vec<String>> = serde_json::from_value(cmd_res.result.unwrap()).unwrap();
    let cmd = cmd.expect("Claude debe devolver comando para abrir en terminal");
    assert_eq!(cmd.len(), 3);
    assert_eq!(cmd[0], "claude");
    assert_eq!(cmd[1], "--resume");
    assert_eq!(cmd[2], "fake-session-1");

    let codex_created: CreateResult = serde_json::from_value(
        dispatch(
            &mgr,
            req(
                METHOD_SESSION_CREATE,
                json!({
                    "mode": "chat",
                    "command": ["codex"],
                    "cwd": dir.path(),
                    "title": "codex-chat",
                }),
            ),
        )
        .await
        .result
        .unwrap(),
    )
    .unwrap();
    wait_for_state(&mgr, &codex_created.id, SessionState::Idle).await;

    let codex_cmd_res = dispatch(
        &mgr,
        req("chat.terminal_command", json!({ "id": codex_created.id })),
    )
    .await;
    assert!(codex_cmd_res.error.is_none(), "{:?}", codex_cmd_res.error);
    let codex_cmd: Option<Vec<String>> =
        serde_json::from_value(codex_cmd_res.result.unwrap()).unwrap();
    assert_eq!(codex_cmd, None);

    dispatch(
        &mgr,
        req(METHOD_SESSION_KILL, json!({ "id": claude_created.id })),
    )
    .await;
    dispatch(
        &mgr,
        req(METHOD_SESSION_KILL, json!({ "id": codex_created.id })),
    )
    .await;
}

#[tokio::test]
async fn chat_create_without_adapter_gives_clear_error() {
    use pasd::acp::AcpEnv;
    use std::ffi::OsString;

    let dir = tempdir().unwrap();
    let store = Store::open_in_memory().unwrap();
    let mgr = Manager::with_acp_env(
        200,
        store,
        Some(dir.path().join("accounts")),
        None,
        Config::default(),
        Arc::new(FakeNotifier::default()),
        AcpEnv {
            data_dir: Some(dir.path().to_path_buf()),
            path_env: Some(OsString::new()),
            npm_bin: None,
        },
    );

    let res = dispatch(
        &mgr,
        req(
            METHOD_SESSION_CREATE,
            json!({
                "mode": "chat",
                "command": ["codex"],
                "cwd": dir.path(),
                "title": "codex-missing",
            }),
        ),
    )
    .await;
    let err = res
        .error
        .expect("session.create en modo chat sin adaptador debe fallar");
    assert_eq!(
        err.message,
        "El chat de Codex necesita el adaptador codex-acp. Instalalo desde Cuentas o abrí la sesión en modo Terminal."
    );
}

#[tokio::test]
async fn permission_tool_update_uses_permission_title_and_visual_follows_text() {
    let dir = tempdir().unwrap();
    let store = Store::open_in_memory().unwrap();
    let mgr = Manager::with_config(
        150,
        store,
        Some(dir.path().join("accounts")),
        None,
        Config::default(),
        Arc::new(FakeNotifier::default()),
    );
    let mut events_rx = mgr.subscribe_events();

    let script = json!([
        { "type": "text", "chunks": ["Busco ", "el archivo."] },
        { "type": "tool", "title": "Leyó src/export.ts", "status": "completed" },
        {
            "type": "permission",
            "title": "Claude quiere ejecutar un comando",
            "detail": "npm test",
            "after_allow": "Permiso aprobado, aplico el cambio."
        },
        {
            "type": "diff",
            "path": "src/export.ts",
            "old_text": "export const v = 1;",
            "new_text": "export const v = 2;"
        },
        {
            "type": "visual",
            "file": "mockup-exportar.html",
            "html": "<html><body><h1>Vista previa</h1></body></html>",
            "message": "Te dejo el visual:"
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
                    "title": "order-and-title-test",
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
        req("chat.prompt", json!({ "id": created.id, "text": "hacelo" })),
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
                return notif.params["event"]["request_id"]
                    .as_str()
                    .unwrap()
                    .to_string();
            }
        }
    })
    .await
    .expect("timeout esperando permission_request");

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

    tokio::time::timeout(Duration::from_secs(5), async {
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
    .expect("timeout esperando agent_message_end");

    let hist_res = dispatch(&mgr, req("chat.history", json!({ "id": created.id }))).await;
    let history: Vec<Value> = serde_json::from_value(hist_res.result.unwrap()).unwrap();

    // Ningún tool_call puede tener título vacío ni el id crudo.
    for ev in &history {
        if ev["type"] == "tool_call" {
            let title = ev["title"].as_str().unwrap_or("");
            assert!(
                !title.is_empty(),
                "tool_call sin título en historial: {ev:?}"
            );
            assert_ne!(
                title,
                ev["id"].as_str().unwrap_or(""),
                "tool_call usa id crudo como título: {ev:?}"
            );
        }
    }

    // La actualización de la herramienta del permiso debe heredar el título del permiso.
    assert!(
        history.iter().any(|e| {
            e["type"] == "tool_call"
                && e["id"] == request_id
                && e["title"] == "Claude quiere ejecutar un comando"
        }),
        "falta tool_call del permiso con su título en historial: {history:?}"
    );

    // El orden debe ser: permission_resolved < "Permiso aprobado..." < diff < "Te dejo el visual:" < visual.
    let idx_resolved = history
        .iter()
        .position(|e| e["type"] == "permission_resolved")
        .expect("falta permission_resolved");
    let idx_after_allow = history
        .iter()
        .position(|e| {
            e["type"] == "agent_chunk"
                && e["text"]
                    .as_str()
                    .is_some_and(|t| t.contains("Permiso aprobado"))
        })
        .expect("falta texto after_allow");
    let idx_diff = history
        .iter()
        .position(|e| e["type"] == "diff")
        .expect("falta diff");
    let idx_vis_text = history
        .iter()
        .position(|e| {
            e["type"] == "agent_chunk"
                && e["text"]
                    .as_str()
                    .is_some_and(|t| t.contains("Te dejo el visual:"))
        })
        .expect("falta texto del visual");
    let idx_visual = history
        .iter()
        .position(|e| e["type"] == "visual" && e["file"] == "mockup-exportar.html")
        .expect("falta evento visual");

    assert!(
        idx_resolved < idx_after_allow
            && idx_after_allow < idx_diff
            && idx_diff < idx_vis_text
            && idx_vis_text < idx_visual,
        "orden incorrecto en historial (resolved={idx_resolved}, after_allow={idx_after_allow}, diff={idx_diff}, vis_text={idx_vis_text}, visual={idx_visual}): {history:?}"
    );

    dispatch(&mgr, req(METHOD_SESSION_KILL, json!({ "id": created.id }))).await;
}

use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use pas_core::protocol::{
    AccountInfo, AgentKind, CreateResult, EVENT_SESSION_FOCUS_REQUEST, METHOD_ACCOUNT_ADD,
    METHOD_SESSION_CREATE, METHOD_SESSION_HOOK, METHOD_SESSION_KILL, METHOD_SESSION_SEND,
    METHOD_SESSION_WAIT, Request, SessionFocusRequestEvent, SessionState,
};
use pasd::config::Config;
use pasd::manager::Manager;
use pasd::notify::{Notifier, WaitingNotice};
use pasd::rpc::dispatch;
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

fn fake_agent_bin() -> String {
    static BIN: OnceLock<String> = OnceLock::new();
    BIN.get_or_init(|| {
        let status = Command::new("cargo")
            .args(["build", "-p", "fake-agent", "--quiet"])
            .current_dir(workspace_root())
            .status()
            .expect("no se pudo compilar fake-agent");
        assert!(status.success(), "falló cargo build -p fake-agent");
        target_debug_dir()
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

#[derive(Default)]
struct FakeNotifier {
    notices: Mutex<Vec<WaitingNotice>>,
    callbacks: Mutex<Vec<Box<dyn FnOnce() + Send>>>,
}

impl FakeNotifier {
    fn notices(&self) -> Vec<WaitingNotice> {
        self.notices.lock().unwrap().clone()
    }

    fn click_last_open(&self) {
        let cb = self.callbacks.lock().unwrap().pop();
        if let Some(f) = cb {
            f();
        }
    }
}

impl Notifier for FakeNotifier {
    fn notify_waiting(&self, notice: WaitingNotice, on_open: Box<dyn FnOnce() + Send>) {
        self.notices.lock().unwrap().push(notice);
        self.callbacks.lock().unwrap().push(on_open);
    }
}

async fn wait_for_notice_count(notifier: &FakeNotifier, expected: usize) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    loop {
        if notifier.notices().len() >= expected {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "esperaba al menos {expected} aviso(s), llegaron {}",
            notifier.notices().len()
        );
        tokio::time::sleep(Duration::from_millis(15)).await;
    }
}

#[tokio::test]
async fn one_notification_per_waiting_transition() {
    let dir = tempdir().unwrap();
    let notifier = Arc::new(FakeNotifier::default());
    let store = Store::open_in_memory().unwrap();
    let mgr = Manager::with_config(150, store, None, None, Config::default(), notifier.clone());

    let mut events_rx = mgr.subscribe_events();

    let created: CreateResult = serde_json::from_value(
        dispatch(
            &mgr,
            req(
                METHOD_SESSION_CREATE,
                json!({
                    "command": [fake_agent_bin()],
                    "cwd": dir.path(),
                    "title": "claude · pockt",
                }),
            ),
        )
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

    // Primera transición a waiting: debe disparar exactamente 1 notificación.
    dispatch(
        &mgr,
        req(
            METHOD_SESSION_HOOK,
            json!({ "id": created.id, "agent": "claude", "event": "PermissionRequest" }),
        ),
    )
    .await;

    wait_for_notice_count(&notifier, 1).await;

    // Hooks repetidos estando ya en waiting no deben duplicar el aviso.
    dispatch(
        &mgr,
        req(
            METHOD_SESSION_HOOK,
            json!({ "id": created.id, "agent": "claude", "event": "PermissionRequest" }),
        ),
    )
    .await;
    dispatch(
        &mgr,
        req(
            METHOD_SESSION_HOOK,
            json!({ "id": created.id, "agent": "claude", "event": "Notification" }),
        ),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(120)).await;

    let notices = notifier.notices();
    assert_eq!(notices.len(), 1, "debe haber 1 solo aviso por transición");
    assert_eq!(notices[0].session_id, created.id);
    assert_eq!(
        notices[0].summary,
        "claude · pockt está esperando tu respuesta"
    );
    assert_eq!(notices[0].action_label, "Abrir");

    // Apretar "Abrir" emite session.focus_request para enfocar la sesión en la GUI.
    notifier.click_last_open();
    let focus_ev = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let ev = events_rx.recv().await.unwrap();
            if ev.method == EVENT_SESSION_FOCUS_REQUEST {
                return serde_json::from_value::<SessionFocusRequestEvent>(ev.params).unwrap();
            }
        }
    })
    .await
    .expect("no llegó session.focus_request al tocar Abrir");
    assert_eq!(focus_ev.id, created.id);

    // Volver a running y luego pasar de nuevo a waiting: debe disparar un 2do aviso.
    dispatch(
        &mgr,
        req(
            METHOD_SESSION_HOOK,
            json!({ "id": created.id, "agent": "claude", "event": "PreToolUse" }),
        ),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(40)).await;
    dispatch(
        &mgr,
        req(
            METHOD_SESSION_HOOK,
            json!({ "id": created.id, "agent": "claude", "event": "PermissionRequest" }),
        ),
    )
    .await;

    wait_for_notice_count(&notifier, 2).await;
    assert_eq!(notifier.notices().len(), 2);

    dispatch(&mgr, req(METHOD_SESSION_KILL, json!({ "id": created.id }))).await;
}

#[tokio::test]
async fn agy_prompt_pattern_sets_waiting() {
    let dir = tempdir().unwrap();
    let accounts_dir = dir.path().join("accounts");
    let notifier = Arc::new(FakeNotifier::default());
    let store = Store::open_in_memory().unwrap();
    let mgr = Manager::with_config(
        120,
        store,
        Some(accounts_dir),
        None,
        Config::default(),
        notifier.clone(),
    );

    let acc: AccountInfo = serde_json::from_value(
        dispatch(
            &mgr,
            req(
                METHOD_ACCOUNT_ADD,
                json!({ "agent": "agy", "name": "Predeterminada" }),
            ),
        )
        .await
        .result
        .unwrap(),
    )
    .unwrap();

    let created: CreateResult = serde_json::from_value(
        dispatch(
            &mgr,
            req(
                METHOD_SESSION_CREATE,
                json!({
                    "command": [fake_agent_bin()],
                    "cwd": dir.path(),
                    "title": "agy · pockt",
                    "account_id": acc.id,
                }),
            ),
        )
        .await
        .result
        .unwrap(),
    )
    .unwrap();

    // El banner inicial termina en "> ", que no coincide con patrón de pregunta -> Idle.
    let state: SessionState = serde_json::from_value(
        dispatch(
            &mgr,
            req(
                METHOD_SESSION_WAIT,
                json!({ "id": created.id, "until": "idle", "timeout_ms": 5000 }),
            ),
        )
        .await
        .result
        .unwrap(),
    )
    .unwrap();
    assert_eq!(state, SessionState::Idle);
    assert!(notifier.notices().is_empty());

    // fake-agent imprime una pregunta y se queda esperando respuesta.
    dispatch(
        &mgr,
        req(
            METHOD_SESSION_SEND,
            json!({ "id": created.id, "text": "ask ¿Permitir Bash(flutter test)?" }),
        ),
    )
    .await;

    let session = mgr.get(&created.id).unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while session.state() != SessionState::Waiting {
        assert!(
            tokio::time::Instant::now() < deadline,
            "la sesión de agy no pasó a Waiting (quedó en {:?})",
            session.state()
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    wait_for_notice_count(&notifier, 1).await;
    let notices = notifier.notices();
    assert_eq!(notices.len(), 1);
    assert_eq!(
        notices[0].summary,
        "agy · pockt está esperando tu respuesta"
    );
    assert!(
        notices[0].body.contains("¿Permitir Bash(flutter test)?"),
        "body inesperado: {:?}",
        notices[0].body
    );

    // Al responder, fake-agent vuelve al prompt "> " y queda Idle sin otro aviso.
    dispatch(
        &mgr,
        req(
            METHOD_SESSION_SEND,
            json!({ "id": created.id, "text": "s" }),
        ),
    )
    .await;
    let after: SessionState = serde_json::from_value(
        dispatch(
            &mgr,
            req(
                METHOD_SESSION_WAIT,
                json!({ "id": created.id, "until": "idle", "timeout_ms": 5000 }),
            ),
        )
        .await
        .result
        .unwrap(),
    )
    .unwrap();
    assert_eq!(after, SessionState::Idle);
    assert_eq!(notifier.notices().len(), 1);

    dispatch(&mgr, req(METHOD_SESSION_KILL, json!({ "id": created.id }))).await;
}

#[tokio::test]
async fn notifications_disabled_in_config() {
    if std::env::var_os("PAS_CONFIG_TEST_CHILD").is_none() {
        let cfg_home = tempdir().unwrap();
        let pas_cfg_dir = cfg_home.path().join("pas");
        std::fs::create_dir_all(&pas_cfg_dir).unwrap();
        std::fs::write(
            pas_cfg_dir.join("config.toml"),
            "notifications = false\nquota_warn_percent = 65\n",
        )
        .unwrap();

        let status = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "notifications_disabled_in_config"])
            .env("PAS_CONFIG_TEST_CHILD", "1")
            .env("XDG_CONFIG_HOME", cfg_home.path())
            .status()
            .unwrap();
        assert!(status.success());
        return;
    }

    let config = Config::load_default().unwrap();
    assert!(!config.notifications);
    assert!((config.quota_warn_percent - 65.0).abs() < f32::EPSILON);

    let dir = tempdir().unwrap();
    let notifier = Arc::new(FakeNotifier::default());
    let store = Store::open_in_memory().unwrap();
    let mgr = Manager::with_config(150, store, None, None, config, notifier.clone());

    let created: CreateResult = serde_json::from_value(
        dispatch(
            &mgr,
            req(
                METHOD_SESSION_CREATE,
                json!({
                    "command": [fake_agent_bin()],
                    "cwd": dir.path(),
                    "title": "claude · silencio",
                }),
            ),
        )
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
            METHOD_SESSION_HOOK,
            json!({ "id": created.id, "agent": "claude", "event": "PermissionRequest" }),
        ),
    )
    .await;

    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(mgr.get(&created.id).unwrap().state(), SessionState::Waiting);
    assert!(
        notifier.notices().is_empty(),
        "con notifications = false no debe enviarse ningún aviso"
    );

    dispatch(&mgr, req(METHOD_SESSION_KILL, json!({ "id": created.id }))).await;
}

#[tokio::test]
async fn custom_waiting_pattern_and_invalid_toml() {
    let dir = tempdir().unwrap();
    let cfg_path = dir.path().join("config.toml");

    // Patrón personalizado para agy: coincide con "CONFIRMAR:" pero no con "?"
    std::fs::write(
        &cfg_path,
        r#"
[waiting_patterns]
agy = ["CONFIRMAR:$"]
"#,
    )
    .unwrap();
    let cfg = Config::load_from_path(&cfg_path).unwrap();
    let pats = cfg.compiled_waiting_patterns(AgentKind::Agy).unwrap();
    assert!(pats.iter().any(|r| r.is_match("CONFIRMAR:")));
    assert!(!pats.iter().any(|r| r.is_match("¿Continuar?")));

    // TOML inválido devuelve error en voseo.
    std::fs::write(&cfg_path, "notifications = [esto no cierra").unwrap();
    let err = Config::load_from_path(&cfg_path).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("Corregilo"),
        "se esperaba error en voseo, salió: {msg}"
    );
}

#[tokio::test]
async fn config_busy_patterns_default_custom_and_invalid() {
    let default_cfg = Config::default();
    let agy_pats = default_cfg.compiled_busy_patterns(AgentKind::Agy).unwrap();
    assert!(
        agy_pats
            .iter()
            .any(|r| r.is_match("esc to cancel · 1 task(s) · /tasks")),
        "default de agy debe matchear 'esc to cancel'"
    );

    // agy sigue esperando un comando en segundo plano aunque muestre "? for shortcuts".
    assert!(
        agy_pats
            .iter()
            .any(|r| r.is_match("? for shortcuts   auto · Gemini · medium · 1 task(s) · /tasks")),
        "default de agy debe matchear tareas en segundo plano"
    );
    assert!(
        !agy_pats
            .iter()
            .any(|r| r.is_match("? for shortcuts   auto · Gemini 4 Hydrogen · medium")),
        "sin tareas, la barra de agy no es 'ocupado'"
    );
    assert!(
        default_cfg
            .compiled_busy_patterns(AgentKind::Claude)
            .unwrap()
            .is_empty()
    );

    let dir = tempdir().unwrap();
    let cfg_path = dir.path().join("config.toml");
    std::fs::write(
        &cfg_path,
        r#"
[busy_patterns]
agy = ["TRABAJANDO"]
claude = ["Running tool"]
"#,
    )
    .unwrap();
    let cfg = Config::load_from_path(&cfg_path).unwrap();
    let agy_custom = cfg.compiled_busy_patterns(AgentKind::Agy).unwrap();
    assert!(agy_custom.iter().any(|r| r.is_match("TRABAJANDO...")));
    assert!(!agy_custom.iter().any(|r| r.is_match("esc to cancel")));
    let claude_custom = cfg.compiled_busy_patterns(AgentKind::Claude).unwrap();
    assert!(claude_custom.iter().any(|r| r.is_match("Running tool")));

    std::fs::write(
        &cfg_path,
        r#"
[busy_patterns]
agy = ["[invalido"]
"#,
    )
    .unwrap();
    assert!(Config::load_from_path(&cfg_path).is_err());
}

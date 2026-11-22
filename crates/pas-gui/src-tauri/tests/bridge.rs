use std::collections::BTreeMap;
use std::fs::{self, Permissions};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use pas_core::protocol::{
    CreateParams, OpenTarget, SessionCreatedEvent, SessionOutputEvent, SessionRemovedEvent,
    SessionState, SessionStateEvent,
};
use pas_gui_lib::bridge::{
    Bridge, EventSink, FRONTEND_EVENT_CONNECTION, FRONTEND_EVENT_CREATED, FRONTEND_EVENT_OUTPUT,
    FRONTEND_EVENT_REMOVED, FRONTEND_EVENT_STATE,
};
use pasd::acp::AcpEnv;
use pasd::config::Config;
use pasd::manager::Manager;
use pasd::notify::NoopNotifier;
use pasd::open::{LaunchRequest, Launcher, OpenEnv};
use pasd::server::{bind, serve};
use pasd::store::Store;
use serde_json::Value;
use tempfile::tempdir;
use tokio::sync::mpsc;

fn fake_agent_bin() -> String {
    static BIN: OnceLock<String> = OnceLock::new();
    BIN.get_or_init(|| {
        let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
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

struct TestSink {
    tx: mpsc::UnboundedSender<(String, Value)>,
}

impl EventSink for TestSink {
    fn emit(&self, event: &str, payload: Value) {
        let _ = self.tx.send((event.to_string(), payload));
    }
}

#[tokio::test]
async fn bridge_forwards_output_events() {
    let dir = tempdir().unwrap();
    let sock = dir.path().join("pas").join("pasd.sock");
    let listener = bind(&sock).unwrap();
    let mgr = Manager::new(200);
    let server = tokio::spawn(serve(listener, mgr.clone()));

    let (tx, mut rx) = mpsc::unbounded_channel();
    let bridge = Bridge::new(sock.clone(), Arc::new(TestSink { tx }));

    // Esperar conexión inicial del canal de eventos.
    let connected = tokio::time::timeout(Duration::from_secs(5), async {
        while let Some((event, payload)) = rx.recv().await {
            if event == FRONTEND_EVENT_CONNECTION && payload == Value::String("connected".into()) {
                return true;
            }
        }
        false
    })
    .await
    .expect("timeout esperando pas://connection connected");
    assert!(connected);

    let created = bridge
        .create_session(CreateParams {
            command: vec![fake_agent_bin()],
            cwd: std::env::temp_dir(),
            title: Some("bridge-test".into()),
            env: BTreeMap::new(),
            cols: 80,
            rows: 24,
            project_id: None,
            account_id: None,
            mode: pas_core::protocol::Mode::Terminal,
            worktree: None,
        })
        .await
        .unwrap();

    // Esperar created y estado Idle vía session.subscribe_all.
    let (mut got_created, mut got_idle) = (false, false);
    tokio::time::timeout(Duration::from_secs(5), async {
        while let Some((event, payload)) = rx.recv().await {
            if event == FRONTEND_EVENT_CREATED {
                let ev: SessionCreatedEvent = serde_json::from_value(payload).unwrap();
                if ev.info.id == created.id {
                    got_created = true;
                }
            } else if event == FRONTEND_EVENT_STATE {
                let ev: SessionStateEvent = serde_json::from_value(payload).unwrap();
                if ev.id == created.id && ev.state == SessionState::Idle {
                    got_idle = true;
                }
            }
            if got_created && got_idle {
                break;
            }
        }
    })
    .await
    .expect("timeout esperando pas://created y pas://state");
    assert!(got_created, "no llegó pas://created");
    assert!(got_idle, "no llegó pas://state Idle");

    bridge.subscribe(created.id.clone()).await.unwrap();

    bridge
        .session_input(created.id.clone(), STANDARD.encode("echo puente-ok\r"))
        .await
        .unwrap();

    let mut collected = String::new();
    let got_output = tokio::time::timeout(Duration::from_secs(5), async {
        while let Some((event, payload)) = rx.recv().await {
            if event == FRONTEND_EVENT_OUTPUT {
                let ev: SessionOutputEvent = serde_json::from_value(payload).unwrap();
                if ev.id == created.id {
                    let bytes = STANDARD.decode(&ev.data_b64).unwrap();
                    collected.push_str(&String::from_utf8_lossy(&bytes));
                    if collected.contains("puente-ok") {
                        return true;
                    }
                }
            }
        }
        false
    })
    .await
    .expect("timeout esperando pas://output");
    assert!(
        got_output,
        "no llegó 'puente-ok' en pas://output: {collected:?}"
    );

    bridge.unsubscribe(created.id.clone()).await.unwrap();
    bridge.kill_session(created.id.clone()).await.unwrap();

    let got_removed = tokio::time::timeout(Duration::from_secs(5), async {
        while let Some((event, payload)) = rx.recv().await {
            if event == FRONTEND_EVENT_REMOVED {
                let ev: SessionRemovedEvent = serde_json::from_value(payload).unwrap();
                if ev.id == created.id {
                    return true;
                }
            }
        }
        false
    })
    .await
    .expect("timeout esperando pas://removed");
    assert!(got_removed);

    bridge.shutdown();
    mgr.kill_all();
    server.abort();
}

#[tokio::test]
async fn bridge_projects_and_groups_roundtrip() {
    let dir = tempdir().unwrap();
    let sock = dir.path().join("pas").join("pasd.sock");
    let listener = bind(&sock).unwrap();
    let mgr = Manager::new(200);
    let server = tokio::spawn(serve(listener, mgr.clone()));

    let repo = dir.path().join("mi-repo");
    std::fs::create_dir_all(repo.join(".git")).unwrap();

    let (tx, _rx) = mpsc::unbounded_channel();
    let bridge = Bridge::new(sock.clone(), Arc::new(TestSink { tx }));

    let proj = bridge.add_project(repo.clone(), None).await.unwrap();
    assert_eq!(proj.name, "mi-repo");
    let orig_group_id = proj.group_id.clone();

    let renamed_proj = bridge
        .rename_project(proj.id.clone(), "renombrado".into())
        .await
        .unwrap();
    assert_eq!(renamed_proj.name, "renombrado");

    let custom_group = bridge.create_group("trabajo".into()).await.unwrap();
    assert_eq!(custom_group.name, "trabajo");

    let renamed_group = bridge
        .rename_group(custom_group.id.clone(), "clientes".into())
        .await
        .unwrap();
    assert_eq!(renamed_group.name, "clientes");

    let moved = bridge
        .move_project(proj.id.clone(), renamed_group.id.clone())
        .await
        .unwrap();
    assert_eq!(moved.group_id, renamed_group.id);
    assert!(moved.group_pinned);

    let unpinned = bridge.unpin_project(proj.id.clone()).await.unwrap();
    assert!(!unpinned.group_pinned);
    assert_ne!(unpinned.group_id, orig_group_id); // grupo original vacío se borró y se recreó uno

    let projects = bridge.list_projects().await.unwrap();
    assert_eq!(projects.len(), 1);
    let groups = bridge.list_groups().await.unwrap();
    assert!(!groups.is_empty());

    bridge.remove_project(proj.id).await.unwrap();
    assert!(bridge.list_projects().await.unwrap().is_empty());

    bridge.shutdown();
    mgr.kill_all();
    server.abort();
}

#[derive(Default)]
struct FakeLauncher {
    calls: Mutex<Vec<LaunchRequest>>,
}

impl Launcher for FakeLauncher {
    fn launch(&self, req: LaunchRequest) -> Result<(), std::io::Error> {
        self.calls.lock().unwrap().push(req);
        Ok(())
    }
}

#[tokio::test]
async fn bridge_open_and_discover_roundtrip() {
    let dir = tempdir().unwrap();
    let sock = dir.path().join("pas").join("pasd.sock");
    let fake_home = dir.path().join("home");
    let bin_dir = dir.path().join("bin");
    fs::create_dir_all(&bin_dir).unwrap();
    let code_bin = bin_dir.join("code");
    fs::write(&code_bin, "#!/bin/sh\nexit 0\n").unwrap();
    fs::set_permissions(&code_bin, Permissions::from_mode(0o755)).unwrap();

    let repo = fake_home.join("Proyectos").join("demo");
    fs::create_dir_all(repo.join(".git")).unwrap();
    let file = repo.join("main.rs");
    fs::write(&file, "fn main() {}\n").unwrap();
    fs::create_dir_all(fake_home.join(".claude")).unwrap();

    let launcher = Arc::new(FakeLauncher::default());
    let mgr = Manager::with_open_env(
        200,
        Store::open_in_memory().unwrap(),
        None,
        None,
        Config::default(),
        Arc::new(NoopNotifier),
        AcpEnv::default(),
        launcher.clone(),
        OpenEnv {
            path_env: Some(bin_dir.into_os_string()),
            home_dir: Some(fake_home.clone()),
        },
    );
    let listener = bind(&sock).unwrap();
    let server = tokio::spawn(serve(listener, mgr.clone()));

    let (tx, _rx) = mpsc::unbounded_channel();
    let bridge = Bridge::new(sock.clone(), Arc::new(TestSink { tx }));

    let projects = bridge.discover_projects().await.unwrap();
    assert_eq!(projects.len(), 1);
    assert_eq!(projects[0].name, "demo");

    let accounts = bridge.discover_accounts().await.unwrap();
    assert_eq!(accounts.len(), 1);
    assert_eq!(accounts[0].suggested_name, "Claude");

    bridge.add_project(repo.clone(), None).await.unwrap();

    let exists = bridge
        .open_exists(repo.clone(), "main.rs".into())
        .await
        .unwrap();
    assert!(exists.exists);

    bridge
        .open_path(file.clone(), Some(3), OpenTarget::Editor)
        .await
        .unwrap();
    let calls = launcher.calls.lock().unwrap().clone();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].program, code_bin);

    bridge.shutdown();
    mgr.kill_all();
    server.abort();
}

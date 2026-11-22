use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use pas_core::client::Client;
use pas_core::protocol::{
    CreateResult, METHOD_DAEMON_PING, METHOD_SESSION_CREATE, METHOD_SESSION_KILL,
    METHOD_SESSION_READ, METHOD_SESSION_SEND, METHOD_SESSION_WAIT, PingResult, ReadParams,
    ReadResult, SendParams,
};
use pasd::manager::Manager;
use pasd::server::{ServerError, bind, serve};
use serde_json::json;
use tempfile::tempdir;
use tokio::io::AsyncWriteExt;
use tokio::net::UnixStream;

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

#[tokio::test]
async fn stale_socket_recovered() {
    let dir = tempdir().unwrap();
    let sock = dir.path().join("pas").join("pasd.sock");
    fs::create_dir_all(sock.parent().unwrap()).unwrap();
    fs::write(&sock, b"huerfano").unwrap();

    let listener = bind(&sock).unwrap();
    drop(listener);
}

#[tokio::test]
async fn second_instance_refused() {
    let dir = tempdir().unwrap();
    let sock = dir.path().join("pas").join("pasd.sock");
    let listener = bind(&sock).unwrap();
    let mgr = Manager::new(200);
    let task = tokio::spawn(serve(listener, mgr));

    let err = bind(&sock).unwrap_err();
    assert!(matches!(err, ServerError::AlreadyRunning { .. }));
    assert!(err.to_string().contains(&sock.display().to_string()));

    task.abort();
}

#[tokio::test]
async fn socket_permissions() {
    let dir = tempdir().unwrap();
    let parent = dir.path().join("pas");
    let sock = parent.join("pasd.sock");

    let listener = bind(&sock).unwrap();

    let dir_mode = fs::metadata(&parent).unwrap().permissions().mode() & 0o777;
    let sock_mode = fs::metadata(&sock).unwrap().permissions().mode() & 0o777;
    assert_eq!(dir_mode, 0o700);
    assert_eq!(sock_mode, 0o600);

    drop(listener);
}

#[tokio::test]
async fn existing_parent_untouched() {
    let dir = tempdir().unwrap();
    let parent = dir.path().join("existing");
    fs::create_dir(&parent).unwrap();
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o755)).unwrap();
    let sock = parent.join("pasd.sock");

    let listener = bind(&sock).unwrap();

    let dir_mode = fs::metadata(&parent).unwrap().permissions().mode() & 0o777;
    assert_eq!(dir_mode, 0o755);

    drop(listener);
}

#[tokio::test]
async fn long_socket_path_rejected_by_bind() {
    let long_path = PathBuf::from(format!("/tmp/{}/pasd.sock", "a".repeat(110)));
    let err = bind(&long_path).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("máximo 107"), "mensaje: {msg}");
    assert!(msg.contains("PAS_SOCKET"), "mensaje: {msg}");
}

#[tokio::test]
async fn concurrent_clients() {
    let dir = tempdir().unwrap();
    let sock = dir.path().join("pas").join("pasd.sock");
    let listener = bind(&sock).unwrap();
    let mgr = Manager::new(200);
    let task = tokio::spawn(serve(listener, mgr.clone()));

    let mut setup = Client::connect(&sock).await.unwrap();
    let created: CreateResult = setup
        .call(
            METHOD_SESSION_CREATE,
            json!({
                "command": [fake_agent_bin()],
                "cwd": std::env::temp_dir(),
                "title": "shared",
            }),
        )
        .await
        .unwrap();
    let _: serde_json::Value = setup
        .call(
            METHOD_SESSION_WAIT,
            json!({ "id": created.id, "until": "idle", "timeout_ms": 5000 }),
        )
        .await
        .unwrap();

    let t0 = Instant::now();
    let mut handles = Vec::new();
    for client_idx in 0..8usize {
        let sock = sock.clone();
        let id = created.id.clone();
        handles.push(tokio::spawn(async move {
            let mut client = Client::connect(&sock).await.unwrap();
            for iter in 0..20usize {
                let _: serde_json::Value = client
                    .call(
                        METHOD_SESSION_SEND,
                        SendParams {
                            id: id.clone(),
                            text: format!("echo c{client_idx}-{iter}"),
                            enter: true,
                        },
                    )
                    .await
                    .unwrap();
                let _: ReadResult = client
                    .call(
                        METHOD_SESSION_READ,
                        ReadParams {
                            id: id.clone(),
                            lines: Some(5),
                        },
                    )
                    .await
                    .unwrap();
            }
        }));
    }

    for h in handles {
        h.await.unwrap();
    }
    assert!(
        t0.elapsed() < Duration::from_secs(10),
        "tardó {:?}",
        t0.elapsed()
    );

    let _: serde_json::Value = setup
        .call(METHOD_SESSION_KILL, json!({ "id": created.id }))
        .await
        .unwrap();
    mgr.kill_all();
    task.abort();
}

#[tokio::test]
async fn client_drop_mid_request() {
    let dir = tempdir().unwrap();
    let sock = dir.path().join("pas").join("pasd.sock");
    let listener = bind(&sock).unwrap();
    let mgr = Manager::new(200);
    let task = tokio::spawn(serve(listener, mgr));

    {
        let mut raw = UnixStream::connect(&sock).await.unwrap();
        raw.write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"meth")
            .await
            .unwrap();
        raw.flush().await.unwrap();
    }

    let mut client = Client::connect(&sock).await.unwrap();
    let ping: PingResult = client.call(METHOD_DAEMON_PING, json!({})).await.unwrap();
    assert!(!ping.version.is_empty());

    task.abort();
}

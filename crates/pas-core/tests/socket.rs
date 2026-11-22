use std::path::PathBuf;
use std::process::Command;

use pas_core::CoreError;
use pas_core::client::Client;
use pas_core::protocol::{
    ERR_SESSION_NOT_FOUND, METHOD_DAEMON_PING, METHOD_SESSION_READ, PingResult, ReadParams,
    Request, Response, RpcError,
};
use pas_core::socket::{read_msg, socket_path, write_msg};
use serde_json::json;
use tempfile::tempdir;
use tokio::io::{AsyncWriteExt, BufReader};
use tokio::net::UnixListener;

#[test]
fn pas_socket_env_wins() {
    if std::env::var_os("PAS_TEST_CHILD").is_some() {
        assert_eq!(socket_path().unwrap(), PathBuf::from("/tmp/x.sock"));
        return;
    }

    let status = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "pas_socket_env_wins"])
        .env("PAS_TEST_CHILD", "1")
        .env("PAS_SOCKET", "/tmp/x.sock")
        .env("XDG_RUNTIME_DIR", "/run/user/1000")
        .status()
        .unwrap();
    assert!(status.success());
}

#[test]
fn xdg_runtime_dir_fallback_and_missing_error() {
    if let Ok(mode) = std::env::var("PAS_TEST_CHILD") {
        match mode.as_str() {
            "xdg" => assert_eq!(
                socket_path().unwrap(),
                PathBuf::from("/run/user/1000/pas/pasd.sock")
            ),
            "none" => {
                let err = socket_path().unwrap_err();
                assert!(matches!(err, CoreError::NoRuntimeDir));
                assert_eq!(
                    err.to_string(),
                    "No encontré XDG_RUNTIME_DIR. Definí PAS_SOCKET con la ruta del socket de pasd."
                );
            }
            _ => unreachable!(),
        }
        return;
    }

    let exe = std::env::current_exe().unwrap();
    let xdg = Command::new(&exe)
        .args(["--exact", "xdg_runtime_dir_fallback_and_missing_error"])
        .env("PAS_TEST_CHILD", "xdg")
        .env_remove("PAS_SOCKET")
        .env("XDG_RUNTIME_DIR", "/run/user/1000")
        .status()
        .unwrap();
    assert!(xdg.success());

    let none = Command::new(&exe)
        .args(["--exact", "xdg_runtime_dir_fallback_and_missing_error"])
        .env("PAS_TEST_CHILD", "none")
        .env_remove("PAS_SOCKET")
        .env_remove("XDG_RUNTIME_DIR")
        .status()
        .unwrap();
    assert!(none.success());
}

#[tokio::test]
async fn framing_roundtrip() {
    let (mut writer, reader) = tokio::io::duplex(4096);
    let mut reader = BufReader::new(reader);

    let req = Request {
        jsonrpc: "2.0".into(),
        id: 42,
        method: METHOD_DAEMON_PING.into(),
        params: json!({}),
    };

    write_msg(&mut writer, &req).await.unwrap();
    writer.shutdown().await.unwrap();

    let got: Option<Request> = read_msg(&mut reader).await.unwrap();
    assert_eq!(got, Some(req));

    let eof: Option<Request> = read_msg(&mut reader).await.unwrap();
    assert_eq!(eof, None);
}

#[tokio::test]
async fn client_maps_rpc_error() {
    let dir = tempdir().unwrap();
    let sock = dir.path().join("pasd.sock");
    let listener = UnixListener::bind(&sock).unwrap();

    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut stream = BufReader::new(stream);
        let req: Request = read_msg(&mut stream).await.unwrap().unwrap();
        let resp = Response::err(
            req.id,
            RpcError {
                code: ERR_SESSION_NOT_FOUND,
                message: "No hay ninguna sesión llamada x.".into(),
            },
        );
        write_msg(&mut stream, &resp).await.unwrap();
    });

    let mut client = Client::connect(&sock).await.unwrap();
    let err = client
        .call::<_, serde_json::Value>(
            METHOD_SESSION_READ,
            ReadParams {
                id: "x".into(),
                lines: None,
            },
        )
        .await
        .unwrap_err();

    match err {
        CoreError::Rpc { code, message } => {
            assert_eq!(code, ERR_SESSION_NOT_FOUND);
            assert_eq!(message, "No hay ninguna sesión llamada x.");
        }
        other => panic!("esperaba CoreError::Rpc, llegó {other:?}"),
    }

    server.await.unwrap();
}

#[tokio::test]
async fn client_increments_ids_and_decodes_result() {
    let dir = tempdir().unwrap();
    let sock = dir.path().join("pasd.sock");
    let listener = UnixListener::bind(&sock).unwrap();

    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut stream = BufReader::new(stream);
        for expected_id in [1u64, 2] {
            let req: Request = read_msg(&mut stream).await.unwrap().unwrap();
            assert_eq!(req.id, expected_id);
            let resp = Response::ok(req.id, json!({ "version": "0.1.0" }));
            write_msg(&mut stream, &resp).await.unwrap();
        }
    });

    let mut client = Client::connect(&sock).await.unwrap();
    let r1: PingResult = client.call(METHOD_DAEMON_PING, json!({})).await.unwrap();
    let r2: PingResult = client.call(METHOD_DAEMON_PING, json!({})).await.unwrap();
    assert_eq!(r1.version, "0.1.0");
    assert_eq!(r2.version, "0.1.0");

    server.await.unwrap();
}

#[tokio::test]
async fn connect_missing_socket_returns_not_running() {
    let dir = tempdir().unwrap();
    let err = Client::connect(&dir.path().join("no-existe.sock"))
        .await
        .unwrap_err();
    assert!(matches!(err, CoreError::NotRunning));
    assert_eq!(
        err.to_string(),
        "pasd no está corriendo. Iniciálo con `pasd` o usá cualquier comando de `pas`, que lo arranca solo."
    );
}

#[tokio::test]
async fn long_socket_path_returns_clear_error() {
    let long_path = PathBuf::from(format!("/tmp/{}/pasd.sock", "a".repeat(110)));
    let err = Client::connect(&long_path).await.unwrap_err();
    assert!(matches!(err, CoreError::SocketPathTooLong { .. }));
    let msg = err.to_string();
    assert!(msg.contains("máximo 107"), "mensaje: {msg}");
    assert!(msg.contains("PAS_SOCKET"), "mensaje: {msg}");
}

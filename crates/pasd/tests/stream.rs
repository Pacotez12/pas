use std::num::NonZeroU16;
use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use pas_core::client::Client;
use pas_core::protocol::{
    CreateResult, EVENT_SESSION_CREATED, EVENT_SESSION_OUTPUT, EVENT_SESSION_REMOVED,
    EVENT_SESSION_STATE, IdParams, InputParams, METHOD_SESSION_CREATE, METHOD_SESSION_INPUT,
    METHOD_SESSION_KILL, METHOD_SESSION_READ, METHOD_SESSION_RESIZE, METHOD_SESSION_SEND,
    METHOD_SESSION_SNAPSHOT, METHOD_SESSION_SUBSCRIBE, METHOD_SESSION_SUBSCRIBE_ALL,
    METHOD_SESSION_UNSUBSCRIBE, METHOD_SESSION_WAIT, ReadParams, ReadResult, ResizeParams,
    SendParams, SessionCreatedEvent, SessionOutputEvent, SessionRemovedEvent, SessionState,
    SessionStateEvent, SnapshotResult, WaitParams, WaitUntil,
};
use pasd::manager::Manager;
use pasd::server::{bind, serve};
use serde_json::{Value, json};
use tempfile::tempdir;

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

fn read_all_parser_lines(parser: &mut vt100::Parser) -> String {
    let (rows, cols) = parser.screen().size();
    let rows = usize::from(rows.get());
    let cols = cols.get();

    parser.screen_mut().set_scrollback(usize::MAX);
    let total_sb = parser.screen().scrollback();

    let mut all = Vec::with_capacity(total_sb + rows);
    let mut offset = total_sb;
    while offset > 0 {
        parser.screen_mut().set_scrollback(offset);
        let take = offset.min(rows);
        all.extend(parser.screen().rows(0, cols).take(take));
        offset -= take;
    }
    parser.screen_mut().set_scrollback(0);
    all.extend(parser.screen().rows(0, cols));

    while all.last().is_some_and(|l| l.trim_end().is_empty()) {
        all.pop();
    }
    all.join("\n")
}

#[tokio::test]
async fn subscribe_receives_output() {
    let dir = tempdir().unwrap();
    let sock = dir.path().join("pas").join("pasd.sock");
    let listener = bind(&sock).unwrap();
    let mgr = Manager::new(200);
    let server = tokio::spawn(serve(listener, mgr.clone()));

    let mut client = Client::connect(&sock).await.unwrap();
    let created: CreateResult = client
        .call(
            METHOD_SESSION_CREATE,
            json!({
                "command": [fake_agent_bin()],
                "cwd": std::env::temp_dir(),
                "title": "sub-test",
            }),
        )
        .await
        .unwrap();

    let _: SessionState = client
        .call(
            METHOD_SESSION_WAIT,
            WaitParams {
                id: created.id.clone(),
                until: WaitUntil::Idle,
                timeout_ms: 5000,
            },
        )
        .await
        .unwrap();

    let _: Value = client
        .call(
            METHOD_SESSION_SUBSCRIBE,
            IdParams {
                id: created.id.clone(),
            },
        )
        .await
        .unwrap();

    let _: Value = client
        .call(
            METHOD_SESSION_SEND,
            SendParams {
                id: created.id.clone(),
                text: "echo x".into(),
                enter: true,
            },
        )
        .await
        .unwrap();

    let mut collected = String::new();
    let found = tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(ev) = client.next_event().await.unwrap() {
            if ev.method == EVENT_SESSION_OUTPUT {
                let payload: SessionOutputEvent = serde_json::from_value(ev.params).unwrap();
                assert_eq!(payload.id, created.id);
                let bytes = STANDARD.decode(&payload.data_b64).unwrap();
                collected.push_str(&String::from_utf8_lossy(&bytes));
                if collected.contains('x') {
                    return true;
                }
            }
        }
        false
    })
    .await
    .expect("timeout esperando notificación session.output");

    assert!(found, "no llegó 'x' en la salida: {collected:?}");

    let _: Value = client
        .call(
            METHOD_SESSION_UNSUBSCRIBE,
            IdParams {
                id: created.id.clone(),
            },
        )
        .await
        .unwrap();

    let _: Value = client
        .call(METHOD_SESSION_KILL, IdParams { id: created.id })
        .await
        .unwrap();
    mgr.kill_all();
    server.abort();
}

#[tokio::test]
async fn input_raw_ctrl_c() {
    let dir = tempdir().unwrap();
    let sock = dir.path().join("pas").join("pasd.sock");
    let listener = bind(&sock).unwrap();
    let mgr = Manager::new(200);
    let server = tokio::spawn(serve(listener, mgr.clone()));

    let mut client = Client::connect(&sock).await.unwrap();
    let created: CreateResult = client
        .call(
            METHOD_SESSION_CREATE,
            json!({
                "command": [fake_agent_bin()],
                "cwd": std::env::temp_dir(),
                "title": "ctrlc-test",
            }),
        )
        .await
        .unwrap();

    let _: SessionState = client
        .call(
            METHOD_SESSION_WAIT,
            WaitParams {
                id: created.id.clone(),
                until: WaitUntil::Idle,
                timeout_ms: 5000,
            },
        )
        .await
        .unwrap();

    let _: Value = client
        .call(
            METHOD_SESSION_SEND,
            SendParams {
                id: created.id.clone(),
                text: "sleep 5000".into(),
                enter: true,
            },
        )
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_millis(100)).await;

    let _: Value = client
        .call(
            METHOD_SESSION_INPUT,
            InputParams {
                id: created.id.clone(),
                data_b64: STANDARD.encode(b"\x03"),
            },
        )
        .await
        .unwrap();

    let state: SessionState = client
        .call(
            METHOD_SESSION_WAIT,
            WaitParams {
                id: created.id.clone(),
                until: WaitUntil::Exited,
                timeout_ms: 3000,
            },
        )
        .await
        .unwrap();

    assert!(
        matches!(state, SessionState::Exited { .. }),
        "esperaba Exited tras Ctrl+C, llegó: {state:?}"
    );

    mgr.kill_all();
    server.abort();
}

#[tokio::test]
async fn resize_applies() {
    let dir = tempdir().unwrap();
    let sock = dir.path().join("pas").join("pasd.sock");
    let listener = bind(&sock).unwrap();
    let mgr = Manager::new(200);
    let server = tokio::spawn(serve(listener, mgr.clone()));

    let mut client = Client::connect(&sock).await.unwrap();
    let created: CreateResult = client
        .call(
            METHOD_SESSION_CREATE,
            json!({
                "command": [fake_agent_bin()],
                "cwd": std::env::temp_dir(),
                "title": "resize-test",
            }),
        )
        .await
        .unwrap();

    let _: SessionState = client
        .call(
            METHOD_SESSION_WAIT,
            WaitParams {
                id: created.id.clone(),
                until: WaitUntil::Idle,
                timeout_ms: 5000,
            },
        )
        .await
        .unwrap();

    let _: Value = client
        .call(
            METHOD_SESSION_RESIZE,
            ResizeParams {
                id: created.id.clone(),
                cols: 100,
                rows: 30,
            },
        )
        .await
        .unwrap();

    let _: Value = client
        .call(
            METHOD_SESSION_SEND,
            SendParams {
                id: created.id.clone(),
                text: "size".into(),
                enter: true,
            },
        )
        .await
        .unwrap();

    let _: SessionState = client
        .call(
            METHOD_SESSION_WAIT,
            WaitParams {
                id: created.id.clone(),
                until: WaitUntil::Idle,
                timeout_ms: 5000,
            },
        )
        .await
        .unwrap();

    let read: ReadResult = client
        .call(
            METHOD_SESSION_READ,
            ReadParams {
                id: created.id.clone(),
                lines: Some(5),
            },
        )
        .await
        .unwrap();

    assert!(
        read.text.contains("30 100"),
        "esperaba '30 100' tras resize 100x30, salió: {:?}",
        read.text
    );

    let _: Value = client
        .call(METHOD_SESSION_KILL, IdParams { id: created.id })
        .await
        .unwrap();
    mgr.kill_all();
    server.abort();
}

#[tokio::test]
async fn snapshot_repaints() {
    let dir = tempdir().unwrap();
    let sock = dir.path().join("pas").join("pasd.sock");
    let listener = bind(&sock).unwrap();
    let mgr = Manager::new(200);
    let server = tokio::spawn(serve(listener, mgr.clone()));

    let mut client = Client::connect(&sock).await.unwrap();
    let created: CreateResult = client
        .call(
            METHOD_SESSION_CREATE,
            json!({
                "command": [fake_agent_bin()],
                "cwd": std::env::temp_dir(),
                "title": "snap-test",
                "cols": 80,
                "rows": 24,
            }),
        )
        .await
        .unwrap();

    let _: SessionState = client
        .call(
            METHOD_SESSION_WAIT,
            WaitParams {
                id: created.id.clone(),
                until: WaitUntil::Idle,
                timeout_ms: 5000,
            },
        )
        .await
        .unwrap();

    let _: Value = client
        .call(
            METHOD_SESSION_SEND,
            SendParams {
                id: created.id.clone(),
                text: "flood 60".into(),
                enter: true,
            },
        )
        .await
        .unwrap();

    let _: SessionState = client
        .call(
            METHOD_SESSION_WAIT,
            WaitParams {
                id: created.id.clone(),
                until: WaitUntil::Idle,
                timeout_ms: 5000,
            },
        )
        .await
        .unwrap();

    let _: Value = client
        .call(
            METHOD_SESSION_SEND,
            SendParams {
                id: created.id.clone(),
                text: "echo marca-final".into(),
                enter: true,
            },
        )
        .await
        .unwrap();

    let _: SessionState = client
        .call(
            METHOD_SESSION_WAIT,
            WaitParams {
                id: created.id.clone(),
                until: WaitUntil::Idle,
                timeout_ms: 5000,
            },
        )
        .await
        .unwrap();

    let expected_full: ReadResult = client
        .call(
            METHOD_SESSION_READ,
            ReadParams {
                id: created.id.clone(),
                lines: None,
            },
        )
        .await
        .unwrap();

    let snap: SnapshotResult = client
        .call(
            METHOD_SESSION_SNAPSHOT,
            IdParams {
                id: created.id.clone(),
            },
        )
        .await
        .unwrap();

    let snap_bytes = STANDARD.decode(&snap.data_b64).unwrap();
    let mut repainted = vt100::Parser::new(
        NonZeroU16::new(24).unwrap(),
        NonZeroU16::new(80).unwrap(),
        10_000,
    );
    repainted.process(&snap_bytes);

    let repainted_full = read_all_parser_lines(&mut repainted);
    assert_eq!(repainted_full, expected_full.text);
    assert!(
        repainted.screen().contents().contains("marca-final"),
        "pantalla repintada no tiene marca-final: {:?}",
        repainted.screen().contents()
    );

    let _: Value = client
        .call(METHOD_SESSION_KILL, IdParams { id: created.id })
        .await
        .unwrap();
    mgr.kill_all();
    server.abort();
}

#[tokio::test]
async fn lagged_subscriber_gets_snapshot() {
    let dir = tempdir().unwrap();
    let sock = dir.path().join("pas").join("pasd.sock");
    let listener = bind(&sock).unwrap();
    let mgr = Manager::new(200);
    let server = tokio::spawn(serve(listener, mgr.clone()));

    let mut ctrl = Client::connect(&sock).await.unwrap();
    let mut sub = Client::connect(&sock).await.unwrap();

    let created: CreateResult = ctrl
        .call(
            METHOD_SESSION_CREATE,
            json!({
                "command": [fake_agent_bin()],
                "cwd": std::env::temp_dir(),
                "title": "lag-test",
            }),
        )
        .await
        .unwrap();

    let _: SessionState = ctrl
        .call(
            METHOD_SESSION_WAIT,
            WaitParams {
                id: created.id.clone(),
                until: WaitUntil::Idle,
                timeout_ms: 5000,
            },
        )
        .await
        .unwrap();

    let _: Value = sub
        .call(
            METHOD_SESSION_SUBSCRIBE,
            IdParams {
                id: created.id.clone(),
            },
        )
        .await
        .unwrap();

    // `sub` no lee mientras `ctrl` dispara un flood lo bastante grande como para
    // llenar el buffer del socket y desbordar el broadcast de capacidad 1024.
    let _: Value = ctrl
        .call(
            METHOD_SESSION_SEND,
            SendParams {
                id: created.id.clone(),
                text: "flood 400000".into(),
                enter: true,
            },
        )
        .await
        .unwrap();

    let _: SessionState = ctrl
        .call(
            METHOD_SESSION_WAIT,
            WaitParams {
                id: created.id.clone(),
                until: WaitUntil::Idle,
                timeout_ms: 60_000,
            },
        )
        .await
        .unwrap();

    let got_snapshot = tokio::time::timeout(Duration::from_secs(15), async {
        while let Some(ev) = sub.next_event().await.unwrap() {
            if ev.method == METHOD_SESSION_SNAPSHOT {
                let payload: SessionOutputEvent = serde_json::from_value(ev.params).unwrap();
                assert_eq!(payload.id, created.id);
                let bytes = STANDARD.decode(&payload.data_b64).unwrap();
                let text = String::from_utf8_lossy(&bytes);
                assert!(
                    text.contains("línea 400000"),
                    "el snapshot tras lag no incluye el final del flood"
                );
                return true;
            }
        }
        false
    })
    .await
    .expect("timeout esperando snapshot tras lag");

    assert!(
        got_snapshot,
        "el suscriptor atrasado no recibió session.snapshot"
    );

    let _: Value = ctrl
        .call(METHOD_SESSION_KILL, IdParams { id: created.id })
        .await
        .unwrap();
    mgr.kill_all();
    server.abort();
}

#[tokio::test]
async fn subscribe_all_gets_created_state_removed() {
    let dir = tempdir().unwrap();
    let sock = dir.path().join("pas").join("pasd.sock");
    let listener = bind(&sock).unwrap();
    let mgr = Manager::new(200);
    let server = tokio::spawn(serve(listener, mgr.clone()));

    let mut sub = Client::connect(&sock).await.unwrap();
    let mut ctrl = Client::connect(&sock).await.unwrap();

    let _: Value = sub
        .call(METHOD_SESSION_SUBSCRIBE_ALL, json!({}))
        .await
        .unwrap();

    let created: CreateResult = ctrl
        .call(
            METHOD_SESSION_CREATE,
            json!({
                "command": [fake_agent_bin()],
                "cwd": std::env::temp_dir(),
                "title": "global-ev-test",
            }),
        )
        .await
        .unwrap();

    let ev_created = tokio::time::timeout(Duration::from_secs(5), sub.next_event())
        .await
        .expect("timeout esperando session.created")
        .unwrap()
        .expect("stream cerrado antes de session.created");
    assert_eq!(ev_created.method, EVENT_SESSION_CREATED);
    let payload: SessionCreatedEvent = serde_json::from_value(ev_created.params).unwrap();
    assert_eq!(payload.info.id, created.id);
    assert_eq!(payload.info.title, "global-ev-test");

    let ev_state = tokio::time::timeout(Duration::from_secs(5), sub.next_event())
        .await
        .expect("timeout esperando session.state")
        .unwrap()
        .expect("stream cerrado antes de session.state");
    assert_eq!(ev_state.method, EVENT_SESSION_STATE);
    let payload: SessionStateEvent = serde_json::from_value(ev_state.params).unwrap();
    assert_eq!(payload.id, created.id);
    assert_eq!(payload.state, SessionState::Idle);

    let _: Value = ctrl
        .call(
            METHOD_SESSION_KILL,
            IdParams {
                id: created.id.clone(),
            },
        )
        .await
        .unwrap();

    let got_removed = tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(ev) = sub.next_event().await.unwrap() {
            assert_ne!(
                ev.method, EVENT_SESSION_OUTPUT,
                "subscribe_all no debe enviar session.output"
            );
            if ev.method == EVENT_SESSION_REMOVED {
                let payload: SessionRemovedEvent = serde_json::from_value(ev.params).unwrap();
                assert_eq!(payload.id, created.id);
                return true;
            }
        }
        false
    })
    .await
    .expect("timeout esperando session.removed");

    assert!(got_removed, "no llegó session.removed");

    mgr.kill_all();
    server.abort();
}

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use pas_core::protocol::{
    CreateResult, METHOD_SESSION_CREATE, METHOD_SESSION_KILL, METHOD_SESSION_WAIT, Request,
    SessionState,
};
use pasd::manager::Manager;
use pasd::rpc::dispatch;
use pasd::server::{bind, serve};
use serde_json::{Value, json};
use tempfile::tempdir;
use tokio::net::UnixListener;

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

fn pas_bin() -> String {
    static BIN: OnceLock<String> = OnceLock::new();
    BIN.get_or_init(|| {
        let status = Command::new("cargo")
            .args(["build", "-p", "pas-cli", "--quiet"])
            .current_dir(workspace_root())
            .status()
            .expect("no se pudo compilar pas-cli");
        assert!(status.success(), "falló cargo build -p pas-cli");
        target_debug_dir()
            .join("pas")
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

struct HookRun {
    code: Option<i32>,
    stdout: String,
    elapsed: Duration,
}

fn run_pas_hook(
    agent: &str,
    event: &str,
    session_id: Option<&str>,
    socket: &std::path::Path,
    stdin_payload: &[u8],
) -> HookRun {
    let mut cmd = Command::new(pas_bin());
    cmd.args(["hook", agent, event])
        .env("PAS_SOCKET", socket)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    match session_id {
        Some(id) => {
            cmd.env("PAS_SESSION_ID", id);
        }
        None => {
            cmd.env_remove("PAS_SESSION_ID");
        }
    }

    let t0 = Instant::now();
    let mut child = cmd.spawn().expect("no se pudo ejecutar pas hook");
    {
        let mut stdin = child.stdin.take().expect("stdin piped");
        stdin
            .write_all(stdin_payload)
            .expect("pas hook debería consumir todo stdin sin romper el pipe");
    }
    let output = child.wait_with_output().expect("falló wait_with_output");
    HookRun {
        code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        elapsed: t0.elapsed(),
    }
}

#[tokio::test]
async fn hook_without_session_id_is_noop() {
    let dir = tempdir().unwrap();
    let sock = dir.path().join("pasd.sock");
    let listener = bind(&sock).unwrap();
    let mgr = Manager::new(150);
    let server_task = tokio::spawn(serve(listener, mgr.clone()));

    let created: CreateResult = serde_json::from_value(
        dispatch(
            &mgr,
            req(
                METHOD_SESSION_CREATE,
                json!({
                    "command": [fake_agent_bin()],
                    "cwd": dir.path(),
                    "title": "s-noop",
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

    let sock_clone = sock.clone();
    let res = tokio::task::spawn_blocking(move || {
        run_pas_hook(
            "claude",
            "PermissionRequest",
            None,
            &sock_clone,
            br#"{"hook_event_name":"PermissionRequest","tool_name":"Bash"}"#,
        )
    })
    .await
    .unwrap();

    assert_eq!(res.code, Some(0), "sin PAS_SESSION_ID debe salir 0");
    assert_eq!(res.stdout.trim(), "{}", "debe devolver JSON vacío");
    assert!(
        res.elapsed < Duration::from_millis(250),
        "debe salir al instante, tardó {:?}",
        res.elapsed
    );

    let session = mgr.get(&created.id).unwrap();
    assert_eq!(
        session.state(),
        SessionState::Idle,
        "sin PAS_SESSION_ID no debe tocar el estado de ninguna sesión"
    );

    dispatch(&mgr, req(METHOD_SESSION_KILL, json!({ "id": created.id }))).await;
    server_task.abort();
}

#[tokio::test]
async fn hook_sets_waiting_then_running() {
    let dir = tempdir().unwrap();
    let sock = dir.path().join("pasd.sock");
    let listener = bind(&sock).unwrap();
    let mgr = Manager::new(500);
    let server_task = tokio::spawn(serve(listener, mgr.clone()));

    let created: CreateResult = serde_json::from_value(
        dispatch(
            &mgr,
            req(
                METHOD_SESSION_CREATE,
                json!({
                    "command": [fake_agent_bin()],
                    "cwd": dir.path(),
                    "title": "s-hooks",
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

    let session = mgr.get(&created.id).unwrap();
    assert_eq!(session.state(), SessionState::Idle);

    // Claude: PermissionRequest -> Waiting, PreToolUse -> Running, Notification -> Waiting,
    // UserPromptSubmit -> Running, Stop -> Idle
    let cases = [
        ("claude", "PermissionRequest", SessionState::Waiting),
        ("claude", "PreToolUse", SessionState::Running),
        ("claude", "Notification", SessionState::Waiting),
        ("claude", "UserPromptSubmit", SessionState::Running),
        ("claude", "Stop", SessionState::Idle),
        ("codex", "permission_request", SessionState::Waiting),
        ("codex", "pre_tool_use", SessionState::Running),
        ("codex", "stop", SessionState::Idle),
        ("codex", "user_prompt_submit", SessionState::Running),
        ("agy", "Stop", SessionState::Idle),
        ("agy", "PreInvocation", SessionState::Running),
        ("agy", "PostInvocation", SessionState::Idle),
        ("agy", "PreToolUse", SessionState::Running),
    ];

    for (agent, event, expected) in cases {
        let sock_c = sock.clone();
        let id_c = created.id.clone();
        let agent_s = agent.to_string();
        let event_s = event.to_string();
        let res = tokio::task::spawn_blocking(move || {
            run_pas_hook(&agent_s, &event_s, Some(&id_c), &sock_c, b"{}")
        })
        .await
        .unwrap();

        assert_eq!(res.code, Some(0), "{agent}/{event} debe salir 0");
        assert_eq!(res.stdout.trim(), "{}", "{agent}/{event} debe imprimir {{}}");
        assert_eq!(
            session.state(),
            expected,
            "{agent}/{event} debería dejar estado {expected:?}"
        );
    }

    dispatch(&mgr, req(METHOD_SESSION_KILL, json!({ "id": created.id }))).await;
    server_task.abort();
}

#[tokio::test]
async fn hook_never_fails_when_pasd_down() {
    let dir = tempdir().unwrap();

    // 1. Socket inexistente (pasd caído) + payload grande en stdin (64 KiB).
    let missing_sock = dir.path().join("no-existe.sock");
    let big_stdin = vec![b'x'; 64 * 1024];
    let sock_c = missing_sock.clone();
    let res = tokio::task::spawn_blocking(move || {
        run_pas_hook("claude", "Stop", Some("deadbeef"), &sock_c, &big_stdin)
    })
    .await
    .unwrap();

    assert_eq!(res.code, Some(0), "con pasd caído debe salir 0");
    assert_eq!(res.stdout.trim(), "{}");
    assert!(
        res.elapsed < Duration::from_millis(400),
        "con pasd caído debe salir en <400 ms, tardó {:?}",
        res.elapsed
    );

    // 2. Socket colgado (acepta conexión pero nunca responde): debe cortar por timeout <400 ms.
    let hung_sock = dir.path().join("hung.sock");
    let hung_listener = UnixListener::bind(&hung_sock).unwrap();
    let hung_task = tokio::spawn(async move {
        if let Ok((_stream, _)) = hung_listener.accept().await {
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    });

    let sock_c = hung_sock.clone();
    let res2 = tokio::task::spawn_blocking(move || {
        run_pas_hook(
            "codex",
            "permission_request",
            Some("deadbeef"),
            &sock_c,
            b"{}",
        )
    })
    .await
    .unwrap();

    assert_eq!(res2.code, Some(0), "con pasd colgado debe salir 0");
    assert_eq!(res2.stdout.trim(), "{}");
    assert!(
        res2.elapsed < Duration::from_millis(400),
        "con pasd colgado debe salir en <400 ms, tardó {:?}",
        res2.elapsed
    );

    hung_task.abort();
}

#[tokio::test]
async fn silence_does_not_override_waiting() {
    let dir = tempdir().unwrap();
    let sock = dir.path().join("pasd.sock");
    let listener = bind(&sock).unwrap();
    let mgr = Manager::new(100);
    let server_task = tokio::spawn(serve(listener, mgr.clone()));

    let created: CreateResult = serde_json::from_value(
        dispatch(
            &mgr,
            req(
                METHOD_SESSION_CREATE,
                json!({
                    "command": [fake_agent_bin()],
                    "cwd": dir.path(),
                    "title": "s-waiting",
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

    let sock_c = sock.clone();
    let id_c = created.id.clone();
    let res = tokio::task::spawn_blocking(move || {
        run_pas_hook("claude", "PermissionRequest", Some(&id_c), &sock_c, b"{}")
    })
    .await
    .unwrap();
    assert_eq!(res.code, Some(0));

    let session = mgr.get(&created.id).unwrap();
    assert_eq!(session.state(), SessionState::Waiting);

    // Esperar más de 3 veces idle_ms (100 ms): la heurística de silencio no puede pisar Waiting.
    tokio::time::sleep(Duration::from_millis(350)).await;
    assert_eq!(
        session.state(),
        SessionState::Waiting,
        "el silencio no debe pisar SessionState::Waiting"
    );

    dispatch(&mgr, req(METHOD_SESSION_KILL, json!({ "id": created.id }))).await;
    server_task.abort();
}

#[tokio::test]
async fn hook_returns_with_stdin_left_open() {
    let dir = tempdir().unwrap();
    let missing_sock = dir.path().join("no-existe.sock");

    let res = tokio::task::spawn_blocking(move || {
        let bin = pas_bin();
        let t0 = Instant::now();
        let mut child = Command::new(bin)
            .args(["hook", "claude", "Stop"])
            .env("PAS_SOCKET", &missing_sock)
            .env_remove("PAS_SESSION_ID")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("no se pudo ejecutar pas hook");

        // Mantener stdin abierto sin cerrarlo nunca mientras el hook corre.
        let _keep_stdin_open = child.stdin.take().expect("stdin piped");

        let deadline = Instant::now() + Duration::from_millis(800);
        loop {
            if let Some(status) = child.try_wait().expect("falló try_wait") {
                let elapsed = t0.elapsed();
                let mut stdout = String::new();
                if let Some(mut out) = child.stdout.take() {
                    let _ = std::io::Read::read_to_string(&mut out, &mut stdout);
                }
                return Ok((status.code(), stdout, elapsed));
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                return Err(t0.elapsed());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    })
    .await
    .unwrap();

    let (code, stdout, elapsed) = res.expect("pas hook se colgó con stdin abierto");
    assert_eq!(code, Some(0), "con stdin abierto debe salir 0");
    assert_eq!(stdout.trim(), "{}");
    assert!(
        elapsed < Duration::from_millis(400),
        "con stdin abierto debe salir en <400 ms, tardó {:?}",
        elapsed
    );
}

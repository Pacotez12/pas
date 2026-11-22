use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use pas_core::protocol::{CreateParams, SessionState, WaitUntil};
use pasd::session::{Session, SessionError};

fn fake_agent_bin() -> PathBuf {
    static BIN: OnceLock<PathBuf> = OnceLock::new();
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
        target_dir.join("debug").join("fake-agent")
    })
    .clone()
}

fn params_for(cmd: Vec<String>) -> CreateParams {
    CreateParams {
        command: cmd,
        cwd: std::env::temp_dir(),
        title: Some("test".into()),
        env: BTreeMap::new(),
        cols: 120,
        rows: 40,
        project_id: None,
    }
}

#[tokio::test]
async fn spawn_and_read_banner() {
    let bin = fake_agent_bin().to_string_lossy().into_owned();
    let s = Session::spawn("s1".into(), params_for(vec![bin]), 200).unwrap();

    let st = s.wait(WaitUntil::Idle, Duration::from_secs(5)).await.unwrap();
    assert_eq!(st, SessionState::Idle);
    assert!(s.read(None).contains("fake-agent listo"));
    s.kill();
}

#[tokio::test]
async fn send_echo_utf8() {
    let bin = fake_agent_bin().to_string_lossy().into_owned();
    let s = Session::spawn("s2".into(), params_for(vec![bin]), 200).unwrap();
    s.wait(WaitUntil::Idle, Duration::from_secs(5)).await.unwrap();

    s.send("echo canción ñandú", true).unwrap();
    s.wait(WaitUntil::Idle, Duration::from_secs(5)).await.unwrap();

    let out = s.read(Some(3));
    assert!(out.contains("canción ñandú"), "salida inesperada: {out:?}");
    s.kill();
}

#[tokio::test]
async fn exit_code_reported() {
    let bin = fake_agent_bin().to_string_lossy().into_owned();
    let s = Session::spawn("s3".into(), params_for(vec![bin]), 200).unwrap();
    s.wait(WaitUntil::Idle, Duration::from_secs(5)).await.unwrap();

    s.send("exit 3", true).unwrap();
    let st = s
        .wait(WaitUntil::Exited, Duration::from_secs(5))
        .await
        .unwrap();
    assert_eq!(st, SessionState::Exited { code: Some(3) });
}

#[tokio::test]
async fn spawn_missing_command() {
    let err = Session::spawn(
        "s4".into(),
        params_for(vec!["no-existe-pas-xyz".into()]),
        200,
    )
    .unwrap_err();

    assert!(matches!(err, SessionError::Spawn { .. }));
    assert!(err.to_string().contains("no-existe-pas-xyz"));
}

#[tokio::test]
async fn flood_is_bounded() {
    let bin = fake_agent_bin().to_string_lossy().into_owned();
    let s = Session::spawn("s5".into(), params_for(vec![bin]), 200).unwrap();
    s.wait(WaitUntil::Idle, Duration::from_secs(5)).await.unwrap();

    s.send("flood 200000", true).unwrap();
    s.wait(WaitUntil::Idle, Duration::from_secs(60)).await.unwrap();

    let t0 = Instant::now();
    let out = s.read(Some(5));
    let elapsed = t0.elapsed();
    assert!(elapsed < Duration::from_secs(1), "read tardó {elapsed:?}");
    assert!(out.contains("línea 200000"), "salida: {out:?}");
    s.kill();
}

#[tokio::test]
async fn wait_timeout() {
    let bin = fake_agent_bin().to_string_lossy().into_owned();
    let s = Session::spawn("s6".into(), params_for(vec![bin]), 200).unwrap();
    s.wait(WaitUntil::Idle, Duration::from_secs(5)).await.unwrap();

    s.send("sleep 3000", true).unwrap();
    let err = s
        .wait(WaitUntil::Exited, Duration::from_millis(500))
        .await
        .unwrap_err();
    assert!(matches!(err, SessionError::Timeout { .. }));
    assert!(
        err.to_string().contains("0.5 s"),
        "mensaje inesperado: {err}"
    );
    s.kill();
}

#[tokio::test]
async fn strips_inherited_agent_env_but_keeps_explicit() {
    if std::env::var_os("PAS_ENV_TEST_CHILD").is_none() {
        let status = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "strips_inherited_agent_env_but_keeps_explicit"])
            .env("PAS_ENV_TEST_CHILD", "1")
            .env("CLAUDECODE", "1")
            .env("CLAUDE_CODE_CHILD_SESSION", "yes")
            .env("ORCA_SESSION", "1")
            .status()
            .unwrap();
        assert!(status.success());
        return;
    }

    let bin = fake_agent_bin().to_string_lossy().into_owned();
    let mut params = params_for(vec![bin]);
    params
        .env
        .insert("CLAUDE_CODE_EXPLICIT".into(), "respetada".into());

    let s = Session::spawn("s7".into(), params, 200).unwrap();
    s.wait(WaitUntil::Idle, Duration::from_secs(5)).await.unwrap();

    for (var, expected) in [
        ("CLAUDECODE", "(vacío)"),
        ("CLAUDE_CODE_CHILD_SESSION", "(vacío)"),
        ("ORCA_SESSION", "(vacío)"),
        ("CLAUDE_CODE_EXPLICIT", "respetada"),
    ] {
        s.send(&format!("env {var}"), true).unwrap();
        s.wait(WaitUntil::Idle, Duration::from_secs(5)).await.unwrap();
        let out = s.read(Some(3));
        assert!(
            out.contains(expected),
            "para {var} esperaba {expected:?}, salió: {out:?}"
        );
    }
    s.kill();
}

#[tokio::test]
async fn read_lines_skips_whitespace_only_lines() {
    let s = Session::spawn(
        "s8".into(),
        params_for(vec![
            "sh".into(),
            "-c".into(),
            "printf 'linea-real\\n     \\n   \\n     \\n'".into(),
        ]),
        200,
    )
    .unwrap();
    s.wait(WaitUntil::Exited, Duration::from_secs(5))
        .await
        .unwrap();

    let out = s.read(Some(2));
    assert!(
        out.contains("linea-real"),
        "esperaba 'linea-real' en las últimas 2 líneas no vacías, salió: {out:?}"
    );
}

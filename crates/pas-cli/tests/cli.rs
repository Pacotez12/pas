use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use assert_cmd::Command as AssertCommand;
use tempfile::{TempDir, tempdir};

struct TestEnv {
    _dir: TempDir,
    sock: PathBuf,
}

impl TestEnv {
    fn new() -> Self {
        ensure_bins_built();
        let dir = tempdir().unwrap();
        let sock = dir.path().join("pasd.sock");
        Self { _dir: dir, sock }
    }

    fn pas(&self) -> AssertCommand {
        let mut cmd = AssertCommand::new(env!("CARGO_BIN_EXE_pas"));
        cmd.env("PAS_SOCKET", &self.sock);
        cmd.env("XDG_DATA_HOME", self._dir.path().join("data"));
        cmd
    }
}

impl Drop for TestEnv {
    fn drop(&mut self) {
        let sock_str = self.sock.to_string_lossy().into_owned();
        let pids = find_pids_matching(&["pasd", &sock_str]);
        for pid in &pids {
            let _ = Command::new("kill").args(["-TERM", pid]).status();
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if pids.iter().all(|p| !Path::new("/proc").join(p).exists()) {
                break;
            }
            std::thread::yield_now();
        }
    }
}

fn find_pids_matching(needles: &[&str]) -> Vec<String> {
    let mut found = Vec::new();
    let Ok(entries) = fs::read_dir("/proc") else {
        return found;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let Ok(bytes) = fs::read(entry.path().join("cmdline")) else {
            continue;
        };
        if bytes.is_empty() {
            continue;
        }
        let cmdline = String::from_utf8_lossy(&bytes);
        if needles.iter().all(|n| cmdline.contains(n)) {
            found.push(name);
        }
    }
    found
}

fn ensure_bins_built() -> PathBuf {
    static FAKE_AGENT: OnceLock<PathBuf> = OnceLock::new();
    FAKE_AGENT
        .get_or_init(|| {
            let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .to_path_buf();

            let status = Command::new("cargo")
                .args(["build", "-p", "pasd", "-p", "fake-agent", "--quiet"])
                .current_dir(&workspace_root)
                .status()
                .expect("no se pudo compilar pasd/fake-agent");
            assert!(status.success(), "falló cargo build -p pasd -p fake-agent");

            let target_dir = std::env::var_os("CARGO_TARGET_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| workspace_root.join("target"));
            target_dir.join("debug").join("fake-agent")
        })
        .clone()
}

fn fake_agent_bin() -> String {
    ensure_bins_built().to_string_lossy().into_owned()
}

#[test]
fn autostart_and_flow() {
    let env = TestEnv::new();

    let out = env
        .pas()
        .args(["new", "--title", "t", "--", &fake_agent_bin()])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let id = String::from_utf8_lossy(&out.stdout).trim().to_string();
    assert_eq!(id.len(), 8);

    env.pas()
        .args(["wait", "t", "--until", "idle", "--timeout", "10"])
        .assert()
        .success();

    env.pas()
        .args(["send", "t", "echo hola"])
        .assert()
        .success();

    env.pas()
        .args(["wait", "t", "--until", "idle", "--timeout", "10"])
        .assert()
        .success();

    let read = env
        .pas()
        .args(["read", "t", "--lines", "3"])
        .output()
        .unwrap();
    assert!(read.status.success());
    let stdout = String::from_utf8_lossy(&read.stdout);
    assert!(stdout.contains("hola"), "read stdout: {stdout:?}");

    env.pas().args(["kill", "t"]).assert().success();
    env.pas()
        .args(["wait", "t", "--until", "exited", "--timeout", "5"])
        .assert()
        .success();

    let list = env.pas().args(["list"]).output().unwrap();
    assert!(list.status.success());
    let list_out = String::from_utf8_lossy(&list.stdout);
    assert!(list_out.contains("t"), "list: {list_out:?}");
    assert!(list_out.contains("exited"), "list: {list_out:?}");
}

#[test]
fn wait_timeout_exit_code() {
    let env = TestEnv::new();

    env.pas()
        .args(["new", "--title", "t", "--", &fake_agent_bin()])
        .assert()
        .success();
    env.pas()
        .args(["wait", "t", "--until", "idle", "--timeout", "10"])
        .assert()
        .success();
    env.pas()
        .args(["send", "t", "sleep 5000"])
        .assert()
        .success();

    env.pas()
        .args(["wait", "t", "--until", "exited", "--timeout", "1"])
        .assert()
        .code(2);
}

#[test]
fn unknown_session_message() {
    let env = TestEnv::new();

    let out = env.pas().args(["read", "nada"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("pas list"), "stderr: {stderr:?}");
}

#[test]
fn kill_leaves_no_orphan_fake_agent() {
    let env = TestEnv::new();
    let marker = format!("pas-orphan-check-{}", std::process::id());

    env.pas()
        .args(["new", "--title", "k", "--", &fake_agent_bin(), &marker])
        .assert()
        .success();
    env.pas()
        .args(["wait", "k", "--until", "idle", "--timeout", "10"])
        .assert()
        .success();

    assert!(
        !find_pids_matching(&[&marker]).is_empty(),
        "fake-agent debería estar vivo antes de kill"
    );

    env.pas().args(["kill", "k"]).assert().success();
    env.pas()
        .args(["wait", "k", "--until", "exited", "--timeout", "5"])
        .assert()
        .success();

    assert!(
        find_pids_matching(&[&marker]).is_empty(),
        "quedó fake-agent huérfano tras pas kill"
    );
}

#[test]
fn relative_cwd_is_resolved_from_caller_dir() {
    let env = TestEnv::new();
    let daemon_dir = tempdir().unwrap();
    let caller_dir = tempdir().unwrap();

    // Arranca pasd desde otra carpeta para que su cwd no coincida con la del llamador.
    env.pas()
        .current_dir(daemon_dir.path())
        .args(["list"])
        .assert()
        .success();

    env.pas()
        .current_dir(caller_dir.path())
        .args([
            "new",
            "--cwd",
            ".",
            "--title",
            "rel",
            "--",
            &fake_agent_bin(),
        ])
        .assert()
        .success();

    let list = env.pas().args(["list"]).output().unwrap();
    let list_out = String::from_utf8_lossy(&list.stdout);
    let want = caller_dir.path().display().to_string();
    assert!(list_out.contains(&want), "list: {list_out:?}, want {want}");
    env.pas().args(["kill", "rel"]).assert().success();
}

#[test]
fn cli_new_worktree() {
    let env = TestEnv::new();
    let repo_dir = tempdir().unwrap();
    let repo = repo_dir.path().join("demo");
    fs::create_dir_all(&repo).unwrap();

    let run_git = |args: &[&str]| {
        let st = Command::new("git")
            .args(args)
            .current_dir(&repo)
            .status()
            .unwrap();
        assert!(st.success(), "git {args:?} falló");
    };
    run_git(&["init", "-b", "main", "--quiet"]);
    run_git(&["config", "user.email", "test@pas.local"]);
    run_git(&["config", "user.name", "Pas Test"]);
    fs::write(repo.join("README.md"), "hola\n").unwrap();
    run_git(&["add", "README.md"]);
    run_git(&["commit", "-m", "init", "--quiet"]);

    let out = env
        .pas()
        .args([
            "new",
            "--cwd",
            repo.to_str().unwrap(),
            "--title",
            "wt-cli",
            "--worktree",
            "--branch",
            "pas/cli-test",
            "--base",
            "main",
            "--",
            &fake_agent_bin(),
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "pas new --worktree falló: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let list = env.pas().args(["list", "--json"]).output().unwrap();
    assert!(list.status.success());
    let sessions: serde_json::Value = serde_json::from_slice(&list.stdout).unwrap();
    let sess = sessions
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["title"] == "wt-cli")
        .expect("no apareció wt-cli en pas list --json");
    assert_eq!(sess["worktree"]["branch"], "pas/cli-test");
    assert_eq!(sess["cwd"], sess["worktree"]["path"]);
    assert_ne!(
        sess["cwd"].as_str().unwrap(),
        repo.to_str().unwrap(),
        "debería correr en el worktree, no en el repo principal"
    );

    env.pas().args(["kill", "wt-cli"]).assert().success();
}

#[test]
fn send_prints_warning_to_stderr_when_text_never_appears() {
    let env = TestEnv::new();

    env.pas()
        .args([
            "new",
            "--title",
            "agy",
            "--",
            "env",
            "FAKE_AGENT_DROP_INPUTS=5",
            &fake_agent_bin(),
        ])
        .assert()
        .success();

    env.pas()
        .args(["wait", "agy", "--until", "idle", "--timeout", "10"])
        .assert()
        .success();

    let out = env
        .pas()
        .args(["send", "agy", "echo perdido"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "pas send debería salir 0 aun con aviso; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("agy no mostró el texto en su prompt; revisá la sesión con pas read"),
        "esperaba aviso en stderr, salió: {stderr:?}"
    );

    env.pas().args(["kill", "agy"]).assert().success();
}

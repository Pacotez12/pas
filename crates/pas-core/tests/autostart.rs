use std::fs;
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use pas_core::CoreError;
use pas_core::autostart::{ENV_PASD_BIN, connect_or_spawn};
use tempfile::tempdir;

#[test]
fn pas_pasd_bin_override_prevents_autostart_when_missing() {
    if std::env::var_os("PAS_TEST_CHILD").is_some() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let dir = tempdir().unwrap();
        let sock = dir.path().join("pasd.sock");
        let err = rt.block_on(connect_or_spawn(&sock)).unwrap_err();
        assert!(
            matches!(err, CoreError::NotRunning),
            "esperaba CoreError::NotRunning, llegó {err:?}"
        );
        assert!(
            !sock.exists(),
            "no debería haberse creado el socket si PAS_PASD_BIN no existe"
        );
        return;
    }

    let status = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "pas_pasd_bin_override_prevents_autostart_when_missing",
        ])
        .env("PAS_TEST_CHILD", "1")
        .env(ENV_PASD_BIN, "/ruta/inexistente/pasd-test")
        .status()
        .unwrap();
    assert!(status.success());
}

#[cfg(target_os = "linux")]
fn proc_is_zombie(pid: u32) -> bool {
    let Ok(stat) = fs::read_to_string(format!("/proc/{pid}/stat")) else {
        return false;
    };
    let Some(after_comm) = stat.rsplit_once(')') else {
        return false;
    };
    after_comm
        .1
        .split_whitespace()
        .next()
        .is_some_and(|state| state == "Z")
}

#[cfg(target_os = "linux")]
#[test]
fn autostart_reaps_child_without_leaving_zombie() {
    if std::env::var_os("PAS_TEST_FAKE_PASD_HELPER").is_some() {
        let pid_file = std::env::var_os("PAS_TEST_PID_FILE").unwrap();
        let sock_path = std::env::var_os("PAS_TEST_SOCK_PATH").unwrap();
        fs::write(pid_file, format!("{}\n", std::process::id())).unwrap();
        let listener = UnixListener::bind(sock_path).unwrap();
        let _ = listener.accept();
        std::process::exit(0);
    }

    if std::env::var_os("PAS_TEST_ZOMBIE_PARENT").is_some() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let dir = tempdir().unwrap();
        let sock = dir.path().join("pasd.sock");
        let pid_file = dir.path().join("child.pid");
        let fake_pasd = dir.path().join("fake-pasd.sh");

        // Script mínimo que actúa de fake pasd: delega al propio binario de test en modo helper.
        let exe = std::env::current_exe().unwrap();
        let script = format!(
            "#!/bin/sh\nexec \"{}\" --exact autostart_reaps_child_without_leaving_zombie\n",
            exe.display()
        );
        fs::write(&fake_pasd, script).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&fake_pasd, fs::Permissions::from_mode(0o755)).unwrap();
        }

        // SAFETY: proceso hijo dedicado a este test.
        unsafe {
            std::env::set_var(ENV_PASD_BIN, &fake_pasd);
            std::env::set_var("PAS_TEST_FAKE_PASD_HELPER", "1");
            std::env::set_var("PAS_TEST_PID_FILE", &pid_file);
            std::env::set_var("PAS_TEST_SOCK_PATH", &sock);
        }

        let _client = rt.block_on(connect_or_spawn(&sock)).unwrap();

        let pid: u32 = fs::read_to_string(&pid_file)
            .expect("el fake pasd debió escribir su PID")
            .trim()
            .parse()
            .unwrap();

        // Esperar a que el proceso hijo termine y verificar que sea reaprovechado (no quede en estado Z).
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if !PathBuf::from(format!("/proc/{pid}/stat")).exists() {
                return;
            }
            std::thread::sleep(Duration::from_millis(25));
        }

        assert!(
            !proc_is_zombie(pid),
            "el proceso autoarrancado {pid} quedó en estado zombie (<defunct>)"
        );
        panic!("el proceso fake pasd {pid} no terminó dentro del tiempo esperado");
    }

    let status = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "autostart_reaps_child_without_leaving_zombie"])
        .env("PAS_TEST_ZOMBIE_PARENT", "1")
        .status()
        .unwrap();
    assert!(status.success());
}

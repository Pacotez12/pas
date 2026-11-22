use std::process::Command;

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
        .args(["--exact", "pas_pasd_bin_override_prevents_autostart_when_missing"])
        .env("PAS_TEST_CHILD", "1")
        .env(ENV_PASD_BIN, "/ruta/inexistente/pasd-test")
        .status()
        .unwrap();
    assert!(status.success());
}

use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::CoreError;
use crate::client::Client;

/// Permite sobreescribir el binario de `pasd` que lanza el autoarranque.
/// Útil en tests e2e para apuntar a un binario inexistente y probar el estado desconectado.
pub const ENV_PASD_BIN: &str = "PAS_PASD_BIN";

pub async fn connect_or_spawn(path: &Path) -> Result<Client, CoreError> {
    match Client::connect(path).await {
        Ok(client) => return Ok(client),
        Err(CoreError::NotRunning) => {}
        Err(err) => return Err(err),
    }

    let pasd = find_pasd();
    let mut cmd = Command::new(pasd);
    cmd.arg("--socket")
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    // SAFETY: setsid() es async-signal-safe y corre entre fork() y exec() para
    // desacoplar pasd de la terminal que lo lanzó, así sobrevive cuando se cierra.
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }

    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(_) => return Err(CoreError::NotRunning),
    };

    // Reaprovechar el hijo al terminar para evitar que quede `[pasd] <defunct>` si muere
    // mientras la GUI sigue corriendo.
    let _ = std::thread::Builder::new()
        .name("pasd-reaper".into())
        .spawn(move || {
            let _ = child.wait();
        });

    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        match Client::connect(path).await {
            Ok(client) => return Ok(client),
            Err(CoreError::NotRunning) => {
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            Err(err) => return Err(err),
        }
    }

    Err(CoreError::NotRunning)
}

fn find_pasd() -> PathBuf {
    if let Some(custom) = std::env::var_os(ENV_PASD_BIN).filter(|v| !v.is_empty()) {
        return PathBuf::from(custom);
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        let sibling = dir.join("pasd");
        if sibling.is_file() {
            return sibling;
        }
    }
    PathBuf::from("pasd")
}

use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use pas_core::CoreError;
use pas_core::client::Client;

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

    if cmd.spawn().is_err() {
        return Err(CoreError::NotRunning);
    }

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

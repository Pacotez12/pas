use std::fs::{self, Permissions};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream as StdUnixStream;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use pas_core::CoreError;
use pas_core::protocol::Request;
use pas_core::socket::{read_msg, validate_socket_path, write_msg};
use tokio::io::BufReader;
use tokio::net::{UnixListener, UnixStream};

use crate::manager::Manager;
use crate::rpc;

#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    #[error("pasd ya está corriendo en {}.", path.display())]
    AlreadyRunning { path: PathBuf },

    #[error(transparent)]
    Core(#[from] CoreError),

    #[error("{0}")]
    Io(#[from] std::io::Error),
}

pub fn bind(path: &Path) -> Result<UnixListener, ServerError> {
    validate_socket_path(path)?;
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
        && !parent.exists()
    {
        fs::create_dir_all(parent)?;
        fs::set_permissions(parent, Permissions::from_mode(0o700))?;
    }

    if path.symlink_metadata().is_ok() {
        if StdUnixStream::connect(path).is_ok() {
            return Err(ServerError::AlreadyRunning {
                path: path.to_path_buf(),
            });
        }
        fs::remove_file(path)?;
    }

    let listener = UnixListener::bind(path)?;
    fs::set_permissions(path, Permissions::from_mode(0o600))?;
    Ok(listener)
}

pub async fn serve(listener: UnixListener, mgr: Arc<Manager>) {
    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                let mgr = Arc::clone(&mgr);
                tokio::spawn(handle_client(stream, mgr));
            }
            Err(err) => {
                tracing::warn!("error aceptando conexión: {err}");
            }
        }
    }
}

async fn handle_client(stream: UnixStream, mgr: Arc<Manager>) {
    let mut stream = BufReader::new(stream);
    loop {
        match read_msg::<Request>(&mut stream).await {
            Ok(Some(req)) => {
                let resp = rpc::dispatch(&mgr, req).await;
                if let Err(err) = write_msg(&mut stream, &resp).await {
                    tracing::debug!("error escribiendo respuesta al cliente: {err}");
                    break;
                }
            }
            Ok(None) => break,
            Err(err) => {
                tracing::debug!("conexión cerrada por error de lectura: {err}");
                break;
            }
        }
    }
}

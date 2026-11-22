use std::path::{Path, PathBuf};

use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt};

use crate::CoreError;

/// Límite de `sun_path` en `sockaddr_un` de Linux (108 bytes incluido el NUL).
pub const MAX_SOCKET_PATH_LEN: usize = 107;

pub fn validate_socket_path(path: &Path) -> Result<(), CoreError> {
    let len = path.as_os_str().as_encoded_bytes().len();
    if len > MAX_SOCKET_PATH_LEN {
        return Err(CoreError::SocketPathTooLong {
            path: path.to_path_buf(),
            len,
            max: MAX_SOCKET_PATH_LEN,
        });
    }
    Ok(())
}

pub fn socket_path() -> Result<PathBuf, CoreError> {
    if let Some(p) = std::env::var_os("PAS_SOCKET").filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(p));
    }
    if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR").filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(dir).join("pas").join("pasd.sock"));
    }
    Err(CoreError::NoRuntimeDir)
}

pub async fn read_msg<T: DeserializeOwned>(
    r: &mut (impl AsyncBufRead + Unpin),
) -> Result<Option<T>, CoreError> {
    let mut line = String::new();
    let n = r.read_line(&mut line).await?;
    if n == 0 {
        return Ok(None);
    }
    Ok(Some(serde_json::from_str(&line)?))
}

pub async fn write_msg<T: Serialize>(
    w: &mut (impl AsyncWrite + Unpin),
    msg: &T,
) -> Result<(), CoreError> {
    let mut buf = serde_json::to_vec(msg)?;
    buf.push(b'\n');
    w.write_all(&buf).await?;
    w.flush().await?;
    Ok(())
}

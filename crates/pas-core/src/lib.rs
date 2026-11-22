use std::path::PathBuf;

pub mod autostart;
pub mod client;
pub mod protocol;
pub mod socket;

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("No encontré XDG_RUNTIME_DIR. Definí PAS_SOCKET con la ruta del socket de pasd.")]
    NoRuntimeDir,

    #[error(
        "La ruta del socket es demasiado larga ({len} bytes, máximo {max}): {}. Usá una ruta más corta con PAS_SOCKET.",
        path.display()
    )]
    SocketPathTooLong {
        path: PathBuf,
        len: usize,
        max: usize,
    },

    #[error("{0}")]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    Json(#[from] serde_json::Error),

    #[error("{message}")]
    Rpc { code: i32, message: String },

    #[error(
        "pasd no está corriendo. Iniciálo con `pasd` o usá cualquier comando de `pas`, que lo arranca solo."
    )]
    NotRunning,
}

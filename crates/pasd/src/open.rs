use std::ffi::{OsStr, OsString};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use pas_core::protocol::{
    ERR_EDITOR_NOT_FOUND, ERR_OPEN_FAILED, ERR_PATH_NOT_FOUND, ERR_PATH_OUTSIDE_PROJECT,
    OpenExistsParams, OpenExistsResult, OpenPathParams, OpenTarget, RpcError,
};

const DEFAULT_EDITORS: &[&str] = &["code", "cursor", "zed", "subl"];
const DEFAULT_TERMINALS: &[&str] = &[
    "x-terminal-emulator",
    "kitty",
    "ptyxis",
    "kgx",
    "gnome-terminal",
    "konsole",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchRequest {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub cwd: Option<PathBuf>,
}

pub trait Launcher: Send + Sync {
    fn launch(&self, req: LaunchRequest) -> Result<(), std::io::Error>;
}

pub struct SystemLauncher;

impl Launcher for SystemLauncher {
    fn launch(&self, req: LaunchRequest) -> Result<(), std::io::Error> {
        let mut cmd = Command::new(&req.program);
        cmd.args(&req.args);
        if let Some(ref cwd) = req.cwd {
            cmd.current_dir(cwd);
        }
        cmd.stdin(Stdio::null());
        cmd.stdout(Stdio::null());
        cmd.stderr(Stdio::null());
        let mut child = cmd.spawn()?;
        // pasd lives long: reap the child so editors/file managers never linger as zombies.
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        Ok(())
    }
}

#[derive(Debug, Clone, Default)]
pub struct OpenEnv {
    pub path_env: Option<OsString>,
    pub home_dir: Option<PathBuf>,
}

#[derive(Debug, thiserror::Error)]
pub enum OpenError {
    #[error("No existe la ruta {}. Verificá que el archivo o carpeta exista.", path.display())]
    PathNotFound { path: PathBuf },

    #[error(
        "La ruta {} está fuera de los proyectos y sesiones abiertos. Agregá el proyecto primero para abrirla.",
        path.display()
    )]
    PathOutsideProject { path: PathBuf },

    #[error(
        "No encontré un editor. Configurá `editor` en ~/.config/pas/config.toml (por ejemplo `editor = \"code\"`)."
    )]
    EditorNotFound,

    #[error("No encontré `xdg-open` en el PATH para abrir el gestor de archivos.")]
    FileManagerNotFound,

    #[error(
        "No encontré una terminal del sistema en el PATH (`x-terminal-emulator`, `kgx`, `gnome-terminal`, `kitty`, `konsole`)."
    )]
    TerminalNotFound,

    #[error("No se pudo abrir {}: {cause}.", program.display())]
    LaunchFailed { program: PathBuf, cause: String },
}

pub fn open_error_to_rpc(err: OpenError) -> RpcError {
    let code = match err {
        OpenError::PathNotFound { .. } => ERR_PATH_NOT_FOUND,
        OpenError::PathOutsideProject { .. } => ERR_PATH_OUTSIDE_PROJECT,
        OpenError::EditorNotFound => ERR_EDITOR_NOT_FOUND,
        OpenError::FileManagerNotFound
        | OpenError::TerminalNotFound
        | OpenError::LaunchFailed { .. } => ERR_OPEN_FAILED,
    };
    RpcError {
        code,
        message: err.to_string(),
    }
}

pub fn open_path(
    params: OpenPathParams,
    configured_editor: Option<&str>,
    allowed_roots: &[PathBuf],
    env: &OpenEnv,
    launcher: &dyn Launcher,
) -> Result<(), OpenError> {
    let expanded = expand_tilde_with_home(&params.path, env.home_dir.as_deref());
    let canonical = fs::canonicalize(&expanded).map_err(|_| OpenError::PathNotFound {
        path: params.path.clone(),
    })?;

    let is_allowed = allowed_roots.iter().any(|root| {
        fs::canonicalize(root)
            .map(|canon_root| canonical.starts_with(&canon_root))
            .unwrap_or(false)
    });
    if !is_allowed {
        return Err(OpenError::PathOutsideProject { path: canonical });
    }

    let path_env = env.path_env.as_deref();
    let req = match params.target {
        OpenTarget::Editor => {
            let program = resolve_editor(configured_editor, path_env)?;
            let is_code = program
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|name| name == "code");
            let args = if is_code && let Some(line) = params.line {
                let mut loc = canonical.as_os_str().to_os_string();
                loc.push(format!(":{line}"));
                vec![OsString::from("-g"), loc]
            } else {
                vec![canonical.into_os_string()]
            };
            LaunchRequest {
                program,
                args,
                cwd: None,
            }
        }
        OpenTarget::Files => {
            let program =
                find_in_path("xdg-open", path_env).ok_or(OpenError::FileManagerNotFound)?;
            let dir = target_dir(&canonical);
            LaunchRequest {
                program,
                args: vec![dir.into_os_string()],
                cwd: None,
            }
        }
        OpenTarget::Terminal => {
            let program = DEFAULT_TERMINALS
                .iter()
                .find_map(|bin| find_in_path(bin, path_env))
                .ok_or(OpenError::TerminalNotFound)?;
            let dir = target_dir(&canonical);
            LaunchRequest {
                program,
                args: Vec::new(),
                cwd: Some(dir),
            }
        }
    };

    let program = req.program.clone();
    launcher.launch(req).map_err(|err| OpenError::LaunchFailed {
        program,
        cause: err.to_string(),
    })
}

pub fn open_exists(params: OpenExistsParams, env: &OpenEnv) -> OpenExistsResult {
    if params.path.is_empty() {
        return OpenExistsResult {
            exists: false,
            resolved: None,
            is_dir: false,
        };
    }

    let expanded = expand_tilde_with_home(Path::new(&params.path), env.home_dir.as_deref());
    let candidate = if expanded.is_absolute() {
        expanded
    } else {
        params.cwd.join(expanded)
    };

    match fs::canonicalize(&candidate) {
        Ok(canon) => {
            let is_dir = fs::metadata(&canon).map(|m| m.is_dir()).unwrap_or(false);
            OpenExistsResult {
                exists: true,
                resolved: Some(canon),
                is_dir,
            }
        }
        Err(_) => OpenExistsResult {
            exists: false,
            resolved: None,
            is_dir: false,
        },
    }
}

pub(crate) fn expand_tilde_with_home(path: &Path, home_dir: Option<&Path>) -> PathBuf {
    let s = path.to_string_lossy();
    let resolve_home = || {
        home_dir.map(Path::to_path_buf).or_else(|| {
            std::env::var_os("HOME")
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        })
    };

    if s == "~" {
        if let Some(home) = resolve_home() {
            return home;
        }
    } else if let Some(rest) = s.strip_prefix("~/")
        && let Some(home) = resolve_home()
    {
        return home.join(rest);
    }
    path.to_path_buf()
}

fn target_dir(canonical: &Path) -> PathBuf {
    if canonical.is_dir() {
        canonical.to_path_buf()
    } else {
        canonical
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| canonical.to_path_buf())
    }
}

fn resolve_editor(
    configured_editor: Option<&str>,
    path_env: Option<&OsStr>,
) -> Result<PathBuf, OpenError> {
    if let Some(ed) = configured_editor.map(str::trim).filter(|s| !s.is_empty()) {
        return find_program(ed, path_env).ok_or(OpenError::EditorNotFound);
    }
    DEFAULT_EDITORS
        .iter()
        .find_map(|bin| find_in_path(bin, path_env))
        .ok_or(OpenError::EditorNotFound)
}

fn find_program(cmd: &str, path_env: Option<&OsStr>) -> Option<PathBuf> {
    let as_path = Path::new(cmd);
    if cmd.contains('/') {
        return is_executable(as_path).then(|| as_path.to_path_buf());
    }
    find_in_path(cmd, path_env)
}

fn find_in_path(bin: &str, path_env: Option<&OsStr>) -> Option<PathBuf> {
    let path_value: Option<OsString> = match path_env {
        Some(p) => Some(p.to_os_string()),
        None => std::env::var_os("PATH"),
    };
    let paths = path_value?;
    for dir in std::env::split_paths(&paths) {
        if dir.as_os_str().is_empty() {
            continue;
        }
        let candidate = dir.join(bin);
        if is_executable(&candidate) {
            return Some(candidate);
        }
    }
    None
}

fn is_executable(path: &Path) -> bool {
    fs::metadata(path)
        .map(|m| m.is_file() && (m.permissions().mode() & 0o111) != 0)
        .unwrap_or(false)
}

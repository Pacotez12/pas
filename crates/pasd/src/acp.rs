use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};

use pas_core::protocol::{AcpAdapterStatus, AgentKind};
use tokio::process::Command;

use crate::chat::summarize_stderr;

pub const CLAUDE_ACP_BIN: &str = "claude-agent-acp";
pub const CODEX_ACP_BIN: &str = "codex-acp";
pub const AGY_ACP_BIN: &str = "agy";

pub const CLAUDE_ACP_PACKAGE: &str = "@agentclientprotocol/claude-agent-acp";
pub const CODEX_ACP_PACKAGE: &str = "@agentclientprotocol/codex-acp";

#[derive(Debug, Clone, Default)]
pub struct AcpEnv {
    pub data_dir: Option<PathBuf>,
    pub path_env: Option<OsString>,
    pub npm_bin: Option<OsString>,
}

#[derive(Debug, thiserror::Error)]
pub enum AcpError {
    #[error("{}", missing_adapter_message(*.0))]
    MissingAdapter(AgentKind),

    #[error(
        "agy ya trae su adaptador; no hace falta instalarlo aparte. Asegurate de tener `agy` en el PATH."
    )]
    BuiltInAdapter,

    #[error("No hay una carpeta de datos configurada para instalar adaptadores.")]
    NoDataDir,

    #[error("{0}")]
    InstallFailed(String),

    #[error("{0}")]
    Io(#[from] std::io::Error),
}

pub fn adapter_bin(agent: AgentKind) -> &'static str {
    match agent {
        AgentKind::Claude => CLAUDE_ACP_BIN,
        AgentKind::Codex => CODEX_ACP_BIN,
        AgentKind::Agy => AGY_ACP_BIN,
    }
}

pub fn adapter_package(agent: AgentKind) -> Option<&'static str> {
    match agent {
        AgentKind::Claude => Some(CLAUDE_ACP_PACKAGE),
        AgentKind::Codex => Some(CODEX_ACP_PACKAGE),
        AgentKind::Agy => None,
    }
}

pub fn missing_adapter_message(agent: AgentKind) -> String {
    match agent {
        AgentKind::Claude => {
            "El chat de Claude necesita el adaptador claude-agent-acp. Instalalo desde Cuentas o abrí la sesión en modo Terminal.".into()
        }
        AgentKind::Codex => {
            "El chat de Codex necesita el adaptador codex-acp. Instalalo desde Cuentas o abrí la sesión en modo Terminal.".into()
        }
        AgentKind::Agy => {
            "El chat de agy necesita el binario agy en el PATH. Instalá agy o abrí la sesión en modo Terminal.".into()
        }
    }
}

pub fn find_adapter(
    agent: AgentKind,
    data_dir: Option<&Path>,
    path_env: Option<&OsStr>,
) -> Option<PathBuf> {
    let bin = adapter_bin(agent);

    if matches!(agent, AgentKind::Claude | AgentKind::Codex)
        && let Some(dir) = data_dir
    {
        let local = dir.join("acp").join("node_modules").join(".bin").join(bin);
        if is_executable(&local) {
            return Some(local);
        }
    }

    let path_value: Option<OsString> = match path_env {
        Some(p) => Some(p.to_os_string()),
        None => std::env::var_os("PATH"),
    };
    if let Some(paths) = path_value {
        for dir in std::env::split_paths(&paths) {
            if dir.as_os_str().is_empty() {
                continue;
            }
            let candidate = dir.join(bin);
            if is_executable(&candidate) {
                return Some(candidate);
            }
        }
    }

    None
}

pub fn adapter_status(
    agent: AgentKind,
    data_dir: Option<&Path>,
    path_env: Option<&OsStr>,
) -> AcpAdapterStatus {
    let path = find_adapter(agent, data_dir, path_env);
    AcpAdapterStatus {
        agent,
        bin: adapter_bin(agent).to_string(),
        installed: path.is_some(),
        path,
        package: adapter_package(agent).map(str::to_string),
    }
}

pub fn status(env: &AcpEnv) -> Vec<AcpAdapterStatus> {
    [AgentKind::Claude, AgentKind::Codex, AgentKind::Agy]
        .into_iter()
        .map(|agent| adapter_status(agent, env.data_dir.as_deref(), env.path_env.as_deref()))
        .collect()
}

pub async fn install(agent: AgentKind, env: &AcpEnv) -> Result<AcpAdapterStatus, AcpError> {
    let Some(package) = adapter_package(agent) else {
        return Err(AcpError::BuiltInAdapter);
    };
    let Some(data_dir) = env.data_dir.as_deref() else {
        return Err(AcpError::NoDataDir);
    };

    let acp_dir = data_dir.join("acp");
    fs::create_dir_all(&acp_dir)?;

    let npm_program = resolve_npm_program(env.npm_bin.as_deref());
    let output = Command::new(&npm_program)
        .args(["install", "--prefix"])
        .arg(&acp_dir)
        .arg(package)
        .output()
        .await
        .map_err(|err| {
            AcpError::InstallFailed(format!(
                "No se pudo ejecutar `{}` para instalar {package}: {err}. Revisá que Node.js y npm estén instalados en el PATH.",
                npm_program.to_string_lossy()
            ))
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let raw = if !stderr.trim().is_empty() {
            stderr.as_ref()
        } else {
            stdout.as_ref()
        };
        let summary = summarize_stderr(raw);
        let code_suffix = match output.status.code() {
            Some(c) => format!(" (código {c})"),
            None => String::new(),
        };
        let detail = if summary.is_empty() {
            format!(
                "Falló la instalación de {package}{code_suffix}. Revisá tu conexión y probá correr `npm install --prefix {} {package}` manualmente.",
                acp_dir.display()
            )
        } else {
            format!(
                "Falló la instalación de {package}{code_suffix}: {summary}. Revisá tu conexión y probá correr `npm install --prefix {} {package}` manualmente.",
                acp_dir.display()
            )
        };
        return Err(AcpError::InstallFailed(detail));
    }

    Ok(adapter_status(
        agent,
        Some(data_dir),
        env.path_env.as_deref(),
    ))
}

pub fn resolve_adapter_command(
    command: &[String],
    agent_kind: Option<AgentKind>,
    env: &AcpEnv,
) -> Result<(Vec<String>, Option<AgentKind>), AcpError> {
    let target_agent = if command.is_empty() {
        agent_kind
    } else if command.len() == 1 {
        let base = Path::new(&command[0])
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(command[0].as_str());
        match base {
            "claude" | CLAUDE_ACP_BIN => Some(AgentKind::Claude),
            "codex" | CODEX_ACP_BIN => Some(AgentKind::Codex),
            "agy" => Some(AgentKind::Agy),
            _ => None,
        }
    } else {
        None
    };

    if let Some(kind) = target_agent {
        let Some(path) = find_adapter(kind, env.data_dir.as_deref(), env.path_env.as_deref())
        else {
            return Err(AcpError::MissingAdapter(kind));
        };
        let resolved = match kind {
            AgentKind::Claude | AgentKind::Codex => vec![path.to_string_lossy().into_owned()],
            AgentKind::Agy => vec![path.to_string_lossy().into_owned(), "acp".into()],
        };
        return Ok((resolved, Some(kind)));
    }

    Ok((command.to_vec(), agent_kind))
}

fn resolve_npm_program(override_bin: Option<&OsStr>) -> OsString {
    if let Some(bin) = override_bin
        && !bin.is_empty()
    {
        return bin.to_os_string();
    }
    if let Some(env_bin) = std::env::var_os("PAS_NPM").filter(|v| !v.is_empty()) {
        return env_bin;
    }
    OsString::from("npm")
}

fn is_executable(path: &Path) -> bool {
    let Ok(meta) = fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn make_exec(path: &Path) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, "#!/bin/sh\nexit 0\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    #[test]
    fn prefers_data_dir_over_path_and_falls_back_to_path() {
        let dir = tempdir().unwrap();
        let data_dir = dir.path().join("data");
        let path_dir = dir.path().join("bin");
        let path_codex = path_dir.join(CODEX_ACP_BIN);
        make_exec(&path_codex);

        let env = AcpEnv {
            data_dir: Some(data_dir.clone()),
            path_env: Some(path_dir.as_os_str().to_os_string()),
            npm_bin: None,
        };

        // Sin binario local, cae a PATH.
        let found = find_adapter(
            AgentKind::Codex,
            env.data_dir.as_deref(),
            env.path_env.as_deref(),
        );
        assert_eq!(found, Some(path_codex));

        // Con binario local, prioriza <data_dir>/acp/node_modules/.bin/.
        let local_codex = data_dir
            .join("acp")
            .join("node_modules")
            .join(".bin")
            .join(CODEX_ACP_BIN);
        make_exec(&local_codex);

        let found_local = find_adapter(
            AgentKind::Codex,
            env.data_dir.as_deref(),
            env.path_env.as_deref(),
        );
        assert_eq!(found_local, Some(local_codex));
    }

    #[test]
    fn agy_resolves_only_from_path_with_acp_subcommand() {
        let dir = tempdir().unwrap();
        let data_dir = dir.path().join("data");
        let local_agy = data_dir
            .join("acp")
            .join("node_modules")
            .join(".bin")
            .join(AGY_ACP_BIN);
        make_exec(&local_agy);

        let env_no_path = AcpEnv {
            data_dir: Some(data_dir.clone()),
            path_env: Some(OsString::new()),
            npm_bin: None,
        };
        assert_eq!(
            find_adapter(
                AgentKind::Agy,
                env_no_path.data_dir.as_deref(),
                env_no_path.path_env.as_deref()
            ),
            None
        );

        let path_dir = dir.path().join("bin");
        let path_agy = path_dir.join(AGY_ACP_BIN);
        make_exec(&path_agy);
        let env_with_path = AcpEnv {
            data_dir: Some(data_dir),
            path_env: Some(path_dir.as_os_str().to_os_string()),
            npm_bin: None,
        };
        let (cmd, kind) =
            resolve_adapter_command(&["agy".into()], Some(AgentKind::Agy), &env_with_path).unwrap();
        assert_eq!(kind, Some(AgentKind::Agy));
        assert_eq!(
            cmd,
            vec![path_agy.to_string_lossy().into_owned(), "acp".into()]
        );
    }
}

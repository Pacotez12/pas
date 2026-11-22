use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use pas_core::protocol::AgentKind;
use regex::Regex;
use serde::{Deserialize, Serialize};

pub const DEFAULT_QUOTA_WARN_PERCENT: f32 = 80.0;

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error(
        "No se pudo leer la configuración en {}: {cause}. Revisá los permisos del archivo.",
        path.display()
    )]
    Io { path: PathBuf, cause: String },

    #[error(
        "El archivo {} no tiene formato TOML válido: {cause}. Corregilo y volvé a intentar.",
        path.display()
    )]
    InvalidToml { path: PathBuf, cause: String },

    #[error(
        "El patrón de espera `{pattern}` para {agent} no es una expresión regular válida: {cause}. Corregilo en config.toml."
    )]
    InvalidPattern {
        agent: String,
        pattern: String,
        cause: String,
    },
}

fn default_true() -> bool {
    true
}

fn default_quota_warn_percent() -> f32 {
    DEFAULT_QUOTA_WARN_PERCENT
}

pub fn default_waiting_patterns() -> BTreeMap<String, Vec<String>> {
    let mut map = BTreeMap::new();
    map.insert(
        AgentKind::Agy.as_str().to_string(),
        vec![
            r"\?\s*$".to_string(),
            r"(?i)(\[[ys]/n\]|\([ys]/n\))\s*$".to_string(),
        ],
    );
    map
}

pub fn default_busy_patterns() -> BTreeMap<String, Vec<String>> {
    let mut map = BTreeMap::new();
    map.insert(
        AgentKind::Agy.as_str().to_string(),
        vec!["esc to cancel".to_string()],
    );
    map
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_true")]
    pub notifications: bool,

    #[serde(default = "default_quota_warn_percent")]
    pub quota_warn_percent: f32,

    #[serde(default)]
    pub waiting_patterns: BTreeMap<String, Vec<String>>,

    #[serde(default)]
    pub busy_patterns: BTreeMap<String, Vec<String>>,

    #[serde(default)]
    pub worktrees_dir: Option<PathBuf>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            notifications: true,
            quota_warn_percent: DEFAULT_QUOTA_WARN_PERCENT,
            waiting_patterns: default_waiting_patterns(),
            busy_patterns: default_busy_patterns(),
            worktrees_dir: None,
        }
    }
}

#[derive(Debug, Deserialize)]
struct RawConfig {
    #[serde(default = "default_true")]
    notifications: bool,

    #[serde(default = "default_quota_warn_percent")]
    quota_warn_percent: f32,

    #[serde(default)]
    waiting_patterns: BTreeMap<String, Vec<String>>,

    #[serde(default)]
    busy_patterns: BTreeMap<String, Vec<String>>,

    #[serde(default)]
    worktrees_dir: Option<PathBuf>,
}

impl Config {
    pub fn default_path() -> Option<PathBuf> {
        if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
            return Some(PathBuf::from(xdg).join("pas").join("config.toml"));
        }
        std::env::var_os("HOME")
            .filter(|v| !v.is_empty())
            .map(|home| {
                PathBuf::from(home)
                    .join(".config")
                    .join("pas")
                    .join("config.toml")
            })
    }

    pub fn load_default() -> Result<Self, ConfigError> {
        match Self::default_path() {
            Some(path) => Self::load_from_path(&path),
            None => Ok(Self::default()),
        }
    }

    pub fn load_from_path(path: &Path) -> Result<Self, ConfigError> {
        let raw_text = match fs::read_to_string(path) {
            Ok(s) => s,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(err) => {
                return Err(ConfigError::Io {
                    path: path.to_path_buf(),
                    cause: err.to_string(),
                });
            }
        };

        let raw: RawConfig = toml::from_str(&raw_text).map_err(|err| ConfigError::InvalidToml {
            path: path.to_path_buf(),
            cause: err.to_string(),
        })?;

        let mut waiting_patterns = default_waiting_patterns();
        for (agent, patterns) in raw.waiting_patterns {
            waiting_patterns.insert(agent, patterns);
        }

        let mut busy_patterns = default_busy_patterns();
        for (agent, patterns) in raw.busy_patterns {
            busy_patterns.insert(agent, patterns);
        }

        let cfg = Self {
            notifications: raw.notifications,
            quota_warn_percent: raw.quota_warn_percent,
            waiting_patterns,
            busy_patterns,
            worktrees_dir: raw.worktrees_dir,
        };
        cfg.validate()?;
        Ok(cfg)
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        validate_pattern_map(&self.waiting_patterns)?;
        validate_pattern_map(&self.busy_patterns)?;
        Ok(())
    }

    pub fn compiled_waiting_patterns(&self, agent: AgentKind) -> Result<Vec<Regex>, ConfigError> {
        compile_patterns_for(&self.waiting_patterns, agent)
    }

    pub fn compiled_busy_patterns(&self, agent: AgentKind) -> Result<Vec<Regex>, ConfigError> {
        compile_patterns_for(&self.busy_patterns, agent)
    }
}

fn validate_pattern_map(map: &BTreeMap<String, Vec<String>>) -> Result<(), ConfigError> {
    for (agent, patterns) in map {
        for pattern in patterns {
            Regex::new(pattern).map_err(|err| ConfigError::InvalidPattern {
                agent: agent.clone(),
                pattern: pattern.clone(),
                cause: err.to_string(),
            })?;
        }
    }
    Ok(())
}

fn compile_patterns_for(
    map: &BTreeMap<String, Vec<String>>,
    agent: AgentKind,
) -> Result<Vec<Regex>, ConfigError> {
    let key = agent.as_str();
    let Some(patterns) = map.get(key) else {
        return Ok(Vec::new());
    };
    let mut compiled = Vec::with_capacity(patterns.len());
    for pattern in patterns {
        let re = Regex::new(pattern).map_err(|err| ConfigError::InvalidPattern {
            agent: key.to_string(),
            pattern: pattern.clone(),
            cause: err.to_string(),
        })?;
        compiled.push(re);
    }
    Ok(compiled)
}

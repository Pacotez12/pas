use std::ffi::OsStr;
use std::fs::{self, OpenOptions, Permissions};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use pas_core::protocol::AgentKind;
use serde_json::{Map, Value, json};

pub const ENV_PAS_BIN: &str = "PAS_BIN";

const CLAUDE_EVENTS: &[&str] = &[
    "UserPromptSubmit",
    "PreToolUse",
    "PermissionRequest",
    "Notification",
    "Stop",
];

const CODEX_EVENTS: &[(&str, &str)] = &[
    ("UserPromptSubmit", "user_prompt_submit"),
    ("PreToolUse", "pre_tool_use"),
    ("PermissionRequest", "permission_request"),
    ("Stop", "stop"),
];

const AGY_SIMPLE_EVENTS: &[&str] = &["PreInvocation", "PostInvocation", "Stop"];
const AGY_MATCHER_EVENTS: &[&str] = &["PreToolUse"];
const AGY_GROUP_NAME: &str = "pas";

#[derive(Debug, thiserror::Error)]
pub enum HooksInstallError {
    #[error(
        "El archivo {} no es JSON válido. Corregilo o restauralo desde su backup y volvé a intentar.",
        path.display()
    )]
    InvalidJson { path: PathBuf },

    #[error(
        "No se encontró el binario de `pas`. Verificá que `pas` esté instalado en el PATH o junto a `pasd`."
    )]
    PasBinNotFound,

    #[error(
        "Error accediendo a archivos de configuración: {0}. Revisá los permisos de la carpeta."
    )]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileSnapshot {
    pub path: PathBuf,
    pub original_bytes: Option<Vec<u8>>,
    pub installed_bytes: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TargetKind {
    ClaudeSettings,
    CodexHooks,
    AgyHooks,
    AgySettings,
}

#[derive(Debug, Clone)]
struct TargetFile {
    path: PathBuf,
    kind: TargetKind,
    create_if_missing: bool,
}

struct LoadedTarget {
    target: TargetFile,
    original_bytes: Option<Vec<u8>>,
    original_mode: Option<u32>,
    value: Value,
}

pub fn find_pas_bin() -> Result<PathBuf, HooksInstallError> {
    if let Some(custom) = std::env::var_os(ENV_PAS_BIN).filter(|v| !v.is_empty()) {
        let p = PathBuf::from(custom);
        return Ok(fs::canonicalize(&p).unwrap_or(p));
    }

    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        let sibling = dir.join("pas");
        if sibling.is_file() {
            return Ok(fs::canonicalize(&sibling).unwrap_or(sibling));
        }
        // En tests de integración (`target/debug/deps/<test>`), el binario vive en `target/debug/pas`.
        if let Some(parent) = dir.parent() {
            let candidate = parent.join("pas");
            if candidate.is_file() {
                return Ok(fs::canonicalize(&candidate).unwrap_or(candidate));
            }
        }
    }

    if let Some(path_var) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let candidate = dir.join("pas");
            if candidate.is_file() {
                return Ok(fs::canonicalize(&candidate).unwrap_or(candidate));
            }
        }
    }

    Err(HooksInstallError::PasBinNotFound)
}

pub fn is_installed(agent: AgentKind, config_dir: &Path) -> bool {
    let targets = resolve_targets(agent, config_dir);
    let Some(primary) = targets.first() else {
        return false;
    };
    let Ok(bytes) = fs::read(&primary.path) else {
        return false;
    };
    let Ok(val) = serde_json::from_slice::<Value>(&bytes) else {
        return false;
    };
    let Some(root) = val.as_object() else {
        return false;
    };

    match primary.kind {
        TargetKind::ClaudeSettings => CLAUDE_EVENTS
            .iter()
            .all(|ev| has_pas_hook_in_event_map(root, ev, AgentKind::Claude, ev)),
        TargetKind::CodexHooks => CODEX_EVENTS
            .iter()
            .all(|(key, ev)| has_pas_hook_in_event_map(root, key, AgentKind::Codex, ev)),
        TargetKind::AgyHooks => {
            let Some(group) = root.get(AGY_GROUP_NAME).and_then(Value::as_object) else {
                return false;
            };
            let simple_ok = AGY_SIMPLE_EVENTS.iter().all(|ev| {
                group.get(*ev).and_then(Value::as_array).is_some_and(|arr| {
                    arr.iter()
                        .any(|item| is_pas_command_obj(item, AgentKind::Agy, ev))
                })
            });
            let matcher_ok = AGY_MATCHER_EVENTS.iter().all(|ev| {
                group.get(*ev).and_then(Value::as_array).is_some_and(|arr| {
                    arr.iter()
                        .any(|item| group_contains_pas_hook(item, AgentKind::Agy, ev))
                })
            });
            simple_ok && matcher_ok
        }
        TargetKind::AgySettings => false,
    }
}

pub fn install(
    agent: AgentKind,
    config_dir: &Path,
    pas_bin: &Path,
) -> Result<Vec<FileSnapshot>, HooksInstallError> {
    let targets = resolve_targets(agent, config_dir);
    let mut loaded = load_and_validate_targets(&targets)?;

    let mut pending: Vec<(LoadedTarget, Vec<u8>)> = Vec::new();
    for mut item in loaded.drain(..) {
        if item.original_bytes.is_none() && !item.target.create_if_missing {
            continue;
        }
        let changed = match item.target.kind {
            TargetKind::ClaudeSettings => {
                let h = ensure_claude_hooks(&mut item.value, pas_bin);
                let s = ensure_wrapped_statusline(&mut item.value, pas_bin, AgentKind::Claude);
                h || s
            }
            TargetKind::CodexHooks => ensure_codex_hooks(&mut item.value, pas_bin),
            TargetKind::AgyHooks => ensure_agy_hooks(&mut item.value, pas_bin),
            TargetKind::AgySettings => {
                ensure_wrapped_statusline(&mut item.value, pas_bin, AgentKind::Agy)
            }
        };
        if changed {
            let mut bytes = serde_json::to_vec_pretty(&item.value)
                .map_err(|e| HooksInstallError::Io(std::io::Error::other(e)))?;
            bytes.push(b'\n');
            pending.push((item, bytes));
        }
    }

    let mut snapshots = Vec::with_capacity(pending.len());
    let today = today_yyyymmdd();
    for (item, new_bytes) in pending {
        if let Some(parent) = item.target.path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent)?;
        }
        if let Some(ref orig) = item.original_bytes {
            create_backup(&item.target.path, orig, item.original_mode, &today)?;
        }
        write_atomic(&item.target.path, &new_bytes, item.original_mode)?;
        snapshots.push(FileSnapshot {
            path: item.target.path,
            original_bytes: item.original_bytes,
            installed_bytes: new_bytes,
        });
    }

    Ok(snapshots)
}

pub fn uninstall(
    agent: AgentKind,
    config_dir: &Path,
    snapshots: &[FileSnapshot],
) -> Result<(), HooksInstallError> {
    let targets = resolve_targets(agent, config_dir);
    let loaded = load_and_validate_targets(&targets)?;

    for mut item in loaded {
        let Some(ref current_bytes) = item.original_bytes else {
            continue;
        };

        if let Some(snap) = snapshots.iter().find(|s| s.path == item.target.path)
            && snap.installed_bytes == *current_bytes
        {
            match &snap.original_bytes {
                Some(orig) => write_atomic(&item.target.path, orig, item.original_mode)?,
                None => {
                    let _ = fs::remove_file(&item.target.path);
                }
            }
            continue;
        }

        let changed = match item.target.kind {
            TargetKind::ClaudeSettings => {
                let h = remove_claude_hooks(&mut item.value);
                let s = unwrap_statusline_in_value(&mut item.value, AgentKind::Claude);
                h || s
            }
            TargetKind::CodexHooks => remove_codex_hooks(&mut item.value),
            TargetKind::AgyHooks => remove_agy_hooks(&mut item.value),
            TargetKind::AgySettings => unwrap_statusline_in_value(&mut item.value, AgentKind::Agy),
        };

        if changed {
            let mut bytes = serde_json::to_vec_pretty(&item.value)
                .map_err(|e| HooksInstallError::Io(std::io::Error::other(e)))?;
            bytes.push(b'\n');
            write_atomic(&item.target.path, &bytes, item.original_mode)?;
        }
    }

    Ok(())
}

fn resolve_targets(agent: AgentKind, config_dir: &Path) -> Vec<TargetFile> {
    match agent {
        AgentKind::Claude => vec![TargetFile {
            path: config_dir.join("settings.json"),
            kind: TargetKind::ClaudeSettings,
            create_if_missing: true,
        }],
        AgentKind::Codex => vec![TargetFile {
            path: config_dir.join("hooks.json"),
            kind: TargetKind::CodexHooks,
            create_if_missing: true,
        }],
        AgentKind::Agy => {
            if config_dir.join("config").is_dir() || config_dir.join("antigravity-cli").is_dir() {
                vec![
                    TargetFile {
                        path: config_dir.join("config").join("hooks.json"),
                        kind: TargetKind::AgyHooks,
                        create_if_missing: true,
                    },
                    TargetFile {
                        path: config_dir.join("antigravity-cli").join("settings.json"),
                        kind: TargetKind::AgySettings,
                        create_if_missing: false,
                    },
                ]
            } else {
                let settings_path = if config_dir.join("settings.json").exists() {
                    config_dir.join("settings.json")
                } else if config_dir.file_name() == Some(OsStr::new("config")) {
                    config_dir
                        .parent()
                        .map(|p| p.join("antigravity-cli").join("settings.json"))
                        .unwrap_or_else(|| config_dir.join("settings.json"))
                } else {
                    config_dir.join("settings.json")
                };
                vec![
                    TargetFile {
                        path: config_dir.join("hooks.json"),
                        kind: TargetKind::AgyHooks,
                        create_if_missing: true,
                    },
                    TargetFile {
                        path: settings_path,
                        kind: TargetKind::AgySettings,
                        create_if_missing: false,
                    },
                ]
            }
        }
    }
}

fn load_and_validate_targets(
    targets: &[TargetFile],
) -> Result<Vec<LoadedTarget>, HooksInstallError> {
    let mut loaded = Vec::with_capacity(targets.len());
    for target in targets {
        match fs::read(&target.path) {
            Ok(bytes) => {
                let mode = fs::metadata(&target.path)
                    .ok()
                    .map(|m| m.permissions().mode() & 0o777);
                let value: Value =
                    serde_json::from_slice(&bytes).map_err(|_| HooksInstallError::InvalidJson {
                        path: target.path.clone(),
                    })?;
                if !value.is_object() {
                    return Err(HooksInstallError::InvalidJson {
                        path: target.path.clone(),
                    });
                }
                loaded.push(LoadedTarget {
                    target: target.clone(),
                    original_bytes: Some(bytes),
                    original_mode: mode,
                    value,
                });
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                loaded.push(LoadedTarget {
                    target: target.clone(),
                    original_bytes: None,
                    original_mode: None,
                    value: Value::Object(Map::new()),
                });
            }
            Err(err) => return Err(HooksInstallError::Io(err)),
        }
    }
    Ok(loaded)
}

fn ensure_claude_hooks(root: &mut Value, pas_bin: &Path) -> bool {
    let mut changed = false;
    let hooks = ensure_object_field(root, "hooks");
    for &event in CLAUDE_EVENTS {
        let cmd = format_hook_command(pas_bin, AgentKind::Claude, event);
        if ensure_event_array_has_pas(hooks, event, AgentKind::Claude, event, &cmd) {
            changed = true;
        }
    }
    changed
}

fn remove_claude_hooks(root: &mut Value) -> bool {
    let Some(root_obj) = root.as_object_mut() else {
        return false;
    };
    let mut changed = false;
    let mut remove_hooks_key = false;
    if let Some(hooks) = root_obj.get_mut("hooks").and_then(Value::as_object_mut) {
        for &event in CLAUDE_EVENTS {
            if remove_pas_from_event_array(hooks, event, AgentKind::Claude, event) {
                changed = true;
            }
        }
        if hooks.is_empty() {
            remove_hooks_key = true;
        }
    }
    if remove_hooks_key {
        root_obj.remove("hooks");
    }
    changed
}

fn ensure_codex_hooks(root: &mut Value, pas_bin: &Path) -> bool {
    let mut changed = false;
    let hooks = ensure_object_field(root, "hooks");
    for &(key, event) in CODEX_EVENTS {
        let cmd = format_hook_command(pas_bin, AgentKind::Codex, event);
        if ensure_event_array_has_pas(hooks, key, AgentKind::Codex, event, &cmd) {
            changed = true;
        }
    }
    changed
}

fn remove_codex_hooks(root: &mut Value) -> bool {
    let Some(root_obj) = root.as_object_mut() else {
        return false;
    };
    let mut changed = false;
    let mut remove_hooks_key = false;
    if let Some(hooks) = root_obj.get_mut("hooks").and_then(Value::as_object_mut) {
        for &(key, event) in CODEX_EVENTS {
            if remove_pas_from_event_array(hooks, key, AgentKind::Codex, event) {
                changed = true;
            }
        }
        if hooks.is_empty() {
            remove_hooks_key = true;
        }
    }
    if remove_hooks_key {
        root_obj.remove("hooks");
    }
    changed
}

fn ensure_agy_hooks(root: &mut Value, pas_bin: &Path) -> bool {
    let desired = build_agy_pas_group(pas_bin);
    let Some(root_obj) = root.as_object_mut() else {
        return false;
    };
    if root_obj.get(AGY_GROUP_NAME) == Some(&desired) {
        return false;
    }
    root_obj.insert(AGY_GROUP_NAME.into(), desired);
    true
}

fn remove_agy_hooks(root: &mut Value) -> bool {
    let Some(root_obj) = root.as_object_mut() else {
        return false;
    };
    root_obj.remove(AGY_GROUP_NAME).is_some()
}

fn build_agy_pas_group(pas_bin: &Path) -> Value {
    let mut group = Map::new();
    for &ev in AGY_SIMPLE_EVENTS {
        let cmd = format_hook_command(pas_bin, AgentKind::Agy, ev);
        group.insert(
            ev.into(),
            json!([
                {
                    "type": "command",
                    "command": cmd,
                    "timeout": 5
                }
            ]),
        );
    }
    for &ev in AGY_MATCHER_EVENTS {
        let cmd = format_hook_command(pas_bin, AgentKind::Agy, ev);
        group.insert(
            ev.into(),
            json!([
                {
                    "matcher": "*",
                    "hooks": [
                        {
                            "type": "command",
                            "command": cmd,
                            "timeout": 5
                        }
                    ]
                }
            ]),
        );
    }
    Value::Object(group)
}

fn ensure_object_field<'a>(root: &'a mut Value, key: &str) -> &'a mut Map<String, Value> {
    let obj = root.as_object_mut().expect("root validado como objeto");
    let entry = obj
        .entry(key.to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    if !entry.is_object() {
        *entry = Value::Object(Map::new());
    }
    entry.as_object_mut().expect("recién garantizado objeto")
}

fn ensure_event_array_has_pas(
    hooks: &mut Map<String, Value>,
    key: &str,
    agent: AgentKind,
    event: &str,
    command: &str,
) -> bool {
    let entry = hooks
        .entry(key.to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
    if !entry.is_array() {
        *entry = Value::Array(Vec::new());
    }
    let arr = entry.as_array_mut().expect("recién garantizado array");
    if arr
        .iter()
        .any(|item| group_contains_pas_hook(item, agent, event))
    {
        return false;
    }
    arr.push(json!({
        "hooks": [
            {
                "type": "command",
                "command": command,
                "timeout": 5
            }
        ]
    }));
    true
}

fn remove_pas_from_event_array(
    hooks: &mut Map<String, Value>,
    key: &str,
    agent: AgentKind,
    event: &str,
) -> bool {
    let Some(arr) = hooks.get_mut(key).and_then(Value::as_array_mut) else {
        return false;
    };
    let mut changed = false;
    arr.retain_mut(|item| {
        if let Some(inner) = item.get_mut("hooks").and_then(Value::as_array_mut) {
            let before = inner.len();
            inner.retain(|h| !is_pas_command_obj(h, agent, event));
            if inner.len() != before {
                changed = true;
            }
            !inner.is_empty()
        } else if is_pas_command_obj(item, agent, event) {
            changed = true;
            false
        } else {
            true
        }
    });
    if arr.is_empty() {
        hooks.remove(key);
    }
    changed
}

fn has_pas_hook_in_event_map(
    root: &Map<String, Value>,
    key: &str,
    agent: AgentKind,
    event: &str,
) -> bool {
    root.get("hooks")
        .and_then(Value::as_object)
        .and_then(|h| h.get(key))
        .and_then(Value::as_array)
        .is_some_and(|arr| {
            arr.iter()
                .any(|item| group_contains_pas_hook(item, agent, event))
        })
}

fn group_contains_pas_hook(item: &Value, agent: AgentKind, event: &str) -> bool {
    if let Some(inner) = item.get("hooks").and_then(Value::as_array) {
        return inner.iter().any(|h| is_pas_command_obj(h, agent, event));
    }
    is_pas_command_obj(item, agent, event)
}

fn is_pas_command_obj(val: &Value, agent: AgentKind, event: &str) -> bool {
    val.get("command")
        .and_then(Value::as_str)
        .is_some_and(|cmd| is_pas_hook_command(cmd, agent, event))
}

fn is_pas_hook_command(cmd: &str, agent: AgentKind, event: &str) -> bool {
    let suffix = format!(" hook {} {event}", agent.as_str());
    let Some(prefix) = cmd.strip_suffix(&suffix) else {
        return false;
    };
    let trimmed = prefix.trim().trim_matches('\'').trim_matches('"');
    Path::new(trimmed).file_name() == Some(OsStr::new("pas"))
}

fn format_hook_command(pas_bin: &Path, agent: AgentKind, event: &str) -> String {
    format!(
        "{} hook {} {event}",
        shell_quote_path(pas_bin),
        agent.as_str()
    )
}

fn ensure_wrapped_statusline(root: &mut Value, pas_bin: &Path, agent: AgentKind) -> bool {
    let Some(sl) = root.get_mut("statusLine").and_then(Value::as_object_mut) else {
        return false;
    };
    let Some(cmd) = sl.get("command").and_then(Value::as_str) else {
        return false;
    };
    let trimmed = cmd.trim();
    if trimmed.is_empty() || is_wrapped_statusline(trimmed, agent) {
        return false;
    }
    let wrapped = wrap_statusline(pas_bin, agent, trimmed);
    sl.insert("command".into(), Value::String(wrapped));
    true
}

fn unwrap_statusline_in_value(root: &mut Value, agent: AgentKind) -> bool {
    let Some(sl) = root.get_mut("statusLine").and_then(Value::as_object_mut) else {
        return false;
    };
    let Some(cmd) = sl.get("command").and_then(Value::as_str) else {
        return false;
    };
    let Some(original) = unwrap_statusline(cmd, agent) else {
        return false;
    };
    sl.insert("command".into(), Value::String(original));
    true
}

fn wrap_statusline(pas_bin: &Path, agent: AgentKind, original: &str) -> String {
    let bin = shell_quote_path(pas_bin);
    if needs_shell_wrap(original) {
        format!(
            "{bin} statusline --agent {} -- /bin/sh -c {}",
            agent.as_str(),
            single_quote(original)
        )
    } else {
        format!("{bin} statusline --agent {} -- {original}", agent.as_str())
    }
}

fn is_wrapped_statusline(cmd: &str, agent: AgentKind) -> bool {
    unwrap_statusline(cmd, agent).is_some()
}

fn unwrap_statusline(cmd: &str, agent: AgentKind) -> Option<String> {
    let marker = format!(" statusline --agent {} -- ", agent.as_str());
    let idx = cmd.find(&marker)?;
    let prefix = cmd[..idx].trim().trim_matches('\'').trim_matches('"');
    if Path::new(prefix).file_name() != Some(OsStr::new("pas")) {
        return None;
    }
    let rest = &cmd[idx + marker.len()..];
    if let Some(quoted) = rest.strip_prefix("/bin/sh -c ")
        && let Some(unescaped) = unquote_single(quoted)
    {
        return Some(unescaped);
    }
    Some(rest.to_string())
}

fn needs_shell_wrap(cmd: &str) -> bool {
    cmd.chars().any(|c| {
        matches!(
            c,
            '|' | '&' | ';' | '<' | '>' | '(' | ')' | '$' | '`' | '\\' | '"' | '\'' | '\n' | '\r'
        )
    })
}

fn shell_quote_path(path: &Path) -> String {
    let s = path.to_string_lossy();
    if s.chars().any(|c| {
        c.is_whitespace()
            || matches!(
                c,
                '|' | '&' | ';' | '<' | '>' | '(' | ')' | '$' | '`' | '\\' | '"' | '\''
            )
    }) {
        single_quote(&s)
    } else {
        s.into_owned()
    }
}

fn single_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for ch in s.chars() {
        if ch == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

fn unquote_single(s: &str) -> Option<String> {
    let inner = s.strip_prefix('\'')?.strip_suffix('\'')?;
    Some(inner.replace("'\\''", "'"))
}

fn create_backup(
    target: &Path,
    bytes: &[u8],
    mode: Option<u32>,
    today: &str,
) -> Result<PathBuf, HooksInstallError> {
    let filename = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "config".into());
    let parent = target.parent().unwrap_or_else(|| Path::new("."));

    let mut candidate = parent.join(format!("{filename}.bak-pas-{today}"));
    let mut suffix = 2u32;
    while candidate.exists() {
        candidate = parent.join(format!("{filename}.bak-pas-{today}.{suffix}"));
        suffix += 1;
    }

    write_atomic(&candidate, bytes, mode)?;
    Ok(candidate)
}

fn write_atomic(path: &Path, bytes: &[u8], mode: Option<u32>) -> Result<(), HooksInstallError> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let filename = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".into());

    let mut rand_bytes = [0u8; 6];
    getrandom::fill(&mut rand_bytes)
        .map_err(|e| HooksInstallError::Io(std::io::Error::other(e.to_string())))?;
    let suffix = format!(
        "{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        rand_bytes[0], rand_bytes[1], rand_bytes[2], rand_bytes[3], rand_bytes[4], rand_bytes[5]
    );
    let tmp_path = parent.join(format!(".{filename}.pas-tmp-{suffix}"));

    let write_res = (|| -> std::io::Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp_path)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        if let Some(m) = mode {
            fs::set_permissions(&tmp_path, Permissions::from_mode(m))?;
        }
        drop(file);
        fs::rename(&tmp_path, path)?;
        Ok(())
    })();

    if let Err(err) = write_res {
        let _ = fs::remove_file(&tmp_path);
        return Err(HooksInstallError::Io(err));
    }
    Ok(())
}

/// Fecha UTC en formato AAAAMMDD usando el algoritmo civil_from_days (sin dependencias externas).
fn today_yyyymmdd() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let days = secs.div_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}{month:02}{day:02}")
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    let y = y + if m <= 2 { 1 } else { 0 };
    (y, m, d)
}

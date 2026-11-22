use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use pas_core::protocol::{
    AccountInfo, AccountQuota, AcpAdapterStatus, AgentKind, ChatEvent, ClaudeConversation,
    CreateParams, CreateResult, GroupInfo, Mode, PermissionChoice, ProjectInfo, SessionInfo,
    SnapshotResult,
};
use tauri::State;

use crate::bridge::Bridge;

#[tauri::command]
pub async fn list_projects(bridge: State<'_, Bridge>) -> Result<Vec<ProjectInfo>, String> {
    bridge.list_projects().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn add_project(
    bridge: State<'_, Bridge>,
    path: String,
    name: Option<String>,
) -> Result<ProjectInfo, String> {
    bridge
        .add_project(PathBuf::from(path), name)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn remove_project(bridge: State<'_, Bridge>, id: String) -> Result<(), String> {
    bridge.remove_project(id).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn rename_project(
    bridge: State<'_, Bridge>,
    id: String,
    name: String,
) -> Result<ProjectInfo, String> {
    bridge
        .rename_project(id, name)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn move_project(
    bridge: State<'_, Bridge>,
    id: String,
    group_id: String,
) -> Result<ProjectInfo, String> {
    bridge
        .move_project(id, group_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn unpin_project(bridge: State<'_, Bridge>, id: String) -> Result<ProjectInfo, String> {
    bridge.unpin_project(id).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn list_groups(bridge: State<'_, Bridge>) -> Result<Vec<GroupInfo>, String> {
    bridge.list_groups().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn create_group(bridge: State<'_, Bridge>, name: String) -> Result<GroupInfo, String> {
    bridge.create_group(name).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn rename_group(
    bridge: State<'_, Bridge>,
    id: String,
    name: String,
) -> Result<GroupInfo, String> {
    bridge
        .rename_group(id, name)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn list_sessions(bridge: State<'_, Bridge>) -> Result<Vec<SessionInfo>, String> {
    bridge.list_sessions().await.map_err(|e| e.to_string())
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn create_session(
    bridge: State<'_, Bridge>,
    project_id: Option<String>,
    account_id: Option<String>,
    command: Vec<String>,
    title: Option<String>,
    cwd: String,
    cols: Option<u16>,
    rows: Option<u16>,
    mode: Option<Mode>,
) -> Result<CreateResult, String> {
    bridge
        .create_session(CreateParams {
            command,
            cwd: PathBuf::from(cwd),
            title,
            env: BTreeMap::new(),
            cols: cols.unwrap_or(120),
            rows: rows.unwrap_or(40),
            project_id,
            account_id,
            mode: mode.unwrap_or(Mode::Terminal),
        })
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn kill_session(bridge: State<'_, Bridge>, id: String) -> Result<(), String> {
    bridge.kill_session(id).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn remove_session(bridge: State<'_, Bridge>, id: String) -> Result<(), String> {
    bridge.remove_session(id).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn claude_conversations(
    bridge: State<'_, Bridge>,
    cwd: String,
    account_id: Option<String>,
) -> Result<Vec<ClaudeConversation>, String> {
    bridge
        .claude_conversations(PathBuf::from(cwd), account_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn list_accounts(bridge: State<'_, Bridge>) -> Result<Vec<AccountInfo>, String> {
    bridge.list_accounts().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn add_account(
    bridge: State<'_, Bridge>,
    agent: AgentKind,
    name: String,
    config_dir: Option<String>,
) -> Result<AccountInfo, String> {
    let dir = config_dir
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .map(PathBuf::from);
    bridge
        .add_account(agent, name, dir)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn rename_account(
    bridge: State<'_, Bridge>,
    id: String,
    name: String,
) -> Result<AccountInfo, String> {
    bridge
        .rename_account(id, name)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn remove_account(
    bridge: State<'_, Bridge>,
    id: String,
    delete_dir: bool,
) -> Result<(), String> {
    bridge
        .remove_account(id, delete_dir)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn install_account_hooks(
    bridge: State<'_, Bridge>,
    id: String,
) -> Result<AccountInfo, String> {
    bridge
        .install_account_hooks(id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn uninstall_account_hooks(
    bridge: State<'_, Bridge>,
    id: String,
) -> Result<AccountInfo, String> {
    bridge
        .uninstall_account_hooks(id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn list_quotas(bridge: State<'_, Bridge>) -> Result<Vec<AccountQuota>, String> {
    bridge.list_quotas().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn acp_status(bridge: State<'_, Bridge>) -> Result<Vec<AcpAdapterStatus>, String> {
    bridge.acp_status().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn acp_install(
    bridge: State<'_, Bridge>,
    agent: AgentKind,
) -> Result<AcpAdapterStatus, String> {
    bridge.acp_install(agent).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn chat_prompt(
    bridge: State<'_, Bridge>,
    id: String,
    text: String,
) -> Result<(), String> {
    bridge.chat_prompt(id, text).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn chat_permission(
    bridge: State<'_, Bridge>,
    id: String,
    request_id: String,
    choice: PermissionChoice,
) -> Result<(), String> {
    bridge
        .chat_permission(id, request_id, choice)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn chat_cancel(bridge: State<'_, Bridge>, id: String) -> Result<(), String> {
    bridge.chat_cancel(id).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn chat_history(bridge: State<'_, Bridge>, id: String) -> Result<Vec<ChatEvent>, String> {
    bridge.chat_history(id).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn chat_terminal_command(
    bridge: State<'_, Bridge>,
    id: String,
) -> Result<Option<Vec<String>>, String> {
    bridge
        .chat_terminal_command(id)
        .await
        .map_err(|e| e.to_string())
}

fn pas_data_dir_path() -> Result<PathBuf, String> {
    if let Some(xdg) = std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(xdg).join("pas"));
    }
    if let Some(home) = std::env::var_os("HOME").filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(home).join(".local").join("share").join("pas"));
    }
    Err("No se encontró HOME ni XDG_DATA_HOME para ubicar los datos de pas.".into())
}

#[tauri::command]
pub fn pas_data_dir() -> Result<String, String> {
    Ok(pas_data_dir_path()?.to_string_lossy().into_owned())
}

fn resolve_visual_path(session_id: &str, file: &str) -> Result<PathBuf, String> {
    if session_id.is_empty()
        || session_id.contains('/')
        || session_id.contains('\\')
        || session_id == "."
        || session_id == ".."
    {
        return Err("Identificador de sesión inválido.".into());
    }
    let rel = Path::new(file);
    if rel.is_absolute()
        || rel.components().any(|c| {
            matches!(
                c,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err("Ruta de visual inválida.".into());
    }
    let chat_dir = pas_data_dir_path()?.join("chats").join(session_id);
    let canon_dir = std::fs::canonicalize(&chat_dir)
        .map_err(|err| format!("No se encontró la carpeta del chat: {err}"))?;
    let candidate = canon_dir.join(rel);
    let canon_file = std::fs::canonicalize(&candidate)
        .map_err(|err| format!("No se encontró el visual {file}: {err}"))?;
    if !canon_file.starts_with(&canon_dir) {
        return Err("El archivo está fuera de la carpeta del chat.".into());
    }
    Ok(canon_file)
}

#[tauri::command]
pub fn chat_read_visual(session_id: String, file: String) -> Result<String, String> {
    let path = resolve_visual_path(&session_id, &file)?;
    std::fs::read_to_string(&path)
        .map_err(|err| format!("No se pudo leer el visual {file}: {err}"))
}

#[tauri::command]
pub fn chat_open_visual(session_id: String, file: String) -> Result<(), String> {
    let path = resolve_visual_path(&session_id, &file)?;
    #[cfg(target_os = "linux")]
    let status = std::process::Command::new("xdg-open").arg(&path).spawn();
    #[cfg(target_os = "macos")]
    let status = std::process::Command::new("open").arg(&path).spawn();
    #[cfg(target_os = "windows")]
    let status = std::process::Command::new("cmd")
        .args(["/C", "start", ""])
        .arg(&path)
        .spawn();
    status
        .map(|_| ())
        .map_err(|err| format!("No se pudo abrir el visual en el navegador: {err}"))
}

#[tauri::command]
pub fn default_agy_dir() -> Result<String, String> {
    if let Some(home) = std::env::var_os("HOME").filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(home)
            .join(".gemini")
            .to_string_lossy()
            .into_owned());
    }
    Err("No se encontró HOME para ubicar la carpeta de agy.".into())
}

#[tauri::command]
pub async fn session_input(
    bridge: State<'_, Bridge>,
    id: String,
    data_b64: String,
) -> Result<(), String> {
    bridge
        .session_input(id, data_b64)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn session_resize(
    bridge: State<'_, Bridge>,
    id: String,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    bridge
        .session_resize(id, cols, rows)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn session_snapshot(
    bridge: State<'_, Bridge>,
    id: String,
) -> Result<SnapshotResult, String> {
    bridge.session_snapshot(id).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn subscribe(bridge: State<'_, Bridge>, id: String) -> Result<(), String> {
    bridge.subscribe(id).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn unsubscribe(bridge: State<'_, Bridge>, id: String) -> Result<(), String> {
    bridge.unsubscribe(id).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub fn reconnect(bridge: State<'_, Bridge>) {
    bridge.reconnect();
}

fn layout_file_path() -> Result<PathBuf, String> {
    if let Some(xdg) = std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(xdg).join("pas").join("gui-layout.json"));
    }
    if let Some(home) = std::env::var_os("HOME").filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(home)
            .join(".local")
            .join("share")
            .join("pas")
            .join("gui-layout.json"));
    }
    Err("No se encontró HOME ni XDG_DATA_HOME para guardar el layout.".into())
}

#[tauri::command]
pub fn load_layout() -> Result<Option<String>, String> {
    let path = layout_file_path()?;
    match std::fs::read_to_string(&path) {
        Ok(content) => Ok(Some(content)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(format!(
            "No se pudo leer el layout guardado en {}: {err}",
            path.display()
        )),
    }
}

#[tauri::command]
pub fn save_layout(json: String) -> Result<(), String> {
    let path = layout_file_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| {
            format!(
                "No se pudo crear la carpeta {} para guardar el layout: {err}",
                parent.display()
            )
        })?;
    }
    std::fs::write(&path, json)
        .map_err(|err| format!("No se pudo guardar el layout en {}: {err}", path.display()))
}

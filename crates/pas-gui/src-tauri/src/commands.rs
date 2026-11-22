use std::collections::BTreeMap;
use std::path::PathBuf;

use pas_core::protocol::{
    CreateParams, CreateResult, GroupInfo, ProjectInfo, SessionInfo, SnapshotResult,
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
pub async fn create_session(
    bridge: State<'_, Bridge>,
    project_id: Option<String>,
    command: Vec<String>,
    title: Option<String>,
    cwd: String,
    cols: Option<u16>,
    rows: Option<u16>,
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
        })
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn kill_session(bridge: State<'_, Bridge>, id: String) -> Result<(), String> {
    bridge.kill_session(id).await.map_err(|e| e.to_string())
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
    std::fs::write(&path, json).map_err(|err| {
        format!(
            "No se pudo guardar el layout en {}: {err}",
            path.display()
        )
    })
}

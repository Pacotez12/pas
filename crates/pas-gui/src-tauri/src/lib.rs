use std::sync::Arc;

use pas_core::socket::socket_path;
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager};

use crate::bridge::{Bridge, EventSink, FRONTEND_EVENT_FOCUS_REQUEST};

pub mod bridge;
pub mod commands;

struct TauriEventSink(AppHandle);

impl EventSink for TauriEventSink {
    fn emit(&self, event: &str, payload: Value) {
        if event == FRONTEND_EVENT_FOCUS_REQUEST
            && let Some(win) = self.0.get_webview_window("main")
        {
            let _ = win.unminimize();
            let _ = win.show();
            let _ = win.set_focus();
        }
        if let Err(err) = self.0.emit(event, payload) {
            tracing::warn!("no se pudo emitir el evento {event}: {err}");
        }
    }
}

pub fn run() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .try_init();

    let sock = match socket_path() {
        Ok(path) => path,
        Err(err) => {
            eprintln!("No se pudo determinar la ruta del socket de pasd: {err}");
            std::process::exit(1);
        }
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(move |app| {
            let sink = Arc::new(TauriEventSink(app.handle().clone()));
            let bridge = Bridge::new(sock.clone(), sink);
            app.manage(bridge);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_projects,
            commands::add_project,
            commands::remove_project,
            commands::rename_project,
            commands::move_project,
            commands::unpin_project,
            commands::list_groups,
            commands::create_group,
            commands::rename_group,
            commands::list_sessions,
            commands::create_session,
            commands::kill_session,
            commands::remove_session,
            commands::claude_conversations,
            commands::list_accounts,
            commands::add_account,
            commands::rename_account,
            commands::remove_account,
            commands::install_account_hooks,
            commands::uninstall_account_hooks,
            commands::list_quotas,
            commands::acp_status,
            commands::acp_install,
            commands::chat_prompt,
            commands::chat_permission,
            commands::chat_cancel,
            commands::chat_history,
            commands::chat_terminal_command,
            commands::worktree_suggest_branch,
            commands::worktree_branches,
            commands::worktree_create,
            commands::worktree_list,
            commands::worktree_merge,
            commands::worktree_remove,
            commands::open_path,
            commands::open_exists,
            commands::discover_projects,
            commands::discover_accounts,
            commands::chat_read_visual,
            commands::chat_open_visual,
            commands::pas_data_dir,
            commands::default_agy_dir,
            commands::session_input,
            commands::session_resize,
            commands::session_snapshot,
            commands::subscribe,
            commands::unsubscribe,
            commands::reconnect,
            commands::load_layout,
            commands::save_layout,
        ])
        .run(tauri::generate_context!())
        .expect(
            "No se pudo iniciar la ventana de pas. Revisá que el entorno gráfico esté disponible.",
        );
}

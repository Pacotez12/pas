use std::fs;
use std::thread;
use std::time::Duration;

use pas_core::protocol::{ClaudeConversation, METHOD_CLAUDE_CONVERSATIONS, Request};
use pasd::manager::Manager;
use pasd::rpc::dispatch;
use serde_json::json;
use tempfile::TempDir;

#[tokio::test]
async fn claude_conversations_lists_titles_newest_first() {
    let tmp = TempDir::new().unwrap();
    let cwd = "/home/x/Proyectos/pockt";
    let proj_dir = tmp.path().join("projects").join("-home-x-Proyectos-pockt");
    fs::create_dir_all(&proj_dir).unwrap();

    // 1) Oldest: no title in first 50 lines (title on line 55 must be ignored)
    let mut no_title_lines = String::new();
    for i in 0..54 {
        no_title_lines.push_str(&format!("{{\"type\":\"user\",\"n\":{i}}}\n"));
    }
    no_title_lines.push_str("{\"type\":\"custom-title\",\"customTitle\":\"Demasiado tarde\"}\n");
    fs::write(proj_dir.join("conv-old.jsonl"), no_title_lines).unwrap();
    thread::sleep(Duration::from_millis(25));

    // 2) Middle: only aiTitle
    fs::write(
        proj_dir.join("conv-mid.jsonl"),
        "{\"type\":\"user\",\"text\":\"hola\"}\n{\"type\":\"ai-title\",\"aiTitle\":\"Claude Chat\"}\n",
    )
    .unwrap();
    thread::sleep(Duration::from_millis(25));

    // 3) Newest: both aiTitle and customTitle -> customTitle wins
    fs::write(
        proj_dir.join("conv-new.jsonl"),
        "{\"type\":\"ai-title\",\"aiTitle\":\"Título IA\"}\n{\"type\":\"custom-title\",\"customTitle\":\"App personal de gastos (2)\"}\n",
    )
    .unwrap();

    // Non-jsonl file should be ignored
    fs::write(proj_dir.join("notes.txt"), "ignorar").unwrap();

    // SAFETY: test binary dedicated to this test; no concurrent env readers.
    unsafe {
        std::env::set_var("CLAUDE_CONFIG_DIR", tmp.path());
    }

    let mgr = Manager::new(200);
    let resp = dispatch(
        &mgr,
        Request {
            jsonrpc: "2.0".into(),
            id: 1,
            method: METHOD_CLAUDE_CONVERSATIONS.into(),
            params: json!({ "cwd": cwd }),
        },
    )
    .await;

    assert!(resp.error.is_none(), "error inesperado: {:?}", resp.error);
    let convs: Vec<ClaudeConversation> = serde_json::from_value(resp.result.unwrap()).unwrap();
    assert_eq!(convs.len(), 3);
    assert_eq!(convs[0].id, "conv-new");
    assert_eq!(convs[0].title, "App personal de gastos (2)");
    assert_eq!(convs[1].id, "conv-mid");
    assert_eq!(convs[1].title, "Claude Chat");
    assert_eq!(convs[2].id, "conv-old");
    assert_eq!(convs[2].title, "Sin título");
    assert!(convs[0].modified_at >= convs[1].modified_at);
    assert!(convs[1].modified_at >= convs[2].modified_at);
}

#[tokio::test]
async fn claude_conversations_uses_account_config_dir() {
    use pas_core::protocol::{AccountInfo, ERR_ACCOUNT_NOT_FOUND, METHOD_ACCOUNT_ADD};

    let tmp_default = TempDir::new().unwrap();
    let tmp_account = TempDir::new().unwrap();
    let cwd = "/home/x/Proyectos/pockt";

    // Conversación en el CLAUDE_CONFIG_DIR por defecto (no debería aparecer al pedir la cuenta).
    let default_proj = tmp_default
        .path()
        .join("projects")
        .join("-home-x-Proyectos-pockt");
    fs::create_dir_all(&default_proj).unwrap();
    fs::write(
        default_proj.join("conv-default.jsonl"),
        "{\"type\":\"custom-title\",\"customTitle\":\"Desde default\"}\n",
    )
    .unwrap();

    // Conversación en el config_dir de la cuenta elegida.
    let account_proj = tmp_account
        .path()
        .join("projects")
        .join("-home-x-Proyectos-pockt");
    fs::create_dir_all(&account_proj).unwrap();
    fs::write(
        account_proj.join("conv-acc.jsonl"),
        "{\"type\":\"custom-title\",\"customTitle\":\"Desde cuenta\"}\n",
    )
    .unwrap();

    // SAFETY: test binary dedicated to claude conversations; no concurrent env readers.
    unsafe {
        std::env::set_var("CLAUDE_CONFIG_DIR", tmp_default.path());
    }

    let mgr = Manager::new(200);
    let add_resp = dispatch(
        &mgr,
        Request {
            jsonrpc: "2.0".into(),
            id: 1,
            method: METHOD_ACCOUNT_ADD.into(),
            params: json!({
                "agent": "claude",
                "name": "Trabajo",
                "config_dir": tmp_account.path(),
            }),
        },
    )
    .await;
    assert!(add_resp.error.is_none(), "error: {:?}", add_resp.error);
    let account: AccountInfo = serde_json::from_value(add_resp.result.unwrap()).unwrap();

    let resp = dispatch(
        &mgr,
        Request {
            jsonrpc: "2.0".into(),
            id: 2,
            method: METHOD_CLAUDE_CONVERSATIONS.into(),
            params: json!({ "cwd": cwd, "account_id": account.id }),
        },
    )
    .await;
    assert!(resp.error.is_none(), "error inesperado: {:?}", resp.error);
    let convs: Vec<ClaudeConversation> = serde_json::from_value(resp.result.unwrap()).unwrap();
    assert_eq!(convs.len(), 1);
    assert_eq!(convs[0].id, "conv-acc");
    assert_eq!(convs[0].title, "Desde cuenta");

    let missing_resp = dispatch(
        &mgr,
        Request {
            jsonrpc: "2.0".into(),
            id: 3,
            method: METHOD_CLAUDE_CONVERSATIONS.into(),
            params: json!({ "cwd": cwd, "account_id": "inexistente" }),
        },
    )
    .await;
    let err = missing_resp.error.expect("debería fallar si la cuenta no existe");
    assert_eq!(err.code, ERR_ACCOUNT_NOT_FOUND);
}

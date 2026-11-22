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

use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use pas_core::protocol::ClaudeConversation;
use serde_json::Value;

const MAX_LINES_TO_SCAN: usize = 50;

pub fn encode_cwd(cwd: &Path) -> String {
    cwd.to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

fn claude_config_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("CLAUDE_CONFIG_DIR").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    std::env::var_os("HOME")
        .filter(|v| !v.is_empty())
        .map(|home| PathBuf::from(home).join(".claude"))
}

pub fn list_conversations(cwd: &Path) -> Vec<ClaudeConversation> {
    let Some(config_dir) = claude_config_dir() else {
        return Vec::new();
    };
    list_conversations_in(&config_dir, cwd)
}

pub fn list_conversations_in(config_dir: &Path, cwd: &Path) -> Vec<ClaudeConversation> {
    let project_dir = config_dir.join("projects").join(encode_cwd(cwd));
    let Ok(entries) = fs::read_dir(&project_dir) else {
        return Vec::new();
    };

    let mut items: Vec<(SystemTime, ClaudeConversation)> = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        let Some(id) = path.file_stem().and_then(|s| s.to_str()).map(String::from) else {
            continue;
        };

        let mtime = meta.modified().unwrap_or(UNIX_EPOCH);
        let modified_at = mtime
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let title = read_conversation_title(&path).unwrap_or_else(|| "Sin título".into());

        items.push((
            mtime,
            ClaudeConversation {
                id,
                title,
                modified_at,
            },
        ));
    }

    items.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.id.cmp(&b.1.id)));
    items.into_iter().map(|(_, conv)| conv).collect()
}

fn read_conversation_title(path: &Path) -> Option<String> {
    let file = File::open(path).ok()?;
    let reader = BufReader::new(file);

    let mut custom_title: Option<String> = None;
    let mut ai_title: Option<String> = None;

    for line in reader.lines().take(MAX_LINES_TO_SCAN) {
        let Ok(line) = line else {
            break;
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(val) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };
        if let Some(t) = val
            .get("customTitle")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            custom_title = Some(t.to_string());
        }
        if let Some(t) = val
            .get("aiTitle")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            ai_title = Some(t.to_string());
        }
    }

    custom_title.or(ai_title)
}

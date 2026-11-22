use std::io::{self, Write};
use std::time::Duration;

use pas_core::CoreError;
use pas_core::client::Client;
use pas_core::protocol::{AgentKind, HookParams, METHOD_SESSION_HOOK};
use pas_core::socket::socket_path;
use serde_json::Value;

const HOOK_TIMEOUT: Duration = Duration::from_millis(300);

pub async fn run(agent: &str, event: &str) {
    // Drenar stdin en un hilo aparte sin esperarlo: si el agente deja stdin
    // abierto, el hook no se cuelga ni traba al agente.
    let _ = std::thread::Builder::new()
        .name("pas-hook-stdin".into())
        .spawn(|| {
            let _ = io::copy(&mut io::stdin().lock(), &mut io::sink());
        });

    let session_id = std::env::var("PAS_SESSION_ID")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    if let (Some(id), Some(agent_kind), Ok(path)) =
        (session_id, AgentKind::parse(agent), socket_path())
    {
        let params = HookParams {
            id,
            agent: agent_kind,
            event: event.to_string(),
        };
        let _ = tokio::time::timeout(HOOK_TIMEOUT, async move {
            let mut client = Client::connect(&path).await?;
            let _: Value = client.call(METHOD_SESSION_HOOK, params).await?;
            Ok::<(), CoreError>(())
        })
        .await;
    }

    if let Some(reply) = hook_reply(agent, event) {
        let _ = writeln!(io::stdout().lock(), "{reply}");
    }
}

/// What the agent expects on stdout. agy reads `{}` on PreToolUse as a denial
/// and wants an empty decision on Stop (same replies Orca's hooks give).
fn hook_reply(agent: &str, event: &str) -> Option<&'static str> {
    match (agent, event) {
        ("agy", "PreToolUse") => None,
        ("agy", "Stop") => Some(r#"{"decision":""}"#),
        _ => Some("{}"),
    }
}

#[cfg(test)]
mod tests {
    use super::hook_reply;

    #[test]
    fn agy_pre_tool_use_prints_nothing_so_tools_are_not_denied() {
        assert_eq!(hook_reply("agy", "PreToolUse"), None);
        assert_eq!(hook_reply("agy", "Stop"), Some(r#"{"decision":""}"#));
        assert_eq!(hook_reply("agy", "PreInvocation"), Some("{}"));
        assert_eq!(hook_reply("claude", "PreToolUse"), Some("{}"));
    }
}

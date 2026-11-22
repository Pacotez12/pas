use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use agent_client_protocol::v1::ConnectionTo;
use agent_client_protocol::v1::schema::{
    AgentCapabilities, CancelNotification, ContentBlock, ContentChunk, Diff, Implementation,
    InitializeRequest, InitializeResponse, NewSessionRequest, NewSessionResponse, PermissionOption,
    PermissionOptionKind, PromptRequest, PromptResponse, RequestPermissionOutcome,
    RequestPermissionRequest, SessionId, SessionNotification, SessionUpdate, StopReason,
    TextContent, ToolCall, ToolCallContent, ToolCallStatus, ToolCallUpdate, ToolCallUpdateFields,
    ToolKind,
};
use agent_client_protocol::{Agent, Client, Result, Stdio};
use serde::Deserialize;
use tokio::sync::watch;

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Step {
    Text {
        #[serde(default)]
        chunks: Vec<String>,
        #[serde(default)]
        text: Option<String>,
    },
    Tool {
        #[serde(default)]
        id: Option<String>,
        title: String,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        status: Option<String>,
    },
    Diff {
        #[serde(default)]
        id: Option<String>,
        #[serde(default)]
        title: Option<String>,
        path: String,
        #[serde(default)]
        old_text: Option<String>,
        new_text: String,
    },
    Permission {
        #[serde(default)]
        id: Option<String>,
        title: String,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        detail: Option<String>,
        #[serde(default)]
        after_allow: Option<String>,
        #[serde(default)]
        after_deny: Option<String>,
    },
    Visual {
        file: String,
        html: String,
        #[serde(default)]
        message: Option<String>,
    },
    Crash {
        #[serde(default = "default_crash_code")]
        code: i32,
        #[serde(default)]
        message: Option<String>,
    },
    Sleep {
        ms: u64,
    },
}

fn default_crash_code() -> i32 {
    1
}

#[derive(Default)]
struct AgentState {
    next_session_id: u64,
    next_tool_id: u64,
    sessions: HashMap<String, PathBuf>,
    cancel_senders: HashMap<String, watch::Sender<bool>>,
}

impl AgentState {
    fn next_tool_call_id(&mut self, explicit: Option<String>) -> String {
        if let Some(id) = explicit {
            return id;
        }
        self.next_tool_id += 1;
        format!("tc-{}", self.next_tool_id)
    }
}

fn load_script() -> std::result::Result<Option<Vec<Step>>, String> {
    let Ok(raw) = std::env::var("FAKE_ACP_SCRIPT") else {
        return Ok(None);
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    serde_json::from_str::<Vec<Step>>(trimmed)
        .map(Some)
        .map_err(|e| format!("invalid FAKE_ACP_SCRIPT JSON: {e}"))
}

fn parse_tool_status(raw: Option<&str>) -> ToolCallStatus {
    match raw {
        Some("pending") => ToolCallStatus::Pending,
        Some("in_progress") => ToolCallStatus::InProgress,
        Some("failed") => ToolCallStatus::Failed,
        _ => ToolCallStatus::Completed,
    }
}

fn extract_prompt_text(blocks: &[ContentBlock]) -> String {
    blocks
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text(t) => Some(t.text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn send_text_chunk(
    cx: &ConnectionTo<Client>,
    session_id: &SessionId,
    text: String,
) -> Result<()> {
    cx.send_notification(SessionNotification::new(
        session_id.clone(),
        SessionUpdate::AgentMessageChunk(ContentChunk::new(ContentBlock::Text(TextContent::new(
            text,
        )))),
    ))
}

async fn run_prompt_steps(
    cx: ConnectionTo<Client>,
    state: Arc<Mutex<AgentState>>,
    script: Option<Vec<Step>>,
    req: PromptRequest,
    mut cancel_rx: watch::Receiver<bool>,
) -> Result<StopReason> {
    let session_id = req.session_id.clone();
    let cwd = state
        .lock()
        .unwrap()
        .sessions
        .get(session_id.0.as_ref())
        .cloned()
        .unwrap_or_else(|| PathBuf::from("."));

    let steps = match script {
        Some(steps) => steps,
        None => {
            let prompt_text = extract_prompt_text(&req.prompt);
            vec![Step::Text {
                chunks: vec!["Eco: ".to_string(), prompt_text],
                text: None,
            }]
        }
    };

    for step in steps {
        if *cancel_rx.borrow() {
            return Ok(StopReason::Cancelled);
        }

        match step {
            Step::Text { chunks, text } => {
                if !chunks.is_empty() {
                    for chunk in chunks {
                        if *cancel_rx.borrow() {
                            return Ok(StopReason::Cancelled);
                        }
                        send_text_chunk(&cx, &session_id, chunk)?;
                    }
                } else if let Some(text) = text {
                    send_text_chunk(&cx, &session_id, text)?;
                }
            }
            Step::Tool {
                id,
                title,
                name,
                status,
            } => {
                let tool_id = state.lock().unwrap().next_tool_call_id(id);
                let mut tc = ToolCall::new(tool_id.clone(), title)
                    .status(ToolCallStatus::InProgress);
                if let Some(name) = name {
                    tc = tc.name(name);
                }
                cx.send_notification(SessionNotification::new(
                    session_id.clone(),
                    SessionUpdate::ToolCall(tc),
                ))?;

                let final_status = parse_tool_status(status.as_deref());
                cx.send_notification(SessionNotification::new(
                    session_id.clone(),
                    SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
                        tool_id,
                        ToolCallUpdateFields::new().status(final_status),
                    )),
                ))?;
            }
            Step::Diff {
                id,
                title,
                path,
                old_text,
                new_text,
            } => {
                let tool_id = state.lock().unwrap().next_tool_call_id(id);
                let title = title.unwrap_or_else(|| format!("Editó {path}"));
                let mut diff = Diff::new(path, new_text);
                if let Some(old_text) = old_text {
                    diff = diff.old_text(old_text);
                }
                let tc = ToolCall::new(tool_id, title)
                    .kind(ToolKind::Edit)
                    .status(ToolCallStatus::Completed)
                    .content(vec![ToolCallContent::Diff(diff)]);
                cx.send_notification(SessionNotification::new(
                    session_id.clone(),
                    SessionUpdate::ToolCall(tc),
                ))?;
            }
            Step::Permission {
                id,
                title,
                name,
                detail,
                after_allow,
                after_deny,
            } => {
                let tool_id = state.lock().unwrap().next_tool_call_id(id);
                let mut fields = ToolCallUpdateFields::new()
                    .title(title)
                    .status(ToolCallStatus::Pending);
                if let Some(name) = name {
                    fields = fields.name(name);
                }
                if let Some(detail) = detail {
                    fields = fields.raw_input(serde_json::Value::String(detail));
                }
                let options = vec![
                    PermissionOption::new(
                        "allow_once",
                        "Permitir",
                        PermissionOptionKind::AllowOnce,
                    ),
                    PermissionOption::new(
                        "allow_always",
                        "Permitir siempre en esta sesión",
                        PermissionOptionKind::AllowAlways,
                    ),
                    PermissionOption::new(
                        "reject_once",
                        "Rechazar",
                        PermissionOptionKind::RejectOnce,
                    ),
                ];
                let perm_req = RequestPermissionRequest::new(
                    session_id.clone(),
                    ToolCallUpdate::new(tool_id.clone(), fields),
                    options,
                );

                let resp = tokio::select! {
                    biased;
                    changed = cancel_rx.changed() => {
                        if changed.is_ok() && *cancel_rx.borrow() {
                            return Ok(StopReason::Cancelled);
                        }
                        return Ok(StopReason::Cancelled);
                    }
                    res = cx.send_request(perm_req).block_task() => res?,
                };

                let allowed = match resp.outcome {
                    RequestPermissionOutcome::Selected(sel) => {
                        matches!(sel.option_id.0.as_ref(), "allow_once" | "allow_always")
                    }
                    RequestPermissionOutcome::Cancelled => {
                        return Ok(StopReason::Cancelled);
                    }
                    _ => false,
                };

                let final_status = if allowed {
                    ToolCallStatus::Completed
                } else {
                    ToolCallStatus::Failed
                };
                cx.send_notification(SessionNotification::new(
                    session_id.clone(),
                    SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
                        tool_id,
                        ToolCallUpdateFields::new().status(final_status),
                    )),
                ))?;

                if allowed {
                    if let Some(msg) = after_allow {
                        send_text_chunk(&cx, &session_id, msg)?;
                    }
                } else if let Some(msg) = after_deny {
                    send_text_chunk(&cx, &session_id, msg)?;
                }
            }
            Step::Visual {
                file,
                html,
                message,
            } => {
                let base_dir = std::env::var("PAS_CHAT_DIR")
                    .ok()
                    .filter(|s| !s.trim().is_empty())
                    .map(PathBuf::from)
                    .unwrap_or_else(|| cwd.clone());
                let target = base_dir.join(&file);
                if let Some(parent) = target.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                std::fs::write(&target, html).map_err(|e| {
                    agent_client_protocol::Error::internal_error()
                        .data(format!("failed to write visual {}: {e}", target.display()))
                })?;
                let text = message.unwrap_or_else(|| format!("Visual: {file}"));
                send_text_chunk(&cx, &session_id, text)?;
            }
            Step::Crash { code, message } => {
                let msg = message.unwrap_or_else(|| "fake-acp-agent crash".to_string());
                let _ = writeln!(std::io::stderr(), "{msg}");
                let _ = std::io::stderr().flush();
                std::process::exit(code);
            }
            Step::Sleep { ms } => {
                tokio::select! {
                    biased;
                    changed = cancel_rx.changed() => {
                        if changed.is_ok() && *cancel_rx.borrow() {
                            return Ok(StopReason::Cancelled);
                        }
                    }
                    () = tokio::time::sleep(Duration::from_millis(ms)) => {}
                }
            }
        }
    }

    Ok(StopReason::EndTurn)
}

#[tokio::main]
async fn main() -> Result<()> {
    let script = match load_script() {
        Ok(script) => script,
        Err(err) => {
            eprintln!("{err}");
            std::process::exit(2);
        }
    };

    let state = Arc::new(Mutex::new(AgentState::default()));
    let state_new = Arc::clone(&state);
    let state_cancel = Arc::clone(&state);
    let state_prompt = Arc::clone(&state);

    Agent
        .v1()
        .name("fake-acp-agent")
        .on_receive_request(
            async move |init: InitializeRequest, responder, _cx| {
                responder.respond(
                    InitializeResponse::new(init.protocol_version)
                        .agent_capabilities(AgentCapabilities::new())
                        .agent_info(Implementation::new(
                            "fake-acp-agent",
                            env!("CARGO_PKG_VERSION"),
                        )),
                )
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |req: NewSessionRequest, responder, _cx| {
                let session_id = {
                    let mut st = state_new.lock().unwrap();
                    st.next_session_id += 1;
                    let id = format!("fake-session-{}", st.next_session_id);
                    st.sessions.insert(id.clone(), req.cwd);
                    id
                };
                responder.respond(NewSessionResponse::new(session_id))
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_notification(
            async move |notif: CancelNotification, _cx| {
                let sender = state_cancel
                    .lock()
                    .unwrap()
                    .cancel_senders
                    .get(notif.session_id.0.as_ref())
                    .cloned();
                if let Some(tx) = sender {
                    let _ = tx.send(true);
                }
                Ok(())
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .on_receive_request(
            async move |req: PromptRequest, responder, cx| {
                let (cancel_tx, cancel_rx) = watch::channel(false);
                let session_key = req.session_id.0.to_string();
                state_prompt
                    .lock()
                    .unwrap()
                    .cancel_senders
                    .insert(session_key.clone(), cancel_tx);

                let state_task = Arc::clone(&state_prompt);
                let script_task = script.clone();
                let cx_task = cx.clone();
                cx.spawn(async move {
                    let res =
                        run_prompt_steps(cx_task, Arc::clone(&state_task), script_task, req, cancel_rx)
                            .await;
                    state_task.lock().unwrap().cancel_senders.remove(&session_key);
                    match res {
                        Ok(stop_reason) => responder.respond(PromptResponse::new(stop_reason)),
                        Err(err) => responder.respond_with_error(err),
                    }
                })
            },
            agent_client_protocol::on_receive_request!(),
        )
        .connect_to(Stdio::new())
        .await
}

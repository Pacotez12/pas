use std::collections::{HashMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::v1::ConnectionTo;
use agent_client_protocol::v1::schema::{
    CancelNotification, ContentBlock, InitializeRequest, NewSessionRequest, PermissionOption,
    PermissionOptionKind, PromptRequest, RequestPermissionOutcome, RequestPermissionRequest,
    RequestPermissionResponse, SelectedPermissionOutcome, SessionNotification, SessionUpdate,
    TextContent, ToolCallContent, ToolCallStatus,
};
use agent_client_protocol::{
    Agent, ByteStreams, Client, is_connection_terminated, is_incoming_transport_closed,
};
use pas_core::protocol::{
    AgentKind, ChatEvent, ChatEventNotification, CreateParams, EVENT_CHAT_EVENT, Notification,
    PermissionChoice, SessionState,
};
use serde_json::{Value, json};
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::sync::{broadcast, mpsc, oneshot, watch};
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

use crate::session::SessionError;

const STDERR_LIMIT_BYTES: usize = 8 * 1024;
const STDERR_SUMMARY_CHARS: usize = 400;

pub fn resolve_adapter_command(command: &[String], agent_kind: Option<AgentKind>) -> Vec<String> {
    if command.is_empty() {
        return match agent_kind {
            Some(AgentKind::Claude) => vec!["claude-agent-acp".into()],
            Some(AgentKind::Codex) => vec!["codex-acp".into()],
            Some(AgentKind::Agy) => vec!["agy".into(), "acp".into()],
            None => Vec::new(),
        };
    }

    if command.len() == 1 {
        let base = Path::new(&command[0])
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(command[0].as_str());
        match base {
            "claude" => return vec!["claude-agent-acp".into()],
            "codex" => return vec!["codex-acp".into()],
            "agy" => return vec!["agy".into(), "acp".into()],
            _ => {}
        }
    }

    command.to_vec()
}

enum ChatCommand {
    Prompt { text: String },
    Cancel,
    Kill,
}

struct PendingPermission {
    title: String,
    tx: oneshot::Sender<PermissionChoice>,
}

#[derive(Clone)]
struct EventSink {
    session_id: String,
    history: Arc<Mutex<Vec<ChatEvent>>>,
    history_file: Option<PathBuf>,
    events_tx: broadcast::Sender<Notification>,
}

impl EventSink {
    fn emit(&self, event: ChatEvent) {
        if let Ok(mut guard) = self.history.lock() {
            guard.push(event.clone());
        }
        if let Some(ref path) = self.history_file
            && let Ok(line) = serde_json::to_string(&event)
            && let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path)
        {
            let _ = writeln!(file, "{line}");
        }
        let _ = self.events_tx.send(Notification::new(
            EVENT_CHAT_EVENT,
            json!(ChatEventNotification {
                id: self.session_id.clone(),
                event,
            }),
        ));
    }
}

pub struct ChatSession {
    title: String,
    state_tx: watch::Sender<SessionState>,
    cmd_tx: mpsc::UnboundedSender<ChatCommand>,
    history: Arc<Mutex<Vec<ChatEvent>>>,
    pending_permissions: Arc<Mutex<HashMap<String, PendingPermission>>>,
    in_flight: Arc<AtomicBool>,
    killed_by_user: Arc<AtomicBool>,
}

impl ChatSession {
    pub fn spawn(
        id: String,
        title: String,
        p: &CreateParams,
        chats_dir: Option<&Path>,
        state_tx: watch::Sender<SessionState>,
        events_tx: broadcast::Sender<Notification>,
    ) -> Result<Self, SessionError> {
        let Some(program) = p.command.first().cloned() else {
            return Err(SessionError::Spawn {
                command: String::new(),
                cause: "comando vacío".into(),
            });
        };

        let (history_file, chat_dir) = if let Some(base) = chats_dir {
            fs::create_dir_all(base)?;
            let dir = base.join(&id);
            fs::create_dir_all(&dir)?;
            (Some(base.join(format!("{id}.jsonl"))), Some(dir))
        } else {
            (None, None)
        };

        let mut cmd = Command::new(&program);
        cmd.args(&p.command[1..]);
        cmd.current_dir(&p.cwd);
        for (key, _) in std::env::vars_os() {
            if let Some(name) = key.to_str()
                && (name == "CLAUDECODE"
                    || name.starts_with("CLAUDE_CODE_")
                    || name.starts_with("ORCA_"))
            {
                cmd.env_remove(&key);
            }
        }
        for (k, v) in &p.env {
            cmd.env(k, v);
        }
        cmd.env("PAS_SESSION_ID", &id);
        if let Some(ref dir) = chat_dir
            && !p.env.contains_key("PAS_CHAT_DIR")
        {
            cmd.env("PAS_CHAT_DIR", dir);
        }
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        let mut child = cmd.spawn().map_err(|e| SessionError::Spawn {
            command: program.clone(),
            cause: e.to_string(),
        })?;

        let stdin = child.stdin.take().ok_or_else(|| SessionError::Spawn {
            command: program.clone(),
            cause: "no se pudo abrir stdin del adaptador".into(),
        })?;
        let stdout = child.stdout.take().ok_or_else(|| SessionError::Spawn {
            command: program.clone(),
            cause: "no se pudo abrir stdout del adaptador".into(),
        })?;
        let mut stderr = child.stderr.take().ok_or_else(|| SessionError::Spawn {
            command: program.clone(),
            cause: "no se pudo abrir stderr del adaptador".into(),
        })?;

        let stderr_buf = Arc::new(Mutex::new(Vec::<u8>::new()));
        let stderr_buf_task = Arc::clone(&stderr_buf);
        let stderr_task = tokio::spawn(async move {
            let mut chunk = [0u8; 2048];
            loop {
                match stderr.read(&mut chunk).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if let Ok(mut buf) = stderr_buf_task.lock() {
                            buf.extend_from_slice(&chunk[..n]);
                            if buf.len() > STDERR_LIMIT_BYTES {
                                let excess = buf.len() - STDERR_LIMIT_BYTES;
                                buf.drain(..excess);
                            }
                        }
                    }
                }
            }
        });

        let history = Arc::new(Mutex::new(Vec::new()));
        let sink = EventSink {
            session_id: id,
            history: Arc::clone(&history),
            history_file,
            events_tx,
        };
        let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<ChatCommand>();
        let pending_permissions: Arc<Mutex<HashMap<String, PendingPermission>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let allowed_tools: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
        let in_flight = Arc::new(AtomicBool::new(false));
        let killed_by_user = Arc::new(AtomicBool::new(false));

        let driver_state_tx = state_tx.clone();
        let driver_pending = Arc::clone(&pending_permissions);
        let driver_allowed = Arc::clone(&allowed_tools);
        let driver_in_flight = Arc::clone(&in_flight);
        let driver_killed = Arc::clone(&killed_by_user);
        let cwd = p.cwd.clone();

        tokio::spawn(async move {
            let tool_titles: Arc<Mutex<HashMap<String, String>>> =
                Arc::new(Mutex::new(HashMap::new()));
            let seen_visuals: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
            let next_perm_seq = Arc::new(AtomicU64::new(1));

            let notif_sink = sink.clone();
            let notif_titles = Arc::clone(&tool_titles);
            let notif_visuals = Arc::clone(&seen_visuals);
            let notif_chat_dir = chat_dir.clone();

            let perm_sink = sink.clone();
            let perm_state_tx = driver_state_tx.clone();
            let perm_pending = Arc::clone(&driver_pending);
            let perm_allowed = Arc::clone(&driver_allowed);
            let perm_seq = Arc::clone(&next_perm_seq);

            let fg_sink = sink.clone();
            let fg_state_tx = driver_state_tx.clone();
            let fg_pending = Arc::clone(&driver_pending);
            let fg_in_flight = Arc::clone(&driver_in_flight);
            let fg_killed = Arc::clone(&driver_killed);
            let fg_visuals = Arc::clone(&seen_visuals);
            let fg_chat_dir = chat_dir.clone();

            let transport = ByteStreams::new(stdin.compat_write(), stdout.compat());

            let conn_res = Client
                .v1()
                .name("pasd")
                .on_receive_notification(
                    async move |notif: SessionNotification, _cx| {
                        handle_session_update(
                            notif.update,
                            &notif_sink,
                            &notif_titles,
                            notif_chat_dir.as_deref(),
                            &notif_visuals,
                        );
                        Ok(())
                    },
                    agent_client_protocol::on_receive_notification!(),
                )
                .on_receive_request(
                    async move |req: RequestPermissionRequest, responder, cx| {
                        let tool_key = req
                            .tool_call
                            .fields
                            .name
                            .clone()
                            .or_else(|| req.tool_call.fields.title.clone())
                            .unwrap_or_else(|| req.tool_call.tool_call_id.0.to_string());

                        let already_allowed = perm_allowed
                            .lock()
                            .map(|set| set.contains(&tool_key))
                            .unwrap_or(false);
                        if already_allowed {
                            let outcome = pick_permission_outcome(
                                &req.options,
                                PermissionChoice::AllowSession,
                            );
                            return responder.respond(RequestPermissionResponse::new(outcome));
                        }

                        let raw_id = req.tool_call.tool_call_id.0.to_string();
                        let title = req
                            .tool_call
                            .fields
                            .title
                            .clone()
                            .unwrap_or_else(|| "Permiso requerido".to_string());
                        let detail = req.tool_call.fields.raw_input.as_ref().map(|v| match v {
                            Value::String(s) => s.clone(),
                            other => other.to_string(),
                        });

                        let (tx, rx) = oneshot::channel::<PermissionChoice>();
                        let request_id = {
                            let mut map = perm_pending.lock().expect("pending permissions lock");
                            let candidate = if !raw_id.is_empty() && !map.contains_key(&raw_id) {
                                raw_id
                            } else {
                                let seq = perm_seq.fetch_add(1, Ordering::Relaxed);
                                format!("perm-{seq}")
                            };
                            map.insert(
                                candidate.clone(),
                                PendingPermission {
                                    title: title.clone(),
                                    tx,
                                },
                            );
                            candidate
                        };

                        perm_sink.emit(ChatEvent::PermissionRequest {
                            request_id: request_id.clone(),
                            title,
                            detail,
                        });
                        set_state_if_alive(&perm_state_tx, SessionState::Waiting);

                        let options = req.options;
                        let sink_cb = perm_sink.clone();
                        let state_cb = perm_state_tx.clone();
                        let pending_cb = Arc::clone(&perm_pending);
                        let allowed_cb = Arc::clone(&perm_allowed);

                        cx.spawn(async move {
                            match rx.await {
                                Ok(choice) => {
                                    if choice == PermissionChoice::AllowSession
                                        && let Ok(mut set) = allowed_cb.lock()
                                    {
                                        set.insert(tool_key);
                                    }
                                    sink_cb.emit(ChatEvent::PermissionResolved {
                                        request_id,
                                        choice,
                                    });
                                    let still_waiting = pending_cb
                                        .lock()
                                        .map(|m| !m.is_empty())
                                        .unwrap_or(false);
                                    if !still_waiting {
                                        set_state_if_alive(&state_cb, SessionState::Running);
                                    }
                                    let outcome = pick_permission_outcome(&options, choice);
                                    responder.respond(RequestPermissionResponse::new(outcome))
                                }
                                Err(_) => responder.respond(RequestPermissionResponse::new(
                                    RequestPermissionOutcome::Cancelled,
                                )),
                            }
                        })
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .connect_with(transport, async move |cx: ConnectionTo<Agent>| {
                    cx.send_request(InitializeRequest::new(ProtocolVersion::V1))
                        .block_task()
                        .await?;

                    let session = cx
                        .send_request(NewSessionRequest::new(cwd))
                        .block_task()
                        .await?;
                    let acp_session_id = session.session_id;

                    set_state_if_alive(&fg_state_tx, SessionState::Idle);

                    loop {
                        tokio::select! {
                            biased;
                            () = cx.incoming_closed() => {
                                break;
                            }
                            cmd = cmd_rx.recv() => {
                                let Some(cmd) = cmd else {
                                    break;
                                };
                                match cmd {
                                    ChatCommand::Prompt { text } => {
                                        fg_sink.emit(ChatEvent::UserMessage { text: text.clone() });
                                        set_state_if_alive(&fg_state_tx, SessionState::Running);

                                        let prompt_cx = cx.clone();
                                        let prompt_sid = acp_session_id.clone();
                                        let prompt_sink = fg_sink.clone();
                                        let prompt_state_tx = fg_state_tx.clone();
                                        let prompt_pending = Arc::clone(&fg_pending);
                                        let prompt_in_flight = Arc::clone(&fg_in_flight);
                                        let prompt_killed = Arc::clone(&fg_killed);
                                        let prompt_visuals = Arc::clone(&fg_visuals);
                                        let prompt_chat_dir = fg_chat_dir.clone();

                                        let _ = cx.spawn(async move {
                                            let res = prompt_cx
                                                .send_request(PromptRequest::new(
                                                    prompt_sid,
                                                    vec![ContentBlock::Text(TextContent::new(text))],
                                                ))
                                                .block_task()
                                                .await;

                                            if let Some(ref dir) = prompt_chat_dir {
                                                scan_new_visuals(dir, &prompt_visuals, &prompt_sink);
                                            }
                                            if let Ok(mut map) = prompt_pending.lock() {
                                                map.clear();
                                            }
                                            prompt_in_flight.store(false, Ordering::Release);

                                            match res {
                                                Ok(_resp) => {
                                                    prompt_sink.emit(ChatEvent::AgentMessageEnd);
                                                    set_state_if_alive(
                                                        &prompt_state_tx,
                                                        SessionState::Idle,
                                                    );
                                                }
                                                Err(err) => {
                                                    let transport_closed =
                                                        is_connection_terminated(&err)
                                                            || is_incoming_transport_closed(&err);
                                                    if !transport_closed
                                                        && !prompt_killed.load(Ordering::Acquire)
                                                    {
                                                        prompt_sink.emit(ChatEvent::Error {
                                                            message: format!(
                                                                "Error del adaptador: {err}"
                                                            ),
                                                        });
                                                        set_state_if_alive(
                                                            &prompt_state_tx,
                                                            SessionState::Idle,
                                                        );
                                                    }
                                                }
                                            }
                                            Ok(())
                                        });
                                    }
                                    ChatCommand::Cancel => {
                                        if let Ok(mut map) = fg_pending.lock() {
                                            map.clear();
                                        }
                                        let _ = cx.send_notification(CancelNotification::new(
                                            acp_session_id.clone(),
                                        ));
                                    }
                                    ChatCommand::Kill => {
                                        break;
                                    }
                                }
                            }
                        }
                    }

                    Ok(())
                })
                .await;

            if driver_killed.load(Ordering::Acquire) {
                let _ = child.kill().await;
            }
            let code = child.wait().await.ok().and_then(|st| st.code());
            let _ = tokio::time::timeout(Duration::from_millis(200), stderr_task).await;

            if let Ok(mut map) = driver_pending.lock() {
                map.clear();
            }
            driver_in_flight.store(false, Ordering::Release);

            let stderr_text = stderr_buf
                .lock()
                .map(|b| String::from_utf8_lossy(&b).trim().to_string())
                .unwrap_or_default();

            let killed = driver_killed.load(Ordering::Acquire);
            if !killed && (code != Some(0) || conn_res.is_err()) {
                let code_suffix = match code {
                    Some(c) => format!(" (código {c})"),
                    None => String::new(),
                };
                let message = if !stderr_text.is_empty() {
                    let summary = summarize_stderr(&stderr_text);
                    format!("El adaptador terminó con error{code_suffix}: {summary}")
                } else if let Err(err) = conn_res {
                    format!("El adaptador terminó con error{code_suffix}: {err}")
                } else {
                    format!("El adaptador terminó con error{code_suffix}.")
                };
                sink.emit(ChatEvent::Error { message });
            }

            let _ = driver_state_tx.send(SessionState::Exited { code });
        });

        Ok(Self {
            title,
            state_tx,
            cmd_tx,
            history,
            pending_permissions,
            in_flight,
            killed_by_user,
        })
    }

    pub fn prompt(&self, text: String) -> Result<(), SessionError> {
        if matches!(*self.state_tx.borrow(), SessionState::Exited { .. }) {
            return Err(SessionError::Exited {
                title: self.title.clone(),
            });
        }
        if self.in_flight.swap(true, Ordering::AcqRel) {
            return Err(SessionError::ChatBusy {
                title: self.title.clone(),
            });
        }
        if self.cmd_tx.send(ChatCommand::Prompt { text }).is_err() {
            self.in_flight.store(false, Ordering::Release);
            return Err(SessionError::Exited {
                title: self.title.clone(),
            });
        }
        Ok(())
    }

    pub fn resolve_permission(
        &self,
        request_id: &str,
        choice: PermissionChoice,
    ) -> Result<(), SessionError> {
        if matches!(*self.state_tx.borrow(), SessionState::Exited { .. }) {
            return Err(SessionError::Exited {
                title: self.title.clone(),
            });
        }
        let pending = self
            .pending_permissions
            .lock()
            .expect("pending permissions lock")
            .remove(request_id);
        let Some(pending) = pending else {
            return Err(SessionError::PermissionNotFound {
                request_id: request_id.to_string(),
            });
        };
        pending
            .tx
            .send(choice)
            .map_err(|_| SessionError::PermissionNotFound {
                request_id: request_id.to_string(),
            })
    }

    pub fn cancel(&self) -> Result<(), SessionError> {
        if matches!(*self.state_tx.borrow(), SessionState::Exited { .. }) {
            return Err(SessionError::Exited {
                title: self.title.clone(),
            });
        }
        let _ = self.cmd_tx.send(ChatCommand::Cancel);
        Ok(())
    }

    pub fn history(&self) -> Vec<ChatEvent> {
        self.history.lock().map(|h| h.clone()).unwrap_or_default()
    }

    pub fn last_visible_line(&self) -> Option<String> {
        self.pending_permissions
            .lock()
            .ok()?
            .values()
            .next()
            .map(|p| p.title.clone())
    }

    pub fn kill(&self) {
        self.killed_by_user.store(true, Ordering::Release);
        if let Ok(mut map) = self.pending_permissions.lock() {
            map.clear();
        }
        let _ = self.cmd_tx.send(ChatCommand::Kill);
    }
}

fn set_state_if_alive(state_tx: &watch::Sender<SessionState>, next: SessionState) {
    state_tx.send_if_modified(|s| {
        if matches!(s, SessionState::Exited { .. }) || *s == next {
            false
        } else {
            *s = next.clone();
            true
        }
    });
}

fn handle_session_update(
    update: SessionUpdate,
    sink: &EventSink,
    tool_titles: &Mutex<HashMap<String, String>>,
    chat_dir: Option<&Path>,
    seen_visuals: &Mutex<HashSet<String>>,
) {
    match update {
        SessionUpdate::AgentMessageChunk(chunk) => {
            if let ContentBlock::Text(text) = chunk.content {
                sink.emit(ChatEvent::AgentChunk { text: text.text });
            }
        }
        SessionUpdate::ToolCall(tc) => {
            let id = tc.tool_call_id.0.to_string();
            let title = tc.title.clone();
            if let Ok(mut titles) = tool_titles.lock() {
                titles.insert(id.clone(), title.clone());
            }
            sink.emit(ChatEvent::ToolCall {
                id,
                title,
                status: format_tool_status(tc.status).to_string(),
            });
            for content in tc.content {
                if let ToolCallContent::Diff(diff) = content {
                    let unified =
                        format_unified_diff(&diff.path, diff.old_text.as_deref(), &diff.new_text);
                    sink.emit(ChatEvent::Diff {
                        path: diff.path.to_string_lossy().into_owned(),
                        unified,
                    });
                }
            }
        }
        SessionUpdate::ToolCallUpdate(upd) => {
            let id = upd.tool_call_id.0.to_string();
            if let Some(ref title) = upd.fields.title
                && let Ok(mut titles) = tool_titles.lock()
            {
                titles.insert(id.clone(), title.clone());
            }
            if upd.fields.status.is_some() || upd.fields.title.is_some() {
                let title = upd.fields.title.clone().unwrap_or_else(|| {
                    tool_titles
                        .lock()
                        .ok()
                        .and_then(|t| t.get(&id).cloned())
                        .unwrap_or_default()
                });
                let status = upd
                    .fields
                    .status
                    .map(format_tool_status)
                    .unwrap_or("in_progress")
                    .to_string();
                sink.emit(ChatEvent::ToolCall { id, title, status });
            }
            if let Some(contents) = upd.fields.content {
                for content in contents {
                    if let ToolCallContent::Diff(diff) = content {
                        let unified = format_unified_diff(
                            &diff.path,
                            diff.old_text.as_deref(),
                            &diff.new_text,
                        );
                        sink.emit(ChatEvent::Diff {
                            path: diff.path.to_string_lossy().into_owned(),
                            unified,
                        });
                    }
                }
            }
        }
        _ => {}
    }

    if let Some(dir) = chat_dir {
        scan_new_visuals(dir, seen_visuals, sink);
    }
}

fn scan_new_visuals(dir: &Path, seen: &Mutex<HashSet<String>>, sink: &EventSink) {
    let mut found = Vec::new();
    collect_html_files(dir, dir, &mut found);
    found.sort();

    let mut new_files = Vec::new();
    if let Ok(mut guard) = seen.lock() {
        for rel in found {
            if guard.insert(rel.clone()) {
                new_files.push(rel);
            }
        }
    }
    for file in new_files {
        sink.emit(ChatEvent::Visual { file });
    }
}

fn collect_html_files(root: &Path, cur: &Path, out: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(cur) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_html_files(root, &path, out);
        } else if path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("html") || ext.eq_ignore_ascii_case("htm"))
            && let Ok(rel) = path.strip_prefix(root)
        {
            out.push(rel.to_string_lossy().into_owned());
        }
    }
}

fn format_tool_status(status: ToolCallStatus) -> &'static str {
    match status {
        ToolCallStatus::Pending => "pending",
        ToolCallStatus::InProgress => "in_progress",
        ToolCallStatus::Completed => "completed",
        ToolCallStatus::Failed => "failed",
        _ => "unknown",
    }
}

fn format_unified_diff(path: &Path, old_text: Option<&str>, new_text: &str) -> String {
    let path_str = path.to_string_lossy();
    let mut out = format!("--- a/{path_str}\n+++ b/{path_str}\n");
    if let Some(old) = old_text {
        for line in old.lines() {
            out.push('-');
            out.push_str(line);
            out.push('\n');
        }
    }
    for line in new_text.lines() {
        out.push('+');
        out.push_str(line);
        out.push('\n');
    }
    out
}

fn pick_permission_outcome(
    options: &[PermissionOption],
    choice: PermissionChoice,
) -> RequestPermissionOutcome {
    // Only one-shot options: "allow_session" is remembered by pas itself, so the
    // agent never writes persistent allow/deny rules into the user's config.
    let preferred: &[PermissionOptionKind] = match choice {
        PermissionChoice::Allow | PermissionChoice::AllowSession => {
            &[PermissionOptionKind::AllowOnce]
        }
        PermissionChoice::Deny => &[PermissionOptionKind::RejectOnce],
    };

    for kind in preferred {
        if let Some(opt) = options.iter().find(|o| &o.kind == kind) {
            return RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(
                opt.option_id.clone(),
            ));
        }
    }

    RequestPermissionOutcome::Cancelled
}

fn summarize_stderr(raw: &str) -> String {
    let lines: Vec<&str> = raw
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let joined = lines.join(" | ");
    if joined.chars().count() <= STDERR_SUMMARY_CHARS {
        joined
    } else {
        let mut out: String = joined.chars().take(STDERR_SUMMARY_CHARS - 1).collect();
        out.push('…');
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn selected(outcome: RequestPermissionOutcome) -> Option<String> {
        match outcome {
            RequestPermissionOutcome::Selected(s) => Some(s.option_id.0.to_string()),
            _ => None,
        }
    }

    #[test]
    fn permission_choices_never_pick_persistent_options() {
        let options = vec![
            PermissionOption::new("always", "siempre", PermissionOptionKind::AllowAlways),
            PermissionOption::new("once", "una vez", PermissionOptionKind::AllowOnce),
            PermissionOption::new("never", "nunca", PermissionOptionKind::RejectAlways),
            PermissionOption::new("no", "no", PermissionOptionKind::RejectOnce),
        ];
        let pick = |c| selected(pick_permission_outcome(&options, c));
        assert_eq!(pick(PermissionChoice::Allow).as_deref(), Some("once"));
        assert_eq!(
            pick(PermissionChoice::AllowSession).as_deref(),
            Some("once")
        );
        assert_eq!(pick(PermissionChoice::Deny).as_deref(), Some("no"));

        let only_always = vec![PermissionOption::new(
            "always",
            "siempre",
            PermissionOptionKind::AllowAlways,
        )];
        assert!(
            selected(pick_permission_outcome(
                &only_always,
                PermissionChoice::Allow
            ))
            .is_none()
        );
    }
}

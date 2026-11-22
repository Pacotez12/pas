use std::fmt;
use std::io::{Read, Write};
use std::num::NonZeroU16;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use bytes::Bytes;
use pas_core::protocol::{
    AgentKind, ChatEvent, CreateParams, Mode, Notification, PermissionChoice, SessionInfo,
    SessionState, WaitUntil,
};
use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};
use regex::Regex;
use tokio::sync::{broadcast, watch};

use crate::chat::ChatSession;

const SCROLLBACK_LINES: usize = 10_000;
const OUTPUT_BROADCAST_CAPACITY: usize = 1024;

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("No se pudo iniciar `{command}`: {cause}. Revisá que esté instalado y en el PATH.")]
    Spawn { command: String, cause: String },

    #[error("La sesión {title} ya terminó. Abrí una nueva con `pas new`.")]
    Exited { title: String },

    #[error("La sesión {title} no llegó a {state} en {secs:.1} s.")]
    Timeout {
        title: String,
        state: &'static str,
        secs: f64,
    },

    #[error("El tamaño de la terminal tiene que ser mayor a cero. Pasá columnas y filas válidas.")]
    InvalidSize,

    #[error("La sesión {title} no es de tipo {expected}. Usá el método que corresponde a su modo.")]
    WrongMode {
        title: String,
        expected: &'static str,
    },

    #[error(
        "No hay ningún permiso pendiente con id {request_id}. Quizás ya se resolvió o se canceló."
    )]
    PermissionNotFound { request_id: String },

    #[error(
        "La sesión {title} ya está respondiendo. Esperá a que termine o cancelala antes de mandar otro mensaje."
    )]
    ChatBusy { title: String },

    #[error("{0}")]
    Io(#[from] std::io::Error),
}

struct TerminalSession {
    parser: Arc<Mutex<vt100::Parser>>,
    writer: Mutex<Box<dyn Write + Send>>,
    master: Mutex<Box<dyn MasterPty + Send>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    output_tx: broadcast::Sender<Bytes>,
    started_at: Instant,
    last_activity_ms: Arc<AtomicU64>,
    hooks_active: AtomicBool,
}

enum SessionKind {
    Terminal(TerminalSession),
    Chat(ChatSession),
}

pub struct Session {
    id: String,
    title: String,
    cwd: PathBuf,
    command: Vec<String>,
    created_at: u64,
    project_id: Option<String>,
    account_id: Option<String>,
    mode: Mode,
    state_tx: watch::Sender<SessionState>,
    kind: SessionKind,
}

impl fmt::Debug for Session {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Session")
            .field("id", &self.id)
            .field("title", &self.title)
            .field("mode", &self.mode)
            .finish()
    }
}

impl Session {
    pub fn spawn(id: String, p: CreateParams, idle_ms: u64) -> Result<Arc<Self>, SessionError> {
        Self::spawn_with_waiting_patterns(id, p, idle_ms, Vec::new())
    }

    pub fn spawn_chat(
        id: String,
        p: CreateParams,
        chats_dir: Option<&Path>,
        events_tx: broadcast::Sender<Notification>,
    ) -> Result<Arc<Self>, SessionError> {
        let program = p.command.first().cloned().unwrap_or_default();
        let title = p.title.clone().unwrap_or(program);
        let created_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let (state_tx, _) = watch::channel(SessionState::Running);

        let chat = ChatSession::spawn(
            id.clone(),
            title.clone(),
            &p,
            chats_dir,
            state_tx.clone(),
            events_tx,
        )?;

        Ok(Arc::new(Self {
            id,
            title,
            cwd: p.cwd,
            command: p.command,
            created_at,
            project_id: p.project_id,
            account_id: p.account_id,
            mode: Mode::Chat,
            state_tx,
            kind: SessionKind::Chat(chat),
        }))
    }

    pub fn spawn_with_waiting_patterns(
        id: String,
        p: CreateParams,
        idle_ms: u64,
        waiting_patterns: Vec<Regex>,
    ) -> Result<Arc<Self>, SessionError> {
        let Some(program) = p.command.first().cloned() else {
            return Err(SessionError::Spawn {
                command: String::new(),
                cause: "comando vacío".into(),
            });
        };

        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: p.rows,
                cols: p.cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| SessionError::Spawn {
                command: program.clone(),
                cause: e.to_string(),
            })?;

        let mut cmd = CommandBuilder::new(&program);
        cmd.args(&p.command[1..]);
        cmd.cwd(&p.cwd);
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

        let mut child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| SessionError::Spawn {
                command: program.clone(),
                cause: e.to_string(),
            })?;
        drop(pair.slave);

        let killer = child.clone_killer();
        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| SessionError::Spawn {
                command: program.clone(),
                cause: e.to_string(),
            })?;
        let writer = pair.master.take_writer().map_err(|e| SessionError::Spawn {
            command: program.clone(),
            cause: e.to_string(),
        })?;

        let title = p.title.unwrap_or(program);
        let created_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let started_at = Instant::now();
        let last_activity_ms = Arc::new(AtomicU64::new(0));
        let parser = Arc::new(Mutex::new(vt100::Parser::new(
            NonZeroU16::new(p.rows).unwrap_or(vt100::Parser::DEFAULT_ROWS),
            NonZeroU16::new(p.cols).unwrap_or(vt100::Parser::DEFAULT_COLS),
            SCROLLBACK_LINES,
        )));
        let (state_tx, mut state_rx) = watch::channel(SessionState::Running);
        let (output_tx, _) = broadcast::channel(OUTPUT_BROADCAST_CAPACITY);

        let r_parser = Arc::clone(&parser);
        let r_activity = Arc::clone(&last_activity_ms);
        let r_state = state_tx.clone();
        let r_output = output_tx.clone();
        thread::spawn(move || {
            let mut buf = [0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if let Ok(mut guard) = r_parser.lock() {
                            guard.process(&buf[..n]);
                        }
                        let now = started_at.elapsed().as_millis() as u64;
                        r_activity.store(now, Ordering::Release);
                        r_state.send_if_modified(|s| {
                            if matches!(s, SessionState::Idle) {
                                *s = SessionState::Running;
                                true
                            } else {
                                false
                            }
                        });
                        let _ = r_output.send(Bytes::copy_from_slice(&buf[..n]));
                    }
                }
            }

            let code = child.wait().ok().and_then(|st| {
                if st.signal().is_some() {
                    None
                } else {
                    Some(st.exit_code() as i32)
                }
            });
            let _ = r_state.send(SessionState::Exited { code });
        });

        let w_activity = Arc::clone(&last_activity_ms);
        let w_state = state_tx.clone();
        let w_parser = Arc::clone(&parser);
        tokio::spawn(async move {
            loop {
                let state = state_rx.borrow_and_update().clone();
                match state {
                    SessionState::Exited { .. } => break,
                    SessionState::Idle | SessionState::Waiting => {
                        if state_rx.changed().await.is_err() {
                            break;
                        }
                        continue;
                    }
                    SessionState::Running => {}
                }

                let now = started_at.elapsed().as_millis() as u64;
                let last = w_activity.load(Ordering::Acquire);
                let elapsed = now.saturating_sub(last);
                if elapsed >= idle_ms {
                    let next_state = if !waiting_patterns.is_empty()
                        && last_visible_line_from_parser(&w_parser).is_some_and(|line| {
                            waiting_patterns.iter().any(|re| re.is_match(&line))
                        }) {
                        SessionState::Waiting
                    } else {
                        SessionState::Idle
                    };
                    w_state.send_if_modified(|s| {
                        if matches!(s, SessionState::Running) {
                            *s = next_state.clone();
                            true
                        } else {
                            false
                        }
                    });
                } else {
                    let remaining = Duration::from_millis(idle_ms - elapsed);
                    tokio::select! {
                        _ = tokio::time::sleep(remaining) => {}
                        res = state_rx.changed() => {
                            if res.is_err() {
                                break;
                            }
                        }
                    }
                }
            }
        });

        Ok(Arc::new(Self {
            id,
            title,
            cwd: p.cwd,
            command: p.command,
            created_at,
            project_id: p.project_id,
            account_id: p.account_id,
            mode: Mode::Terminal,
            state_tx,
            kind: SessionKind::Terminal(TerminalSession {
                parser,
                writer: Mutex::new(writer),
                master: Mutex::new(pair.master),
                killer: Mutex::new(killer),
                output_tx,
                started_at,
                last_activity_ms,
                hooks_active: AtomicBool::new(false),
            }),
        }))
    }

    fn as_terminal(&self) -> Result<&TerminalSession, SessionError> {
        match &self.kind {
            SessionKind::Terminal(t) => Ok(t),
            SessionKind::Chat(_) => Err(SessionError::WrongMode {
                title: self.title.clone(),
                expected: "terminal",
            }),
        }
    }

    fn as_chat(&self) -> Result<&ChatSession, SessionError> {
        match &self.kind {
            SessionKind::Chat(c) => Ok(c),
            SessionKind::Terminal(_) => Err(SessionError::WrongMode {
                title: self.title.clone(),
                expected: "chat",
            }),
        }
    }

    pub fn subscribe_output(&self) -> broadcast::Receiver<Bytes> {
        match &self.kind {
            SessionKind::Terminal(t) => t.output_tx.subscribe(),
            SessionKind::Chat(_) => {
                let (tx, rx) = broadcast::channel(1);
                drop(tx);
                rx
            }
        }
    }

    pub fn subscribe_state(&self) -> watch::Receiver<SessionState> {
        self.state_tx.subscribe()
    }

    pub fn send(&self, text: &str, enter: bool) -> Result<(), SessionError> {
        if enter {
            let mut buf = Vec::with_capacity(text.len() + 1);
            buf.extend_from_slice(text.as_bytes());
            buf.push(b'\r');
            self.input(&buf)
        } else {
            self.input(text.as_bytes())
        }
    }

    pub fn input(&self, data: &[u8]) -> Result<(), SessionError> {
        let term = self.as_terminal()?;
        if matches!(self.state(), SessionState::Exited { .. }) {
            return Err(SessionError::Exited {
                title: self.title.clone(),
            });
        }

        let now = term.started_at.elapsed().as_millis() as u64;
        term.last_activity_ms.store(now, Ordering::Release);
        self.state_tx.send_if_modified(|s| {
            if matches!(s, SessionState::Idle | SessionState::Waiting) {
                *s = SessionState::Running;
                true
            } else {
                false
            }
        });

        let mut writer = term.writer.lock().expect("writer lock poisoned");
        let write_res = (|| -> std::io::Result<()> {
            writer.write_all(data)?;
            writer.flush()
        })();

        if let Err(e) = write_res {
            if matches!(self.state(), SessionState::Exited { .. }) {
                return Err(SessionError::Exited {
                    title: self.title.clone(),
                });
            }
            return Err(SessionError::Io(e));
        }

        Ok(())
    }

    pub fn resize(&self, cols: u16, rows: u16) -> Result<(), SessionError> {
        let term = self.as_terminal()?;
        let Some(cols_nz) = NonZeroU16::new(cols) else {
            return Err(SessionError::InvalidSize);
        };
        let Some(rows_nz) = NonZeroU16::new(rows) else {
            return Err(SessionError::InvalidSize);
        };

        if matches!(self.state(), SessionState::Exited { .. }) {
            return Err(SessionError::Exited {
                title: self.title.clone(),
            });
        }

        let master = term.master.lock().expect("master lock poisoned");
        if let Err(e) = master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        }) {
            if matches!(self.state(), SessionState::Exited { .. }) {
                return Err(SessionError::Exited {
                    title: self.title.clone(),
                });
            }
            return Err(SessionError::Io(std::io::Error::other(e.to_string())));
        }
        drop(master);

        let mut parser = term.parser.lock().expect("parser lock poisoned");
        parser.set_size(rows_nz, cols_nz);
        Ok(())
    }

    pub fn snapshot(&self) -> Vec<u8> {
        let SessionKind::Terminal(term) = &self.kind else {
            return Vec::new();
        };
        let mut parser = term.parser.lock().expect("parser lock poisoned");
        let (rows, cols) = parser.screen().size();
        let rows_usize = usize::from(rows.get());
        let cols_u16 = cols.get();

        let mut out = String::from("\x1b[3J\x1b[2J\x1b[H");

        if !parser.screen().alternate_screen() {
            parser.screen_mut().set_scrollback(usize::MAX);
            let total_sb = parser.screen().scrollback();

            if total_sb > 0 {
                let mut offset = total_sb;
                while offset > 0 {
                    parser.screen_mut().set_scrollback(offset);
                    let take = offset.min(rows_usize);
                    for row in parser.screen().rows_formatted(0, cols_u16).take(take) {
                        out.push_str(&row);
                        out.push_str("\x1b[0m\r\n");
                    }
                    offset -= take;
                }
                parser.screen_mut().set_scrollback(0);
                for _ in 0..rows.get().saturating_sub(1) {
                    out.push_str("\r\n");
                }
            } else {
                parser.screen_mut().set_scrollback(0);
            }
        }

        out.push_str(&parser.screen().contents_formatted());
        out.into_bytes()
    }

    pub fn read(&self, lines: Option<usize>) -> String {
        let SessionKind::Terminal(term) = &self.kind else {
            return String::new();
        };
        let mut parser = term.parser.lock().expect("parser lock poisoned");
        let (rows, cols) = parser.screen().size();
        let rows = usize::from(rows.get());
        let cols = cols.get();

        parser.screen_mut().set_scrollback(usize::MAX);
        let total_sb = parser.screen().scrollback();

        let mut all = Vec::with_capacity(total_sb + rows);
        let mut offset = total_sb;
        while offset > 0 {
            parser.screen_mut().set_scrollback(offset);
            let take = offset.min(rows);
            all.extend(parser.screen().rows(0, cols).take(take));
            offset -= take;
        }
        parser.screen_mut().set_scrollback(0);
        all.extend(parser.screen().rows(0, cols));

        match lines {
            Some(n) => {
                let non_empty: Vec<&str> = all
                    .iter()
                    .map(String::as_str)
                    .filter(|l| !l.trim_end().is_empty())
                    .collect();
                let start = non_empty.len().saturating_sub(n);
                non_empty[start..].join("\n")
            }
            None => {
                while all.last().is_some_and(|l| l.trim_end().is_empty()) {
                    all.pop();
                }
                all.join("\n")
            }
        }
    }

    pub fn chat_prompt(&self, text: String) -> Result<(), SessionError> {
        self.as_chat()?.prompt(text)
    }

    pub fn chat_permission(
        &self,
        request_id: &str,
        choice: PermissionChoice,
    ) -> Result<(), SessionError> {
        self.as_chat()?.resolve_permission(request_id, choice)
    }

    pub fn chat_cancel(&self) -> Result<(), SessionError> {
        self.as_chat()?.cancel()
    }

    pub fn chat_history(&self) -> Result<Vec<ChatEvent>, SessionError> {
        Ok(self.as_chat()?.history())
    }

    pub fn state(&self) -> SessionState {
        self.state_tx.borrow().clone()
    }

    pub fn last_visible_line(&self) -> Option<String> {
        match &self.kind {
            SessionKind::Terminal(t) => last_visible_line_from_parser(&t.parser),
            SessionKind::Chat(c) => c.last_visible_line(),
        }
    }

    pub fn hooks_active(&self) -> bool {
        match &self.kind {
            SessionKind::Terminal(t) => t.hooks_active.load(Ordering::Acquire),
            SessionKind::Chat(_) => false,
        }
    }

    pub fn apply_hook(&self, agent: AgentKind, event: &str) {
        let SessionKind::Terminal(term) = &self.kind else {
            return;
        };
        term.hooks_active.store(true, Ordering::Release);
        let Some(next) = state_for_hook(agent, event) else {
            return;
        };
        let now = term.started_at.elapsed().as_millis() as u64;
        term.last_activity_ms.store(now, Ordering::Release);
        self.state_tx.send_if_modified(|s| {
            if matches!(s, SessionState::Exited { .. }) || *s == next {
                false
            } else {
                *s = next.clone();
                true
            }
        });
    }

    pub fn info(&self) -> SessionInfo {
        SessionInfo {
            id: self.id.clone(),
            title: self.title.clone(),
            cwd: self.cwd.clone(),
            command: self.command.clone(),
            state: self.state(),
            created_at: self.created_at,
            project_id: self.project_id.clone(),
            account_id: self.account_id.clone(),
            mode: self.mode,
        }
    }

    pub fn kill(&self) {
        match &self.kind {
            SessionKind::Terminal(t) => {
                if let Ok(mut killer) = t.killer.lock() {
                    let _ = killer.kill();
                }
            }
            SessionKind::Chat(c) => c.kill(),
        }
    }

    pub async fn wait(
        &self,
        until: WaitUntil,
        timeout: Duration,
    ) -> Result<SessionState, SessionError> {
        let mut rx = self.state_tx.subscribe();
        let target = tokio::time::timeout(timeout, async move {
            loop {
                let st = rx.borrow_and_update().clone();
                if matches_until(&st, until) {
                    return st;
                }
                if rx.changed().await.is_err() {
                    return rx.borrow().clone();
                }
            }
        })
        .await;

        target.map_err(|_| SessionError::Timeout {
            title: self.title.clone(),
            state: match until {
                WaitUntil::Idle => "idle",
                WaitUntil::Exited => "exited",
            },
            secs: timeout.as_secs_f64(),
        })
    }
}

pub fn state_for_hook(agent: AgentKind, event: &str) -> Option<SessionState> {
    match (agent, event) {
        (AgentKind::Claude, "UserPromptSubmit" | "PreToolUse") => Some(SessionState::Running),
        (AgentKind::Claude, "PermissionRequest" | "Notification") => Some(SessionState::Waiting),
        (AgentKind::Claude, "Stop") => Some(SessionState::Idle),
        (AgentKind::Codex, "user_prompt_submit" | "pre_tool_use") => Some(SessionState::Running),
        (AgentKind::Codex, "permission_request") => Some(SessionState::Waiting),
        (AgentKind::Codex, "stop") => Some(SessionState::Idle),
        (AgentKind::Agy, "PreInvocation" | "PreToolUse") => Some(SessionState::Running),
        (AgentKind::Agy, "Stop" | "PostInvocation") => Some(SessionState::Idle),
        _ => None,
    }
}

fn last_visible_line_from_parser(parser: &Mutex<vt100::Parser>) -> Option<String> {
    let parser = parser.lock().ok()?;
    let (_, cols) = parser.screen().size();
    let cols = cols.get();
    parser
        .screen()
        .rows(0, cols)
        .map(|row| row.trim_end().to_string())
        .filter(|row| !row.is_empty())
        .last()
}

fn matches_until(state: &SessionState, until: WaitUntil) -> bool {
    match state {
        SessionState::Exited { .. } => true,
        SessionState::Idle => matches!(until, WaitUntil::Idle),
        SessionState::Running | SessionState::Waiting => false,
    }
}

use std::fmt;
use std::io::{Read, Write};
use std::num::NonZeroU16;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use pas_core::protocol::{CreateParams, SessionInfo, SessionState, WaitUntil};
use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};
use tokio::sync::watch;

const SCROLLBACK_LINES: usize = 10_000;

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

    #[error("{0}")]
    Io(#[from] std::io::Error),
}

pub struct Session {
    id: String,
    title: String,
    cwd: PathBuf,
    command: Vec<String>,
    created_at: u64,
    parser: Arc<Mutex<vt100::Parser>>,
    writer: Mutex<Box<dyn Write + Send>>,
    _master: Mutex<Box<dyn MasterPty + Send>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    state_tx: watch::Sender<SessionState>,
    started_at: Instant,
    last_activity_ms: Arc<AtomicU64>,
}

impl fmt::Debug for Session {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Session")
            .field("id", &self.id)
            .field("title", &self.title)
            .finish()
    }
}

impl Session {
    pub fn spawn(id: String, p: CreateParams, idle_ms: u64) -> Result<Arc<Self>, SessionError> {
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
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| SessionError::Spawn {
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

        let r_parser = Arc::clone(&parser);
        let r_activity = Arc::clone(&last_activity_ms);
        let r_state = state_tx.clone();
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
        tokio::spawn(async move {
            loop {
                let state = state_rx.borrow_and_update().clone();
                match state {
                    SessionState::Exited { .. } => break,
                    SessionState::Idle => {
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
                    w_state.send_if_modified(|s| {
                        if matches!(s, SessionState::Running) {
                            *s = SessionState::Idle;
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
            parser,
            writer: Mutex::new(writer),
            _master: Mutex::new(pair.master),
            killer: Mutex::new(killer),
            state_tx,
            started_at,
            last_activity_ms,
        }))
    }

    pub fn send(&self, text: &str, enter: bool) -> Result<(), SessionError> {
        if matches!(self.state(), SessionState::Exited { .. }) {
            return Err(SessionError::Exited {
                title: self.title.clone(),
            });
        }

        let now = self.started_at.elapsed().as_millis() as u64;
        self.last_activity_ms.store(now, Ordering::Release);
        self.state_tx.send_if_modified(|s| {
            if matches!(s, SessionState::Idle) {
                *s = SessionState::Running;
                true
            } else {
                false
            }
        });

        let mut writer = self.writer.lock().expect("writer lock poisoned");
        let write_res = (|| -> std::io::Result<()> {
            writer.write_all(text.as_bytes())?;
            if enter {
                writer.write_all(b"\r")?;
            }
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

    pub fn read(&self, lines: Option<usize>) -> String {
        let mut parser = self.parser.lock().expect("parser lock poisoned");
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

    pub fn state(&self) -> SessionState {
        self.state_tx.borrow().clone()
    }

    pub fn info(&self) -> SessionInfo {
        SessionInfo {
            id: self.id.clone(),
            title: self.title.clone(),
            cwd: self.cwd.clone(),
            command: self.command.clone(),
            state: self.state(),
            created_at: self.created_at,
        }
    }

    pub fn kill(&self) {
        if let Ok(mut killer) = self.killer.lock() {
            let _ = killer.kill();
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

fn matches_until(state: &SessionState, until: WaitUntil) -> bool {
    match state {
        SessionState::Exited { .. } => true,
        SessionState::Idle => matches!(until, WaitUntil::Idle),
        SessionState::Running => false,
    }
}

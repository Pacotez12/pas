use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use pas_core::protocol::{
    AccountQuota, AcpAdapterStatus, AgentKind, CreateParams, ERR_ADAPTER_INSTALL_FAILED,
    ERR_ADAPTER_NOT_FOUND, ERR_CHAT_BUSY, ERR_INVALID_PARAMS, ERR_NOT_A_GIT_REPO,
    ERR_PERMISSION_NOT_FOUND, ERR_SESSION_EXITED, ERR_SESSION_NOT_FOUND, ERR_SESSION_RUNNING,
    ERR_SPAWN_FAILED, ERR_WAIT_TIMEOUT, ERR_WRONG_SESSION_MODE, EVENT_QUOTA_CHANGED,
    EVENT_QUOTA_WARNING, EVENT_SESSION_CREATED, EVENT_SESSION_FOCUS_REQUEST, EVENT_SESSION_REMOVED,
    EVENT_SESSION_STATE, Mode, Notification, QuotaReportParams, RpcError, SessionCreatedEvent,
    SessionFocusRequestEvent, SessionInfo, SessionRemovedEvent, SessionState, SessionStateEvent,
    WorktreeCreateParams,
};
use pas_core::socket::socket_path as default_socket_path;
use serde_json::json;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;

use crate::accounts;
use crate::acp::{self, AcpEnv, AcpError};
use crate::config::Config;
use crate::notify::{NoopNotifier, Notifier, WaitingNotice};
use crate::open::{Launcher, OpenEnv, SystemLauncher};
use crate::quota::{self, QuotaTracker, QuotaUpdate};
use crate::rpc::store_error_to_rpc;
use crate::session::{Session, SessionError};
use crate::store::{Store, StoreError};
use crate::worktree;

const EVENTS_BROADCAST_CAPACITY: usize = 1024;

pub struct Manager {
    idle_ms: u64,
    sessions: RwLock<HashMap<String, Arc<Session>>>,
    events_tx: broadcast::Sender<Notification>,
    store: Store,
    accounts_dir: Option<PathBuf>,
    socket_path: Option<PathBuf>,
    acp_env: AcpEnv,
    quotas: QuotaTracker,
    config: Config,
    notifier: Arc<dyn Notifier>,
    launcher: Arc<dyn Launcher>,
    open_env: OpenEnv,
}

impl fmt::Debug for Manager {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Manager")
            .field("idle_ms", &self.idle_ms)
            .field("accounts_dir", &self.accounts_dir)
            .field("socket_path", &self.socket_path)
            .finish()
    }
}

impl Manager {
    pub fn new(idle_ms: u64) -> Arc<Self> {
        let store = Store::open_in_memory().expect("no se pudo abrir SQLite en memoria");
        Self::with_store(idle_ms, store)
    }

    pub fn with_store(idle_ms: u64, store: Store) -> Arc<Self> {
        Self::with_paths(idle_ms, store, None, None)
    }

    pub fn with_paths(
        idle_ms: u64,
        store: Store,
        accounts_dir: Option<PathBuf>,
        socket_path: Option<PathBuf>,
    ) -> Arc<Self> {
        Self::with_config(
            idle_ms,
            store,
            accounts_dir,
            socket_path,
            Config::default(),
            Arc::new(NoopNotifier),
        )
    }

    pub fn with_config(
        idle_ms: u64,
        store: Store,
        accounts_dir: Option<PathBuf>,
        socket_path: Option<PathBuf>,
        config: Config,
        notifier: Arc<dyn Notifier>,
    ) -> Arc<Self> {
        let data_dir = accounts_dir
            .as_deref()
            .and_then(Path::parent)
            .map(Path::to_path_buf);
        let acp_env = AcpEnv {
            data_dir,
            path_env: None,
            npm_bin: None,
        };
        Self::with_acp_env(
            idle_ms,
            store,
            accounts_dir,
            socket_path,
            config,
            notifier,
            acp_env,
        )
    }

    pub fn with_acp_env(
        idle_ms: u64,
        store: Store,
        accounts_dir: Option<PathBuf>,
        socket_path: Option<PathBuf>,
        config: Config,
        notifier: Arc<dyn Notifier>,
        acp_env: AcpEnv,
    ) -> Arc<Self> {
        Self::with_open_env(
            idle_ms,
            store,
            accounts_dir,
            socket_path,
            config,
            notifier,
            acp_env,
            Arc::new(SystemLauncher),
            OpenEnv::default(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn with_open_env(
        idle_ms: u64,
        store: Store,
        accounts_dir: Option<PathBuf>,
        socket_path: Option<PathBuf>,
        config: Config,
        notifier: Arc<dyn Notifier>,
        acp_env: AcpEnv,
        launcher: Arc<dyn Launcher>,
        open_env: OpenEnv,
    ) -> Arc<Self> {
        let (events_tx, _) = broadcast::channel(EVENTS_BROADCAST_CAPACITY);
        let quotas = QuotaTracker::default();
        quotas.set_warn_percent(config.quota_warn_percent);
        Arc::new(Self {
            idle_ms,
            sessions: RwLock::new(HashMap::new()),
            events_tx,
            store,
            accounts_dir,
            socket_path,
            acp_env,
            quotas,
            config,
            notifier,
            launcher,
            open_env,
        })
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn launcher(&self) -> &dyn Launcher {
        self.launcher.as_ref()
    }

    pub fn open_env(&self) -> &OpenEnv {
        &self.open_env
    }

    pub fn allowed_open_roots(&self) -> Vec<PathBuf> {
        let mut roots = Vec::new();
        if let Ok(projects) = self.store.list_projects() {
            for p in projects {
                if let Ok(wts) = self.store.list_worktrees(&p.id) {
                    for wt in wts {
                        roots.push(wt.path);
                    }
                }
                roots.push(p.path);
            }
        }
        for s in self.list() {
            roots.push(s.cwd);
        }
        roots
    }

    pub fn acp_status(&self) -> Vec<AcpAdapterStatus> {
        acp::status(&self.acp_env)
    }

    pub async fn acp_install(&self, agent: AgentKind) -> Result<AcpAdapterStatus, RpcError> {
        acp::install(agent, &self.acp_env)
            .await
            .map_err(acp_error_to_rpc)
    }

    pub fn set_quota_warn_percent(&self, percent: f32) {
        self.quotas.set_warn_percent(percent);
    }

    pub fn remove_account_quota(&self, account_id: &str) {
        self.quotas.remove(account_id);
    }

    pub fn list_quotas(&self) -> Result<Vec<AccountQuota>, StoreError> {
        let accounts = self.store.list_accounts()?;
        Ok(accounts
            .into_iter()
            .map(|acc| {
                let quota = self.quotas.get(&acc.id);
                AccountQuota {
                    account_id: acc.id,
                    quota,
                }
            })
            .collect())
    }

    pub fn report_quota(&self, params: QuotaReportParams) {
        let Some(account_id) = self.resolve_quota_account(&params) else {
            return;
        };
        let now = quota::now_unix_secs();
        let Some(parsed) = quota::parse_report(params.agent, &params.payload, now) else {
            return;
        };
        let update = self.quotas.record(&account_id, parsed);
        self.emit_quota_update(update);
    }

    pub fn poll_codex_quotas(&self) {
        let Ok(accounts) = self.store.list_accounts() else {
            return;
        };
        let now = quota::now_unix_secs();
        for acc in accounts {
            if acc.agent != AgentKind::Codex {
                continue;
            }
            let update = self
                .quotas
                .poll_codex_account(&acc.id, &acc.config_dir, now);
            self.emit_quota_update(update);
        }
    }

    pub fn spawn_quota_poller(self: &Arc<Self>, interval: Duration) -> JoinHandle<()> {
        let mgr = Arc::clone(self);
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                ticker.tick().await;
                let mgr_clone = Arc::clone(&mgr);
                let _ = tokio::task::spawn_blocking(move || mgr_clone.poll_codex_quotas()).await;
            }
        })
    }

    fn resolve_quota_account(&self, params: &QuotaReportParams) -> Option<String> {
        if let Some(ref account_id) = params.account_id
            && !account_id.trim().is_empty()
        {
            return Some(account_id.trim().to_string());
        }
        if let Some(ref session_id) = params.session_id
            && let Ok(session) = self.get(session_id)
            && let Some(account_id) = session.info().account_id
        {
            return Some(account_id);
        }
        let accounts = self.store.list_accounts().ok()?;
        let mut matching = accounts.into_iter().filter(|a| a.agent == params.agent);
        let first = matching.next()?;
        if matching.next().is_none() {
            Some(first.id)
        } else {
            None
        }
    }

    fn emit_quota_update(&self, update: QuotaUpdate) {
        if let Some(changed) = update.changed {
            let _ = self
                .events_tx
                .send(Notification::new(EVENT_QUOTA_CHANGED, json!(changed)));
        }
        for warning in update.warnings {
            let _ = self
                .events_tx
                .send(Notification::new(EVENT_QUOTA_WARNING, json!(warning)));
        }
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    pub fn accounts_dir(&self) -> Option<&Path> {
        self.accounts_dir.as_deref()
    }

    pub fn worktrees_dir(&self) -> Option<PathBuf> {
        if let Some(ref custom) = self.config.worktrees_dir {
            return Some(worktree::expand_tilde(custom));
        }
        self.acp_env
            .data_dir
            .as_deref()
            .map(|p| p.join("worktrees"))
            .or_else(|| {
                self.accounts_dir
                    .as_deref()
                    .and_then(Path::parent)
                    .map(|p| p.join("worktrees"))
            })
    }

    pub fn socket_path(&self) -> Option<&Path> {
        self.socket_path.as_deref()
    }

    pub fn subscribe_events(&self) -> broadcast::Receiver<Notification> {
        self.events_tx.subscribe()
    }

    pub fn create(&self, mut params: CreateParams) -> Result<Arc<Session>, RpcError> {
        let has_project_id = params
            .project_id
            .as_deref()
            .is_some_and(|id| !id.trim().is_empty());
        if !has_project_id {
            params.project_id = self
                .store
                .find_project_for_cwd(&params.cwd)
                .ok()
                .flatten()
                .map(|p| p.id);
        }

        let mut agent_kind = params
            .command
            .first()
            .and_then(|cmd| Path::new(cmd).file_name())
            .and_then(|name| name.to_str())
            .and_then(AgentKind::parse);

        params.account_id = params
            .account_id
            .take()
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty());
        if let Some(ref account_id) = params.account_id {
            let account = self
                .store
                .get_account(account_id)
                .map_err(store_error_to_rpc)?;
            agent_kind = Some(account.agent);
            if let Some((key, val)) = accounts::env_for_account(&account) {
                params.env.insert(key.to_string(), val);
            }
        }

        let waiting_patterns = agent_kind
            .and_then(|kind| self.config.compiled_waiting_patterns(kind).ok())
            .unwrap_or_default();
        let busy_patterns = agent_kind
            .and_then(|kind| self.config.compiled_busy_patterns(kind).ok())
            .unwrap_or_default();

        if let Some(sock) = self
            .socket_path
            .clone()
            .or_else(|| default_socket_path().ok())
        {
            params.env.insert(
                "PAS_SOCKET".to_string(),
                sock.to_string_lossy().into_owned(),
            );
        }

        if params.mode == Mode::Chat {
            let (resolved_cmd, resolved_agent) =
                acp::resolve_adapter_command(&params.command, agent_kind, &self.acp_env)
                    .map_err(acp_error_to_rpc)?;
            params.command = resolved_cmd;
            agent_kind = resolved_agent;
        }

        let base_title = params
            .title
            .take()
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| params.command.first().cloned().unwrap_or_default());

        let mut created_worktree = if let Some(wt_req) = params.worktree.take() {
            let project_id = match params
                .project_id
                .as_deref()
                .map(str::trim)
                .filter(|id| !id.is_empty())
            {
                Some(id) => id.to_string(),
                None => {
                    let Some(root) = worktree::git_repo_root(&params.cwd) else {
                        return Err(RpcError {
                            code: ERR_NOT_A_GIT_REPO,
                            message: format!(
                                "{} no es un repositorio git. Entrá a un repo git para abrir una sesión en worktree.",
                                params.cwd.display()
                            ),
                        });
                    };
                    let proj = match self.store.find_project_for_cwd(&root) {
                        Ok(Some(existing)) => existing,
                        _ => match self.store.add_project(&root, None) {
                            Ok(added) => added,
                            Err(StoreError::ProjectAlreadyExists { .. }) => self
                                .store
                                .find_project_for_cwd(&root)
                                .map_err(store_error_to_rpc)?
                                .ok_or_else(|| RpcError {
                                    code: ERR_NOT_A_GIT_REPO,
                                    message: format!(
                                        "{} no es un repositorio git. Entrá a un repo git para abrir una sesión en worktree.",
                                        params.cwd.display()
                                    ),
                                })?,
                            Err(err) => return Err(store_error_to_rpc(err)),
                        },
                    };
                    params.project_id = Some(proj.id.clone());
                    proj.id
                }
            };

            let agent_name = agent_kind.map(|a| a.as_str().to_string()).or_else(|| {
                params
                    .command
                    .first()
                    .and_then(|c| Path::new(c).file_name())
                    .and_then(|n| n.to_str())
                    .map(ToOwned::to_owned)
            });
            let wt_dir = self.worktrees_dir();
            let wt = worktree::create(
                &self.store,
                wt_dir.as_deref(),
                WorktreeCreateParams {
                    project_id,
                    branch: wt_req.branch,
                    base: wt_req.base,
                    agent: agent_name,
                    title: Some(base_title.clone()),
                },
            )
            .map_err(worktree::worktree_error_to_rpc)?;
            params.cwd = wt.path.clone();
            Some(wt)
        } else {
            None
        };

        let mut sessions = self.sessions.write().expect("sessions lock poisoned");

        let title = unique_title(&sessions, &base_title);
        let id = unique_id(&sessions);
        params.title = Some(title.clone());
        if let Some(ref mut wt) = created_worktree {
            wt.session_id = Some(id.clone());
        }

        let spawn_result = match params.mode {
            Mode::Terminal => Session::spawn_terminal(
                id.clone(),
                params,
                self.idle_ms,
                waiting_patterns,
                busy_patterns,
                created_worktree.clone(),
            ),
            Mode::Chat => {
                let chats_dir = self
                    .acp_env
                    .data_dir
                    .as_deref()
                    .map(|p| p.join("chats"))
                    .or_else(|| {
                        self.accounts_dir
                            .as_deref()
                            .and_then(Path::parent)
                            .map(|p| p.join("chats"))
                    });
                Session::spawn_chat(
                    id.clone(),
                    params,
                    agent_kind,
                    chats_dir.as_deref(),
                    self.events_tx.clone(),
                    created_worktree.clone(),
                )
            }
        };
        let session = match spawn_result {
            Ok(session) => session,
            Err(err) => {
                drop(sessions);
                if let Some(wt) = created_worktree {
                    let _ = worktree::remove(&self.store, &wt.id, true);
                }
                return Err(session_error_to_rpc(err));
            }
        };
        sessions.insert(id.clone(), Arc::clone(&session));
        drop(sessions);

        if let Some(ref wt) = created_worktree {
            let _ = self.store.set_worktree_session(&wt.id, Some(&id));
        }

        let mut state_rx = session.subscribe_state();
        let mut info = session.info();
        let initial_state = state_rx.borrow_and_update().clone();
        info.state = initial_state.clone();
        let initial_exited = matches!(info.state, SessionState::Exited { .. });
        let _ = self.events_tx.send(Notification::new(
            EVENT_SESSION_CREATED,
            json!(SessionCreatedEvent { info }),
        ));

        if !initial_exited {
            let events_tx = self.events_tx.clone();
            let notifier = Arc::clone(&self.notifier);
            let notifications_enabled = self.config.notifications;
            let session_ref = Arc::clone(&session);
            tokio::spawn(async move {
                let mut prev_state = initial_state;
                while state_rx.changed().await.is_ok() {
                    let state = state_rx.borrow_and_update().clone();
                    let is_exited = matches!(state, SessionState::Exited { .. });
                    let _ = events_tx.send(Notification::new(
                        EVENT_SESSION_STATE,
                        json!(SessionStateEvent {
                            id: id.clone(),
                            state: state.clone(),
                        }),
                    ));
                    if state == SessionState::Waiting
                        && prev_state != SessionState::Waiting
                        && notifications_enabled
                    {
                        let last_line = session_ref.last_visible_line();
                        let notice = WaitingNotice::new(&id, &title, last_line.as_deref());
                        let focus_tx = events_tx.clone();
                        let focus_id = id.clone();
                        notifier.notify_waiting(
                            notice,
                            Box::new(move || {
                                let _ = focus_tx.send(Notification::new(
                                    EVENT_SESSION_FOCUS_REQUEST,
                                    json!(SessionFocusRequestEvent { id: focus_id }),
                                ));
                            }),
                        );
                    }
                    prev_state = state;
                    if is_exited {
                        break;
                    }
                }
            });
        }

        Ok(session)
    }

    pub fn kill(&self, id_or_title: &str) -> Result<(), RpcError> {
        let session = self.get(id_or_title)?;
        let id = session.info().id;
        session.kill();
        let _ = self.events_tx.send(Notification::new(
            EVENT_SESSION_REMOVED,
            json!(SessionRemovedEvent { id }),
        ));
        Ok(())
    }

    pub fn remove(&self, id_or_title: &str) -> Result<(), RpcError> {
        let mut sessions = self.sessions.write().expect("sessions lock poisoned");
        let id = if sessions.contains_key(id_or_title) {
            id_or_title.to_string()
        } else if let Some((k, _)) = sessions.iter().find(|(_, s)| s.info().title == id_or_title) {
            k.clone()
        } else {
            return Err(RpcError {
                code: ERR_SESSION_NOT_FOUND,
                message: format!(
                    "No hay ninguna sesión llamada {id_or_title}. Mirá las disponibles con `pas list`."
                ),
            });
        };

        let session = sessions.get(&id).expect("sesión recién localizada");
        if !matches!(session.state(), SessionState::Exited { .. }) {
            return Err(RpcError {
                code: ERR_SESSION_RUNNING,
                message: "La sesión sigue corriendo. Cerrala primero con Ctrl+C o matala.".into(),
            });
        }

        sessions.remove(&id);
        drop(sessions);

        let _ = self.events_tx.send(Notification::new(
            EVENT_SESSION_REMOVED,
            json!(SessionRemovedEvent { id }),
        ));
        Ok(())
    }

    pub fn get(&self, id_or_title: &str) -> Result<Arc<Session>, RpcError> {
        let sessions = self.sessions.read().expect("sessions lock poisoned");
        if let Some(s) = sessions.get(id_or_title) {
            return Ok(Arc::clone(s));
        }
        if let Some(s) = sessions.values().find(|s| s.info().title == id_or_title) {
            return Ok(Arc::clone(s));
        }
        Err(RpcError {
            code: ERR_SESSION_NOT_FOUND,
            message: format!(
                "No hay ninguna sesión llamada {id_or_title}. Mirá las disponibles con `pas list`."
            ),
        })
    }

    pub fn list(&self) -> Vec<SessionInfo> {
        let sessions = self.sessions.read().expect("sessions lock poisoned");
        let mut list: Vec<SessionInfo> = sessions.values().map(|s| s.info()).collect();
        list.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
        list
    }

    pub fn kill_all(&self) {
        let sessions = self.sessions.read().expect("sessions lock poisoned");
        for s in sessions.values() {
            s.kill();
        }
    }
}

pub(crate) fn session_error_to_rpc(err: SessionError) -> RpcError {
    let code = match err {
        SessionError::Spawn { .. } => ERR_SPAWN_FAILED,
        SessionError::Timeout { .. } => ERR_WAIT_TIMEOUT,
        SessionError::Exited { .. } => ERR_SESSION_EXITED,
        SessionError::InvalidSize => ERR_INVALID_PARAMS,
        SessionError::WrongMode { .. } => ERR_WRONG_SESSION_MODE,
        SessionError::PermissionNotFound { .. } => ERR_PERMISSION_NOT_FOUND,
        SessionError::ChatBusy { .. } => ERR_CHAT_BUSY,
        SessionError::Io(_) => -32603,
    };
    RpcError {
        code,
        message: err.to_string(),
    }
}

pub(crate) fn acp_error_to_rpc(err: AcpError) -> RpcError {
    let code = match err {
        AcpError::MissingAdapter(_) => ERR_ADAPTER_NOT_FOUND,
        AcpError::BuiltInAdapter | AcpError::NoDataDir => ERR_INVALID_PARAMS,
        AcpError::InstallFailed(_) => ERR_ADAPTER_INSTALL_FAILED,
        AcpError::Io(_) => -32603,
    };
    RpcError {
        code,
        message: err.to_string(),
    }
}

fn unique_title(sessions: &HashMap<String, Arc<Session>>, base: &str) -> String {
    let exists = |candidate: &str| sessions.values().any(|s| s.info().title == candidate);
    if !exists(base) {
        return base.to_string();
    }
    let mut n = 2u32;
    loop {
        let candidate = format!("{base}-{n}");
        if !exists(&candidate) {
            return candidate;
        }
        n += 1;
    }
}

fn unique_id(sessions: &HashMap<String, Arc<Session>>) -> String {
    let mut bytes = [0u8; 4];
    loop {
        getrandom::fill(&mut bytes).expect("falló getrandom");
        let id = format!(
            "{:02x}{:02x}{:02x}{:02x}",
            bytes[0], bytes[1], bytes[2], bytes[3]
        );
        if !sessions.contains_key(&id) {
            return id;
        }
    }
}

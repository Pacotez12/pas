use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use pas_core::protocol::{
    CreateParams, ERR_INVALID_PARAMS, ERR_SESSION_EXITED, ERR_SESSION_NOT_FOUND,
    ERR_SESSION_RUNNING, ERR_SPAWN_FAILED, ERR_WAIT_TIMEOUT, EVENT_SESSION_CREATED,
    EVENT_SESSION_REMOVED, EVENT_SESSION_STATE, Notification, RpcError, SessionCreatedEvent,
    SessionInfo, SessionRemovedEvent, SessionState, SessionStateEvent,
};
use serde_json::json;
use tokio::sync::broadcast;

use crate::session::{Session, SessionError};
use crate::store::Store;

const EVENTS_BROADCAST_CAPACITY: usize = 256;

#[derive(Debug)]
pub struct Manager {
    idle_ms: u64,
    sessions: RwLock<HashMap<String, Arc<Session>>>,
    events_tx: broadcast::Sender<Notification>,
    store: Store,
}

impl Manager {
    pub fn new(idle_ms: u64) -> Arc<Self> {
        let store = Store::open_in_memory().expect("no se pudo abrir SQLite en memoria");
        Self::with_store(idle_ms, store)
    }

    pub fn with_store(idle_ms: u64, store: Store) -> Arc<Self> {
        let (events_tx, _) = broadcast::channel(EVENTS_BROADCAST_CAPACITY);
        Arc::new(Self {
            idle_ms,
            sessions: RwLock::new(HashMap::new()),
            events_tx,
            store,
        })
    }

    pub fn store(&self) -> &Store {
        &self.store
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

        let base_title = params
            .title
            .take()
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| params.command.first().cloned().unwrap_or_default());

        let mut sessions = self.sessions.write().expect("sessions lock poisoned");

        let title = unique_title(&sessions, &base_title);
        let id = unique_id(&sessions);
        params.title = Some(title);

        let session =
            Session::spawn(id.clone(), params, self.idle_ms).map_err(session_error_to_rpc)?;
        sessions.insert(id.clone(), Arc::clone(&session));
        drop(sessions);

        let mut state_rx = session.subscribe_state();
        let mut info = session.info();
        info.state = state_rx.borrow_and_update().clone();
        let initial_exited = matches!(info.state, SessionState::Exited { .. });
        let _ = self.events_tx.send(Notification::new(
            EVENT_SESSION_CREATED,
            json!(SessionCreatedEvent { info }),
        ));

        if !initial_exited {
            let events_tx = self.events_tx.clone();
            tokio::spawn(async move {
                while state_rx.changed().await.is_ok() {
                    let state = state_rx.borrow_and_update().clone();
                    let is_exited = matches!(state, SessionState::Exited { .. });
                    let _ = events_tx.send(Notification::new(
                        EVENT_SESSION_STATE,
                        json!(SessionStateEvent {
                            id: id.clone(),
                            state,
                        }),
                    ));
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
        SessionError::Io(_) => -32603,
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

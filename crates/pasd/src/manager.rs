use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use pas_core::protocol::{
    CreateParams, ERR_SESSION_EXITED, ERR_SESSION_NOT_FOUND, ERR_SPAWN_FAILED, ERR_WAIT_TIMEOUT,
    RpcError, SessionInfo,
};

use crate::session::{Session, SessionError};

#[derive(Debug)]
pub struct Manager {
    idle_ms: u64,
    sessions: RwLock<HashMap<String, Arc<Session>>>,
}

impl Manager {
    pub fn new(idle_ms: u64) -> Arc<Self> {
        Arc::new(Self {
            idle_ms,
            sessions: RwLock::new(HashMap::new()),
        })
    }

    pub fn create(&self, mut params: CreateParams) -> Result<Arc<Session>, RpcError> {
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
        sessions.insert(id, Arc::clone(&session));
        Ok(session)
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

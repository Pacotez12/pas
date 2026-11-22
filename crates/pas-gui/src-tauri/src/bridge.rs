use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use pas_core::CoreError;
use pas_core::autostart::connect_or_spawn;
use pas_core::client::Client;
use pas_core::protocol::{
    CreateParams, CreateResult, ERR_SESSION_NOT_FOUND, EVENT_SESSION_CREATED, EVENT_SESSION_OUTPUT,
    EVENT_SESSION_REMOVED, EVENT_SESSION_STATE, GroupCreateParams, GroupInfo, GroupRenameParams,
    IdParams, InputParams, METHOD_GROUP_CREATE, METHOD_GROUP_LIST, METHOD_GROUP_RENAME,
    METHOD_PROJECT_ADD, METHOD_PROJECT_LIST, METHOD_PROJECT_MOVE, METHOD_PROJECT_REMOVE,
    METHOD_PROJECT_RENAME, METHOD_PROJECT_UNPIN, METHOD_SESSION_CREATE, METHOD_SESSION_INPUT,
    METHOD_SESSION_KILL, METHOD_SESSION_LIST, METHOD_SESSION_RESIZE, METHOD_SESSION_SNAPSHOT,
    METHOD_SESSION_SUBSCRIBE, METHOD_SESSION_SUBSCRIBE_ALL, METHOD_SESSION_UNSUBSCRIBE,
    Notification, ProjectAddParams, ProjectInfo, ProjectMoveParams, ProjectRenameParams,
    ResizeParams, SessionInfo, SessionRemovedEvent, SnapshotResult,
};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio::sync::{Mutex, mpsc, oneshot};
use tokio::task::JoinHandle;

pub const FRONTEND_EVENT_OUTPUT: &str = "pas://output";
pub const FRONTEND_EVENT_SNAPSHOT: &str = "pas://snapshot";
pub const FRONTEND_EVENT_STATE: &str = "pas://state";
pub const FRONTEND_EVENT_CREATED: &str = "pas://created";
pub const FRONTEND_EVENT_REMOVED: &str = "pas://removed";
pub const FRONTEND_EVENT_CONNECTION: &str = "pas://connection";

const BACKOFF_MS: &[u64] = &[250, 500, 1_000, 2_000, 5_000];

pub trait EventSink: Send + Sync + 'static {
    fn emit(&self, event: &str, payload: Value);
}

enum SubCmd {
    Subscribe {
        id: String,
        reply: oneshot::Sender<Result<(), CoreError>>,
    },
    Unsubscribe {
        id: String,
        reply: oneshot::Sender<Result<(), CoreError>>,
    },
}

pub struct Bridge {
    socket_path: PathBuf,
    rpc_client: Mutex<Option<Client>>,
    sub_tx: mpsc::UnboundedSender<SubCmd>,
    event_task: std::sync::Mutex<Option<JoinHandle<()>>>,
}

impl Bridge {
    pub fn new(socket_path: PathBuf, sink: Arc<dyn EventSink>) -> Self {
        let (sub_tx, sub_rx) = mpsc::unbounded_channel();
        let handle = tokio::runtime::Handle::try_current()
            .unwrap_or_else(|_| tauri::async_runtime::handle().inner().clone());
        let event_task = handle.spawn(run_event_loop(socket_path.clone(), sink, sub_rx));
        Self {
            socket_path,
            rpc_client: Mutex::new(None),
            sub_tx,
            event_task: std::sync::Mutex::new(Some(event_task)),
        }
    }

    async fn call<P: Serialize + Clone, R: DeserializeOwned>(
        &self,
        method: &str,
        params: P,
    ) -> Result<R, CoreError> {
        let mut guard = self.rpc_client.lock().await;
        let was_cached = guard.is_some();

        if guard.is_none() {
            *guard = Some(connect_or_spawn(&self.socket_path).await?);
        }

        let res = guard
            .as_mut()
            .expect("cliente RPC inicializado")
            .call(method, params.clone())
            .await;

        match res {
            Ok(val) => Ok(val),
            Err(CoreError::Io(_)) if was_cached => {
                *guard = None;
                let mut client = connect_or_spawn(&self.socket_path).await?;
                let retry = client.call(method, params).await;
                if retry.is_ok() {
                    *guard = Some(client);
                }
                retry
            }
            Err(err @ CoreError::Io(_)) => {
                *guard = None;
                Err(err)
            }
            Err(err) => Err(err),
        }
    }

    pub async fn list_projects(&self) -> Result<Vec<ProjectInfo>, CoreError> {
        self.call(METHOD_PROJECT_LIST, json!({})).await
    }

    pub async fn add_project(
        &self,
        path: PathBuf,
        name: Option<String>,
    ) -> Result<ProjectInfo, CoreError> {
        self.call(METHOD_PROJECT_ADD, ProjectAddParams { path, name })
            .await
    }

    pub async fn remove_project(&self, id: String) -> Result<(), CoreError> {
        let _: Value = self.call(METHOD_PROJECT_REMOVE, IdParams { id }).await?;
        Ok(())
    }

    pub async fn rename_project(
        &self,
        id: String,
        name: String,
    ) -> Result<ProjectInfo, CoreError> {
        self.call(METHOD_PROJECT_RENAME, ProjectRenameParams { id, name })
            .await
    }

    pub async fn move_project(
        &self,
        id: String,
        group_id: String,
    ) -> Result<ProjectInfo, CoreError> {
        self.call(METHOD_PROJECT_MOVE, ProjectMoveParams { id, group_id })
            .await
    }

    pub async fn unpin_project(&self, id: String) -> Result<ProjectInfo, CoreError> {
        self.call(METHOD_PROJECT_UNPIN, IdParams { id }).await
    }

    pub async fn list_groups(&self) -> Result<Vec<GroupInfo>, CoreError> {
        self.call(METHOD_GROUP_LIST, json!({})).await
    }

    pub async fn create_group(&self, name: String) -> Result<GroupInfo, CoreError> {
        self.call(METHOD_GROUP_CREATE, GroupCreateParams { name })
            .await
    }

    pub async fn rename_group(&self, id: String, name: String) -> Result<GroupInfo, CoreError> {
        self.call(METHOD_GROUP_RENAME, GroupRenameParams { id, name })
            .await
    }

    pub async fn list_sessions(&self) -> Result<Vec<SessionInfo>, CoreError> {
        self.call(METHOD_SESSION_LIST, json!({})).await
    }

    pub async fn create_session(&self, params: CreateParams) -> Result<CreateResult, CoreError> {
        self.call(METHOD_SESSION_CREATE, params).await
    }

    pub async fn kill_session(&self, id: String) -> Result<(), CoreError> {
        let _: Value = self.call(METHOD_SESSION_KILL, IdParams { id }).await?;
        Ok(())
    }

    pub async fn session_input(&self, id: String, data_b64: String) -> Result<(), CoreError> {
        let _: Value = self
            .call(METHOD_SESSION_INPUT, InputParams { id, data_b64 })
            .await?;
        Ok(())
    }

    pub async fn session_resize(&self, id: String, cols: u16, rows: u16) -> Result<(), CoreError> {
        let _: Value = self
            .call(METHOD_SESSION_RESIZE, ResizeParams { id, cols, rows })
            .await?;
        Ok(())
    }

    pub async fn session_snapshot(&self, id: String) -> Result<SnapshotResult, CoreError> {
        self.call(METHOD_SESSION_SNAPSHOT, IdParams { id }).await
    }

    pub async fn subscribe(&self, id: String) -> Result<(), CoreError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sub_tx
            .send(SubCmd::Subscribe {
                id,
                reply: reply_tx,
            })
            .map_err(|_| CoreError::NotRunning)?;
        reply_rx.await.map_err(|_| CoreError::NotRunning)?
    }

    pub async fn unsubscribe(&self, id: String) -> Result<(), CoreError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sub_tx
            .send(SubCmd::Unsubscribe {
                id,
                reply: reply_tx,
            })
            .map_err(|_| CoreError::NotRunning)?;
        reply_rx.await.map_err(|_| CoreError::NotRunning)?
    }

    pub fn shutdown(&self) {
        if let Ok(mut guard) = self.event_task.lock()
            && let Some(handle) = guard.take()
        {
            handle.abort();
        }
    }
}

impl Drop for Bridge {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn backoff_duration(attempt: usize) -> Duration {
    let idx = attempt.min(BACKOFF_MS.len() - 1);
    Duration::from_millis(BACKOFF_MS[idx])
}

async fn run_event_loop(
    socket_path: PathBuf,
    sink: Arc<dyn EventSink>,
    mut sub_rx: mpsc::UnboundedReceiver<SubCmd>,
) {
    let mut subscribed: HashSet<String> = HashSet::new();
    let mut attempt: usize = 0;
    let mut emitted_disconnected = false;

    loop {
        let mut client = match connect_or_spawn(&socket_path).await {
            Ok(c) => c,
            Err(err) => {
                tracing::debug!("no se pudo conectar al canal de eventos de pasd: {err}");
                if !emitted_disconnected {
                    sink.emit(FRONTEND_EVENT_CONNECTION, json!("disconnected"));
                    emitted_disconnected = true;
                }
                if !sleep_with_sub_queue(backoff_duration(attempt), &mut sub_rx, &mut subscribed)
                    .await
                {
                    return;
                }
                attempt = attempt.saturating_add(1);
                continue;
            }
        };

        if let Err(err) = client
            .call::<_, Value>(METHOD_SESSION_SUBSCRIBE_ALL, json!({}))
            .await
        {
            tracing::debug!("falló session.subscribe_all: {err}");
            if !emitted_disconnected {
                sink.emit(FRONTEND_EVENT_CONNECTION, json!("disconnected"));
                emitted_disconnected = true;
            }
            if !sleep_with_sub_queue(backoff_duration(attempt), &mut sub_rx, &mut subscribed).await
            {
                return;
            }
            attempt = attempt.saturating_add(1);
            continue;
        }

        attempt = 0;
        sink.emit(FRONTEND_EVENT_CONNECTION, json!("connected"));

        // Re-suscribir sesiones activas tras una reconexión.
        let mut dead_ids = Vec::new();
        let mut resub_io_err = false;
        for id in &subscribed {
            match client
                .call::<_, Value>(METHOD_SESSION_SUBSCRIBE, IdParams { id: id.clone() })
                .await
            {
                Ok(_) => {}
                Err(CoreError::Rpc {
                    code: ERR_SESSION_NOT_FOUND,
                    ..
                }) => {
                    dead_ids.push(id.clone());
                }
                Err(CoreError::Io(_)) => {
                    resub_io_err = true;
                    break;
                }
                Err(err) => {
                    tracing::debug!("error re-suscribiendo sesión {id}: {err}");
                }
            }
        }
        for id in dead_ids {
            subscribed.remove(&id);
        }
        if resub_io_err {
            sink.emit(FRONTEND_EVENT_CONNECTION, json!("disconnected"));
            emitted_disconnected = true;
            if !sleep_with_sub_queue(backoff_duration(attempt), &mut sub_rx, &mut subscribed).await
            {
                return;
            }
            attempt = attempt.saturating_add(1);
            continue;
        }

        loop {
            tokio::select! {
                ev = client.next_event() => {
                    match ev {
                        Ok(Some(notif)) => {
                            forward_notification(&sink, &mut subscribed, notif);
                        }
                        Ok(None) | Err(_) => {
                            break;
                        }
                    }
                }
                cmd = sub_rx.recv() => {
                    let Some(cmd) = cmd else {
                        return;
                    };
                    match cmd {
                        SubCmd::Subscribe { id, reply } => {
                            let res = client
                                .call::<_, Value>(
                                    METHOD_SESSION_SUBSCRIBE,
                                    IdParams { id: id.clone() },
                                )
                                .await
                                .map(|_| ());
                            let broken = matches!(res, Err(CoreError::Io(_)));
                            if res.is_ok() {
                                subscribed.insert(id);
                            }
                            let _ = reply.send(res);
                            if broken {
                                break;
                            }
                        }
                        SubCmd::Unsubscribe { id, reply } => {
                            subscribed.remove(&id);
                            let res = client
                                .call::<_, Value>(METHOD_SESSION_UNSUBSCRIBE, IdParams { id })
                                .await
                                .map(|_| ());
                            let broken = matches!(res, Err(CoreError::Io(_)));
                            let _ = reply.send(res);
                            if broken {
                                break;
                            }
                        }
                    }
                }
            }
        }

        sink.emit(FRONTEND_EVENT_CONNECTION, json!("disconnected"));
        emitted_disconnected = true;
        if !sleep_with_sub_queue(backoff_duration(attempt), &mut sub_rx, &mut subscribed).await {
            return;
        }
        attempt = attempt.saturating_add(1);
    }
}

async fn sleep_with_sub_queue(
    dur: Duration,
    sub_rx: &mut mpsc::UnboundedReceiver<SubCmd>,
    subscribed: &mut HashSet<String>,
) -> bool {
    let sleep = tokio::time::sleep(dur);
    tokio::pin!(sleep);

    loop {
        tokio::select! {
            _ = &mut sleep => return true,
            cmd = sub_rx.recv() => {
                let Some(cmd) = cmd else {
                    return false;
                };
                match cmd {
                    SubCmd::Subscribe { id, reply } => {
                        subscribed.insert(id);
                        let _ = reply.send(Err(CoreError::NotRunning));
                    }
                    SubCmd::Unsubscribe { id, reply } => {
                        subscribed.remove(&id);
                        let _ = reply.send(Ok(()));
                    }
                }
            }
        }
    }
}

fn forward_notification(
    sink: &Arc<dyn EventSink>,
    subscribed: &mut HashSet<String>,
    notif: Notification,
) {
    match notif.method.as_str() {
        EVENT_SESSION_OUTPUT => sink.emit(FRONTEND_EVENT_OUTPUT, notif.params),
        METHOD_SESSION_SNAPSHOT => sink.emit(FRONTEND_EVENT_SNAPSHOT, notif.params),
        EVENT_SESSION_STATE => sink.emit(FRONTEND_EVENT_STATE, notif.params),
        EVENT_SESSION_CREATED => sink.emit(FRONTEND_EVENT_CREATED, notif.params),
        EVENT_SESSION_REMOVED => {
            if let Ok(ev) = serde_json::from_value::<SessionRemovedEvent>(notif.params.clone()) {
                subscribed.remove(&ev.id);
            }
            sink.emit(FRONTEND_EVENT_REMOVED, notif.params);
        }
        other => {
            tracing::debug!("notificación desconocida de pasd: {other}");
        }
    }
}

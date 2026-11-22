use std::collections::HashMap;
use std::fs::{self, Permissions};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream as StdUnixStream;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use pas_core::CoreError;
use pas_core::protocol::{
    ERR_INVALID_PARAMS, EVENT_SESSION_OUTPUT, IdParams, METHOD_SESSION_SNAPSHOT,
    METHOD_SESSION_SUBSCRIBE, METHOD_SESSION_SUBSCRIBE_ALL, METHOD_SESSION_UNSUBSCRIBE,
    Notification, Request, Response, RpcError, SessionOutputEvent,
};
use pas_core::socket::{read_msg, validate_socket_path};
use serde::Serialize;
use serde_json::json;
use tokio::io::{AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;

use crate::manager::Manager;
use crate::rpc;

const CLIENT_OUTBOUND_QUEUE: usize = 32;

#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    #[error("pasd ya está corriendo en {}.", path.display())]
    AlreadyRunning { path: PathBuf },

    #[error(transparent)]
    Core(#[from] CoreError),

    #[error("{0}")]
    Io(#[from] std::io::Error),
}

pub fn bind(path: &Path) -> Result<UnixListener, ServerError> {
    validate_socket_path(path)?;
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
        && !parent.exists()
    {
        fs::create_dir_all(parent)?;
        fs::set_permissions(parent, Permissions::from_mode(0o700))?;
    }

    if path.symlink_metadata().is_ok() {
        if StdUnixStream::connect(path).is_ok() {
            return Err(ServerError::AlreadyRunning {
                path: path.to_path_buf(),
            });
        }
        fs::remove_file(path)?;
    }

    let listener = UnixListener::bind(path)?;
    fs::set_permissions(path, Permissions::from_mode(0o600))?;
    Ok(listener)
}

pub async fn serve(listener: UnixListener, mgr: Arc<Manager>) {
    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                let mgr = Arc::clone(&mgr);
                tokio::spawn(handle_client(stream, mgr));
            }
            Err(err) => {
                tracing::warn!("error aceptando conexión: {err}");
            }
        }
    }
}

async fn handle_client(stream: UnixStream, mgr: Arc<Manager>) {
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let (tx, mut rx) = mpsc::channel::<Vec<u8>>(CLIENT_OUTBOUND_QUEUE);

    let writer_task = tokio::spawn(async move {
        while let Some(frame) = rx.recv().await {
            if let Err(err) = writer.write_all(&frame).await {
                tracing::debug!("error escribiendo al cliente: {err}");
                break;
            }
            if let Err(err) = writer.flush().await {
                tracing::debug!("error haciendo flush al cliente: {err}");
                break;
            }
        }
    });

    let mut subs: HashMap<String, JoinHandle<()>> = HashMap::new();
    let mut global_sub: Option<JoinHandle<()>> = None;

    loop {
        match read_msg::<Request>(&mut reader).await {
            Ok(Some(req)) => {
                let resp = match req.method.as_str() {
                    METHOD_SESSION_SUBSCRIBE => handle_subscribe(&mgr, &mut subs, &tx, req),
                    METHOD_SESSION_UNSUBSCRIBE => handle_unsubscribe(&mgr, &mut subs, req),
                    METHOD_SESSION_SUBSCRIBE_ALL => {
                        handle_subscribe_all(&mgr, &mut global_sub, &tx, req)
                    }
                    _ => rpc::dispatch(&mgr, req).await,
                };
                if send_frame(&tx, &resp).await.is_err() {
                    break;
                }
            }
            Ok(None) => break,
            Err(err) => {
                tracing::debug!("conexión cerrada por error de lectura: {err}");
                break;
            }
        }
    }

    if let Some(handle) = global_sub {
        handle.abort();
    }
    for (_, handle) in subs {
        handle.abort();
    }
    drop(tx);
    let _ = writer_task.await;
}

fn handle_subscribe_all(
    mgr: &Manager,
    global_sub: &mut Option<JoinHandle<()>>,
    tx: &mpsc::Sender<Vec<u8>>,
    req: Request,
) -> Response {
    if let Some(prev) = global_sub.take() {
        prev.abort();
    }

    let mut ev_rx = mgr.subscribe_events();
    let tx = tx.clone();
    let handle = tokio::spawn(async move {
        loop {
            match ev_rx.recv().await {
                Ok(notif) => {
                    if send_frame(&tx, &notif).await.is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    *global_sub = Some(handle);
    Response::ok(req.id, json!({}))
}

fn handle_subscribe(
    mgr: &Manager,
    subs: &mut HashMap<String, JoinHandle<()>>,
    tx: &mpsc::Sender<Vec<u8>>,
    req: Request,
) -> Response {
    let id = req.id;
    let params: IdParams = match serde_json::from_value(req.params) {
        Ok(p) => p,
        Err(e) => {
            return Response::err(
                id,
                RpcError {
                    code: ERR_INVALID_PARAMS,
                    message: format!("Parámetros inválidos: {e}. Revisá el formato del pedido."),
                },
            );
        }
    };

    let session = match mgr.get(&params.id) {
        Ok(s) => s,
        Err(err) => return Response::err(id, err),
    };

    let sid = session.info().id;
    if let Some(prev) = subs.remove(&sid) {
        prev.abort();
    }

    let mut out_rx = session.subscribe_output();
    let tx = tx.clone();
    let task_sid = sid.clone();
    let handle = tokio::spawn(async move {
        loop {
            match out_rx.recv().await {
                Ok(chunk) => {
                    let notif = Notification::new(
                        EVENT_SESSION_OUTPUT,
                        json!(SessionOutputEvent {
                            id: task_sid.clone(),
                            data_b64: STANDARD.encode(&chunk),
                        }),
                    );
                    if send_frame(&tx, &notif).await.is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    let snap = session.snapshot();
                    let notif = Notification::new(
                        METHOD_SESSION_SNAPSHOT,
                        json!(SessionOutputEvent {
                            id: task_sid.clone(),
                            data_b64: STANDARD.encode(&snap),
                        }),
                    );
                    if send_frame(&tx, &notif).await.is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    subs.insert(sid, handle);
    Response::ok(id, json!({}))
}

fn handle_unsubscribe(
    mgr: &Manager,
    subs: &mut HashMap<String, JoinHandle<()>>,
    req: Request,
) -> Response {
    let id = req.id;
    let params: IdParams = match serde_json::from_value(req.params) {
        Ok(p) => p,
        Err(e) => {
            return Response::err(
                id,
                RpcError {
                    code: ERR_INVALID_PARAMS,
                    message: format!("Parámetros inválidos: {e}. Revisá el formato del pedido."),
                },
            );
        }
    };

    let key = mgr
        .get(&params.id)
        .map(|s| s.info().id)
        .unwrap_or(params.id);
    if let Some(handle) = subs.remove(&key) {
        handle.abort();
    }
    Response::ok(id, json!({}))
}

async fn send_frame<T: Serialize>(tx: &mpsc::Sender<Vec<u8>>, msg: &T) -> Result<(), ()> {
    let mut buf = serde_json::to_vec(msg).map_err(|_| ())?;
    buf.push(b'\n');
    tx.send(buf).await.map_err(|_| ())
}

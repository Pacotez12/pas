use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use pas_core::protocol::{
    ClaudeConversationsParams, CreateParams, CreateResult, ERR_GROUP_ALREADY_EXISTS,
    ERR_GROUP_NOT_FOUND, ERR_INVALID_PARAMS, ERR_METHOD_NOT_FOUND, ERR_NOT_A_GIT_REPO,
    ERR_PROJECT_ALREADY_EXISTS, ERR_PROJECT_NOT_FOUND, GroupCreateParams, GroupRenameParams,
    IdParams, InputParams, METHOD_CLAUDE_CONVERSATIONS, METHOD_DAEMON_PING, METHOD_GROUP_CREATE,
    METHOD_GROUP_LIST, METHOD_GROUP_RENAME, METHOD_PROJECT_ADD, METHOD_PROJECT_LIST,
    METHOD_PROJECT_MOVE, METHOD_PROJECT_REMOVE, METHOD_PROJECT_RENAME, METHOD_PROJECT_UNPIN,
    METHOD_SESSION_CREATE, METHOD_SESSION_INPUT, METHOD_SESSION_KILL, METHOD_SESSION_LIST,
    METHOD_SESSION_READ, METHOD_SESSION_REMOVE, METHOD_SESSION_RESIZE, METHOD_SESSION_SEND,
    METHOD_SESSION_SNAPSHOT, METHOD_SESSION_WAIT, PingResult, ProjectAddParams, ProjectMoveParams,
    ProjectRenameParams, ReadParams, ReadResult, Request, ResizeParams, Response, RpcError,
    SendParams, SnapshotResult, WaitParams,
};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::claude;
use crate::manager::{Manager, session_error_to_rpc};
use crate::store::StoreError;

pub async fn dispatch(mgr: &Manager, req: Request) -> Response {
    let id = req.id;
    match handle(mgr, req).await {
        Ok(val) => Response::ok(id, val),
        Err(err) => Response::err(id, err),
    }
}

async fn handle(mgr: &Manager, req: Request) -> Result<Value, RpcError> {
    match req.method.as_str() {
        METHOD_DAEMON_PING => ok_json(PingResult {
            version: env!("CARGO_PKG_VERSION").into(),
        }),
        METHOD_SESSION_CREATE => {
            let params: CreateParams = parse_params(req.params)?;
            let session = mgr.create(params)?;
            ok_json(CreateResult {
                id: session.info().id,
            })
        }
        METHOD_SESSION_LIST => ok_json(mgr.list()),
        METHOD_SESSION_SEND => {
            let params: SendParams = parse_params(req.params)?;
            let session = mgr.get(&params.id)?;
            session
                .send(&params.text, params.enter)
                .map_err(session_error_to_rpc)?;
            Ok(json!({}))
        }
        METHOD_SESSION_INPUT => {
            let params: InputParams = parse_params(req.params)?;
            let bytes = STANDARD.decode(&params.data_b64).map_err(|_| RpcError {
                code: ERR_INVALID_PARAMS,
                message: "El campo `data_b64` no es base64 válido. Codificá los bytes en base64 estándar."
                    .into(),
            })?;
            let session = mgr.get(&params.id)?;
            session.input(&bytes).map_err(session_error_to_rpc)?;
            Ok(json!({}))
        }
        METHOD_SESSION_RESIZE => {
            let params: ResizeParams = parse_params(req.params)?;
            let session = mgr.get(&params.id)?;
            session
                .resize(params.cols, params.rows)
                .map_err(session_error_to_rpc)?;
            Ok(json!({}))
        }
        METHOD_SESSION_SNAPSHOT => {
            let params: IdParams = parse_params(req.params)?;
            let session = mgr.get(&params.id)?;
            let bytes = session.snapshot();
            ok_json(SnapshotResult {
                data_b64: STANDARD.encode(bytes),
            })
        }
        METHOD_SESSION_READ => {
            let params: ReadParams = parse_params(req.params)?;
            let session = mgr.get(&params.id)?;
            ok_json(ReadResult {
                text: session.read(params.lines),
            })
        }
        METHOD_SESSION_WAIT => {
            let params: WaitParams = parse_params(req.params)?;
            let session = mgr.get(&params.id)?;
            let state = session
                .wait(params.until, Duration::from_millis(params.timeout_ms))
                .await
                .map_err(session_error_to_rpc)?;
            ok_json(state)
        }
        METHOD_SESSION_KILL => {
            let params: IdParams = parse_params(req.params)?;
            mgr.kill(&params.id)?;
            Ok(json!({}))
        }
        METHOD_SESSION_REMOVE => {
            let params: IdParams = parse_params(req.params)?;
            mgr.remove(&params.id)?;
            Ok(json!({}))
        }
        METHOD_CLAUDE_CONVERSATIONS => {
            let params: ClaudeConversationsParams = parse_params(req.params)?;
            ok_json(claude::list_conversations(&params.cwd))
        }
        METHOD_PROJECT_ADD => {
            let params: ProjectAddParams = parse_params(req.params)?;
            let project = mgr
                .store()
                .add_project(&params.path, params.name.as_deref())
                .map_err(store_error_to_rpc)?;
            ok_json(project)
        }
        METHOD_PROJECT_LIST => {
            let projects = mgr.store().list_projects().map_err(store_error_to_rpc)?;
            ok_json(projects)
        }
        METHOD_PROJECT_REMOVE => {
            let params: IdParams = parse_params(req.params)?;
            mgr.store()
                .remove_project(&params.id)
                .map_err(store_error_to_rpc)?;
            Ok(json!({}))
        }
        METHOD_PROJECT_RENAME => {
            let params: ProjectRenameParams = parse_params(req.params)?;
            let project = mgr
                .store()
                .rename_project(&params.id, &params.name)
                .map_err(store_error_to_rpc)?;
            ok_json(project)
        }
        METHOD_PROJECT_MOVE => {
            let params: ProjectMoveParams = parse_params(req.params)?;
            let project = mgr
                .store()
                .move_project(&params.id, &params.group_id)
                .map_err(store_error_to_rpc)?;
            ok_json(project)
        }
        METHOD_PROJECT_UNPIN => {
            let params: IdParams = parse_params(req.params)?;
            let project = mgr
                .store()
                .unpin_project(&params.id)
                .map_err(store_error_to_rpc)?;
            ok_json(project)
        }
        METHOD_GROUP_LIST => {
            let groups = mgr.store().list_groups().map_err(store_error_to_rpc)?;
            ok_json(groups)
        }
        METHOD_GROUP_CREATE => {
            let params: GroupCreateParams = parse_params(req.params)?;
            let group = mgr
                .store()
                .create_group(&params.name)
                .map_err(store_error_to_rpc)?;
            ok_json(group)
        }
        METHOD_GROUP_RENAME => {
            let params: GroupRenameParams = parse_params(req.params)?;
            let group = mgr
                .store()
                .rename_group(&params.id, &params.name)
                .map_err(store_error_to_rpc)?;
            ok_json(group)
        }
        other => Err(RpcError {
            code: ERR_METHOD_NOT_FOUND,
            message: format!("Método desconocido: {other}."),
        }),
    }
}

fn store_error_to_rpc(err: StoreError) -> RpcError {
    let code = match err {
        StoreError::NotAGitRepo { .. } => ERR_NOT_A_GIT_REPO,
        StoreError::ProjectAlreadyExists { .. } => ERR_PROJECT_ALREADY_EXISTS,
        StoreError::ProjectNotFound { .. } => ERR_PROJECT_NOT_FOUND,
        StoreError::GroupNotFound { .. } => ERR_GROUP_NOT_FOUND,
        StoreError::GroupAlreadyExists { .. } => ERR_GROUP_ALREADY_EXISTS,
        StoreError::EmptyName { .. } => ERR_INVALID_PARAMS,
        StoreError::Sqlite(_) | StoreError::Io(_) => -32603,
    };
    RpcError {
        code,
        message: err.to_string(),
    }
}

fn parse_params<T: DeserializeOwned>(value: Value) -> Result<T, RpcError> {
    serde_json::from_value(value).map_err(|e| RpcError {
        code: ERR_INVALID_PARAMS,
        message: format!("Parámetros inválidos: {e}. Revisá el formato del pedido."),
    })
}

fn ok_json<T: serde::Serialize>(val: T) -> Result<Value, RpcError> {
    serde_json::to_value(val).map_err(|e| RpcError {
        code: -32603,
        message: e.to_string(),
    })
}

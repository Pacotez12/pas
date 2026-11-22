use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use pas_core::protocol::{
    CreateParams, CreateResult, ERR_INVALID_PARAMS, ERR_METHOD_NOT_FOUND, IdParams, InputParams,
    METHOD_DAEMON_PING, METHOD_SESSION_CREATE, METHOD_SESSION_INPUT, METHOD_SESSION_KILL,
    METHOD_SESSION_LIST, METHOD_SESSION_READ, METHOD_SESSION_RESIZE, METHOD_SESSION_SEND,
    METHOD_SESSION_SNAPSHOT, METHOD_SESSION_WAIT, PingResult, ReadParams, ReadResult, Request,
    ResizeParams, Response, RpcError, SendParams, SnapshotResult, WaitParams,
};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::manager::{Manager, session_error_to_rpc};

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
        other => Err(RpcError {
            code: ERR_METHOD_NOT_FOUND,
            message: format!("Método desconocido: {other}."),
        }),
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

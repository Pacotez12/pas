use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use pas_core::protocol::{
    AccountAddParams, AccountRemoveParams, AccountRenameParams, ClaudeConversationsParams,
    CreateParams, CreateResult, ERR_ACCOUNT_ALREADY_EXISTS, ERR_ACCOUNT_DIR_INVALID,
    ERR_ACCOUNT_NOT_FOUND, ERR_GROUP_ALREADY_EXISTS, ERR_GROUP_NOT_FOUND,
    ERR_HOOKS_CONFIG_INVALID, ERR_INVALID_PARAMS, ERR_METHOD_NOT_FOUND, ERR_NOT_A_GIT_REPO,
    ERR_PROJECT_ALREADY_EXISTS, ERR_PROJECT_NOT_FOUND, GroupCreateParams, GroupRenameParams,
    HookParams, IdParams, InputParams, METHOD_ACCOUNT_ADD, METHOD_ACCOUNT_INSTALL_HOOKS,
    METHOD_ACCOUNT_LIST, METHOD_ACCOUNT_REMOVE, METHOD_ACCOUNT_RENAME,
    METHOD_ACCOUNT_UNINSTALL_HOOKS, METHOD_CLAUDE_CONVERSATIONS, METHOD_DAEMON_PING,
    METHOD_GROUP_CREATE, METHOD_GROUP_LIST, METHOD_GROUP_RENAME, METHOD_PROJECT_ADD,
    METHOD_PROJECT_LIST, METHOD_PROJECT_MOVE, METHOD_PROJECT_REMOVE, METHOD_PROJECT_RENAME,
    METHOD_PROJECT_UNPIN, METHOD_QUOTA_LIST, METHOD_QUOTA_REPORT, METHOD_SESSION_CREATE,
    METHOD_SESSION_HOOK,
    METHOD_SESSION_INPUT, METHOD_SESSION_KILL, METHOD_SESSION_LIST, METHOD_SESSION_READ,
    METHOD_SESSION_REMOVE, METHOD_SESSION_RESIZE, METHOD_SESSION_SEND, METHOD_SESSION_SNAPSHOT,
    METHOD_SESSION_WAIT, PingResult, ProjectAddParams, ProjectMoveParams, ProjectRenameParams,
    QuotaReportParams, ReadParams, ReadResult, Request, ResizeParams, Response, RpcError,
    SendParams, SnapshotResult, WaitParams,
};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::accounts::{self, AccountHooksError};
use crate::claude;
use crate::hooks_install::HooksInstallError;
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
        METHOD_SESSION_HOOK => {
            let params: HookParams = parse_params(req.params)?;
            let session = mgr.get(&params.id)?;
            session.apply_hook(params.agent, &params.event);
            Ok(json!({}))
        }
        METHOD_CLAUDE_CONVERSATIONS => {
            let params: ClaudeConversationsParams = parse_params(req.params)?;
            let account_id = params
                .account_id
                .as_deref()
                .map(str::trim)
                .filter(|id| !id.is_empty());
            let convs = if let Some(account_id) = account_id {
                let account = mgr
                    .store()
                    .get_account(account_id)
                    .map_err(store_error_to_rpc)?;
                claude::list_conversations_in(&account.config_dir, &params.cwd)
            } else {
                claude::list_conversations(&params.cwd)
            };
            ok_json(convs)
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
        METHOD_ACCOUNT_LIST => {
            let accounts = accounts::list_accounts(mgr.store()).map_err(store_error_to_rpc)?;
            ok_json(accounts)
        }
        METHOD_ACCOUNT_ADD => {
            let params: AccountAddParams = parse_params(req.params)?;
            let account = accounts::add_account(mgr.store(), mgr.accounts_dir(), params)
                .map_err(store_error_to_rpc)?;
            ok_json(account)
        }
        METHOD_ACCOUNT_RENAME => {
            let params: AccountRenameParams = parse_params(req.params)?;
            let account = accounts::rename_account(mgr.store(), &params.id, &params.name)
                .map_err(store_error_to_rpc)?;
            ok_json(account)
        }
        METHOD_ACCOUNT_REMOVE => {
            let params: AccountRemoveParams = parse_params(req.params)?;
            accounts::remove_account(mgr.store(), &params.id, params.delete_dir)
                .map_err(store_error_to_rpc)?;
            mgr.remove_account_quota(&params.id);
            Ok(json!({}))
        }
        METHOD_ACCOUNT_INSTALL_HOOKS => {
            let params: IdParams = parse_params(req.params)?;
            let account = accounts::install_account_hooks(mgr.store(), &params.id)
                .map_err(account_hooks_error_to_rpc)?;
            ok_json(account)
        }
        METHOD_ACCOUNT_UNINSTALL_HOOKS => {
            let params: IdParams = parse_params(req.params)?;
            let account = accounts::uninstall_account_hooks(mgr.store(), &params.id)
                .map_err(account_hooks_error_to_rpc)?;
            ok_json(account)
        }
        METHOD_QUOTA_REPORT => {
            let params: QuotaReportParams = parse_params(req.params)?;
            mgr.report_quota(params);
            Ok(json!({}))
        }
        METHOD_QUOTA_LIST => {
            let list = mgr.list_quotas().map_err(store_error_to_rpc)?;
            ok_json(list)
        }
        other => Err(RpcError {
            code: ERR_METHOD_NOT_FOUND,
            message: format!("Método desconocido: {other}."),
        }),
    }
}

pub(crate) fn store_error_to_rpc(err: StoreError) -> RpcError {
    let code = match err {
        StoreError::NotAGitRepo { .. } => ERR_NOT_A_GIT_REPO,
        StoreError::ProjectAlreadyExists { .. } => ERR_PROJECT_ALREADY_EXISTS,
        StoreError::ProjectNotFound { .. } => ERR_PROJECT_NOT_FOUND,
        StoreError::GroupNotFound { .. } => ERR_GROUP_NOT_FOUND,
        StoreError::GroupAlreadyExists { .. } => ERR_GROUP_ALREADY_EXISTS,
        StoreError::AccountNotFound => ERR_ACCOUNT_NOT_FOUND,
        StoreError::AccountAlreadyExists { .. } => ERR_ACCOUNT_ALREADY_EXISTS,
        StoreError::AccountDirInvalid { .. } => ERR_ACCOUNT_DIR_INVALID,
        StoreError::NoAccountsDir | StoreError::EmptyName { .. } => ERR_INVALID_PARAMS,
        StoreError::Sqlite(_) | StoreError::Io(_) => -32603,
    };
    RpcError {
        code,
        message: err.to_string(),
    }
}

fn account_hooks_error_to_rpc(err: AccountHooksError) -> RpcError {
    match err {
        AccountHooksError::Store(e) => store_error_to_rpc(e),
        AccountHooksError::Hooks(e) => {
            let code = match e {
                HooksInstallError::InvalidJson { .. } => ERR_HOOKS_CONFIG_INVALID,
                HooksInstallError::PasBinNotFound | HooksInstallError::Io(_) => -32603,
            };
            RpcError {
                code,
                message: e.to_string(),
            }
        }
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

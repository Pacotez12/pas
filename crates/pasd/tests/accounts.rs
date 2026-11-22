use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

use pas_core::protocol::{
    AccountInfo, AgentKind, CreateResult, ERR_ACCOUNT_ALREADY_EXISTS, ERR_ACCOUNT_DIR_INVALID,
    ERR_ACCOUNT_NOT_FOUND, ERR_INVALID_PARAMS, METHOD_ACCOUNT_ADD, METHOD_ACCOUNT_LIST,
    METHOD_ACCOUNT_REMOVE, METHOD_ACCOUNT_RENAME, METHOD_SESSION_CREATE, METHOD_SESSION_KILL,
    METHOD_SESSION_LIST, METHOD_SESSION_READ, METHOD_SESSION_SEND, METHOD_SESSION_WAIT, ReadResult,
    Request, SessionInfo,
};
use pasd::manager::Manager;
use pasd::rpc::dispatch;
use pasd::store::Store;
use serde_json::{Value, json};
use tempfile::tempdir;

fn fake_agent_bin() -> String {
    static BIN: OnceLock<String> = OnceLock::new();
    BIN.get_or_init(|| {
        let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();

        let status = Command::new("cargo")
            .args(["build", "-p", "fake-agent", "--quiet"])
            .current_dir(&workspace_root)
            .status()
            .expect("no se pudo compilar fake-agent");
        assert!(status.success(), "falló cargo build -p fake-agent");

        let target_dir = std::env::var_os("CARGO_TARGET_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| workspace_root.join("target"));
        target_dir
            .join("debug")
            .join("fake-agent")
            .to_string_lossy()
            .into_owned()
    })
    .clone()
}

fn req(method: &str, params: Value) -> Request {
    Request {
        jsonrpc: "2.0".into(),
        id: 1,
        method: method.into(),
        params,
    }
}

#[tokio::test]
async fn add_owned_creates_dir() {
    let tmp = tempdir().unwrap();
    let db = tmp.path().join("data").join("pas.db");
    let mgr = Manager::with_store(200, Store::open(&db).unwrap());

    let r = dispatch(
        &mgr,
        req(
            METHOD_ACCOUNT_ADD,
            json!({ "agent": "claude", "name": "Personal" }),
        ),
    )
    .await;
    assert!(r.error.is_none(), "error inesperado: {:?}", r.error);
    let acc: AccountInfo = serde_json::from_value(r.result.unwrap()).unwrap();

    assert!(!acc.id.is_empty());
    assert_eq!(acc.agent, AgentKind::Claude);
    assert_eq!(acc.name, "Personal");
    assert!(acc.owned);
    assert!(
        acc.config_dir.is_dir(),
        "debería haber creado {}",
        acc.config_dir.display()
    );
    assert!(
        acc.config_dir
            .starts_with(tmp.path().join("data").join("accounts")),
        "el directorio propio debe vivir bajo data/accounts: {}",
        acc.config_dir.display()
    );

    let r = dispatch(&mgr, req(METHOD_ACCOUNT_LIST, json!({}))).await;
    assert!(r.error.is_none());
    let list: Vec<AccountInfo> = serde_json::from_value(r.result.unwrap()).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0], acc);

    // Persiste al reabrir SQLite.
    let mgr2 = Manager::with_store(200, Store::open(&db).unwrap());
    let r = dispatch(&mgr2, req(METHOD_ACCOUNT_LIST, json!({}))).await;
    let list2: Vec<AccountInfo> = serde_json::from_value(r.result.unwrap()).unwrap();
    assert_eq!(list2, vec![acc]);
}

#[tokio::test]
async fn add_existing_dir_not_owned() {
    let tmp = tempdir().unwrap();
    let db = tmp.path().join("data").join("pas.db");
    let existing = tmp.path().join("fake-codex-home");
    std::fs::create_dir_all(&existing).unwrap();
    let mgr = Manager::with_store(200, Store::open(&db).unwrap());

    let r = dispatch(
        &mgr,
        req(
            METHOD_ACCOUNT_ADD,
            json!({
                "agent": "codex",
                "name": "Trabajo",
                "config_dir": existing,
            }),
        ),
    )
    .await;
    assert!(r.error.is_none(), "error inesperado: {:?}", r.error);
    let acc: AccountInfo = serde_json::from_value(r.result.unwrap()).unwrap();
    assert_eq!(acc.agent, AgentKind::Codex);
    assert_eq!(acc.name, "Trabajo");
    assert!(!acc.owned);
    assert_eq!(acc.config_dir, std::fs::canonicalize(&existing).unwrap());

    // Directorio inexistente se rechaza con mensaje claro.
    let missing = tmp.path().join("no-existe");
    let r_err = dispatch(
        &mgr,
        req(
            METHOD_ACCOUNT_ADD,
            json!({
                "agent": "claude",
                "name": "Rota",
                "config_dir": missing,
            }),
        ),
    )
    .await;
    let err = r_err
        .error
        .expect("debería rechazar directorio inexistente");
    assert_eq!(err.code, ERR_ACCOUNT_DIR_INVALID);
}

#[tokio::test]
async fn same_dir_twice_is_error() {
    let tmp = tempdir().unwrap();
    let db = tmp.path().join("data").join("pas.db");
    let dir = tmp.path().join("shared-dir");
    std::fs::create_dir_all(&dir).unwrap();
    let mgr = Manager::with_store(200, Store::open(&db).unwrap());

    let r1 = dispatch(
        &mgr,
        req(
            METHOD_ACCOUNT_ADD,
            json!({ "agent": "claude", "name": "Uno", "config_dir": dir }),
        ),
    )
    .await;
    assert!(r1.error.is_none(), "primer add falló: {:?}", r1.error);

    let r2 = dispatch(
        &mgr,
        req(
            METHOD_ACCOUNT_ADD,
            json!({ "agent": "claude", "name": "Dos", "config_dir": dir }),
        ),
    )
    .await;
    let err = r2
        .error
        .expect("dos cuentas no pueden compartir directorio");
    assert_eq!(err.code, ERR_ACCOUNT_ALREADY_EXISTS);
    assert!(
        err.message.contains("no pueden compartir directorio"),
        "mensaje poco claro: {}",
        err.message
    );
}

#[tokio::test]
async fn rename_account_works_and_validates() {
    let tmp = tempdir().unwrap();
    let db = tmp.path().join("data").join("pas.db");
    let mgr = Manager::with_store(200, Store::open(&db).unwrap());

    let acc: AccountInfo = serde_json::from_value(
        dispatch(
            &mgr,
            req(
                METHOD_ACCOUNT_ADD,
                json!({ "agent": "claude", "name": "Viejo" }),
            ),
        )
        .await
        .result
        .unwrap(),
    )
    .unwrap();

    let renamed: AccountInfo = serde_json::from_value(
        dispatch(
            &mgr,
            req(
                METHOD_ACCOUNT_RENAME,
                json!({ "id": acc.id, "name": "Nuevo" }),
            ),
        )
        .await
        .result
        .unwrap(),
    )
    .unwrap();
    assert_eq!(renamed.name, "Nuevo");

    let empty = dispatch(
        &mgr,
        req(
            METHOD_ACCOUNT_RENAME,
            json!({ "id": acc.id, "name": "   " }),
        ),
    )
    .await;
    assert_eq!(empty.error.unwrap().code, ERR_INVALID_PARAMS);

    let missing = dispatch(
        &mgr,
        req(
            METHOD_ACCOUNT_RENAME,
            json!({ "id": "inexistente", "name": "Otro" }),
        ),
    )
    .await;
    assert_eq!(missing.error.unwrap().code, ERR_ACCOUNT_NOT_FOUND);
}

#[tokio::test]
async fn remove_owned_deletes_only_when_asked() {
    let tmp = tempdir().unwrap();
    let db = tmp.path().join("data").join("pas.db");
    let mgr = Manager::with_store(200, Store::open(&db).unwrap());

    // 1. Owned con delete_dir = false: conserva el directorio.
    let a: AccountInfo = serde_json::from_value(
        dispatch(
            &mgr,
            req(
                METHOD_ACCOUNT_ADD,
                json!({ "agent": "claude", "name": "KeepDir" }),
            ),
        )
        .await
        .result
        .unwrap(),
    )
    .unwrap();
    assert!(a.config_dir.is_dir());

    let r = dispatch(
        &mgr,
        req(
            METHOD_ACCOUNT_REMOVE,
            json!({ "id": a.id, "delete_dir": false }),
        ),
    )
    .await;
    assert!(r.error.is_none(), "error al quitar: {:?}", r.error);
    assert!(
        a.config_dir.is_dir(),
        "delete_dir=false no debe borrar el directorio"
    );

    // 2. Owned con delete_dir = true: borra el directorio.
    let b: AccountInfo = serde_json::from_value(
        dispatch(
            &mgr,
            req(
                METHOD_ACCOUNT_ADD,
                json!({ "agent": "codex", "name": "DropDir" }),
            ),
        )
        .await
        .result
        .unwrap(),
    )
    .unwrap();
    assert!(b.config_dir.is_dir());

    let r = dispatch(
        &mgr,
        req(
            METHOD_ACCOUNT_REMOVE,
            json!({ "id": b.id, "delete_dir": true }),
        ),
    )
    .await;
    assert!(r.error.is_none(), "error al quitar: {:?}", r.error);
    assert!(
        !b.config_dir.exists(),
        "delete_dir=true en cuenta owned debe borrar el directorio"
    );

    // 3. No owned con delete_dir = true: NUNCA borra el directorio externo.
    let external = tmp.path().join("external-profile");
    std::fs::create_dir_all(&external).unwrap();
    let c: AccountInfo = serde_json::from_value(
        dispatch(
            &mgr,
            req(
                METHOD_ACCOUNT_ADD,
                json!({ "agent": "claude", "name": "Externa", "config_dir": external }),
            ),
        )
        .await
        .result
        .unwrap(),
    )
    .unwrap();

    let r = dispatch(
        &mgr,
        req(
            METHOD_ACCOUNT_REMOVE,
            json!({ "id": c.id, "delete_dir": true }),
        ),
    )
    .await;
    assert!(r.error.is_none(), "error al quitar: {:?}", r.error);
    assert!(
        external.is_dir(),
        "una cuenta no owned nunca debe borrar su directorio aunque delete_dir=true"
    );

    let list: Vec<AccountInfo> = serde_json::from_value(
        dispatch(&mgr, req(METHOD_ACCOUNT_LIST, json!({})))
            .await
            .result
            .unwrap(),
    )
    .unwrap();
    assert!(list.is_empty());
}

async fn read_env_var(mgr: &Manager, session_id: &str, var: &str) -> String {
    let r = dispatch(
        mgr,
        req(
            METHOD_SESSION_WAIT,
            json!({ "id": session_id, "until": "idle", "timeout_ms": 5000 }),
        ),
    )
    .await;
    assert!(r.error.is_none(), "wait idle falló: {:?}", r.error);

    let r = dispatch(
        mgr,
        req(
            METHOD_SESSION_SEND,
            json!({ "id": session_id, "text": format!("env {var}") }),
        ),
    )
    .await;
    assert!(r.error.is_none(), "send env falló: {:?}", r.error);

    let r = dispatch(
        mgr,
        req(
            METHOD_SESSION_WAIT,
            json!({ "id": session_id, "until": "idle", "timeout_ms": 5000 }),
        ),
    )
    .await;
    assert!(r.error.is_none(), "wait idle 2 falló: {:?}", r.error);

    let r = dispatch(
        mgr,
        req(METHOD_SESSION_READ, json!({ "id": session_id, "lines": 5 })),
    )
    .await;
    let read: ReadResult = serde_json::from_value(r.result.unwrap()).unwrap();
    read.text
}

#[tokio::test]
async fn session_env_has_config_dir_and_session_id() {
    let tmp = tempdir().unwrap();
    let db = tmp.path().join("data").join("pas.db");
    let sock = tmp.path().join("pasd.sock");
    let mgr = Manager::with_paths(
        200,
        Store::open(&db).unwrap(),
        Some(tmp.path().join("data").join("accounts")),
        Some(sock.clone()),
    );

    let claude_acc: AccountInfo = serde_json::from_value(
        dispatch(
            &mgr,
            req(
                METHOD_ACCOUNT_ADD,
                json!({ "agent": "claude", "name": "ClaudeTrabajo" }),
            ),
        )
        .await
        .result
        .unwrap(),
    )
    .unwrap();

    let codex_acc: AccountInfo = serde_json::from_value(
        dispatch(
            &mgr,
            req(
                METHOD_ACCOUNT_ADD,
                json!({ "agent": "codex", "name": "CodexTrabajo" }),
            ),
        )
        .await
        .result
        .unwrap(),
    )
    .unwrap();

    let agy_acc: AccountInfo = serde_json::from_value(
        dispatch(
            &mgr,
            req(
                METHOD_ACCOUNT_ADD,
                json!({ "agent": "agy", "name": "AgyDefault" }),
            ),
        )
        .await
        .result
        .unwrap(),
    )
    .unwrap();

    let sock_str = sock.to_string_lossy().into_owned();
    let claude_dir = claude_acc.config_dir.to_string_lossy().into_owned();
    let codex_dir = codex_acc.config_dir.to_string_lossy().into_owned();
    let agy_dir = agy_acc.config_dir.to_string_lossy().into_owned();

    // Sesión Claude
    let r = dispatch(
        &mgr,
        req(
            METHOD_SESSION_CREATE,
            json!({
                "command": [fake_agent_bin()],
                "cwd": tmp.path(),
                "title": "s-claude",
                "account_id": claude_acc.id,
            }),
        ),
    )
    .await;
    assert!(r.error.is_none(), "create claude falló: {:?}", r.error);
    let s_claude: CreateResult = serde_json::from_value(r.result.unwrap()).unwrap();

    let out_dir = read_env_var(&mgr, &s_claude.id, "CLAUDE_CONFIG_DIR").await;
    assert!(
        out_dir.contains(&claude_dir),
        "esperaba CLAUDE_CONFIG_DIR={claude_dir}, salida: {out_dir:?}"
    );
    let out_sid = read_env_var(&mgr, &s_claude.id, "PAS_SESSION_ID").await;
    assert!(
        out_sid.contains(&s_claude.id),
        "esperaba PAS_SESSION_ID={}, salida: {out_sid:?}",
        s_claude.id
    );
    let out_sock = read_env_var(&mgr, &s_claude.id, "PAS_SOCKET").await;
    assert!(
        out_sock.contains(&sock_str),
        "esperaba PAS_SOCKET={sock_str}, salida: {out_sock:?}"
    );

    // Sesión Codex
    let r = dispatch(
        &mgr,
        req(
            METHOD_SESSION_CREATE,
            json!({
                "command": [fake_agent_bin()],
                "cwd": tmp.path(),
                "title": "s-codex",
                "account_id": codex_acc.id,
            }),
        ),
    )
    .await;
    assert!(r.error.is_none(), "create codex falló: {:?}", r.error);
    let s_codex: CreateResult = serde_json::from_value(r.result.unwrap()).unwrap();

    let out_codex = read_env_var(&mgr, &s_codex.id, "CODEX_HOME").await;
    assert!(
        out_codex.contains(&codex_dir),
        "esperaba CODEX_HOME={codex_dir}, salida: {out_codex:?}"
    );
    let out_sid2 = read_env_var(&mgr, &s_codex.id, "PAS_SESSION_ID").await;
    assert!(
        out_sid2.contains(&s_codex.id),
        "esperaba PAS_SESSION_ID={}, salida: {out_sid2:?}",
        s_codex.id
    );
    let out_sock2 = read_env_var(&mgr, &s_codex.id, "PAS_SOCKET").await;
    assert!(
        out_sock2.contains(&sock_str),
        "esperaba PAS_SOCKET={sock_str}, salida: {out_sock2:?}"
    );

    // Sesión agy: no inyecta CLAUDE_CONFIG_DIR ni CODEX_HOME, sí PAS_SESSION_ID y PAS_SOCKET.
    let r = dispatch(
        &mgr,
        req(
            METHOD_SESSION_CREATE,
            json!({
                "command": [fake_agent_bin()],
                "cwd": tmp.path(),
                "title": "s-agy",
                "account_id": agy_acc.id,
            }),
        ),
    )
    .await;
    assert!(r.error.is_none(), "create agy falló: {:?}", r.error);
    let s_agy: CreateResult = serde_json::from_value(r.result.unwrap()).unwrap();
    let out_sid3 = read_env_var(&mgr, &s_agy.id, "PAS_SESSION_ID").await;
    assert!(
        out_sid3.contains(&s_agy.id),
        "esperaba PAS_SESSION_ID={}, salida: {out_sid3:?}",
        s_agy.id
    );
    let out_agy_claude = read_env_var(&mgr, &s_agy.id, "CLAUDE_CONFIG_DIR").await;
    assert!(
        !out_agy_claude.contains(&agy_dir),
        "agy no debe setear CLAUDE_CONFIG_DIR: {out_agy_claude:?}"
    );
    let out_agy_codex = read_env_var(&mgr, &s_agy.id, "CODEX_HOME").await;
    assert!(
        !out_agy_codex.contains(&agy_dir),
        "agy no debe setear CODEX_HOME: {out_agy_codex:?}"
    );

    // Cuenta inexistente en session.create devuelve ERR_ACCOUNT_NOT_FOUND.
    let bad = dispatch(
        &mgr,
        req(
            METHOD_SESSION_CREATE,
            json!({
                "command": [fake_agent_bin()],
                "cwd": tmp.path(),
                "title": "s-bad",
                "account_id": "no-existe",
            }),
        ),
    )
    .await;
    assert_eq!(bad.error.unwrap().code, ERR_ACCOUNT_NOT_FOUND);

    // SessionInfo incluye account_id.
    let sessions: Vec<SessionInfo> = serde_json::from_value(
        dispatch(&mgr, req(METHOD_SESSION_LIST, json!({})))
            .await
            .result
            .unwrap(),
    )
    .unwrap();
    let find_acc = |sid: &str| {
        sessions
            .iter()
            .find(|s| s.id == sid)
            .unwrap()
            .account_id
            .clone()
    };
    assert_eq!(
        find_acc(&s_claude.id).as_deref(),
        Some(claude_acc.id.as_str())
    );
    assert_eq!(
        find_acc(&s_codex.id).as_deref(),
        Some(codex_acc.id.as_str())
    );
    assert_eq!(find_acc(&s_agy.id).as_deref(), Some(agy_acc.id.as_str()));

    dispatch(&mgr, req(METHOD_SESSION_KILL, json!({ "id": s_claude.id }))).await;
    dispatch(&mgr, req(METHOD_SESSION_KILL, json!({ "id": s_codex.id }))).await;
    dispatch(&mgr, req(METHOD_SESSION_KILL, json!({ "id": s_agy.id }))).await;
}

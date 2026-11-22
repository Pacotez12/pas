use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use pas_core::protocol::{
    CreateResult, ERR_INVALID_PARAMS, ERR_METHOD_NOT_FOUND, ERR_SESSION_EXITED,
    ERR_SESSION_RUNNING, EVENT_SESSION_REMOVED, METHOD_PROJECT_ADD, METHOD_SESSION_CREATE,
    METHOD_SESSION_KILL, METHOD_SESSION_LIST, METHOD_SESSION_READ, METHOD_SESSION_REMOVE,
    METHOD_SESSION_SEND, METHOD_SESSION_WAIT, ProjectInfo, ReadResult, Request, SessionInfo,
    SessionRemovedEvent, SessionState,
};
use pasd::manager::Manager;
use pasd::rpc::dispatch;
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

fn create_params(title: &str) -> Value {
    json!({
        "command": [fake_agent_bin()],
        "cwd": std::env::temp_dir(),
        "title": title,
    })
}

#[tokio::test]
async fn create_list_kill() {
    let mgr = Manager::new(200);

    let r = dispatch(&mgr, req(METHOD_SESSION_CREATE, create_params("a"))).await;
    let created: CreateResult = serde_json::from_value(r.result.unwrap()).unwrap();
    assert_eq!(created.id.len(), 8);

    let r = dispatch(&mgr, req(METHOD_SESSION_LIST, json!({}))).await;
    let list: Vec<SessionInfo> = serde_json::from_value(r.result.unwrap()).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].title, "a");

    let r = dispatch(&mgr, req(METHOD_SESSION_KILL, json!({ "id": created.id }))).await;
    assert!(r.error.is_none());

    let r = dispatch(
        &mgr,
        req(
            METHOD_SESSION_WAIT,
            json!({ "id": created.id, "until": "exited", "timeout_ms": 5000 }),
        ),
    )
    .await;
    let st: SessionState = serde_json::from_value(r.result.unwrap()).unwrap();
    assert!(matches!(st, SessionState::Exited { .. }));
}

#[tokio::test]
async fn duplicate_title_suffix() {
    let mgr = Manager::new(200);

    let r1 = dispatch(&mgr, req(METHOD_SESSION_CREATE, create_params("a"))).await;
    let c1: CreateResult = serde_json::from_value(r1.result.unwrap()).unwrap();
    let r2 = dispatch(&mgr, req(METHOD_SESSION_CREATE, create_params("a"))).await;
    let c2: CreateResult = serde_json::from_value(r2.result.unwrap()).unwrap();

    let r = dispatch(&mgr, req(METHOD_SESSION_LIST, json!({}))).await;
    let mut titles: Vec<String> = serde_json::from_value::<Vec<SessionInfo>>(r.result.unwrap())
        .unwrap()
        .into_iter()
        .map(|s| s.title)
        .collect();
    titles.sort();
    assert_eq!(titles, vec!["a", "a-2"]);

    dispatch(&mgr, req(METHOD_SESSION_KILL, json!({ "id": c1.id }))).await;
    dispatch(&mgr, req(METHOD_SESSION_KILL, json!({ "id": c2.id }))).await;
}

#[tokio::test]
async fn unknown_method() {
    let mgr = Manager::new(200);
    let r = dispatch(&mgr, req("session.foo", json!({}))).await;
    assert_eq!(r.error.unwrap().code, ERR_METHOD_NOT_FOUND);
}

#[tokio::test]
async fn bad_params() {
    let mgr = Manager::new(200);
    let r = dispatch(&mgr, req(METHOD_SESSION_SEND, json!({ "text": "hola" }))).await;
    assert_eq!(r.error.unwrap().code, ERR_INVALID_PARAMS);
}

#[tokio::test]
async fn get_by_title() {
    let mgr = Manager::new(200);
    dispatch(&mgr, req(METHOD_SESSION_CREATE, create_params("a"))).await;
    dispatch(
        &mgr,
        req(
            METHOD_SESSION_WAIT,
            json!({ "id": "a", "until": "idle", "timeout_ms": 5000 }),
        ),
    )
    .await;

    let r = dispatch(
        &mgr,
        req(
            METHOD_SESSION_SEND,
            json!({ "id": "a", "text": "echo ping" }),
        ),
    )
    .await;
    assert!(r.error.is_none());

    dispatch(
        &mgr,
        req(
            METHOD_SESSION_WAIT,
            json!({ "id": "a", "until": "idle", "timeout_ms": 5000 }),
        ),
    )
    .await;

    let r = dispatch(
        &mgr,
        req(METHOD_SESSION_READ, json!({ "id": "a", "lines": 3 })),
    )
    .await;
    let read: ReadResult = serde_json::from_value(r.result.unwrap()).unwrap();
    assert!(read.text.contains("ping"), "salida: {:?}", read.text);

    dispatch(&mgr, req(METHOD_SESSION_KILL, json!({ "id": "a" }))).await;
}

#[tokio::test]
async fn send_to_exited() {
    let mgr = Manager::new(200);
    dispatch(&mgr, req(METHOD_SESSION_CREATE, create_params("bye"))).await;
    dispatch(
        &mgr,
        req(
            METHOD_SESSION_WAIT,
            json!({ "id": "bye", "until": "idle", "timeout_ms": 5000 }),
        ),
    )
    .await;

    dispatch(
        &mgr,
        req(
            METHOD_SESSION_SEND,
            json!({ "id": "bye", "text": "exit 0" }),
        ),
    )
    .await;
    dispatch(
        &mgr,
        req(
            METHOD_SESSION_WAIT,
            json!({ "id": "bye", "until": "exited", "timeout_ms": 5000 }),
        ),
    )
    .await;

    let r = dispatch(
        &mgr,
        req(METHOD_SESSION_SEND, json!({ "id": "bye", "text": "hola" })),
    )
    .await;
    assert_eq!(r.error.unwrap().code, ERR_SESSION_EXITED);
}

#[tokio::test]
async fn session_remove_exited_ok() {
    let mgr = Manager::new(200);
    let mut events = mgr.subscribe_events();

    let created: CreateResult = serde_json::from_value(
        dispatch(&mgr, req(METHOD_SESSION_CREATE, create_params("done")))
            .await
            .result
            .unwrap(),
    )
    .unwrap();

    dispatch(
        &mgr,
        req(
            METHOD_SESSION_WAIT,
            json!({ "id": created.id, "until": "idle", "timeout_ms": 5000 }),
        ),
    )
    .await;
    dispatch(
        &mgr,
        req(
            METHOD_SESSION_SEND,
            json!({ "id": created.id, "text": "exit 0" }),
        ),
    )
    .await;
    dispatch(
        &mgr,
        req(
            METHOD_SESSION_WAIT,
            json!({ "id": created.id, "until": "exited", "timeout_ms": 5000 }),
        ),
    )
    .await;

    let r = dispatch(
        &mgr,
        req(METHOD_SESSION_REMOVE, json!({ "id": created.id })),
    )
    .await;
    assert!(r.error.is_none(), "error inesperado: {:?}", r.error);

    let list: Vec<SessionInfo> = serde_json::from_value(
        dispatch(&mgr, req(METHOD_SESSION_LIST, json!({})))
            .await
            .result
            .unwrap(),
    )
    .unwrap();
    assert!(list.is_empty(), "la sesión debería haber salido de list");

    let mut saw_removed = false;
    while let Ok(notif) = events.try_recv() {
        if notif.method == EVENT_SESSION_REMOVED {
            let ev: SessionRemovedEvent = serde_json::from_value(notif.params).unwrap();
            if ev.id == created.id {
                saw_removed = true;
            }
        }
    }
    assert!(saw_removed, "debería emitir session.removed");
}

#[tokio::test]
async fn session_remove_running_is_error() {
    let mgr = Manager::new(200);
    let created: CreateResult = serde_json::from_value(
        dispatch(&mgr, req(METHOD_SESSION_CREATE, create_params("alive")))
            .await
            .result
            .unwrap(),
    )
    .unwrap();

    let r = dispatch(
        &mgr,
        req(METHOD_SESSION_REMOVE, json!({ "id": created.id })),
    )
    .await;
    let err = r
        .error
        .expect("debería fallar si la sesión sigue corriendo");
    assert_eq!(err.code, ERR_SESSION_RUNNING);
    assert_eq!(
        err.message,
        "La sesión sigue corriendo. Cerrala primero con Ctrl+C o matala."
    );

    let list: Vec<SessionInfo> = serde_json::from_value(
        dispatch(&mgr, req(METHOD_SESSION_LIST, json!({})))
            .await
            .result
            .unwrap(),
    )
    .unwrap();
    assert_eq!(list.len(), 1);

    dispatch(&mgr, req(METHOD_SESSION_KILL, json!({ "id": created.id }))).await;
}

fn make_git_dir(dir: &Path) -> PathBuf {
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    dir.to_path_buf()
}

async fn add_project(mgr: &Manager, path: &Path) -> ProjectInfo {
    let r = dispatch(mgr, req(METHOD_PROJECT_ADD, json!({ "path": path }))).await;
    assert!(r.error.is_none(), "error agregando proyecto: {:?}", r.error);
    serde_json::from_value(r.result.unwrap()).unwrap()
}

async fn create_session_in(
    mgr: &Manager,
    cwd: &Path,
    title: &str,
    project_id: Option<&str>,
) -> SessionInfo {
    let mut params = json!({
        "command": [fake_agent_bin()],
        "cwd": cwd,
        "title": title,
    });
    if let Some(pid) = project_id {
        params["project_id"] = json!(pid);
    }
    let r = dispatch(mgr, req(METHOD_SESSION_CREATE, params)).await;
    assert!(r.error.is_none(), "error creando sesión: {:?}", r.error);
    let created: CreateResult = serde_json::from_value(r.result.unwrap()).unwrap();

    let list: Vec<SessionInfo> = serde_json::from_value(
        dispatch(mgr, req(METHOD_SESSION_LIST, json!({})))
            .await
            .result
            .unwrap(),
    )
    .unwrap();
    list.into_iter().find(|s| s.id == created.id).unwrap()
}

#[tokio::test]
async fn create_infers_project_from_cwd() {
    let tmp = tempdir().unwrap();
    let repo = make_git_dir(&tmp.path().join("pockt"));
    let subdir = repo.join("src").join("nested");
    std::fs::create_dir_all(&subdir).unwrap();

    let mgr = Manager::new(200);
    let project = add_project(&mgr, &repo).await;

    let s_root = create_session_in(&mgr, &repo, "root", None).await;
    assert_eq!(s_root.project_id.as_deref(), Some(project.id.as_str()));

    let s_sub = create_session_in(&mgr, &subdir, "sub", None).await;
    assert_eq!(s_sub.project_id.as_deref(), Some(project.id.as_str()));

    dispatch(&mgr, req(METHOD_SESSION_KILL, json!({ "id": s_root.id }))).await;
    dispatch(&mgr, req(METHOD_SESSION_KILL, json!({ "id": s_sub.id }))).await;
}

#[tokio::test]
async fn create_infers_deepest_nested_project() {
    let tmp = tempdir().unwrap();
    let outer_dir = make_git_dir(&tmp.path().join("outer"));
    let inner_dir = make_git_dir(&outer_dir.join("inner"));
    let deep_dir = inner_dir.join("pkg");
    let outer_other = outer_dir.join("other");
    std::fs::create_dir_all(&deep_dir).unwrap();
    std::fs::create_dir_all(&outer_other).unwrap();

    let mgr = Manager::new(200);
    let outer = add_project(&mgr, &outer_dir).await;
    let inner = add_project(&mgr, &inner_dir).await;

    let s_inner = create_session_in(&mgr, &deep_dir, "deep", None).await;
    assert_eq!(s_inner.project_id.as_deref(), Some(inner.id.as_str()));

    let s_outer = create_session_in(&mgr, &outer_other, "outer", None).await;
    assert_eq!(s_outer.project_id.as_deref(), Some(outer.id.as_str()));

    dispatch(&mgr, req(METHOD_SESSION_KILL, json!({ "id": s_inner.id }))).await;
    dispatch(&mgr, req(METHOD_SESSION_KILL, json!({ "id": s_outer.id }))).await;
}

#[tokio::test]
async fn create_sibling_prefix_is_not_inside() {
    let tmp = tempdir().unwrap();
    let repo = make_git_dir(&tmp.path().join("pockt"));
    let sibling = tmp.path().join("pockt2");
    std::fs::create_dir_all(&sibling).unwrap();

    let mgr = Manager::new(200);
    add_project(&mgr, &repo).await;

    let s = create_session_in(&mgr, &sibling, "sibling", None).await;
    assert_eq!(s.project_id, None);

    dispatch(&mgr, req(METHOD_SESSION_KILL, json!({ "id": s.id }))).await;
}

#[tokio::test]
async fn create_explicit_project_id_wins() {
    let tmp = tempdir().unwrap();
    let pockt_dir = make_git_dir(&tmp.path().join("pockt"));
    let other_dir = make_git_dir(&tmp.path().join("other"));

    let mgr = Manager::new(200);
    add_project(&mgr, &pockt_dir).await;
    let other = add_project(&mgr, &other_dir).await;

    let s = create_session_in(&mgr, &pockt_dir, "explicit", Some(&other.id)).await;
    assert_eq!(s.project_id.as_deref(), Some(other.id.as_str()));

    dispatch(&mgr, req(METHOD_SESSION_KILL, json!({ "id": s.id }))).await;
}

#[tokio::test]
async fn send_retries_when_first_input_is_discarded() {
    let mgr = Manager::new(200);
    let r = dispatch(
        &mgr,
        req(
            METHOD_SESSION_CREATE,
            json!({
                "command": [fake_agent_bin()],
                "cwd": std::env::temp_dir(),
                "title": "agy-drop-one",
                "env": { "FAKE_AGENT_DROP_INPUTS": "1" },
            }),
        ),
    )
    .await;
    let created: CreateResult = serde_json::from_value(r.result.unwrap()).unwrap();

    dispatch(
        &mgr,
        req(
            METHOD_SESSION_WAIT,
            json!({ "id": created.id, "until": "idle", "timeout_ms": 5000 }),
        ),
    )
    .await;

    let send_res = dispatch(
        &mgr,
        req(
            METHOD_SESSION_SEND,
            json!({ "id": created.id, "text": "echo primer mensaje recuperado" }),
        ),
    )
    .await;
    assert!(send_res.error.is_none(), "error: {:?}", send_res.error);
    assert!(
        send_res
            .result
            .as_ref()
            .and_then(|v| v.get("warning"))
            .is_none(),
        "no debería advertir si el reintento mostró el texto: {:?}",
        send_res.result
    );

    dispatch(
        &mgr,
        req(
            METHOD_SESSION_WAIT,
            json!({ "id": created.id, "until": "idle", "timeout_ms": 5000 }),
        ),
    )
    .await;

    let read_res = dispatch(
        &mgr,
        req(METHOD_SESSION_READ, json!({ "id": created.id, "lines": 5 })),
    )
    .await;
    let read: ReadResult = serde_json::from_value(read_res.result.unwrap()).unwrap();
    assert!(
        read.text.contains("primer mensaje recuperado"),
        "el reintento debería haber entregado el texto; salida: {:?}",
        read.text
    );

    dispatch(&mgr, req(METHOD_SESSION_KILL, json!({ "id": created.id }))).await;
}

#[tokio::test]
async fn send_warns_when_text_never_appears() {
    let mgr = Manager::new(200);
    let r = dispatch(
        &mgr,
        req(
            METHOD_SESSION_CREATE,
            json!({
                "command": [fake_agent_bin()],
                "cwd": std::env::temp_dir(),
                "title": "agy",
                "env": { "FAKE_AGENT_DROP_INPUTS": "5" },
            }),
        ),
    )
    .await;
    let created: CreateResult = serde_json::from_value(r.result.unwrap()).unwrap();

    dispatch(
        &mgr,
        req(
            METHOD_SESSION_WAIT,
            json!({ "id": created.id, "until": "idle", "timeout_ms": 5000 }),
        ),
    )
    .await;

    let send_res = dispatch(
        &mgr,
        req(
            METHOD_SESSION_SEND,
            json!({ "id": created.id, "text": "echo nunca aparece" }),
        ),
    )
    .await;
    assert!(send_res.error.is_none(), "error: {:?}", send_res.error);
    let warning = send_res
        .result
        .as_ref()
        .and_then(|v| v.get("warning"))
        .and_then(Value::as_str);
    assert_eq!(
        warning,
        Some("agy no mostró el texto en su prompt y no mandé Enter; revisá la sesión con pas read")
    );

    dispatch(&mgr, req(METHOD_SESSION_KILL, json!({ "id": created.id }))).await;
}

#[tokio::test]
async fn send_multiline_delivers_all_lines() {
    let mgr = Manager::new(200);
    let r = dispatch(&mgr, req(METHOD_SESSION_CREATE, create_params("multi"))).await;
    let created: CreateResult = serde_json::from_value(r.result.unwrap()).unwrap();

    dispatch(
        &mgr,
        req(
            METHOD_SESSION_WAIT,
            json!({ "id": created.id, "until": "idle", "timeout_ms": 5000 }),
        ),
    )
    .await;

    let send_res = dispatch(
        &mgr,
        req(
            METHOD_SESSION_SEND,
            json!({ "id": created.id, "text": "echo primera linea\necho segunda linea" }),
        ),
    )
    .await;
    assert!(send_res.error.is_none(), "error: {:?}", send_res.error);
    assert!(
        send_res
            .result
            .as_ref()
            .and_then(|v| v.get("warning"))
            .is_none(),
        "no debería advertir en envío multilínea: {:?}",
        send_res.result
    );

    dispatch(
        &mgr,
        req(
            METHOD_SESSION_WAIT,
            json!({ "id": created.id, "until": "idle", "timeout_ms": 5000 }),
        ),
    )
    .await;

    let read_res = dispatch(
        &mgr,
        req(
            METHOD_SESSION_READ,
            json!({ "id": created.id, "lines": 10 }),
        ),
    )
    .await;
    let read: ReadResult = serde_json::from_value(read_res.result.unwrap()).unwrap();
    assert!(
        read.text.contains("primera linea") && read.text.contains("segunda linea"),
        "salida multilínea incompleta: {:?}",
        read.text
    );

    dispatch(&mgr, req(METHOD_SESSION_KILL, json!({ "id": created.id }))).await;
}

#[tokio::test]
async fn send_without_echo_does_not_press_enter() {
    // Si el texto nunca aparece (p. ej. agy muestra un diálogo), un Enter a ciegas
    // podría confirmar ese diálogo: pas no debe mandarlo.
    let mgr = Manager::new(200);
    let r = dispatch(
        &mgr,
        req(
            METHOD_SESSION_CREATE,
            json!({
                "command": [fake_agent_bin()],
                "cwd": std::env::temp_dir(),
                "title": "sin-enter",
                "env": { "FAKE_AGENT_DROP_INPUTS": "2" },
            }),
        ),
    )
    .await;
    let created: CreateResult = serde_json::from_value(r.result.unwrap()).unwrap();
    let wait_idle = || {
        req(
            METHOD_SESSION_WAIT,
            json!({ "id": created.id, "until": "idle", "timeout_ms": 5000 }),
        )
    };
    dispatch(&mgr, wait_idle()).await;

    let first = dispatch(
        &mgr,
        req(
            METHOD_SESSION_SEND,
            json!({ "id": created.id, "text": "echo uno" }),
        ),
    )
    .await;
    assert!(
        first
            .result
            .as_ref()
            .and_then(|v| v.get("warning"))
            .is_some()
    );

    let second = dispatch(
        &mgr,
        req(
            METHOD_SESSION_SEND,
            json!({ "id": created.id, "text": "echo dos" }),
        ),
    )
    .await;
    assert!(second.error.is_none(), "error: {:?}", second.error);
    dispatch(&mgr, wait_idle()).await;

    let read_res = dispatch(
        &mgr,
        req(
            METHOD_SESSION_READ,
            json!({ "id": created.id, "lines": 10 }),
        ),
    )
    .await;
    let read: ReadResult = serde_json::from_value(read_res.result.unwrap()).unwrap();
    let lines: Vec<&str> = read.text.lines().map(str::trim_end).collect();
    let pos = lines
        .iter()
        .position(|l| l.contains("echo dos"))
        .expect("echo dos debería verse");
    assert_eq!(
        lines[pos - 1],
        "fake-agent listo",
        "no debería haber un Enter suelto antes; pantalla: {lines:?}"
    );

    dispatch(&mgr, req(METHOD_SESSION_KILL, json!({ "id": created.id }))).await;
}

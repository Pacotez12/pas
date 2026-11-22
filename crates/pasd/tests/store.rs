use std::path::{Path, PathBuf};
use std::process::Command;

use pas_core::protocol::{
    ERR_GROUP_ALREADY_EXISTS, ERR_NOT_A_GIT_REPO, ERR_PROJECT_ALREADY_EXISTS, GroupInfo,
    METHOD_GROUP_CREATE, METHOD_GROUP_LIST, METHOD_GROUP_RENAME, METHOD_PROJECT_ADD,
    METHOD_PROJECT_LIST, METHOD_PROJECT_MOVE, METHOD_PROJECT_REMOVE, METHOD_PROJECT_RENAME,
    METHOD_PROJECT_UNPIN, ProjectInfo, Request,
};
use pasd::manager::Manager;
use pasd::remote::remote_key;
use pasd::rpc::dispatch;
use pasd::store::Store;
use serde_json::{Value, json};
use tempfile::tempdir;

fn req(method: &str, params: Value) -> Request {
    Request {
        jsonrpc: "2.0".into(),
        id: 1,
        method: method.into(),
        params,
    }
}

fn init_git_repo(dir: &Path, origin: Option<&str>) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    let status = Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(dir)
        .status()
        .expect("no se pudo ejecutar git init");
    assert!(status.success(), "falló git init en {}", dir.display());

    if let Some(url) = origin {
        let status = Command::new("git")
            .args(["remote", "add", "origin", url])
            .current_dir(dir)
            .status()
            .expect("no se pudo ejecutar git remote add");
        assert!(status.success(), "falló git remote add origin");
    }

    dir.to_path_buf()
}

#[test]
fn remote_key_normalizes() {
    let expected = Some("github.com/a/b".to_string());
    assert_eq!(remote_key("git@github.com:A/b.git"), expected);
    assert_eq!(remote_key("https://github.com/a/b"), expected);
    assert_eq!(remote_key("https://GITHUB.COM/A/B.git"), expected);
    assert_eq!(remote_key("https://github.com/a/b/"), expected);
    assert_eq!(remote_key("ssh://git@github.com/A/b.git"), expected);
    assert_eq!(remote_key("ssh://git@github.com:22/A/b.git"), expected);
    assert_eq!(remote_key("git://github.com/A/b.git"), expected);
    assert_eq!(
        remote_key("git@gitlab.com:Org/Sub/Repo.git"),
        Some("gitlab.com/org/sub/repo".to_string())
    );
    assert_eq!(remote_key(""), None);
    assert_eq!(remote_key("   "), None);
}

#[tokio::test]
async fn add_list_remove() {
    let tmp = tempdir().unwrap();
    let repo = init_git_repo(
        &tmp.path().join("mi-app"),
        Some("git@github.com:org/mi-app.git"),
    );
    let db = tmp.path().join("data").join("pas.db");
    let mgr = Manager::with_store(200, Store::open(&db).unwrap());

    let r = dispatch(&mgr, req(METHOD_PROJECT_ADD, json!({ "path": repo }))).await;
    assert!(r.error.is_none(), "error inesperado: {:?}", r.error);
    let added: ProjectInfo = serde_json::from_value(r.result.unwrap()).unwrap();
    assert_eq!(added.name, "mi-app");
    assert!(!added.id.is_empty());
    assert!(!added.group_id.is_empty());
    assert!(!added.group_pinned);

    let r = dispatch(&mgr, req(METHOD_PROJECT_LIST, json!({}))).await;
    let list: Vec<ProjectInfo> = serde_json::from_value(r.result.unwrap()).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].id, added.id);

    let r = dispatch(&mgr, req(METHOD_PROJECT_REMOVE, json!({ "id": added.id }))).await;
    assert!(r.error.is_none(), "error al borrar: {:?}", r.error);

    let r = dispatch(&mgr, req(METHOD_PROJECT_LIST, json!({}))).await;
    let list: Vec<ProjectInfo> = serde_json::from_value(r.result.unwrap()).unwrap();
    assert!(list.is_empty());
}

#[tokio::test]
async fn add_twice_same_path_is_error() {
    let tmp = tempdir().unwrap();
    let repo = init_git_repo(&tmp.path().join("repo"), None);
    let mgr = Manager::new(200);

    let r1 = dispatch(&mgr, req(METHOD_PROJECT_ADD, json!({ "path": repo }))).await;
    assert!(r1.error.is_none(), "primer add falló: {:?}", r1.error);

    let r2 = dispatch(&mgr, req(METHOD_PROJECT_ADD, json!({ "path": repo }))).await;
    let err = r2
        .error
        .expect("debería fallar al agregar la misma ruta dos veces");
    assert_eq!(err.code, ERR_PROJECT_ALREADY_EXISTS);
}

#[tokio::test]
async fn add_non_git_dir_message() {
    let tmp = tempdir().unwrap();
    let plain_dir = tmp.path().join("no-es-repo");
    std::fs::create_dir_all(&plain_dir).unwrap();
    let mgr = Manager::new(200);

    let r = dispatch(&mgr, req(METHOD_PROJECT_ADD, json!({ "path": plain_dir }))).await;
    let err = r.error.expect("debería rechazar carpeta sin .git");
    assert_eq!(err.code, ERR_NOT_A_GIT_REPO);
    assert_eq!(
        err.message,
        format!(
            "{} no es un repositorio git. Elegí la carpeta raíz del repo.",
            plain_dir.display()
        )
    );
}

#[tokio::test]
async fn survives_restart() {
    let tmp = tempdir().unwrap();
    let repo = init_git_repo(
        &tmp.path().join("persistente"),
        Some("https://github.com/org/persistente.git"),
    );
    let db = tmp.path().join("data").join("pas.db");

    let added_id;
    let group_id;
    {
        let mgr = Manager::with_store(200, Store::open(&db).unwrap());
        let r = dispatch(
            &mgr,
            req(
                METHOD_PROJECT_ADD,
                json!({ "path": repo, "name": "mi-nombre" }),
            ),
        )
        .await;
        let p: ProjectInfo = serde_json::from_value(r.result.unwrap()).unwrap();
        added_id = p.id;
        group_id = p.group_id;
    }

    let mgr2 = Manager::with_store(200, Store::open(&db).unwrap());
    let r = dispatch(&mgr2, req(METHOD_PROJECT_LIST, json!({}))).await;
    let list: Vec<ProjectInfo> = serde_json::from_value(r.result.unwrap()).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].id, added_id);
    assert_eq!(list[0].name, "mi-nombre");
    assert_eq!(list[0].group_id, group_id);

    let r = dispatch(&mgr2, req(METHOD_GROUP_LIST, json!({}))).await;
    let groups: Vec<GroupInfo> = serde_json::from_value(r.result.unwrap()).unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].id, group_id);
    assert_eq!(
        groups[0].remote_key.as_deref(),
        Some("github.com/org/persistente")
    );
}

#[tokio::test]
async fn same_remote_same_group() {
    let tmp = tempdir().unwrap();
    let clone_ssh = init_git_repo(
        &tmp.path().join("clone-a"),
        Some("git@github.com:Org/App.git"),
    );
    let clone_https = init_git_repo(
        &tmp.path().join("clone-b"),
        Some("https://github.com/org/app"),
    );
    let mgr = Manager::new(200);

    let p1: ProjectInfo = serde_json::from_value(
        dispatch(&mgr, req(METHOD_PROJECT_ADD, json!({ "path": clone_ssh })))
            .await
            .result
            .unwrap(),
    )
    .unwrap();
    let p2: ProjectInfo = serde_json::from_value(
        dispatch(
            &mgr,
            req(METHOD_PROJECT_ADD, json!({ "path": clone_https })),
        )
        .await
        .result
        .unwrap(),
    )
    .unwrap();

    assert_eq!(p1.group_id, p2.group_id);

    let groups: Vec<GroupInfo> = serde_json::from_value(
        dispatch(&mgr, req(METHOD_GROUP_LIST, json!({})))
            .await
            .result
            .unwrap(),
    )
    .unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].id, p1.group_id);
    assert_eq!(groups[0].name, "app");
}

#[tokio::test]
async fn move_pins_project() {
    let tmp = tempdir().unwrap();
    let clone1 = init_git_repo(&tmp.path().join("c1"), Some("git@github.com:org/srv.git"));
    let clone2 = init_git_repo(
        &tmp.path().join("c2"),
        Some("https://github.com/org/srv.git"),
    );
    let mgr = Manager::new(200);

    let p1: ProjectInfo = serde_json::from_value(
        dispatch(&mgr, req(METHOD_PROJECT_ADD, json!({ "path": clone1 })))
            .await
            .result
            .unwrap(),
    )
    .unwrap();

    let manual: GroupInfo = serde_json::from_value(
        dispatch(&mgr, req(METHOD_GROUP_CREATE, json!({ "name": "Trabajo" })))
            .await
            .result
            .unwrap(),
    )
    .unwrap();
    assert_eq!(manual.remote_key, None);

    let moved: ProjectInfo = serde_json::from_value(
        dispatch(
            &mgr,
            req(
                METHOD_PROJECT_MOVE,
                json!({ "id": p1.id, "group_id": manual.id }),
            ),
        )
        .await
        .result
        .unwrap(),
    )
    .unwrap();
    assert_eq!(moved.group_id, manual.id);
    assert!(moved.group_pinned);

    // Agregar otro clon del remoto original no saca a p1 del grupo elegido.
    let p2: ProjectInfo = serde_json::from_value(
        dispatch(&mgr, req(METHOD_PROJECT_ADD, json!({ "path": clone2 })))
            .await
            .result
            .unwrap(),
    )
    .unwrap();
    assert_ne!(p2.group_id, manual.id);

    let list: Vec<ProjectInfo> = serde_json::from_value(
        dispatch(&mgr, req(METHOD_PROJECT_LIST, json!({})))
            .await
            .result
            .unwrap(),
    )
    .unwrap();
    let p1_after = list.iter().find(|p| p.id == p1.id).unwrap();
    assert_eq!(p1_after.group_id, manual.id);
    assert!(p1_after.group_pinned);
}

#[tokio::test]
async fn unpin_returns_to_remote_group() {
    let tmp = tempdir().unwrap();
    let clone1 = init_git_repo(&tmp.path().join("c1"), Some("git@github.com:org/lib.git"));
    let clone2 = init_git_repo(&tmp.path().join("c2"), Some("https://github.com/org/lib"));
    let mgr = Manager::new(200);

    let p1: ProjectInfo = serde_json::from_value(
        dispatch(&mgr, req(METHOD_PROJECT_ADD, json!({ "path": clone1 })))
            .await
            .result
            .unwrap(),
    )
    .unwrap();
    let p2: ProjectInfo = serde_json::from_value(
        dispatch(&mgr, req(METHOD_PROJECT_ADD, json!({ "path": clone2 })))
            .await
            .result
            .unwrap(),
    )
    .unwrap();
    assert_eq!(p1.group_id, p2.group_id);
    let remote_group_id = p1.group_id.clone();

    let manual: GroupInfo = serde_json::from_value(
        dispatch(&mgr, req(METHOD_GROUP_CREATE, json!({ "name": "Temp" })))
            .await
            .result
            .unwrap(),
    )
    .unwrap();

    let _: ProjectInfo = serde_json::from_value(
        dispatch(
            &mgr,
            req(
                METHOD_PROJECT_MOVE,
                json!({ "id": p1.id, "group_id": manual.id }),
            ),
        )
        .await
        .result
        .unwrap(),
    )
    .unwrap();

    let unpinned: ProjectInfo = serde_json::from_value(
        dispatch(&mgr, req(METHOD_PROJECT_UNPIN, json!({ "id": p1.id })))
            .await
            .result
            .unwrap(),
    )
    .unwrap();
    assert_eq!(unpinned.group_id, remote_group_id);
    assert!(!unpinned.group_pinned);

    // El grupo manual quedó vacío tras desanclar p1 y debe haberse borrado solo.
    let groups: Vec<GroupInfo> = serde_json::from_value(
        dispatch(&mgr, req(METHOD_GROUP_LIST, json!({})))
            .await
            .result
            .unwrap(),
    )
    .unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].id, remote_group_id);
}

#[tokio::test]
async fn empty_group_is_deleted() {
    let tmp = tempdir().unwrap();
    let repo = init_git_repo(
        &tmp.path().join("solo"),
        Some("git@github.com:org/solo.git"),
    );
    let mgr = Manager::new(200);

    let p: ProjectInfo = serde_json::from_value(
        dispatch(&mgr, req(METHOD_PROJECT_ADD, json!({ "path": repo })))
            .await
            .result
            .unwrap(),
    )
    .unwrap();

    let groups: Vec<GroupInfo> = serde_json::from_value(
        dispatch(&mgr, req(METHOD_GROUP_LIST, json!({})))
            .await
            .result
            .unwrap(),
    )
    .unwrap();
    assert_eq!(groups.len(), 1);

    let r = dispatch(&mgr, req(METHOD_PROJECT_REMOVE, json!({ "id": p.id }))).await;
    assert!(r.error.is_none());

    let groups: Vec<GroupInfo> = serde_json::from_value(
        dispatch(&mgr, req(METHOD_GROUP_LIST, json!({})))
            .await
            .result
            .unwrap(),
    )
    .unwrap();
    assert!(groups.is_empty(), "el grupo vacío debería haberse borrado");
}

#[tokio::test]
async fn rename_project_and_group() {
    let tmp = tempdir().unwrap();
    let repo = init_git_repo(&tmp.path().join("orig"), None);
    let mgr = Manager::new(200);

    let p: ProjectInfo = serde_json::from_value(
        dispatch(&mgr, req(METHOD_PROJECT_ADD, json!({ "path": repo })))
            .await
            .result
            .unwrap(),
    )
    .unwrap();

    let renamed_p: ProjectInfo = serde_json::from_value(
        dispatch(
            &mgr,
            req(
                METHOD_PROJECT_RENAME,
                json!({ "id": p.id, "name": "nuevo-nombre" }),
            ),
        )
        .await
        .result
        .unwrap(),
    )
    .unwrap();
    assert_eq!(renamed_p.name, "nuevo-nombre");

    let renamed_g: GroupInfo = serde_json::from_value(
        dispatch(
            &mgr,
            req(
                METHOD_GROUP_RENAME,
                json!({ "id": p.group_id, "name": "Grupo Renombrado" }),
            ),
        )
        .await
        .result
        .unwrap(),
    )
    .unwrap();
    assert_eq!(renamed_g.name, "Grupo Renombrado");

    // Crear un grupo con un nombre que ya existe devuelve error.
    let dup = dispatch(
        &mgr,
        req(METHOD_GROUP_CREATE, json!({ "name": "Grupo Renombrado" })),
    )
    .await;
    assert_eq!(dup.error.unwrap().code, ERR_GROUP_ALREADY_EXISTS);
}

#[tokio::test]
async fn remote_group_named_after_remote_repo() {
    let tmp = tempdir().unwrap();
    let repo_with_remote = init_git_repo(
        &tmp.path().join("suenolar"),
        Some("git@github.com:osmar/possible_rfid.git"),
    );
    let repo_local_only = init_git_repo(&tmp.path().join("experimento"), None);
    let mgr = Manager::new(200);

    let p_remote: ProjectInfo = serde_json::from_value(
        dispatch(
            &mgr,
            req(METHOD_PROJECT_ADD, json!({ "path": repo_with_remote })),
        )
        .await
        .result
        .unwrap(),
    )
    .unwrap();
    assert_eq!(p_remote.name, "suenolar");

    let p_local: ProjectInfo = serde_json::from_value(
        dispatch(
            &mgr,
            req(METHOD_PROJECT_ADD, json!({ "path": repo_local_only })),
        )
        .await
        .result
        .unwrap(),
    )
    .unwrap();
    assert_eq!(p_local.name, "experimento");

    let groups: Vec<GroupInfo> = serde_json::from_value(
        dispatch(&mgr, req(METHOD_GROUP_LIST, json!({})))
            .await
            .result
            .unwrap(),
    )
    .unwrap();

    let remote_group = groups.iter().find(|g| g.id == p_remote.group_id).unwrap();
    assert_eq!(remote_group.name, "possible_rfid");
    assert_eq!(
        remote_group.remote_key.as_deref(),
        Some("github.com/osmar/possible_rfid")
    );

    let local_group = groups.iter().find(|g| g.id == p_local.group_id).unwrap();
    assert_eq!(local_group.name, "experimento");
    assert_eq!(local_group.remote_key, None);
}

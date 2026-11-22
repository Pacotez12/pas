use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use pas_core::protocol::{
    CreateResult, ERR_WORKTREE_CONFIRM_REQUIRED, ERR_WORKTREE_DIRTY_MAIN,
    ERR_WORKTREE_MERGE_CONFLICT, ERR_WORKTREE_WRONG_BASE, METHOD_PROJECT_ADD,
    METHOD_SESSION_CREATE, METHOD_SESSION_KILL, METHOD_SESSION_LIST, METHOD_SESSION_READ,
    METHOD_SESSION_WAIT, METHOD_WORKTREE_CREATE, METHOD_WORKTREE_LIST, METHOD_WORKTREE_MERGE,
    METHOD_WORKTREE_REMOVE, METHOD_WORKTREE_SUGGEST_BRANCH, ProjectInfo, ReadResult, Request,
    SessionInfo, WorktreeInfo, WorktreeListItem, WorktreeState, WorktreeSuggestBranchResult,
};
use pasd::config::Config;
use pasd::manager::Manager;
use pasd::notify::NoopNotifier;
use pasd::rpc::dispatch;
use pasd::store::Store;
use serde_json::{Value, json};
use std::sync::Arc;
use tempfile::{TempDir, tempdir};

fn req(method: &str, params: Value) -> Request {
    Request {
        jsonrpc: "2.0".into(),
        id: 1,
        method: method.into(),
        params,
    }
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap_or_else(|e| panic!("no se pudo ejecutar git {args:?}: {e}"));
    assert!(
        out.status.success(),
        "git {args:?} falló en {}:\nstdout: {}\nstderr: {}",
        dir.display(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn git_ok(dir: &Path, args: &[&str]) -> bool {
    Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn init_repo_with_commit(dir: &Path) -> PathBuf {
    fs::create_dir_all(dir).unwrap();
    git(dir, &["init", "-b", "main", "--quiet"]);
    git(dir, &["config", "user.email", "test@pas.local"]);
    git(dir, &["config", "user.name", "Pas Test"]);
    fs::write(dir.join("README.md"), "hola\n").unwrap();
    git(dir, &["add", "README.md"]);
    git(dir, &["commit", "-m", "inicial", "--quiet"]);
    dir.to_path_buf()
}

struct WorktreeEnv {
    _tmp: TempDir,
    data_dir: PathBuf,
    repo: PathBuf,
    mgr: Arc<Manager>,
    project: ProjectInfo,
}

impl WorktreeEnv {
    async fn new() -> Self {
        Self::with_config(Config::default()).await
    }

    async fn with_config(config: Config) -> Self {
        let tmp = tempdir().unwrap();
        let data_dir = tmp.path().join("data");
        let repo = init_repo_with_commit(&tmp.path().join("mi-proyecto"));
        let store = Store::open(&data_dir.join("pas.db")).unwrap();
        let mgr = Manager::with_config(
            200,
            store,
            Some(data_dir.join("accounts")),
            None,
            config,
            Arc::new(NoopNotifier),
        );
        let r = dispatch(&mgr, req(METHOD_PROJECT_ADD, json!({ "path": repo }))).await;
        assert!(r.error.is_none(), "error agregando proyecto: {:?}", r.error);
        let project: ProjectInfo = serde_json::from_value(r.result.unwrap()).unwrap();
        Self {
            _tmp: tmp,
            data_dir,
            repo,
            mgr,
            project,
        }
    }
}

#[tokio::test]
async fn create_makes_branch_and_folder() {
    let env = WorktreeEnv::new().await;

    let r = dispatch(
        &env.mgr,
        req(
            METHOD_WORKTREE_CREATE,
            json!({
                "project_id": env.project.id,
                "agent": "agy",
                "title": "Fix login",
            }),
        ),
    )
    .await;
    assert!(r.error.is_none(), "error creando worktree: {:?}", r.error);
    let wt: WorktreeInfo = serde_json::from_value(r.result.unwrap()).unwrap();

    assert_eq!(wt.project_id, env.project.id);
    assert_eq!(wt.branch, "pas/agy-fix-login");
    assert_eq!(wt.base, "main");
    assert_eq!(wt.state, WorktreeState::Active);
    assert_eq!(
        wt.path,
        env.data_dir
            .join("worktrees")
            .join("mi-proyecto")
            .join("pas-agy-fix-login")
    );
    assert!(wt.path.is_dir(), "debería existir la carpeta del worktree");
    assert_eq!(
        git(&wt.path, &["rev-parse", "--abbrev-ref", "HEAD"]),
        "pas/agy-fix-login"
    );
}

#[tokio::test]
async fn suggest_branch_adds_suffix_when_taken() {
    let env = WorktreeEnv::new().await;

    let r1 = dispatch(
        &env.mgr,
        req(
            METHOD_WORKTREE_SUGGEST_BRANCH,
            json!({
                "project_id": env.project.id,
                "agent": "agy",
                "title": "Fix login",
            }),
        ),
    )
    .await;
    assert!(
        r1.error.is_none(),
        "error en suggest_branch: {:?}",
        r1.error
    );
    let s1: WorktreeSuggestBranchResult = serde_json::from_value(r1.result.unwrap()).unwrap();
    assert_eq!(s1.branch, "pas/agy-fix-login");
    assert_eq!(s1.base, "main");
    assert_eq!(
        s1.path,
        env.data_dir
            .join("worktrees")
            .join("mi-proyecto")
            .join("pas-agy-fix-login")
    );

    // Creamos la rama propuesta: la siguiente sugerencia debe llevar sufijo -2.
    git(&env.repo, &["branch", "pas/agy-fix-login"]);

    let r2 = dispatch(
        &env.mgr,
        req(
            METHOD_WORKTREE_SUGGEST_BRANCH,
            json!({
                "project_id": env.project.id,
                "agent": "agy",
                "title": "Fix login",
            }),
        ),
    )
    .await;
    let s2: WorktreeSuggestBranchResult = serde_json::from_value(r2.result.unwrap()).unwrap();
    assert_eq!(s2.branch, "pas/agy-fix-login-2");

    // Y si -2 también existe, pasa a -3, sin pisar nada.
    git(&env.repo, &["branch", "pas/agy-fix-login-2"]);
    let r3 = dispatch(
        &env.mgr,
        req(
            METHOD_WORKTREE_CREATE,
            json!({
                "project_id": env.project.id,
                "agent": "agy",
                "title": "Fix login",
            }),
        ),
    )
    .await;
    assert!(
        r3.error.is_none(),
        "error creando worktree -3: {:?}",
        r3.error
    );
    let wt3: WorktreeInfo = serde_json::from_value(r3.result.unwrap()).unwrap();
    assert_eq!(wt3.branch, "pas/agy-fix-login-3");
}

#[tokio::test]
async fn status_reports_uncommitted_and_ahead() {
    let env = WorktreeEnv::new().await;

    let wt: WorktreeInfo = serde_json::from_value(
        dispatch(
            &env.mgr,
            req(
                METHOD_WORKTREE_CREATE,
                json!({
                    "project_id": env.project.id,
                    "agent": "claude",
                    "title": "cambio",
                }),
            ),
        )
        .await
        .result
        .unwrap(),
    )
    .unwrap();

    // 2 commits en el worktree.
    fs::write(wt.path.join("a.txt"), "uno\n").unwrap();
    git(&wt.path, &["add", "a.txt"]);
    git(&wt.path, &["commit", "-m", "commit 1", "--quiet"]);

    fs::write(wt.path.join("b.txt"), "dos\n").unwrap();
    git(&wt.path, &["add", "b.txt"]);
    git(&wt.path, &["commit", "-m", "commit 2", "--quiet"]);

    // 1 modificado + 1 sin rastrear.
    fs::write(wt.path.join("a.txt"), "uno editado\n").unwrap();
    fs::write(wt.path.join("sin-commit.txt"), "pendiente\n").unwrap();

    let r = dispatch(
        &env.mgr,
        req(
            METHOD_WORKTREE_LIST,
            json!({ "project_id": env.project.id }),
        ),
    )
    .await;
    assert!(r.error.is_none(), "error listando worktrees: {:?}", r.error);
    let list: Vec<WorktreeListItem> = serde_json::from_value(r.result.unwrap()).unwrap();
    assert_eq!(list.len(), 1);
    let st = &list[0].status;
    assert_eq!(st.ahead, 2);
    assert!(!st.merged);
    assert!(!st.missing);
    assert!(
        st.uncommitted.iter().any(|f| f == "a.txt"),
        "falta a.txt en {:?}",
        st.uncommitted
    );
    assert!(
        st.uncommitted.iter().any(|f| f == "sin-commit.txt"),
        "falta sin-commit.txt en {:?}",
        st.uncommitted
    );
}

#[tokio::test]
async fn merge_refuses_dirty_or_wrong_branch_main() {
    let env = WorktreeEnv::new().await;

    let wt: WorktreeInfo = serde_json::from_value(
        dispatch(
            &env.mgr,
            req(
                METHOD_WORKTREE_CREATE,
                json!({
                    "project_id": env.project.id,
                    "agent": "agy",
                    "title": "feature",
                }),
            ),
        )
        .await
        .result
        .unwrap(),
    )
    .unwrap();

    fs::write(wt.path.join("feat.txt"), "ok\n").unwrap();
    git(&wt.path, &["add", "feat.txt"]);
    git(&wt.path, &["commit", "-m", "feat", "--quiet"]);

    let head_before = git(&env.repo, &["rev-parse", "HEAD"]);

    // Caso 1: repo principal con cambios sin commitear.
    fs::write(env.repo.join("sucio.txt"), "wip\n").unwrap();
    let r_dirty = dispatch(&env.mgr, req(METHOD_WORKTREE_MERGE, json!({ "id": wt.id }))).await;
    let err_dirty = r_dirty
        .error
        .expect("debería rechazar merge con repo principal sucio");
    assert_eq!(err_dirty.code, ERR_WORKTREE_DIRTY_MAIN);
    assert!(
        !err_dirty.message.contains(&wt.id),
        "el mensaje no debe incluir ids internos: {}",
        err_dirty.message
    );
    assert_eq!(git(&env.repo, &["rev-parse", "HEAD"]), head_before);
    assert!(env.repo.join("sucio.txt").exists());
    assert!(!env.repo.join("feat.txt").exists());

    // Limpiamos sucio.txt y cambiamos a otra rama en el repo principal.
    fs::remove_file(env.repo.join("sucio.txt")).unwrap();
    git(&env.repo, &["checkout", "-b", "otra", "--quiet"]);
    let head_otra = git(&env.repo, &["rev-parse", "HEAD"]);

    // Caso 2: repo principal en otra rama distinta de base (main).
    let r_wrong = dispatch(&env.mgr, req(METHOD_WORKTREE_MERGE, json!({ "id": wt.id }))).await;
    let err_wrong = r_wrong
        .error
        .expect("debería rechazar merge en rama equivocada");
    assert_eq!(err_wrong.code, ERR_WORKTREE_WRONG_BASE);
    assert_eq!(
        git(&env.repo, &["rev-parse", "--abbrev-ref", "HEAD"]),
        "otra"
    );
    assert_eq!(git(&env.repo, &["rev-parse", "HEAD"]), head_otra);
    assert!(!env.repo.join("feat.txt").exists());
}

#[tokio::test]
async fn merge_conflict_aborts_and_leaves_repo_clean() {
    let env = WorktreeEnv::new().await;

    let wt: WorktreeInfo = serde_json::from_value(
        dispatch(
            &env.mgr,
            req(
                METHOD_WORKTREE_CREATE,
                json!({
                    "project_id": env.project.id,
                    "agent": "codex",
                    "title": "conflicto",
                }),
            ),
        )
        .await
        .result
        .unwrap(),
    )
    .unwrap();

    fs::write(wt.path.join("README.md"), "version worktree\n").unwrap();
    git(&wt.path, &["add", "README.md"]);
    git(&wt.path, &["commit", "-m", "cambio wt", "--quiet"]);

    fs::write(env.repo.join("README.md"), "version main\n").unwrap();
    git(&env.repo, &["add", "README.md"]);
    git(&env.repo, &["commit", "-m", "cambio main", "--quiet"]);

    let r = dispatch(&env.mgr, req(METHOD_WORKTREE_MERGE, json!({ "id": wt.id }))).await;
    let err = r.error.expect("debería fallar por conflicto de merge");
    assert_eq!(err.code, ERR_WORKTREE_MERGE_CONFLICT);
    assert!(
        err.message.contains("README.md"),
        "el error debe nombrar el archivo en conflicto: {}",
        err.message
    );

    assert!(
        !env.repo.join(".git").join("MERGE_HEAD").exists(),
        "no debe quedar MERGE_HEAD tras abortar"
    );
    assert_eq!(
        git(&env.repo, &["status", "--porcelain"]),
        "",
        "el repo principal debe quedar limpio"
    );
    assert_eq!(
        fs::read_to_string(env.repo.join("README.md")).unwrap(),
        "version main\n"
    );
}

#[tokio::test]
async fn merge_then_remove_cleans_branch_and_folder() {
    let env = WorktreeEnv::new().await;

    let wt: WorktreeInfo = serde_json::from_value(
        dispatch(
            &env.mgr,
            req(
                METHOD_WORKTREE_CREATE,
                json!({
                    "project_id": env.project.id,
                    "agent": "agy",
                    "title": "limpio",
                }),
            ),
        )
        .await
        .result
        .unwrap(),
    )
    .unwrap();

    fs::write(wt.path.join("listo.txt"), "hecho\n").unwrap();
    git(&wt.path, &["add", "listo.txt"]);
    git(&wt.path, &["commit", "-m", "listo", "--quiet"]);

    let r_merge = dispatch(&env.mgr, req(METHOD_WORKTREE_MERGE, json!({ "id": wt.id }))).await;
    assert!(
        r_merge.error.is_none(),
        "error mergeando: {:?}",
        r_merge.error
    );
    let merged_info: WorktreeInfo = serde_json::from_value(r_merge.result.unwrap()).unwrap();
    assert_eq!(merged_info.state, WorktreeState::Merged);
    assert_eq!(
        fs::read_to_string(env.repo.join("listo.txt")).unwrap(),
        "hecho\n"
    );

    // El repo principal puede estar en otra rama al borrar: no debe fallar.
    git(&env.repo, &["checkout", "-q", "-b", "otra-rama", "HEAD~1"]);

    let r_rm = dispatch(
        &env.mgr,
        req(
            METHOD_WORKTREE_REMOVE,
            json!({ "id": wt.id, "confirm_loss": false }),
        ),
    )
    .await;
    assert!(
        r_rm.error.is_none(),
        "error borrando mergeado: {:?}",
        r_rm.error
    );
    assert!(
        !wt.path.exists(),
        "la carpeta del worktree debería haberse borrado"
    );
    assert!(
        !git_ok(
            &env.repo,
            &[
                "show-ref",
                "--verify",
                "--quiet",
                &format!("refs/heads/{}", wt.branch)
            ]
        ),
        "la rama debería haberse borrado"
    );

    let list: Vec<WorktreeListItem> = serde_json::from_value(
        dispatch(
            &env.mgr,
            req(
                METHOD_WORKTREE_LIST,
                json!({ "project_id": env.project.id }),
            ),
        )
        .await
        .result
        .unwrap(),
    )
    .unwrap();
    assert!(list.is_empty());
}

#[tokio::test]
async fn remove_with_loss_requires_confirm() {
    let env = WorktreeEnv::new().await;

    // Caso 1: cambios sin commitear.
    let wt1: WorktreeInfo = serde_json::from_value(
        dispatch(
            &env.mgr,
            req(
                METHOD_WORKTREE_CREATE,
                json!({
                    "project_id": env.project.id,
                    "agent": "agy",
                    "title": "con-cambios",
                }),
            ),
        )
        .await
        .result
        .unwrap(),
    )
    .unwrap();
    fs::write(wt1.path.join("wip.txt"), "sin guardar\n").unwrap();

    let r_no = dispatch(
        &env.mgr,
        req(
            METHOD_WORKTREE_REMOVE,
            json!({ "id": wt1.id, "confirm_loss": false }),
        ),
    )
    .await;
    let err = r_no.error.expect("debería pedir confirmación");
    assert_eq!(err.code, ERR_WORKTREE_CONFIRM_REQUIRED);
    assert!(
        !err.message.contains(&wt1.id),
        "sin ids internos: {}",
        err.message
    );
    assert!(wt1.path.exists(), "la carpeta debe seguir intacta");

    let r_yes = dispatch(
        &env.mgr,
        req(
            METHOD_WORKTREE_REMOVE,
            json!({ "id": wt1.id, "confirm_loss": true }),
        ),
    )
    .await;
    assert!(
        r_yes.error.is_none(),
        "error borrando con confirmación: {:?}",
        r_yes.error
    );
    assert!(!wt1.path.exists());

    // Caso 2: commits sin mergear.
    let wt2: WorktreeInfo = serde_json::from_value(
        dispatch(
            &env.mgr,
            req(
                METHOD_WORKTREE_CREATE,
                json!({
                    "project_id": env.project.id,
                    "agent": "agy",
                    "title": "sin-mergear",
                }),
            ),
        )
        .await
        .result
        .unwrap(),
    )
    .unwrap();
    fs::write(wt2.path.join("commit.txt"), "trabajo\n").unwrap();
    git(&wt2.path, &["add", "commit.txt"]);
    git(&wt2.path, &["commit", "-m", "sin mergear", "--quiet"]);

    let r_no2 = dispatch(
        &env.mgr,
        req(
            METHOD_WORKTREE_REMOVE,
            json!({ "id": wt2.id, "confirm_loss": false }),
        ),
    )
    .await;
    assert_eq!(r_no2.error.unwrap().code, ERR_WORKTREE_CONFIRM_REQUIRED);
    assert!(wt2.path.exists());
    assert!(git_ok(
        &env.repo,
        &[
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{}", wt2.branch)
        ]
    ));

    let r_yes2 = dispatch(
        &env.mgr,
        req(
            METHOD_WORKTREE_REMOVE,
            json!({ "id": wt2.id, "confirm_loss": true }),
        ),
    )
    .await;
    assert!(
        r_yes2.error.is_none(),
        "error borrando commits con confirmación: {:?}",
        r_yes2.error
    );
    assert!(!wt2.path.exists());
    assert!(!git_ok(
        &env.repo,
        &[
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{}", wt2.branch)
        ]
    ));
}

#[tokio::test]
async fn missing_folder_is_reported_and_prunable() {
    let env = WorktreeEnv::new().await;

    let wt: WorktreeInfo = serde_json::from_value(
        dispatch(
            &env.mgr,
            req(
                METHOD_WORKTREE_CREATE,
                json!({
                    "project_id": env.project.id,
                    "agent": "agy",
                    "title": "borrado-a-mano",
                }),
            ),
        )
        .await
        .result
        .unwrap(),
    )
    .unwrap();

    fs::remove_dir_all(&wt.path).unwrap();

    let list: Vec<WorktreeListItem> = serde_json::from_value(
        dispatch(
            &env.mgr,
            req(
                METHOD_WORKTREE_LIST,
                json!({ "project_id": env.project.id }),
            ),
        )
        .await
        .result
        .unwrap(),
    )
    .unwrap();
    assert_eq!(list.len(), 1);
    assert!(list[0].status.missing, "debería reportar missing = true");

    let r_rm = dispatch(
        &env.mgr,
        req(
            METHOD_WORKTREE_REMOVE,
            json!({ "id": wt.id, "confirm_loss": false }),
        ),
    )
    .await;
    assert!(
        r_rm.error.is_none(),
        "error limpiando missing: {:?}",
        r_rm.error
    );

    let list_after: Vec<WorktreeListItem> = serde_json::from_value(
        dispatch(
            &env.mgr,
            req(
                METHOD_WORKTREE_LIST,
                json!({ "project_id": env.project.id }),
            ),
        )
        .await
        .result
        .unwrap(),
    )
    .unwrap();
    assert!(list_after.is_empty());

    let porcelain = git(&env.repo, &["worktree", "list", "--porcelain"]);
    assert!(
        !porcelain.contains(&wt.path.to_string_lossy().into_owned()),
        "git worktree prune debería haber limpiado la entrada: {porcelain}"
    );
}

#[tokio::test]
async fn manual_worktrees_are_ignored() {
    let env = WorktreeEnv::new().await;
    let manual_path = env._tmp.path().join("manual-wt");
    git(
        &env.repo,
        &[
            "worktree",
            "add",
            "-b",
            "manual/rama",
            manual_path.to_str().unwrap(),
            "main",
        ],
    );

    let wt: WorktreeInfo = serde_json::from_value(
        dispatch(
            &env.mgr,
            req(
                METHOD_WORKTREE_CREATE,
                json!({
                    "project_id": env.project.id,
                    "agent": "agy",
                    "title": "propio",
                }),
            ),
        )
        .await
        .result
        .unwrap(),
    )
    .unwrap();

    let list: Vec<WorktreeListItem> = serde_json::from_value(
        dispatch(
            &env.mgr,
            req(
                METHOD_WORKTREE_LIST,
                json!({ "project_id": env.project.id }),
            ),
        )
        .await
        .result
        .unwrap(),
    )
    .unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].info.id, wt.id);

    dispatch(
        &env.mgr,
        req(
            METHOD_WORKTREE_REMOVE,
            json!({ "id": wt.id, "confirm_loss": false }),
        ),
    )
    .await;
    assert!(manual_path.is_dir(), "el worktree manual no debe tocarse");
    assert!(git_ok(
        &env.repo,
        &["show-ref", "--verify", "--quiet", "refs/heads/manual/rama"]
    ));
}

#[tokio::test]
async fn custom_worktrees_dir_from_config() {
    let cfg_tmp = tempdir().unwrap();
    let custom_dir = cfg_tmp.path().join("mis-worktrees");
    let cfg_path = cfg_tmp.path().join("config.toml");
    fs::write(
        &cfg_path,
        format!("worktrees_dir = {:?}\n", custom_dir.to_string_lossy()),
    )
    .unwrap();
    let cfg = Config::load_from_path(&cfg_path).unwrap();
    let env = WorktreeEnv::with_config(cfg).await;

    let r = dispatch(
        &env.mgr,
        req(
            METHOD_WORKTREE_CREATE,
            json!({
                "project_id": env.project.id,
                "agent": "agy",
                "title": "custom",
            }),
        ),
    )
    .await;
    assert!(
        r.error.is_none(),
        "error creando en custom dir: {:?}",
        r.error
    );
    let wt: WorktreeInfo = serde_json::from_value(r.result.unwrap()).unwrap();
    assert_eq!(
        wt.path,
        custom_dir.join("mi-proyecto").join("pas-agy-custom")
    );
    assert!(wt.path.is_dir());
}

#[tokio::test]
async fn session_in_worktree_runs_in_its_folder() {
    let env = WorktreeEnv::new().await;

    let r = dispatch(
        &env.mgr,
        req(
            METHOD_SESSION_CREATE,
            json!({
                "command": ["sh", "-c", "pwd"],
                "cwd": env.repo,
                "title": "tarea-wt",
                "project_id": env.project.id,
                "worktree": {},
            }),
        ),
    )
    .await;
    assert!(
        r.error.is_none(),
        "error creando sesión en worktree: {:?}",
        r.error
    );
    let created: CreateResult = serde_json::from_value(r.result.unwrap()).unwrap();

    dispatch(
        &env.mgr,
        req(
            METHOD_SESSION_WAIT,
            json!({ "id": created.id, "until": "exited", "timeout_ms": 5000 }),
        ),
    )
    .await;

    let read: ReadResult = serde_json::from_value(
        dispatch(
            &env.mgr,
            req(METHOD_SESSION_READ, json!({ "id": created.id })),
        )
        .await
        .result
        .unwrap(),
    )
    .unwrap();

    let list: Vec<WorktreeListItem> = serde_json::from_value(
        dispatch(
            &env.mgr,
            req(
                METHOD_WORKTREE_LIST,
                json!({ "project_id": env.project.id }),
            ),
        )
        .await
        .result
        .unwrap(),
    )
    .unwrap();
    assert_eq!(list.len(), 1);
    let wt = &list[0].info;
    assert_eq!(wt.session_id.as_deref(), Some(created.id.as_str()));
    assert!(
        read.text.contains(&wt.path.to_string_lossy().into_owned()),
        "pwd esperado {}, salida: {:?}",
        wt.path.display(),
        read.text
    );
}

#[tokio::test]
async fn session_info_includes_branch() {
    let env = WorktreeEnv::new().await;

    let r = dispatch(
        &env.mgr,
        req(
            METHOD_SESSION_CREATE,
            json!({
                "command": ["sh", "-c", "sleep 30"],
                "cwd": env.repo,
                "title": "con-rama",
                "project_id": env.project.id,
                "worktree": { "branch": "pas/mi-rama" },
            }),
        ),
    )
    .await;
    assert!(r.error.is_none(), "error creando sesión: {:?}", r.error);
    let created: CreateResult = serde_json::from_value(r.result.unwrap()).unwrap();

    let sessions: Vec<SessionInfo> = serde_json::from_value(
        dispatch(&env.mgr, req(METHOD_SESSION_LIST, json!({})))
            .await
            .result
            .unwrap(),
    )
    .unwrap();
    let sess = sessions.into_iter().find(|s| s.id == created.id).unwrap();
    let wt = sess.worktree.expect("SessionInfo debería incluir worktree");
    assert_eq!(wt.branch, "pas/mi-rama");
    assert_eq!(wt.path, sess.cwd);
    assert_eq!(wt.session_id.as_deref(), Some(created.id.as_str()));

    dispatch(
        &env.mgr,
        req(METHOD_SESSION_KILL, json!({ "id": created.id })),
    )
    .await;
}

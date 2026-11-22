use std::ffi::OsString;
use std::fs::{self, Permissions};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use pas_core::protocol::{
    AccountInfo, AgentKind, DiscoveredAccount, DiscoveredProject, ERR_EDITOR_NOT_FOUND,
    ERR_PATH_NOT_FOUND, ERR_PATH_OUTSIDE_PROJECT, METHOD_ACCOUNT_ADD, METHOD_ACCOUNT_DISCOVER,
    METHOD_OPEN_EXISTS, METHOD_OPEN_PATH, METHOD_PROJECT_ADD, METHOD_PROJECT_DISCOVER,
    OpenExistsResult, ProjectInfo, Request,
};
use pasd::acp::AcpEnv;
use pasd::config::Config;
use pasd::discover::discover_projects_with_limits;
use pasd::manager::Manager;
use pasd::notify::NoopNotifier;
use pasd::open::{LaunchRequest, Launcher, OpenEnv};
use pasd::rpc::dispatch;
use pasd::store::Store;
use serde_json::{Value, json};
use tempfile::tempdir;

#[derive(Default)]
struct FakeLauncher {
    calls: Mutex<Vec<LaunchRequest>>,
}

impl FakeLauncher {
    fn calls(&self) -> Vec<LaunchRequest> {
        self.calls.lock().unwrap().clone()
    }
}

impl Launcher for FakeLauncher {
    fn launch(&self, req: LaunchRequest) -> Result<(), std::io::Error> {
        self.calls.lock().unwrap().push(req);
        Ok(())
    }
}

fn req(method: &str, params: Value) -> Request {
    Request {
        jsonrpc: "2.0".into(),
        id: 1,
        method: method.into(),
        params,
    }
}

fn make_executable(dir: &Path, name: &str) -> PathBuf {
    fs::create_dir_all(dir).unwrap();
    let bin = dir.join(name);
    fs::write(&bin, "#!/bin/sh\nexit 0\n").unwrap();
    fs::set_permissions(&bin, Permissions::from_mode(0o755)).unwrap();
    bin
}

fn make_git_dir(dir: &Path) -> PathBuf {
    fs::create_dir_all(dir.join(".git")).unwrap();
    dir.to_path_buf()
}

fn make_manager(
    config: Config,
    path_env: Option<OsString>,
    home_dir: Option<PathBuf>,
) -> (Arc<Manager>, Arc<FakeLauncher>) {
    let launcher = Arc::new(FakeLauncher::default());
    let store = Store::open_in_memory().unwrap();
    let mgr = Manager::with_open_env(
        200,
        store,
        None,
        None,
        config,
        Arc::new(NoopNotifier),
        AcpEnv::default(),
        launcher.clone(),
        OpenEnv { path_env, home_dir },
    );
    (mgr, launcher)
}

async fn add_project(mgr: &Manager, path: &Path) -> ProjectInfo {
    let r = dispatch(mgr, req(METHOD_PROJECT_ADD, json!({ "path": path }))).await;
    assert!(r.error.is_none(), "error agregando proyecto: {:?}", r.error);
    serde_json::from_value(r.result.unwrap()).unwrap()
}

#[tokio::test]
async fn open_path_uses_code_with_line() {
    let tmp = tempdir().unwrap();
    let bin_dir = tmp.path().join("bin");
    let code_bin = make_executable(&bin_dir, "code");
    let repo = make_git_dir(&tmp.path().join("pockt"));
    let file = repo.join("lib").join("main.dart");
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(&file, "void main() {}\n").unwrap();

    let cfg = Config {
        editor: Some("code".into()),
        ..Config::default()
    };
    let (mgr, launcher) = make_manager(cfg, Some(bin_dir.into_os_string()), None);
    add_project(&mgr, &repo).await;

    let r = dispatch(
        &mgr,
        req(
            METHOD_OPEN_PATH,
            json!({ "path": file, "line": 42, "target": "editor" }),
        ),
    )
    .await;
    assert!(r.error.is_none(), "error inesperado: {:?}", r.error);

    let calls = launcher.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].program, code_bin);
    let canon_file = fs::canonicalize(&file).unwrap();
    assert_eq!(
        calls[0].args,
        vec![
            OsString::from("-g"),
            OsString::from(format!("{}:42", canon_file.display())),
        ]
    );
}

#[tokio::test]
async fn open_path_falls_back_to_first_editor_in_path() {
    let tmp = tempdir().unwrap();
    let bin_dir = tmp.path().join("bin");
    let cursor_bin = make_executable(&bin_dir, "cursor");
    let _zed_bin = make_executable(&bin_dir, "zed");
    let repo = make_git_dir(&tmp.path().join("pockt"));
    let file = repo.join("README.md");
    fs::write(&file, "# hola\n").unwrap();

    let (mgr, launcher) = make_manager(Config::default(), Some(bin_dir.into_os_string()), None);
    add_project(&mgr, &repo).await;

    let r = dispatch(
        &mgr,
        req(
            METHOD_OPEN_PATH,
            json!({ "path": file, "line": 10, "target": "editor" }),
        ),
    )
    .await;
    assert!(r.error.is_none(), "error inesperado: {:?}", r.error);

    let calls = launcher.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].program, cursor_bin);
    // Para otros editores distintos de `code`, solo pasa la ruta sin `-g`.
    let canon_file = fs::canonicalize(&file).unwrap();
    assert_eq!(calls[0].args, vec![canon_file.into_os_string()]);
}

#[tokio::test]
async fn open_path_missing_editor_gives_clear_error() {
    let tmp = tempdir().unwrap();
    let empty_bin = tmp.path().join("empty_bin");
    fs::create_dir_all(&empty_bin).unwrap();
    let repo = make_git_dir(&tmp.path().join("pockt"));

    let (mgr, launcher) = make_manager(Config::default(), Some(empty_bin.into_os_string()), None);
    add_project(&mgr, &repo).await;

    let r = dispatch(
        &mgr,
        req(
            METHOD_OPEN_PATH,
            json!({ "path": repo, "target": "editor" }),
        ),
    )
    .await;
    let err = r.error.expect("debería fallar sin editor en PATH");
    assert_eq!(err.code, ERR_EDITOR_NOT_FOUND);
    assert_eq!(
        err.message,
        "No encontré un editor. Configurá `editor` en ~/.config/pas/config.toml (por ejemplo `editor = \"code\"`)."
    );
    assert!(launcher.calls().is_empty());
}

#[tokio::test]
async fn open_path_rejects_missing_path() {
    let tmp = tempdir().unwrap();
    let bin_dir = tmp.path().join("bin");
    make_executable(&bin_dir, "code");
    let repo = make_git_dir(&tmp.path().join("pockt"));

    let (mgr, launcher) = make_manager(Config::default(), Some(bin_dir.into_os_string()), None);
    add_project(&mgr, &repo).await;

    let missing = repo.join("no-existe.rs");
    let r = dispatch(
        &mgr,
        req(
            METHOD_OPEN_PATH,
            json!({ "path": missing, "target": "editor" }),
        ),
    )
    .await;
    let err = r.error.expect("debería rechazar ruta inexistente");
    assert_eq!(err.code, ERR_PATH_NOT_FOUND);
    assert!(launcher.calls().is_empty());
}

#[tokio::test]
async fn open_path_rejects_path_outside_projects() {
    let tmp = tempdir().unwrap();
    let bin_dir = tmp.path().join("bin");
    make_executable(&bin_dir, "code");
    let repo = make_git_dir(&tmp.path().join("pockt"));
    let outside = tmp.path().join("afuera.txt");
    fs::write(&outside, "secreto").unwrap();

    let (mgr, launcher) = make_manager(Config::default(), Some(bin_dir.into_os_string()), None);
    add_project(&mgr, &repo).await;

    let r = dispatch(
        &mgr,
        req(
            METHOD_OPEN_PATH,
            json!({ "path": outside, "target": "editor" }),
        ),
    )
    .await;
    let err = r
        .error
        .expect("debería rechazar ruta fuera de proyectos abiertos");
    assert_eq!(err.code, ERR_PATH_OUTSIDE_PROJECT);
    assert!(launcher.calls().is_empty());
}

#[tokio::test]
async fn open_path_keeps_spaces_in_one_arg() {
    let tmp = tempdir().unwrap();
    let bin_dir = tmp.path().join("bin");
    let code_bin = make_executable(&bin_dir, "code");
    let repo = make_git_dir(&tmp.path().join("mi proyecto (2026)"));
    let tricky = repo
        .join("carpeta con espacios & $; 'comillas'")
        .join("archivo prueba.txt");
    fs::create_dir_all(tricky.parent().unwrap()).unwrap();
    fs::write(&tricky, "hola\n").unwrap();

    let (mgr, launcher) = make_manager(Config::default(), Some(bin_dir.into_os_string()), None);
    add_project(&mgr, &repo).await;

    let r = dispatch(
        &mgr,
        req(
            METHOD_OPEN_PATH,
            json!({ "path": tricky, "line": 7, "target": "editor" }),
        ),
    )
    .await;
    assert!(r.error.is_none(), "error inesperado: {:?}", r.error);

    let calls = launcher.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].program, code_bin);
    let canon = fs::canonicalize(&tricky).unwrap();
    assert_eq!(calls[0].args.len(), 2);
    assert_eq!(calls[0].args[0], OsString::from("-g"));
    assert_eq!(
        calls[0].args[1],
        OsString::from(format!("{}:7", canon.display()))
    );
}

#[tokio::test]
async fn open_path_files_and_terminal_targets() {
    let tmp = tempdir().unwrap();
    let bin_dir = tmp.path().join("bin");
    let xdg_bin = make_executable(&bin_dir, "xdg-open");
    let _kgx_bin = make_executable(&bin_dir, "kgx");
    let kitty_bin = make_executable(&bin_dir, "kitty");
    let repo = make_git_dir(&tmp.path().join("pockt"));
    let file = repo.join("src").join("app.rs");
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(&file, "fn main() {}\n").unwrap();

    let (mgr, launcher) = make_manager(Config::default(), Some(bin_dir.into_os_string()), None);
    add_project(&mgr, &repo).await;

    let r_files = dispatch(
        &mgr,
        req(METHOD_OPEN_PATH, json!({ "path": file, "target": "files" })),
    )
    .await;
    assert!(r_files.error.is_none(), "error: {:?}", r_files.error);

    let r_term = dispatch(
        &mgr,
        req(
            METHOD_OPEN_PATH,
            json!({ "path": file, "target": "terminal" }),
        ),
    )
    .await;
    assert!(r_term.error.is_none(), "error: {:?}", r_term.error);

    let calls = launcher.calls();
    assert_eq!(calls.len(), 2);
    let canon_dir = fs::canonicalize(file.parent().unwrap()).unwrap();

    assert_eq!(calls[0].program, xdg_bin);
    assert_eq!(calls[0].args, vec![canon_dir.clone().into_os_string()]);

    assert_eq!(calls[1].program, kitty_bin);
    assert!(calls[1].args.is_empty());
    assert_eq!(calls[1].cwd.as_deref(), Some(canon_dir.as_path()));
}

#[tokio::test]
async fn open_exists_checks_relative_and_missing_paths() {
    let tmp = tempdir().unwrap();
    let repo = make_git_dir(&tmp.path().join("pockt"));
    let file = repo.join("lib").join("a.dart");
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(&file, "// dart\n").unwrap();

    let (mgr, _) = make_manager(Config::default(), None, Some(tmp.path().to_path_buf()));

    let r_ok = dispatch(
        &mgr,
        req(
            METHOD_OPEN_EXISTS,
            json!({ "cwd": repo, "path": "lib/a.dart" }),
        ),
    )
    .await;
    let res_ok: OpenExistsResult = serde_json::from_value(r_ok.result.unwrap()).unwrap();
    assert!(res_ok.exists);
    assert!(!res_ok.is_dir);
    assert_eq!(res_ok.resolved, Some(fs::canonicalize(&file).unwrap()));

    let r_missing = dispatch(
        &mgr,
        req(
            METHOD_OPEN_EXISTS,
            json!({ "cwd": repo, "path": "lib/no_existe.dart" }),
        ),
    )
    .await;
    let res_missing: OpenExistsResult = serde_json::from_value(r_missing.result.unwrap()).unwrap();
    assert!(!res_missing.exists);
    assert!(res_missing.resolved.is_none());
}

#[tokio::test]
async fn discover_finds_repos_and_skips_added() {
    let tmp = tempdir().unwrap();
    let fake_home = tmp.path().join("home");
    let repo_a = make_git_dir(&fake_home.join("Proyectos").join("pockt"));
    let repo_b = make_git_dir(&fake_home.join("orca").join("projects").join("pas"));
    let repo_c = make_git_dir(&fake_home.join("Trabajo").join("cliente").join("api"));

    let (mgr, _) = make_manager(Config::default(), None, Some(fake_home));
    add_project(&mgr, &repo_b).await;

    let r = dispatch(&mgr, req(METHOD_PROJECT_DISCOVER, json!({}))).await;
    assert!(r.error.is_none(), "error inesperado: {:?}", r.error);
    let found: Vec<DiscoveredProject> = serde_json::from_value(r.result.unwrap()).unwrap();
    let paths: Vec<PathBuf> = found.into_iter().map(|p| p.path).collect();

    assert!(paths.contains(&fs::canonicalize(&repo_a).unwrap()));
    assert!(paths.contains(&fs::canonicalize(&repo_c).unwrap()));
    assert!(
        !paths.contains(&fs::canonicalize(&repo_b).unwrap()),
        "no debería incluir proyectos ya agregados"
    );
}

#[tokio::test]
async fn discover_respects_depth_and_skips_node_modules() {
    let tmp = tempdir().unwrap();
    let fake_home = tmp.path().join("home");
    let root = fake_home.join("Proyectos");
    let depth3 = make_git_dir(&root.join("a").join("b").join("repo3"));
    let depth4 = make_git_dir(&root.join("a").join("b").join("c").join("repo4"));
    let in_node_modules = make_git_dir(&root.join("node_modules").join("pkg"));
    let in_target = make_git_dir(&root.join("target").join("dep"));
    let in_build = make_git_dir(&root.join("build").join("out"));

    let (mgr, _) = make_manager(Config::default(), None, Some(fake_home));
    let r = dispatch(&mgr, req(METHOD_PROJECT_DISCOVER, json!({}))).await;
    assert!(r.error.is_none(), "error inesperado: {:?}", r.error);
    let found: Vec<DiscoveredProject> = serde_json::from_value(r.result.unwrap()).unwrap();
    let paths: Vec<PathBuf> = found.into_iter().map(|p| p.path).collect();

    assert!(paths.contains(&fs::canonicalize(&depth3).unwrap()));
    assert!(!paths.contains(&fs::canonicalize(&depth4).unwrap()));
    assert!(!paths.contains(&fs::canonicalize(&in_node_modules).unwrap()));
    assert!(!paths.contains(&fs::canonicalize(&in_target).unwrap()));
    assert!(!paths.contains(&fs::canonicalize(&in_build).unwrap()));
}

#[test]
fn discover_respects_max_results_and_timeout() {
    let tmp = tempdir().unwrap();
    let fake_home = tmp.path().join("home");
    let root = fake_home.join("Proyectos");
    for i in 0..25 {
        make_git_dir(&root.join(format!("repo-{i:02}")));
    }

    let store = Store::open_in_memory().unwrap();
    let capped =
        discover_projects_with_limits(&store, None, Some(&fake_home), 10, Duration::from_secs(2))
            .unwrap();
    assert_eq!(capped.len(), 10);

    // Tiempo agotado inmediatamente (0 ns) corta antes de recorrer todo.
    let timed_out =
        discover_projects_with_limits(&store, None, Some(&fake_home), 200, Duration::from_nanos(0))
            .unwrap();
    assert!(timed_out.len() < 25);
}

#[tokio::test]
async fn account_discover_lists_orca_profiles_and_marks_added() {
    let tmp = tempdir().unwrap();
    let fake_home = tmp.path().join("home");
    let orca_work = fake_home
        .join(".config")
        .join("orca")
        .join("claude-profiles")
        .join("trabajo")
        .join("home");
    let orca_personal = fake_home
        .join(".config")
        .join("orca")
        .join("claude-profiles")
        .join("personal")
        .join("home");
    let dot_claude = fake_home.join(".claude");
    let dot_codex = fake_home.join(".codex");
    fs::create_dir_all(&orca_work).unwrap();
    fs::create_dir_all(&orca_personal).unwrap();
    fs::create_dir_all(&dot_claude).unwrap();
    fs::create_dir_all(&dot_codex).unwrap();

    // Archivo de credenciales sin permisos de lectura: account.discover nunca debe abrirlo.
    let creds = orca_work.join(".credentials.json");
    fs::write(&creds, r#"{"token":"secreto"}"#).unwrap();
    fs::set_permissions(&creds, Permissions::from_mode(0o000)).unwrap();

    let (mgr, _) = make_manager(Config::default(), None, Some(fake_home));

    let added_res = dispatch(
        &mgr,
        req(
            METHOD_ACCOUNT_ADD,
            json!({
                "agent": "claude",
                "name": "Trabajo Orca",
                "config_dir": orca_work,
            }),
        ),
    )
    .await;
    assert!(added_res.error.is_none(), "{:?}", added_res.error);
    let _added: AccountInfo = serde_json::from_value(added_res.result.unwrap()).unwrap();

    let r = dispatch(&mgr, req(METHOD_ACCOUNT_DISCOVER, json!({}))).await;
    assert!(r.error.is_none(), "error inesperado: {:?}", r.error);
    let found: Vec<DiscoveredAccount> = serde_json::from_value(r.result.unwrap()).unwrap();

    // Restaurar permisos para que tempdir pueda limpiar.
    let _ = fs::set_permissions(&creds, Permissions::from_mode(0o600));

    assert_eq!(found.len(), 4);

    let canon_work = fs::canonicalize(&orca_work).unwrap();
    let work_item = found.iter().find(|a| a.config_dir == canon_work).unwrap();
    assert_eq!(work_item.agent, AgentKind::Claude);
    assert_eq!(work_item.suggested_name, "trabajo");
    assert_eq!(work_item.already_added_as.as_deref(), Some("Trabajo Orca"));

    let canon_personal = fs::canonicalize(&orca_personal).unwrap();
    let personal_item = found
        .iter()
        .find(|a| a.config_dir == canon_personal)
        .unwrap();
    assert_eq!(personal_item.agent, AgentKind::Claude);
    assert_eq!(personal_item.suggested_name, "personal");
    assert_eq!(personal_item.already_added_as, None);

    let canon_claude = fs::canonicalize(&dot_claude).unwrap();
    assert!(
        found
            .iter()
            .any(|a| a.config_dir == canon_claude && a.agent == AgentKind::Claude)
    );

    let canon_codex = fs::canonicalize(&dot_codex).unwrap();
    assert!(
        found
            .iter()
            .any(|a| a.config_dir == canon_codex && a.agent == AgentKind::Codex)
    );
}

#[test]
fn config_loads_editor_and_discover_dirs() {
    let tmp = tempdir().unwrap();
    let cfg_path = tmp.path().join("config.toml");
    fs::write(
        &cfg_path,
        r#"
editor = "code"
discover_dirs = ["~/Proyectos", "/srv/repos"]
"#,
    )
    .unwrap();

    let cfg = Config::load_from_path(&cfg_path).unwrap();
    assert_eq!(cfg.editor.as_deref(), Some("code"));
    assert_eq!(
        cfg.discover_dirs,
        Some(vec![
            PathBuf::from("~/Proyectos"),
            PathBuf::from("/srv/repos")
        ])
    );
}

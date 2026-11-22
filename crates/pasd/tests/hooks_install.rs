use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use pas_core::protocol::{
    AccountInfo, ERR_HOOKS_CONFIG_INVALID, METHOD_ACCOUNT_ADD, METHOD_ACCOUNT_INSTALL_HOOKS,
    METHOD_ACCOUNT_LIST, METHOD_ACCOUNT_UNINSTALL_HOOKS, Request,
};
use pasd::manager::Manager;
use pasd::rpc::dispatch;
use pasd::server::{bind, serve};
use pasd::store::Store;
use serde_json::{Value, json};
use tempfile::tempdir;
use tokio::net::UnixListener;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn target_debug_dir() -> PathBuf {
    std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| workspace_root().join("target"))
        .join("debug")
}

fn pas_bin() -> String {
    static BIN: OnceLock<String> = OnceLock::new();
    BIN.get_or_init(|| {
        let status = Command::new("cargo")
            .args(["build", "-p", "pas-cli", "--quiet"])
            .current_dir(workspace_root())
            .status()
            .expect("no se pudo compilar pas-cli");
        assert!(status.success(), "falló cargo build -p pas-cli");
        target_debug_dir()
            .join("pas")
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

/// Fixture sintético que imita la estructura real de Claude `settings.json`
/// (con orden de claves no alfabético, hooks de herdr y orca, y statusLine).
const CLAUDE_SETTINGS_FIXTURE: &str = r#"{
  "theme": "dark",
  "model": "claude-sonnet",
  "statusLine": {
    "type": "command",
    "command": "bash /fake/claude/scripts/statusline.sh",
    "padding": 0
  },
  "hooks": {
    "SessionStart": [
      {
        "matcher": "^(startup|resume)$",
        "hooks": [
          {
            "type": "command",
            "command": "bash '/fake/claude/hooks/herdr-agent-state.sh' session",
            "timeout": 10
          }
        ]
      }
    ],
    "UserPromptSubmit": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "if [ -n \"${ORCA_PANE_KEY-}\" ]; then /bin/sh /fake/orca/claude-hook.sh; else printf '{}\\n'; fi",
            "timeout": 10
          }
        ]
      }
    ],
    "Stop": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "bash '/fake/claude/hooks/herdr-agent-state.sh' stop",
            "timeout": 10
          }
        ]
      }
    ]
  },
  "language": "es"
}"#;

/// Fixture sintético que imita la estructura real de Codex `hooks.json`.
const CODEX_HOOKS_FIXTURE: &str = r#"{
  "hooks": {
    "SessionStart": [
      {
        "hooks": [
          {
            "type": "command",
            "command": ": orca-agent-hook-form=2; if [ -n \"${ORCA_PANE_KEY-}\" ]; then /bin/sh /fake/orca/codex-hook.sh; fi",
            "timeout": 10,
            "statusMessage": "Reporting status to Orca"
          }
        ]
      }
    ],
    "UserPromptSubmit": [
      {
        "hooks": [
          {
            "type": "command",
            "command": ": orca-agent-hook-form=2; if [ -n \"${ORCA_PANE_KEY-}\" ]; then /bin/sh /fake/orca/codex-hook.sh; fi",
            "timeout": 10,
            "statusMessage": "Reporting status to Orca"
          }
        ]
      }
    ],
    "Stop": [
      {
        "hooks": [
          {
            "type": "command",
            "command": ": orca-agent-hook-form=2; if [ -n \"${ORCA_PANE_KEY-}\" ]; then /bin/sh /fake/orca/codex-hook.sh; fi",
            "timeout": 10,
            "statusMessage": "Reporting status to Orca"
          }
        ]
      }
    ]
  }
}
"#;

/// Fixture sintético que imita `~/.gemini/config/hooks.json`.
const AGY_HOOKS_FIXTURE: &str = r#"{
  "herdr": {
    "PreInvocation": [
      {
        "command": "bash '/fake/gemini/config/hooks/herdr-agent-state.sh' session",
        "timeout": 10,
        "type": "command"
      }
    ]
  },
  "orca-status": {
    "PreInvocation": [
      {
        "type": "command",
        "command": "/bin/sh -c 'if [ -n \"${ORCA_PANE_KEY-}\" ]; then /bin/sh /fake/orca/agy-hook.sh; else printf \"{}\\n\"; fi'",
        "timeout": 10
      }
    ],
    "PreToolUse": [
      {
        "matcher": "*",
        "hooks": [
          {
            "type": "command",
            "command": "/bin/sh -c 'if [ -n \"${ORCA_PANE_KEY-}\" ]; then /bin/sh /fake/orca/agy-hook.sh; fi'",
            "timeout": 10
          }
        ]
      }
    ]
  }
}
"#;

/// Fixture sintético que imita `~/.gemini/antigravity-cli/settings.json`
/// (incluyendo escapes unicode y orden propio).
const AGY_SETTINGS_FIXTURE: &str = r#"{
  "altScreenMode": "always",
  "permissions": {
    "allow": [
      "command(echo a \u0026\u0026 echo b)"
    ]
  },
  "statusLine": {
    "type": "command",
    "command": "bash /fake/gemini/scripts/statusline.sh",
    "stack_with_default": true
  }
}"#;

fn list_backups(dir: &Path, filename: &str) -> Vec<PathBuf> {
    let prefix = format!("{filename}.bak-pas-");
    let mut out = Vec::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            if let Some(name) = entry.file_name().to_str()
                && name.starts_with(&prefix)
            {
                out.push(entry.path());
            }
        }
    }
    out.sort();
    out
}

async fn add_account(mgr: &Manager, agent: &str, name: &str, dir: &Path) -> AccountInfo {
    let _ = pas_bin();
    let r = dispatch(
        mgr,
        req(
            METHOD_ACCOUNT_ADD,
            json!({
                "agent": agent,
                "name": name,
                "config_dir": dir,
            }),
        ),
    )
    .await;
    assert!(r.error.is_none(), "account.add falló: {:?}", r.error);
    serde_json::from_value(r.result.unwrap()).unwrap()
}

#[tokio::test]
async fn install_then_uninstall_is_identity() {
    let tmp = tempdir().unwrap();
    let db = tmp.path().join("data").join("pas.db");
    let mgr = Manager::with_store(200, Store::open(&db).unwrap());

    // 1. Claude
    let claude_dir = tmp.path().join("claude-profile");
    fs::create_dir_all(&claude_dir).unwrap();
    let claude_settings = claude_dir.join("settings.json");
    fs::write(&claude_settings, CLAUDE_SETTINGS_FIXTURE).unwrap();
    let claude_acc = add_account(&mgr, "claude", "Claude", &claude_dir).await;
    assert!(!claude_acc.hooks_installed);

    let r = dispatch(
        &mgr,
        req(METHOD_ACCOUNT_INSTALL_HOOKS, json!({ "id": claude_acc.id })),
    )
    .await;
    assert!(r.error.is_none(), "install claude falló: {:?}", r.error);
    let installed: AccountInfo = serde_json::from_value(r.result.unwrap()).unwrap();
    assert!(installed.hooks_installed);

    let r = dispatch(
        &mgr,
        req(METHOD_ACCOUNT_UNINSTALL_HOOKS, json!({ "id": claude_acc.id })),
    )
    .await;
    assert!(r.error.is_none(), "uninstall claude falló: {:?}", r.error);
    let uninstalled: AccountInfo = serde_json::from_value(r.result.unwrap()).unwrap();
    assert!(!uninstalled.hooks_installed);

    let after_claude = fs::read(&claude_settings).unwrap();
    assert_eq!(
        after_claude,
        CLAUDE_SETTINGS_FIXTURE.as_bytes(),
        "Claude settings.json debe quedar idéntico byte a byte tras quitar hooks"
    );

    // 2. Codex
    let codex_dir = tmp.path().join("codex-home");
    fs::create_dir_all(&codex_dir).unwrap();
    let codex_hooks = codex_dir.join("hooks.json");
    fs::write(&codex_hooks, CODEX_HOOKS_FIXTURE).unwrap();
    let codex_acc = add_account(&mgr, "codex", "Codex", &codex_dir).await;

    let r = dispatch(
        &mgr,
        req(METHOD_ACCOUNT_INSTALL_HOOKS, json!({ "id": codex_acc.id })),
    )
    .await;
    assert!(r.error.is_none(), "install codex falló: {:?}", r.error);
    let r = dispatch(
        &mgr,
        req(METHOD_ACCOUNT_UNINSTALL_HOOKS, json!({ "id": codex_acc.id })),
    )
    .await;
    assert!(r.error.is_none(), "uninstall codex falló: {:?}", r.error);

    let after_codex = fs::read(&codex_hooks).unwrap();
    assert_eq!(
        after_codex,
        CODEX_HOOKS_FIXTURE.as_bytes(),
        "Codex hooks.json debe quedar idéntico byte a byte tras quitar hooks"
    );

    // 3. agy (config/hooks.json + antigravity-cli/settings.json)
    let agy_dir = tmp.path().join("gemini-home");
    fs::create_dir_all(agy_dir.join("config")).unwrap();
    fs::create_dir_all(agy_dir.join("antigravity-cli")).unwrap();
    let agy_hooks = agy_dir.join("config").join("hooks.json");
    let agy_settings = agy_dir.join("antigravity-cli").join("settings.json");
    fs::write(&agy_hooks, AGY_HOOKS_FIXTURE).unwrap();
    fs::write(&agy_settings, AGY_SETTINGS_FIXTURE).unwrap();
    let agy_acc = add_account(&mgr, "agy", "Agy", &agy_dir).await;

    let r = dispatch(
        &mgr,
        req(METHOD_ACCOUNT_INSTALL_HOOKS, json!({ "id": agy_acc.id })),
    )
    .await;
    assert!(r.error.is_none(), "install agy falló: {:?}", r.error);
    let r = dispatch(
        &mgr,
        req(METHOD_ACCOUNT_UNINSTALL_HOOKS, json!({ "id": agy_acc.id })),
    )
    .await;
    assert!(r.error.is_none(), "uninstall agy falló: {:?}", r.error);

    assert_eq!(
        fs::read(&agy_hooks).unwrap(),
        AGY_HOOKS_FIXTURE.as_bytes(),
        "agy hooks.json debe quedar idéntico byte a byte tras quitar hooks"
    );
    assert_eq!(
        fs::read(&agy_settings).unwrap(),
        AGY_SETTINGS_FIXTURE.as_bytes(),
        "agy settings.json debe quedar idéntico byte a byte tras quitar hooks"
    );

    // 4. Si el usuario editó el archivo entre instalar y quitar, conserva sus ediciones y quita solo pas.
    let r = dispatch(
        &mgr,
        req(METHOD_ACCOUNT_INSTALL_HOOKS, json!({ "id": claude_acc.id })),
    )
    .await;
    assert!(r.error.is_none());

    let mut edited: Value =
        serde_json::from_slice(&fs::read(&claude_settings).unwrap()).unwrap();
    edited
        .as_object_mut()
        .unwrap()
        .insert("userCustomKey".into(), json!("mantener"));
    fs::write(&claude_settings, serde_json::to_string_pretty(&edited).unwrap()).unwrap();

    let r = dispatch(
        &mgr,
        req(METHOD_ACCOUNT_UNINSTALL_HOOKS, json!({ "id": claude_acc.id })),
    )
    .await;
    assert!(r.error.is_none());

    let after_edit: Value =
        serde_json::from_slice(&fs::read(&claude_settings).unwrap()).unwrap();
    assert_eq!(after_edit["userCustomKey"], "mantener");
    assert_eq!(
        after_edit["statusLine"]["command"],
        "bash /fake/claude/scripts/statusline.sh"
    );
    let rendered = after_edit.to_string();
    assert!(
        !rendered.contains(" hook claude "),
        "no deben quedar hooks de pas tras quitar con archivo editado: {rendered}"
    );
    assert!(
        rendered.contains("herdr-agent-state.sh"),
        "debe conservar hooks de herdr"
    );
    assert!(
        rendered.contains("ORCA_PANE_KEY"),
        "debe conservar hooks de orca"
    );
}

#[tokio::test]
async fn install_keeps_foreign_hooks() {
    let tmp = tempdir().unwrap();
    let db = tmp.path().join("data").join("pas.db");
    let mgr = Manager::with_store(200, Store::open(&db).unwrap());

    // Claude
    let claude_dir = tmp.path().join("claude-foreign");
    fs::create_dir_all(&claude_dir).unwrap();
    let claude_settings = claude_dir.join("settings.json");
    fs::write(&claude_settings, CLAUDE_SETTINGS_FIXTURE).unwrap();
    let claude_acc = add_account(&mgr, "claude", "Claude", &claude_dir).await;

    let r = dispatch(
        &mgr,
        req(METHOD_ACCOUNT_INSTALL_HOOKS, json!({ "id": claude_acc.id })),
    )
    .await;
    assert!(r.error.is_none(), "install claude falló: {:?}", r.error);

    let claude_backups = list_backups(&claude_dir, "settings.json");
    assert_eq!(claude_backups.len(), 1, "debe crear backup *.bak-pas-AAAAMMDD");
    assert_eq!(
        fs::read(&claude_backups[0]).unwrap(),
        CLAUDE_SETTINGS_FIXTURE.as_bytes()
    );

    let val: Value = serde_json::from_slice(&fs::read(&claude_settings).unwrap()).unwrap();
    assert_eq!(val["theme"], "dark");
    assert_eq!(val["language"], "es");
    assert_eq!(val["statusLine"]["padding"], 0);
    let sl_cmd = val["statusLine"]["command"].as_str().unwrap();
    assert!(
        sl_cmd.contains("statusline --agent claude -- bash /fake/claude/scripts/statusline.sh"),
        "statusLine debe envolver el comando original: {sl_cmd}"
    );
    // Hooks extranjeros siguen intactos
    let session_start = val["hooks"]["SessionStart"].to_string();
    assert!(session_start.contains("herdr-agent-state.sh"));
    let ups = val["hooks"]["UserPromptSubmit"].as_array().unwrap();
    assert_eq!(ups.len(), 2, "debe mantener orca y sumar pas en UserPromptSubmit");
    assert!(ups[0].to_string().contains("ORCA_PANE_KEY"));
    assert!(ups[1].to_string().contains(" hook claude UserPromptSubmit"));
    for ev in [
        "UserPromptSubmit",
        "PreToolUse",
        "PermissionRequest",
        "Notification",
        "Stop",
    ] {
        let s = val["hooks"][ev].to_string();
        assert!(
            s.contains(&format!(" hook claude {ev}")),
            "falta hook de pas para Claude {ev}: {s}"
        );
    }

    // Codex
    let codex_dir = tmp.path().join("codex-foreign");
    fs::create_dir_all(&codex_dir).unwrap();
    let codex_hooks = codex_dir.join("hooks.json");
    fs::write(&codex_hooks, CODEX_HOOKS_FIXTURE).unwrap();
    let codex_acc = add_account(&mgr, "codex", "Codex", &codex_dir).await;

    let r = dispatch(
        &mgr,
        req(METHOD_ACCOUNT_INSTALL_HOOKS, json!({ "id": codex_acc.id })),
    )
    .await;
    assert!(r.error.is_none(), "install codex falló: {:?}", r.error);
    assert_eq!(list_backups(&codex_dir, "hooks.json").len(), 1);

    let cval: Value = serde_json::from_slice(&fs::read(&codex_hooks).unwrap()).unwrap();
    assert!(cval["hooks"]["SessionStart"].to_string().contains("orca-agent-hook-form=2"));
    assert_eq!(cval["hooks"]["UserPromptSubmit"].as_array().unwrap().len(), 2);
    assert!(
        cval["hooks"]["PermissionRequest"]
            .to_string()
            .contains(" hook codex permission_request")
    );
    assert!(cval["hooks"]["Stop"].to_string().contains(" hook codex stop"));

    // agy
    let agy_dir = tmp.path().join("agy-foreign");
    fs::create_dir_all(agy_dir.join("config")).unwrap();
    fs::create_dir_all(agy_dir.join("antigravity-cli")).unwrap();
    let agy_hooks = agy_dir.join("config").join("hooks.json");
    let agy_settings = agy_dir.join("antigravity-cli").join("settings.json");
    fs::write(&agy_hooks, AGY_HOOKS_FIXTURE).unwrap();
    fs::write(&agy_settings, AGY_SETTINGS_FIXTURE).unwrap();
    let agy_acc = add_account(&mgr, "agy", "Agy", &agy_dir).await;

    let r = dispatch(
        &mgr,
        req(METHOD_ACCOUNT_INSTALL_HOOKS, json!({ "id": agy_acc.id })),
    )
    .await;
    assert!(r.error.is_none(), "install agy falló: {:?}", r.error);
    assert_eq!(list_backups(&agy_dir.join("config"), "hooks.json").len(), 1);
    assert_eq!(
        list_backups(&agy_dir.join("antigravity-cli"), "settings.json").len(),
        1
    );

    let aval: Value = serde_json::from_slice(&fs::read(&agy_hooks).unwrap()).unwrap();
    assert!(aval["herdr"].is_object(), "grupo herdr debe seguir intacto");
    assert!(
        aval["orca-status"].is_object(),
        "grupo orca-status debe seguir intacto"
    );
    assert!(aval["pas"].is_object(), "debe existir grupo pas");
    for ev in ["PreInvocation", "PostInvocation", "Stop", "PreToolUse"] {
        assert!(
            aval["pas"][ev]
                .to_string()
                .contains(&format!(" hook agy {ev}")),
            "falta evento {ev} en grupo pas de agy"
        );
    }

    let asettings: Value = serde_json::from_slice(&fs::read(&agy_settings).unwrap()).unwrap();
    let agy_sl = asettings["statusLine"]["command"].as_str().unwrap();
    assert!(
        agy_sl.contains("statusline --agent agy -- bash /fake/gemini/scripts/statusline.sh"),
        "statusLine de agy debe quedar envuelto: {agy_sl}"
    );
    assert_eq!(asettings["statusLine"]["stack_with_default"], true);

    // account.list refleja hooks_installed = true en las 3 cuentas
    let list: Vec<AccountInfo> = serde_json::from_value(
        dispatch(&mgr, req(METHOD_ACCOUNT_LIST, json!({})))
            .await
            .result
            .unwrap(),
    )
    .unwrap();
    assert_eq!(list.len(), 3);
    assert!(list.iter().all(|a| a.hooks_installed));
}

#[tokio::test]
async fn install_is_idempotent() {
    let tmp = tempdir().unwrap();
    let db = tmp.path().join("data").join("pas.db");
    let mgr = Manager::with_store(200, Store::open(&db).unwrap());

    let claude_dir = tmp.path().join("claude-idem");
    fs::create_dir_all(&claude_dir).unwrap();
    let claude_settings = claude_dir.join("settings.json");
    fs::write(&claude_settings, CLAUDE_SETTINGS_FIXTURE).unwrap();
    let claude_acc = add_account(&mgr, "claude", "Claude", &claude_dir).await;

    let r1 = dispatch(
        &mgr,
        req(METHOD_ACCOUNT_INSTALL_HOOKS, json!({ "id": claude_acc.id })),
    )
    .await;
    assert!(r1.error.is_none());
    let after_first = fs::read(&claude_settings).unwrap();
    let backups_after_first = list_backups(&claude_dir, "settings.json");
    assert_eq!(backups_after_first.len(), 1);

    let r2 = dispatch(
        &mgr,
        req(METHOD_ACCOUNT_INSTALL_HOOKS, json!({ "id": claude_acc.id })),
    )
    .await;
    assert!(r2.error.is_none());
    let after_second = fs::read(&claude_settings).unwrap();
    let backups_after_second = list_backups(&claude_dir, "settings.json");

    assert_eq!(
        after_first, after_second,
        "instalar dos veces no debe cambiar el archivo ni duplicar entradas"
    );
    assert_eq!(
        backups_after_first, backups_after_second,
        "instalar dos veces no debe crear backups duplicados"
    );

    let val: Value = serde_json::from_slice(&after_second).unwrap();
    assert_eq!(val["hooks"]["UserPromptSubmit"].as_array().unwrap().len(), 2);
    let sl = val["statusLine"]["command"].as_str().unwrap();
    assert_eq!(
        sl.matches("statusline --agent claude --").count(),
        1,
        "no debe envolver statusLine dos veces: {sl}"
    );

    // Y quitar después de dos instalaciones sigue devolviendo el archivo original byte a byte.
    let r3 = dispatch(
        &mgr,
        req(METHOD_ACCOUNT_UNINSTALL_HOOKS, json!({ "id": claude_acc.id })),
    )
    .await;
    assert!(r3.error.is_none());
    assert_eq!(
        fs::read(&claude_settings).unwrap(),
        CLAUDE_SETTINGS_FIXTURE.as_bytes()
    );
}

#[tokio::test]
async fn invalid_json_untouched() {
    let tmp = tempdir().unwrap();
    let db = tmp.path().join("data").join("pas.db");
    let mgr = Manager::with_store(200, Store::open(&db).unwrap());

    let broken = b"{ \"hooks\": [ roto sin cerrar ";

    // Claude settings.json roto
    let claude_dir = tmp.path().join("claude-broken");
    fs::create_dir_all(&claude_dir).unwrap();
    let claude_settings = claude_dir.join("settings.json");
    fs::write(&claude_settings, broken).unwrap();
    let claude_acc = add_account(&mgr, "claude", "ClaudeRoto", &claude_dir).await;

    let r = dispatch(
        &mgr,
        req(METHOD_ACCOUNT_INSTALL_HOOKS, json!({ "id": claude_acc.id })),
    )
    .await;
    let err = r.error.expect("debe fallar con JSON inválido");
    assert_eq!(err.code, ERR_HOOKS_CONFIG_INVALID);
    assert!(
        err.message.contains("no es JSON válido")
            && err.message.contains("Corregilo o restauralo desde su backup"),
        "mensaje esperado en voseo: {}",
        err.message
    );
    assert_eq!(
        fs::read(&claude_settings).unwrap(),
        broken,
        "el archivo con JSON inválido no debe tocarse"
    );
    assert!(
        list_backups(&claude_dir, "settings.json").is_empty(),
        "no debe crear backup si el JSON es inválido"
    );

    // agy: hooks.json válido pero settings.json roto -> ninguno de los dos se toca
    let agy_dir = tmp.path().join("agy-broken-settings");
    fs::create_dir_all(agy_dir.join("config")).unwrap();
    fs::create_dir_all(agy_dir.join("antigravity-cli")).unwrap();
    let agy_hooks = agy_dir.join("config").join("hooks.json");
    let agy_settings = agy_dir.join("antigravity-cli").join("settings.json");
    fs::write(&agy_hooks, AGY_HOOKS_FIXTURE).unwrap();
    fs::write(&agy_settings, broken).unwrap();
    let agy_acc = add_account(&mgr, "agy", "AgyRoto", &agy_dir).await;

    let r = dispatch(
        &mgr,
        req(METHOD_ACCOUNT_INSTALL_HOOKS, json!({ "id": agy_acc.id })),
    )
    .await;
    let err = r.error.expect("debe fallar si settings.json de agy es inválido");
    assert_eq!(err.code, ERR_HOOKS_CONFIG_INVALID);
    assert_eq!(
        fs::read(&agy_hooks).unwrap(),
        AGY_HOOKS_FIXTURE.as_bytes(),
        "hooks.json de agy no debe tocarse si settings.json es inválido"
    );
    assert_eq!(fs::read(&agy_settings).unwrap(), broken);
}

#[tokio::test]
async fn statusline_wrapper_output_identical() {
    let tmp = tempdir().unwrap();
    let script = tmp.path().join("fake-statusline.sh");
    fs::write(
        &script,
        "#!/bin/sh\ninput=$(cat)\nprintf '\\033[32mok\\033[0m|%s' \"$input\"\n",
    )
    .unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();

    let payload = br#"{"model":{"display_name":"Sonnet"},"rate_limits":{"five_hour":{"used_percentage":42}}}"#;

    // 1. Ejecución directa del script original
    let direct = {
        let mut child = Command::new("sh")
            .arg(&script)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(payload).unwrap();
        child.wait_with_output().unwrap()
    };
    assert!(direct.status.success());

    // 2. Ejecución envuelta con pasd activo
    let sock = tmp.path().join("pasd.sock");
    let listener = bind(&sock).unwrap();
    let mgr = Manager::new(200);
    let server_task = tokio::spawn(serve(listener, mgr.clone()));

    let script_c = script.clone();
    let sock_c = sock.clone();
    let wrapped_ok = tokio::task::spawn_blocking(move || {
        let mut child = Command::new(pas_bin())
            .args(["statusline", "--agent", "claude", "--", "sh"])
            .arg(&script_c)
            .env("PAS_SOCKET", &sock_c)
            .env("PAS_SESSION_ID", "sess-test")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(payload).unwrap();
        child.wait_with_output().unwrap()
    })
    .await
    .unwrap();

    assert_eq!(wrapped_ok.status.code(), direct.status.code());
    assert_eq!(
        wrapped_ok.stdout, direct.stdout,
        "stdout del wrapper debe ser idéntico byte a byte al original"
    );
    assert_eq!(wrapped_ok.stderr, direct.stderr);
    server_task.abort();

    // 3. Con pasd caído (socket inexistente): salida idéntica y < 50 ms
    let missing_sock = tmp.path().join("no-pasd.sock");
    let script_c = script.clone();
    let (wrapped_down, elapsed_down) = tokio::task::spawn_blocking(move || {
        let t0 = Instant::now();
        let mut child = Command::new(pas_bin())
            .args(["statusline", "--agent", "claude", "--", "sh"])
            .arg(&script_c)
            .env("PAS_SOCKET", &missing_sock)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(payload).unwrap();
        let out = child.wait_with_output().unwrap();
        (out, t0.elapsed())
    })
    .await
    .unwrap();

    assert_eq!(wrapped_down.status.code(), direct.status.code());
    assert_eq!(wrapped_down.stdout, direct.stdout);
    assert!(
        elapsed_down < Duration::from_millis(50),
        "con pasd caído no debe demorar más de 50 ms, tardó {elapsed_down:?}"
    );

    // 4. Con pasd colgado (acepta conexión pero nunca responde): salida idéntica y <= 50 ms
    let hung_sock = tmp.path().join("hung.sock");
    let hung_listener = UnixListener::bind(&hung_sock).unwrap();
    let hung_task = tokio::spawn(async move {
        if let Ok((_stream, _)) = hung_listener.accept().await {
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    });

    let script_c = script.clone();
    let hung_sock_c = hung_sock.clone();
    let (wrapped_hung, elapsed_hung) = tokio::task::spawn_blocking(move || {
        let t0 = Instant::now();
        let mut child = Command::new(pas_bin())
            .args(["statusline", "--agent", "agy", "--", "sh"])
            .arg(&script_c)
            .env("PAS_SOCKET", &hung_sock_c)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(payload).unwrap();
        let out = child.wait_with_output().unwrap();
        (out, t0.elapsed())
    })
    .await
    .unwrap();

    assert_eq!(wrapped_hung.status.code(), direct.status.code());
    assert_eq!(wrapped_hung.stdout, direct.stdout);
    assert!(
        elapsed_hung <= Duration::from_millis(50),
        "con pasd colgado no debe demorar más de 50 ms, tardó {elapsed_hung:?}"
    );

    hung_task.abort();
}

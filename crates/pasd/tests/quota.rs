use std::fs;
use std::time::Duration;

use pas_core::protocol::{
    AccountInfo, AccountQuota, EVENT_QUOTA_CHANGED, EVENT_QUOTA_WARNING, METHOD_ACCOUNT_ADD,
    METHOD_QUOTA_LIST, METHOD_QUOTA_REPORT, Notification, QuotaChangedEvent, QuotaWarningEvent,
    QuotaWindowKind, Request,
};
use pasd::manager::Manager;
use pasd::rpc::dispatch;
use pasd::store::Store;
use serde_json::{Value, json};
use tempfile::tempdir;
use tokio::sync::broadcast;

fn req(method: &str, params: Value) -> Request {
    Request {
        jsonrpc: "2.0".into(),
        id: 1,
        method: method.into(),
        params,
    }
}

fn drain_events(rx: &mut broadcast::Receiver<Notification>) -> Vec<Notification> {
    let mut out = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        out.push(ev);
    }
    out
}

async fn add_account(
    mgr: &Manager,
    agent: &str,
    name: &str,
    config_dir: Option<&std::path::Path>,
) -> AccountInfo {
    let params = match config_dir {
        Some(dir) => json!({ "agent": agent, "name": name, "config_dir": dir }),
        None => json!({ "agent": agent, "name": name }),
    };
    let resp = dispatch(mgr, req(METHOD_ACCOUNT_ADD, params)).await;
    assert!(
        resp.error.is_none(),
        "error creando cuenta: {:?}",
        resp.error
    );
    serde_json::from_value(resp.result.unwrap()).unwrap()
}

async fn list_quotas(mgr: &Manager) -> Vec<AccountQuota> {
    let resp = dispatch(mgr, req(METHOD_QUOTA_LIST, json!({}))).await;
    assert!(
        resp.error.is_none(),
        "error en quota.list: {:?}",
        resp.error
    );
    serde_json::from_value(resp.result.unwrap()).unwrap()
}

#[tokio::test]
async fn codex_rollout_parsed() {
    let tmp = tempdir().unwrap();
    let db = tmp.path().join("data").join("pas.db");
    let codex_home = tmp.path().join("fake-codex");

    // Rollout viejo (día anterior)
    let old_dir = codex_home.join("sessions/2026/11/21");
    fs::create_dir_all(&old_dir).unwrap();
    fs::write(
        old_dir.join("rollout-2026-11-21T10-00-00.jsonl"),
        concat!(
            r#"{"type":"event_msg","payload":{"type":"token_count","rate_limits":{"primary":{"used_percent":99.0,"window_minutes":300,"resets_at":1795200000}}}}"#,
            "\n"
        ),
    )
    .unwrap();

    // Asegurar mtime distinto en sistemas con resolución gruesa
    std::thread::sleep(Duration::from_millis(20));

    // Rollout más reciente con varias líneas: una sin rate_limits, una vieja, la última válida y otra sin rate_limits al final
    let new_dir = codex_home.join("sessions/2026/11/22");
    fs::create_dir_all(&new_dir).unwrap();
    fs::write(
        new_dir.join("rollout-2026-11-22T12-00-00.jsonl"),
        concat!(
            r#"{"type":"session_meta","payload":{"id":"abc"}}"#, "\n",
            r#"{"type":"event_msg","payload":{"type":"token_count","rate_limits":{"primary":{"used_percent":12.0,"window_minutes":300,"resets_at":1795370000},"secondary":{"used_percent":5.0,"window_minutes":10080,"resets_at":1795900000}}}}"#, "\n",
            r#"{"type":"event_msg","payload":{"type":"token_count","rate_limits":{"primary":{"used_percent":42.5,"window_minutes":300,"resets_at":1795370000},"secondary":{"used_percent":18.0,"window_minutes":10080,"resets_at":1795900000}}}}"#, "\n",
            r#"{"type":"event_msg","payload":{"type":"agent_message","message":"listo"}}"#, "\n",
        ),
    )
    .unwrap();

    let mgr = Manager::with_store(200, Store::open(&db).unwrap());
    let acc = add_account(&mgr, "codex", "Trabajo", Some(&codex_home)).await;
    let mut rx = mgr.subscribe_events();

    mgr.poll_codex_quotas();

    let list = list_quotas(&mgr).await;
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].account_id, acc.id);
    let q = list[0]
        .quota
        .as_ref()
        .expect("debería haber cuota de Codex");
    let five_h = q.five_h.as_ref().expect("debería tener ventana 5h");
    let week = q.week.as_ref().expect("debería tener ventana 7d");
    assert!((five_h.used_percent - 42.5).abs() < 0.01);
    assert_eq!(five_h.resets_at, 1795370000);
    assert!((week.used_percent - 18.0).abs() < 0.01);
    assert_eq!(week.resets_at, 1795900000);
    assert!(q.updated_at > 0);

    let events = drain_events(&mut rx);
    let changed: Vec<&Notification> = events
        .iter()
        .filter(|e| e.method == EVENT_QUOTA_CHANGED)
        .collect();
    assert_eq!(changed.len(), 1);
    let payload: QuotaChangedEvent = serde_json::from_value(changed[0].params.clone()).unwrap();
    assert_eq!(payload.account_id, acc.id);
    assert_eq!(payload.quota, *q);

    // Un segundo poll sin cambios no vuelve a emitir quota.changed.
    mgr.poll_codex_quotas();
    let events2 = drain_events(&mut rx);
    assert!(
        events2.iter().all(|e| e.method != EVENT_QUOTA_CHANGED),
        "no debería emitir quota.changed si la cuota no cambió"
    );
}

#[tokio::test]
async fn claude_report_parsed() {
    let tmp = tempdir().unwrap();
    let db = tmp.path().join("data").join("pas.db");
    let mgr = Manager::with_store(200, Store::open(&db).unwrap());
    let acc = add_account(&mgr, "claude", "Personal", None).await;
    let mut rx = mgr.subscribe_events();

    let statusline_json = json!({
        "session_id": "sess-1",
        "model": { "id": "claude-opus-4-6", "display_name": "Opus 4.6" },
        "context_window": { "used_percentage": 24.0 },
        "rate_limits": {
            "five_hour": {
                "used_percentage": 37.5,
                "resets_at": 1795365000
            },
            "seven_day": {
                "used_percentage": 61.2,
                "resets_at": 1795900000
            }
        }
    });

    let r = dispatch(
        &mgr,
        req(
            METHOD_QUOTA_REPORT,
            json!({
                "agent": "claude",
                "account_id": acc.id,
                "payload": statusline_json,
            }),
        ),
    )
    .await;
    assert!(r.error.is_none(), "error en quota.report: {:?}", r.error);

    let list = list_quotas(&mgr).await;
    assert_eq!(list.len(), 1);
    let q = list[0]
        .quota
        .as_ref()
        .expect("debería haber cuota de Claude");
    let five_h = q.five_h.as_ref().expect("ventana 5h");
    let week = q.week.as_ref().expect("ventana 7d");
    assert!((five_h.used_percent - 37.5).abs() < 0.01);
    assert_eq!(five_h.resets_at, 1795365000);
    assert!((week.used_percent - 61.2).abs() < 0.01);
    assert_eq!(week.resets_at, 1795900000);

    let events = drain_events(&mut rx);
    assert!(
        events.iter().any(|e| e.method == EVENT_QUOTA_CHANGED),
        "debería emitir quota.changed"
    );
}

#[tokio::test]
async fn agy_report_parsed() {
    let tmp = tempdir().unwrap();
    let db = tmp.path().join("data").join("pas.db");
    let agy_dir = tmp.path().join("fake-agy");
    fs::create_dir_all(&agy_dir).unwrap();
    let mgr = Manager::with_store(200, Store::open(&db).unwrap());
    let acc = add_account(&mgr, "agy", "Predeterminada", Some(&agy_dir)).await;
    let mut rx = mgr.subscribe_events();

    let statusline_json = json!({
        "model": { "display_name": "Gemini 3.8 Flash (High)" },
        "context_window": { "context_window_size": 1000000, "used_percentage": 15.0 },
        "quota": {
            "gemini-5h": {
                "remaining_fraction": 0.65,
                "reset_in_seconds": 7200
            },
            "gemini-weekly": {
                "remaining_fraction": 0.80,
                "reset_in_seconds": 259200
            }
        }
    });

    // Sin account_id ni session_id: al haber una sola cuenta de agy se atribuye a esa cuenta.
    let r = dispatch(
        &mgr,
        req(
            METHOD_QUOTA_REPORT,
            json!({
                "agent": "agy",
                "payload": statusline_json,
            }),
        ),
    )
    .await;
    assert!(r.error.is_none(), "error en quota.report: {:?}", r.error);

    let list = list_quotas(&mgr).await;
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].account_id, acc.id);
    let q = list[0].quota.as_ref().expect("debería haber cuota de agy");
    let five_h = q.five_h.as_ref().expect("ventana 5h");
    let week = q.week.as_ref().expect("ventana 7d");
    assert!((five_h.used_percent - 35.0).abs() < 0.01);
    assert!((five_h.resets_at - (q.updated_at + 7200)).abs() <= 2);
    assert!((week.used_percent - 20.0).abs() < 0.01);
    assert!((week.resets_at - (q.updated_at + 259200)).abs() <= 2);

    let events = drain_events(&mut rx);
    assert!(events.iter().any(|e| e.method == EVENT_QUOTA_CHANGED));
}

#[tokio::test]
async fn missing_data_is_none() {
    let tmp = tempdir().unwrap();
    let db = tmp.path().join("data").join("pas.db");
    let codex_home = tmp.path().join("empty-codex");
    fs::create_dir_all(codex_home.join("sessions/2026/11/22")).unwrap();
    fs::write(
        codex_home.join("sessions/2026/11/22/rollout-empty.jsonl"),
        "{\"type\":\"session_meta\"}\n",
    )
    .unwrap();
    let agy_dir = tmp.path().join("empty-agy");
    fs::create_dir_all(&agy_dir).unwrap();

    let mgr = Manager::with_store(200, Store::open(&db).unwrap());
    let claude_acc = add_account(&mgr, "claude", "Claude", None).await;
    let codex_acc = add_account(&mgr, "codex", "Codex", Some(&codex_home)).await;
    let agy_acc = add_account(&mgr, "agy", "Agy", Some(&agy_dir)).await;

    mgr.poll_codex_quotas();

    // Reportes sin datos de cuota no inventan ceros.
    let _ = dispatch(
        &mgr,
        req(
            METHOD_QUOTA_REPORT,
            json!({
                "agent": "claude",
                "account_id": claude_acc.id,
                "payload": { "model": { "display_name": "Sonnet" } }
            }),
        ),
    )
    .await;
    let _ = dispatch(
        &mgr,
        req(
            METHOD_QUOTA_REPORT,
            json!({
                "agent": "agy",
                "account_id": agy_acc.id,
                "payload": { "model": { "display_name": "Gemini" }, "quota": {} }
            }),
        ),
    )
    .await;

    let list = list_quotas(&mgr).await;
    assert_eq!(list.len(), 3);
    for item in &list {
        assert!(
            item.quota.is_none(),
            "cuenta {} sin datos debería ser None, llegó {:?}",
            item.account_id,
            item.quota
        );
    }
    assert!(list.iter().any(|q| q.account_id == codex_acc.id));

    // Reporte parcial: solo 5h presente, 7d queda None.
    let _ = dispatch(
        &mgr,
        req(
            METHOD_QUOTA_REPORT,
            json!({
                "agent": "claude",
                "account_id": claude_acc.id,
                "payload": {
                    "rate_limits": {
                        "five_hour": { "used_percentage": 15.0, "resets_at": 1795370000 }
                    }
                }
            }),
        ),
    )
    .await;
    let list = list_quotas(&mgr).await;
    let claude_q = list
        .iter()
        .find(|q| q.account_id == claude_acc.id)
        .unwrap()
        .quota
        .as_ref()
        .expect("debería tener cuota parcial");
    assert!(claude_q.five_h.is_some());
    assert!(claude_q.week.is_none());
}

#[tokio::test]
async fn warning_fires_once_per_window() {
    let tmp = tempdir().unwrap();
    let db = tmp.path().join("data").join("pas.db");
    let mgr = Manager::with_store(200, Store::open(&db).unwrap());
    let acc = add_account(&mgr, "claude", "Personal", None).await;
    let mut rx = mgr.subscribe_events();

    let send_5h = |used: f32, resets_at: i64| {
        req(
            METHOD_QUOTA_REPORT,
            json!({
                "agent": "claude",
                "account_id": acc.id,
                "payload": {
                    "rate_limits": {
                        "five_hour": { "used_percentage": used, "resets_at": resets_at }
                    }
                }
            }),
        )
    };

    // Debajo del umbral (80 %): sin aviso.
    let _ = dispatch(&mgr, send_5h(75.0, 1795370000)).await;
    let warnings: Vec<Notification> = drain_events(&mut rx)
        .into_iter()
        .filter(|e| e.method == EVENT_QUOTA_WARNING)
        .collect();
    assert!(warnings.is_empty());

    // Cruza 80 %: dispara un aviso.
    let _ = dispatch(&mgr, send_5h(82.0, 1795370000)).await;
    let warnings: Vec<Notification> = drain_events(&mut rx)
        .into_iter()
        .filter(|e| e.method == EVENT_QUOTA_WARNING)
        .collect();
    assert_eq!(warnings.len(), 1);
    let ev: QuotaWarningEvent = serde_json::from_value(warnings[0].params.clone()).unwrap();
    assert_eq!(ev.account_id, acc.id);
    assert_eq!(ev.window, QuotaWindowKind::FiveH);
    assert!((ev.used_percent - 82.0).abs() < 0.01);
    assert_eq!(ev.resets_at, 1795370000);
    assert!((ev.threshold_percent - 80.0).abs() < 0.01);

    // Sigue arriba en la misma ventana: no vuelve a avisar.
    let _ = dispatch(&mgr, send_5h(91.0, 1795370000)).await;
    let warnings: Vec<Notification> = drain_events(&mut rx)
        .into_iter()
        .filter(|e| e.method == EVENT_QUOTA_WARNING)
        .collect();
    assert!(
        warnings.is_empty(),
        "no debe repetir quota.warning dentro de la misma ventana"
    );

    // Nueva ventana (+5h) que vuelve a cruzar 80 %: vuelve a avisar una vez.
    let _ = dispatch(&mgr, send_5h(85.0, 1795388000)).await;
    let warnings: Vec<Notification> = drain_events(&mut rx)
        .into_iter()
        .filter(|e| e.method == EVENT_QUOTA_WARNING)
        .collect();
    assert_eq!(warnings.len(), 1);
    let ev2: QuotaWarningEvent = serde_json::from_value(warnings[0].params.clone()).unwrap();
    assert_eq!(ev2.resets_at, 1795388000);
}

#[tokio::test]
async fn report_resolves_account_from_session_id() {
    let tmp = tempdir().unwrap();
    let db = tmp.path().join("data").join("pas.db");
    let mgr = Manager::with_store(200, Store::open(&db).unwrap());
    let acc1 = add_account(&mgr, "claude", "Personal", None).await;
    let acc2 = add_account(&mgr, "claude", "Trabajo", None).await;

    // Sesión asociada a acc2.
    let create_resp = dispatch(
        &mgr,
        req(
            pas_core::protocol::METHOD_SESSION_CREATE,
            json!({
                "command": ["/bin/sh", "-c", "sleep 2"],
                "cwd": tmp.path(),
                "account_id": acc2.id,
            }),
        ),
    )
    .await;
    assert!(create_resp.error.is_none());
    let created: pas_core::protocol::CreateResult =
        serde_json::from_value(create_resp.result.unwrap()).unwrap();

    // Reporte con session_id (como manda `pas statusline`) sin account_id explícito.
    let _ = dispatch(
        &mgr,
        req(
            METHOD_QUOTA_REPORT,
            json!({
                "agent": "claude",
                "session_id": created.id,
                "payload": {
                    "rate_limits": {
                        "five_hour": { "used_percentage": 55.0, "resets_at": 1795370000 }
                    }
                }
            }),
        ),
    )
    .await;

    let list = list_quotas(&mgr).await;
    let q1 = list.iter().find(|q| q.account_id == acc1.id).unwrap();
    let q2 = list.iter().find(|q| q.account_id == acc2.id).unwrap();
    assert!(q1.quota.is_none());
    assert!(
        (q2.quota
            .as_ref()
            .unwrap()
            .five_h
            .as_ref()
            .unwrap()
            .used_percent
            - 55.0)
            .abs()
            < 0.01
    );

    let _ = dispatch(
        &mgr,
        req(
            pas_core::protocol::METHOD_SESSION_KILL,
            json!({ "id": created.id }),
        ),
    )
    .await;
}

#[test]
fn codex_reads_only_tail() {
    use pasd::quota::read_codex_quota;
    use std::io::Write;

    let tmp = tempdir().unwrap();
    let codex_home = tmp.path().join("codex-tail");
    let day_dir = codex_home.join("sessions/2026/11/22");
    fs::create_dir_all(&day_dir).unwrap();

    // Archivo con ~5 MB de relleno al principio y rate_limits al final.
    let path = day_dir.join("rollout-big.jsonl");
    {
        let mut f = fs::File::create(&path).unwrap();
        let pad_line = format!(
            "{{\"type\":\"event_msg\",\"payload\":{{\"text\":\"{}\"}}}}\n",
            "x".repeat(1000)
        );
        for _ in 0..5200 {
            f.write_all(pad_line.as_bytes()).unwrap();
        }
        writeln!(
            f,
            r#"{{"type":"event_msg","payload":{{"type":"token_count","rate_limits":{{"primary":{{"used_percent":67.5,"window_minutes":300,"resets_at":1795370000}},"secondary":{{"used_percent":22.0,"window_minutes":10080,"resets_at":1795900000}}}}}}}}"#
        )
        .unwrap();
    }

    let q = read_codex_quota(&codex_home, 1795360000)
        .expect("debería parsear rate_limits al final de un archivo de 5 MB");
    assert!((q.five_h.unwrap().used_percent - 67.5).abs() < 0.01);
    assert!((q.week.unwrap().used_percent - 22.0).abs() < 0.01);

    // Si rate_limits está antes de los 5 MB de relleno (fuera de los últimos 256 KB),
    // no se lee el archivo entero y devuelve None.
    let old_only_home = tmp.path().join("codex-head-only");
    let old_day_dir = old_only_home.join("sessions/2026/11/22");
    fs::create_dir_all(&old_day_dir).unwrap();
    {
        let mut f = fs::File::create(old_day_dir.join("rollout-head.jsonl")).unwrap();
        writeln!(
            f,
            r#"{{"type":"event_msg","payload":{{"type":"token_count","rate_limits":{{"primary":{{"used_percent":11.0,"window_minutes":300,"resets_at":1795370000}}}}}}}}"#
        )
        .unwrap();
        let pad_line = format!(
            "{{\"type\":\"event_msg\",\"payload\":{{\"text\":\"{}\"}}}}\n",
            "y".repeat(1000)
        );
        for _ in 0..5200 {
            f.write_all(pad_line.as_bytes()).unwrap();
        }
    }

    assert!(
        read_codex_quota(&old_only_home, 1795360000).is_none(),
        "solo debería leer los últimos 256 KB, no el archivo completo"
    );
}

#[test]
fn codex_skips_unchanged_file() {
    use pasd::quota::{CodexScanCache, read_codex_quota_cached};
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;

    let tmp = tempdir().unwrap();
    let codex_home = tmp.path().join("codex-cache");
    let day_dir = codex_home.join("sessions/2026/11/22");
    fs::create_dir_all(&day_dir).unwrap();
    let rollout = day_dir.join("rollout-1.jsonl");
    fs::write(
        &rollout,
        concat!(
            r#"{"type":"event_msg","payload":{"type":"token_count","rate_limits":{"primary":{"used_percent":30.0,"window_minutes":300,"resets_at":1795370000}}}}"#,
            "\n"
        ),
    )
    .unwrap();

    let mut cache = CodexScanCache::default();
    let first = read_codex_quota_cached(&codex_home, &mut cache, 1795360000);
    assert_eq!(first.files_read, 1);
    assert!(
        (first
            .quota
            .as_ref()
            .unwrap()
            .five_h
            .as_ref()
            .unwrap()
            .used_percent
            - 30.0)
            .abs()
            < 0.01
    );

    // Quitar permiso de lectura sin cambiar mtime ni tamaño: si intentara abrir el archivo fallaría.
    fs::set_permissions(&rollout, fs::Permissions::from_mode(0o000)).unwrap();
    let second = read_codex_quota_cached(&codex_home, &mut cache, 1795360060);
    fs::set_permissions(&rollout, fs::Permissions::from_mode(0o644)).unwrap();

    assert_eq!(
        second.files_read, 0,
        "no debería releer el archivo si mtime y tamaño no cambiaron"
    );
    assert!(
        (second
            .quota
            .as_ref()
            .unwrap()
            .five_h
            .as_ref()
            .unwrap()
            .used_percent
            - 30.0)
            .abs()
            < 0.01
    );

    // Al agregar una línea nueva (cambia tamaño/mtime), sí relee y actualiza.
    std::thread::sleep(Duration::from_millis(20));
    {
        let mut f = fs::OpenOptions::new().append(true).open(&rollout).unwrap();
        writeln!(
            f,
            r#"{{"type":"event_msg","payload":{{"type":"token_count","rate_limits":{{"primary":{{"used_percent":48.0,"window_minutes":300,"resets_at":1795370000}}}}}}}}"#
        )
        .unwrap();
    }
    let third = read_codex_quota_cached(&codex_home, &mut cache, 1795360120);
    assert_eq!(third.files_read, 1);
    assert!(
        (third
            .quota
            .as_ref()
            .unwrap()
            .five_h
            .as_ref()
            .unwrap()
            .used_percent
            - 48.0)
            .abs()
            < 0.01
    );
}

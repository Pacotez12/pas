use std::io::{self, Read, Write};
use std::process::{Command, ExitCode, Stdio};
use std::time::Duration;

use pas_core::CoreError;
use pas_core::client::Client;
use pas_core::protocol::{AgentKind, METHOD_QUOTA_REPORT, QuotaReportParams};
use pas_core::socket::socket_path;
use serde_json::Value;

const QUOTA_REPORT_TIMEOUT: Duration = Duration::from_millis(25);

pub async fn run(agent: &str, command: &[String]) -> ExitCode {
    let mut stdin_bytes = Vec::new();
    let _ = io::stdin().lock().read_to_end(&mut stdin_bytes);

    let payload = serde_json::from_slice::<Value>(&stdin_bytes).ok();
    let report_fut = report_quota(agent, payload);

    let cmd = command.to_vec();
    let child_fut = tokio::task::spawn_blocking(move || run_original(&cmd, &stdin_bytes));

    let ((), child_res) = tokio::join!(report_fut, child_fut);
    match child_res {
        Ok(code) => code,
        Err(_) => ExitCode::FAILURE,
    }
}

async fn report_quota(agent: &str, payload: Option<Value>) {
    let Some(payload) = payload else {
        return;
    };
    let Some(agent_kind) = AgentKind::parse(agent) else {
        return;
    };
    let Ok(path) = socket_path() else {
        return;
    };
    let session_id = std::env::var("PAS_SESSION_ID")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let params = QuotaReportParams {
        agent: agent_kind,
        session_id,
        payload,
    };

    let _ = tokio::time::timeout(QUOTA_REPORT_TIMEOUT, async move {
        let mut client = Client::connect(&path).await?;
        let _: Value = client.call(METHOD_QUOTA_REPORT, params).await?;
        Ok::<(), CoreError>(())
    })
    .await;
}

fn run_original(command: &[String], stdin_bytes: &[u8]) -> ExitCode {
    let Some((program, args)) = command.split_first() else {
        return ExitCode::FAILURE;
    };

    let mut child = match Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
    {
        Ok(c) => c,
        Err(_) => return ExitCode::FAILURE,
    };

    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(stdin_bytes);
    }

    match child.wait() {
        Ok(status) => status
            .code()
            .and_then(|c| u8::try_from(c).ok())
            .map(ExitCode::from)
            .unwrap_or(ExitCode::FAILURE),
        Err(_) => ExitCode::FAILURE,
    }
}

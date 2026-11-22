use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::v1::ConnectionTo;
use agent_client_protocol::v1::schema::{
    CancelNotification, ContentBlock, InitializeRequest, NewSessionRequest, PermissionOptionKind,
    PromptRequest, RequestPermissionOutcome, RequestPermissionRequest, RequestPermissionResponse,
    SelectedPermissionOutcome, SessionNotification, SessionUpdate, StopReason, TextContent,
    ToolCallContent, ToolCallStatus,
};
use agent_client_protocol::{AcpAgent, AcpAgentConfig, Agent, Client};

fn bin_path() -> &'static str {
    env!("CARGO_BIN_EXE_fake-acp-agent")
}

fn agent_with_script(script: &str) -> AcpAgent {
    AcpAgent::new(AcpAgentConfig::new(bin_path()).env("FAKE_ACP_SCRIPT", script))
}

#[tokio::test]
async fn fake_agent_speaks_acp() {
    let script = serde_json::json!([
        { "type": "text", "chunks": ["Hola ", "desde ", "ACP"] }
    ])
    .to_string();

    let chunks: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let chunks_cb = Arc::clone(&chunks);

    let agent = agent_with_script(&script);
    let stop_reason = Client
        .v1()
        .name("test-client")
        .on_receive_notification(
            async move |notif: SessionNotification, _cx| {
                if let SessionUpdate::AgentMessageChunk(chunk) = notif.update
                    && let ContentBlock::Text(text) = chunk.content
                {
                    chunks_cb.lock().unwrap().push(text.text);
                }
                Ok(())
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .connect_with(agent, |cx: ConnectionTo<Agent>| async move {
            let init = cx
                .send_request(InitializeRequest::new(ProtocolVersion::V1))
                .block_task()
                .await?;
            assert_eq!(init.protocol_version, ProtocolVersion::V1);

            let session = cx
                .send_request(NewSessionRequest::new(PathBuf::from("/tmp")))
                .block_task()
                .await?;
            assert!(!session.session_id.0.is_empty());

            let prompt = cx
                .send_request(PromptRequest::new(
                    session.session_id,
                    vec![ContentBlock::Text(TextContent::new("probando"))],
                ))
                .block_task()
                .await?;
            Ok(prompt.stop_reason)
        })
        .await
        .expect("ACP handshake and prompt should succeed");

    assert_eq!(stop_reason, StopReason::EndTurn);
    assert_eq!(*chunks.lock().unwrap(), vec!["Hola ", "desde ", "ACP"]);
}

#[tokio::test]
async fn script_emits_tool_diff_and_visual() {
    let tmp = tempfile::tempdir().unwrap();
    let script = serde_json::json!([
        { "type": "tool", "id": "tc-1", "title": "Leyó main.rs", "name": "read_file" },
        {
            "type": "diff",
            "id": "tc-2",
            "path": "src/lib.rs",
            "old_text": "fn a() {}\n",
            "new_text": "fn a() { 1 }\n"
        },
        {
            "type": "visual",
            "file": "mockup.html",
            "html": "<h1>Hola</h1>",
            "message": "Te dejo mockup.html"
        }
    ])
    .to_string();

    let updates: Arc<Mutex<Vec<SessionUpdate>>> = Arc::new(Mutex::new(Vec::new()));
    let updates_cb = Arc::clone(&updates);

    let agent = AcpAgent::new(
        AcpAgentConfig::new(bin_path())
            .env("FAKE_ACP_SCRIPT", &script)
            .env("PAS_CHAT_DIR", tmp.path().to_str().unwrap()),
    );

    let cwd = tmp.path().to_path_buf();
    Client
        .v1()
        .on_receive_notification(
            async move |notif: SessionNotification, _cx| {
                updates_cb.lock().unwrap().push(notif.update);
                Ok(())
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .connect_with(agent, |cx: ConnectionTo<Agent>| async move {
            cx.send_request(InitializeRequest::new(ProtocolVersion::V1))
                .block_task()
                .await?;
            let session = cx
                .send_request(NewSessionRequest::new(cwd))
                .block_task()
                .await?;
            cx.send_request(PromptRequest::new(
                session.session_id,
                vec![ContentBlock::Text(TextContent::new("go"))],
            ))
            .block_task()
            .await?;
            Ok(())
        })
        .await
        .expect("scripted tool/diff/visual should succeed");

    let updates = updates.lock().unwrap();
    assert!(
        updates.iter().any(|u| matches!(
            u,
            SessionUpdate::ToolCall(tc) if tc.tool_call_id.0.as_ref() == "tc-1" && tc.title == "Leyó main.rs"
        )),
        "expected ToolCall tc-1 in {updates:?}"
    );
    assert!(
        updates.iter().any(|u| matches!(
            u,
            SessionUpdate::ToolCallUpdate(upd)
                if upd.tool_call_id.0.as_ref() == "tc-1"
                    && upd.fields.status == Some(ToolCallStatus::Completed)
        )),
        "expected ToolCallUpdate completed for tc-1 in {updates:?}"
    );
    assert!(
        updates.iter().any(|u| matches!(
            u,
            SessionUpdate::ToolCall(tc)
                if tc.content.iter().any(|c| matches!(
                    c,
                    ToolCallContent::Diff(d)
                        if d.path == std::path::Path::new("src/lib.rs")
                            && d.old_text.as_deref() == Some("fn a() {}\n")
                            && d.new_text == "fn a() { 1 }\n"
                ))
        )),
        "expected Diff in ToolCall content: {updates:?}"
    );
    assert!(
        updates.iter().any(|u| matches!(
            u,
            SessionUpdate::AgentMessageChunk(chunk)
                if matches!(&chunk.content, ContentBlock::Text(t) if t.text.contains("mockup.html"))
        )),
        "expected visual message chunk in {updates:?}"
    );

    let visual_path = tmp.path().join("mockup.html");
    assert_eq!(std::fs::read_to_string(&visual_path).unwrap(), "<h1>Hola</h1>");
}

#[tokio::test]
async fn script_requests_permission_and_handles_choice() {
    let script = serde_json::json!([
        {
            "type": "permission",
            "id": "perm-1",
            "title": "Ejecutar cargo test",
            "name": "bash",
            "after_allow": "Permiso concedido"
        }
    ])
    .to_string();

    let seen_perm: Arc<Mutex<Option<RequestPermissionRequest>>> = Arc::new(Mutex::new(None));
    let seen_perm_cb = Arc::clone(&seen_perm);
    let chunks: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let chunks_cb = Arc::clone(&chunks);

    let agent = agent_with_script(&script);
    Client
        .v1()
        .on_receive_notification(
            async move |notif: SessionNotification, _cx| {
                if let SessionUpdate::AgentMessageChunk(chunk) = notif.update
                    && let ContentBlock::Text(text) = chunk.content
                {
                    chunks_cb.lock().unwrap().push(text.text);
                }
                Ok(())
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .on_receive_request(
            async move |req: RequestPermissionRequest, responder, _cx| {
                let allow_id = req
                    .options
                    .iter()
                    .find(|o| o.kind == PermissionOptionKind::AllowOnce)
                    .map(|o| o.option_id.clone())
                    .expect("AllowOnce option should exist");
                *seen_perm_cb.lock().unwrap() = Some(req);
                responder.respond(RequestPermissionResponse::new(
                    RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(allow_id)),
                ))
            },
            agent_client_protocol::on_receive_request!(),
        )
        .connect_with(agent, |cx: ConnectionTo<Agent>| async move {
            cx.send_request(InitializeRequest::new(ProtocolVersion::V1))
                .block_task()
                .await?;
            let session = cx
                .send_request(NewSessionRequest::new(PathBuf::from("/tmp")))
                .block_task()
                .await?;
            let resp = cx
                .send_request(PromptRequest::new(
                    session.session_id,
                    vec![ContentBlock::Text(TextContent::new("corré tests"))],
                ))
                .block_task()
                .await?;
            assert_eq!(resp.stop_reason, StopReason::EndTurn);
            Ok(())
        })
        .await
        .expect("permission flow should succeed");

    let req = seen_perm
        .lock()
        .unwrap()
        .take()
        .expect("should have received permission request");
    assert_eq!(req.tool_call.fields.title.as_deref(), Some("Ejecutar cargo test"));
    assert!(req.options.iter().any(|o| o.kind == PermissionOptionKind::AllowOnce));
    assert!(req.options.iter().any(|o| o.kind == PermissionOptionKind::AllowAlways));
    assert!(req.options.iter().any(|o| o.kind == PermissionOptionKind::RejectOnce));
    assert_eq!(*chunks.lock().unwrap(), vec!["Permiso concedido"]);
}

#[tokio::test]
async fn script_crashes_with_nonzero_exit() {
    let script = serde_json::json!([
        { "type": "crash", "code": 42, "message": "boom desde fake" }
    ])
    .to_string();

    let agent = agent_with_script(&script);
    let prompt_result = Client
        .v1()
        .connect_with(agent, |cx: ConnectionTo<Agent>| async move {
            cx.send_request(InitializeRequest::new(ProtocolVersion::V1))
                .block_task()
                .await
                .expect("initialize should succeed before crash");
            let session = cx
                .send_request(NewSessionRequest::new(PathBuf::from("/tmp")))
                .block_task()
                .await
                .expect("session/new should succeed before crash");
            let res = cx
                .send_request(PromptRequest::new(
                    session.session_id,
                    vec![ContentBlock::Text(TextContent::new("crash"))],
                ))
                .block_task()
                .await;
            Ok(res)
        })
        .await;

    if let Ok(res) = prompt_result {
        assert!(res.is_err(), "expected prompt error when agent crashes");
    }
}

#[tokio::test]
async fn script_sleep_is_cancellable() {
    let script = serde_json::json!([
        { "type": "sleep", "ms": 10000 }
    ])
    .to_string();

    let started = Instant::now();
    let agent = agent_with_script(&script);
    let stop_reason = Client
        .v1()
        .connect_with(agent, |cx: ConnectionTo<Agent>| async move {
            cx.send_request(InitializeRequest::new(ProtocolVersion::V1))
                .block_task()
                .await?;
            let session = cx
                .send_request(NewSessionRequest::new(PathBuf::from("/tmp")))
                .block_task()
                .await?;
            let session_id = session.session_id.clone();
            let prompt_req = cx.send_request(PromptRequest::new(
                session_id.clone(),
                vec![ContentBlock::Text(TextContent::new("dormí"))],
            ));
            tokio::time::sleep(Duration::from_millis(80)).await;
            cx.send_notification(CancelNotification::new(session_id))?;
            let resp = prompt_req.block_task().await?;
            Ok(resp.stop_reason)
        })
        .await
        .expect("cancelled prompt should return StopReason::Cancelled");

    assert_eq!(stop_reason, StopReason::Cancelled);
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "cancellation should interrupt sleep promptly, took {:?}",
        started.elapsed()
    );
}

use std::path::PathBuf;

use pas_core::protocol::{
    CreateParams, METHOD_SESSION_SEND, Request, Response, RpcError, SendParams, SessionState,
};
use serde_json::json;

#[test]
fn request_roundtrip() {
    let req = Request {
        jsonrpc: "2.0".into(),
        id: 1,
        method: METHOD_SESSION_SEND.into(),
        params: serde_json::to_value(SendParams {
            id: "abc".into(),
            text: "hola".into(),
            enter: true,
        })
        .unwrap(),
    };

    let serialized = serde_json::to_string(&req).unwrap();
    let deserialized: Request = serde_json::from_str(&serialized).unwrap();
    assert_eq!(deserialized, req);
}

#[test]
fn state_json_shape() {
    assert_eq!(
        serde_json::to_string(&SessionState::Running).unwrap(),
        r#"{"state":"running"}"#
    );
    assert_eq!(
        serde_json::to_string(&SessionState::Idle).unwrap(),
        r#"{"state":"idle"}"#
    );
    assert_eq!(
        serde_json::to_string(&SessionState::Exited { code: Some(0) }).unwrap(),
        r#"{"state":"exited","code":0}"#
    );
}

#[test]
fn create_params_defaults() {
    let params: CreateParams =
        serde_json::from_str(r#"{"command":["sh"],"cwd":"/tmp"}"#).unwrap();

    assert_eq!(params.command, vec!["sh"]);
    assert_eq!(params.cwd, PathBuf::from("/tmp"));
    assert_eq!(params.cols, 120);
    assert_eq!(params.rows, 40);
    assert!(params.env.is_empty());
    assert_eq!(params.title, None);
}

#[test]
fn send_params_default_enter() {
    let params: SendParams =
        serde_json::from_str(r#"{"id":"s1","text":"ls"}"#).unwrap();
    assert!(params.enter);
}

#[test]
fn response_ok_and_err() {
    let ok = Response::ok(7, json!({"version": "0.1.0"}));
    assert_eq!(ok.jsonrpc, "2.0");
    assert_eq!(ok.id, 7);
    assert!(ok.result.is_some());
    assert!(ok.error.is_none());

    let err = Response::err(
        8,
        RpcError {
            code: 1001,
            message: "no existe".into(),
        },
    );
    assert_eq!(err.jsonrpc, "2.0");
    assert_eq!(err.id, 8);
    assert!(err.result.is_none());
    assert_eq!(err.error.unwrap().code, 1001);
}

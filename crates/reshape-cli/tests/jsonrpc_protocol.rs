use reshape_cli::rpc_protocol::{JsonRpcErrorCode, RpcRequest, RpcResponse, parse_rpc_request};
use reshape_core::protocol::{
    DEFAULT_SCHEMA_VERSION, DEFAULT_SESSION_KEY, Envelope, InputEvent, InputSource, OutputEvent,
};

#[test]
fn user_text_request_converts_to_websocket_input_envelope() {
    let request = parse_rpc_request(
        r#"{
            "jsonrpc": "2.0",
            "id": "turn-1",
            "method": "reshape.input",
            "params": {
                "sessionKey": "local:main",
                "schemaVersion": "1.0",
                "metadata": {"client": "test"},
                "input": {
                    "type": "user_text",
                    "text": "create a landing page"
                }
            }
        }"#,
    )
    .unwrap();

    let envelope = request.into_input_envelope().unwrap();

    assert_eq!(envelope.header.session_key, DEFAULT_SESSION_KEY);
    assert_eq!(envelope.header.schema_version, DEFAULT_SCHEMA_VERSION);
    assert_eq!(
        envelope.payload,
        InputEvent::UserText {
            text: "create a landing page".to_string(),
            source: InputSource::WebSocket,
        }
    );
    assert_eq!(envelope.metadata["client"], "test");
    assert_eq!(envelope.metadata["jsonrpc_id"], "turn-1");
}

#[test]
fn completed_output_converts_to_stable_wire_response() {
    let envelope = Envelope::new(OutputEvent::Completed {
        summary: "Mock page generated in index.html".to_string(),
    });

    let response = RpcResponse::success("turn-1", envelope);

    assert_eq!(response.jsonrpc, "2.0");
    assert_eq!(response.id, "turn-1");
    let result = response.result.unwrap();
    assert_eq!(result["schemaVersion"], DEFAULT_SCHEMA_VERSION);
    assert_eq!(result["output"]["type"], "completed");
    assert_eq!(
        result["output"]["summary"],
        "Mock page generated in index.html"
    );
}

#[test]
fn unknown_method_returns_method_not_found_error() {
    let request = RpcRequest::from_json_value(serde_json::json!({
        "jsonrpc": "2.0",
        "id": "turn-1",
        "method": "reshape.unknown",
        "params": {}
    }))
    .unwrap();

    let error = request.into_input_envelope().unwrap_err();

    assert_eq!(error.code, JsonRpcErrorCode::MethodNotFound);
}

#[test]
fn missing_user_text_returns_invalid_params_error() {
    let request = parse_rpc_request(
        r#"{
            "jsonrpc": "2.0",
            "id": "turn-1",
            "method": "reshape.input",
            "params": {
                "input": {
                    "type": "user_text"
                }
            }
        }"#,
    )
    .unwrap();

    let error = request.into_input_envelope().unwrap_err();

    assert_eq!(error.code, JsonRpcErrorCode::InvalidParams);
    assert!(error.message.contains("input.text"));
}

#[test]
fn invalid_json_returns_parse_error_response() {
    let error = parse_rpc_request("{not-json").unwrap_err();

    assert_eq!(error.code, JsonRpcErrorCode::ParseError);
}

#[test]
fn error_response_contains_jsonrpc_error_fields() {
    let error = parse_rpc_request("{not-json").unwrap_err();
    let response = RpcResponse::error(None, error);
    let value = serde_json::to_value(response).unwrap();

    assert_eq!(value["jsonrpc"], "2.0");
    assert!(value["id"].is_null());
    assert_eq!(value["error"]["code"], -32700);
    assert!(value["error"]["message"].is_string());
    assert_eq!(value["error"]["data"]["errorCode"], "InvalidSchema");
}

#[test]
fn numeric_jsonrpc_id_is_preserved_in_success_response() {
    let request = parse_rpc_request(
        r#"{
            "jsonrpc": "2.0",
            "id": 7,
            "method": "reshape.input",
            "params": {
                "input": {
                    "type": "user_text",
                    "text": "create a page"
                }
            }
        }"#,
    )
    .unwrap();
    let id = request.id.clone();
    let envelope = Envelope::new(OutputEvent::FinalMessage {
        text: "ok".to_string(),
    });

    let response = RpcResponse::success(id, envelope);
    let value = serde_json::to_value(response).unwrap();

    assert_eq!(value["id"], 7);
}

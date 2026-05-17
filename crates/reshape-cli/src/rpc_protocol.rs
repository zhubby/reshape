use std::collections::BTreeMap;

use reshape_core::protocol::{
    DEFAULT_SCHEMA_VERSION, DEFAULT_SESSION_KEY, Envelope, ErrorCode, InputEvent, InputSource,
    OutputEvent, TurnProgressEvent, TurnProgressKind,
};
use reshape_core::session::{Session, SessionMessage};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::{Config, ExportError, TS};

type RpcResult<T> = std::result::Result<T, RpcError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsonRpcErrorCode {
    ParseError,
    InvalidRequest,
    MethodNotFound,
    InvalidParams,
    ServerError,
}

impl JsonRpcErrorCode {
    #[must_use]
    pub fn as_i64(self) -> i64 {
        match self {
            Self::ParseError => -32700,
            Self::InvalidRequest => -32600,
            Self::MethodNotFound => -32601,
            Self::InvalidParams => -32602,
            Self::ServerError => -32000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RpcError {
    pub code: JsonRpcErrorCode,
    pub message: String,
    pub error_code: ErrorCode,
}

impl RpcError {
    fn parse(message: impl Into<String>) -> Self {
        Self {
            code: JsonRpcErrorCode::ParseError,
            message: message.into(),
            error_code: ErrorCode::InvalidSchema,
        }
    }

    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self {
            code: JsonRpcErrorCode::InvalidRequest,
            message: message.into(),
            error_code: ErrorCode::InvalidSchema,
        }
    }

    fn method_not_found(method: &str) -> Self {
        Self {
            code: JsonRpcErrorCode::MethodNotFound,
            message: format!("method not found: {method}"),
            error_code: ErrorCode::ValidationFailed,
        }
    }

    fn invalid_params(message: impl Into<String>) -> Self {
        Self {
            code: JsonRpcErrorCode::InvalidParams,
            message: message.into(),
            error_code: ErrorCode::ValidationFailed,
        }
    }

    #[must_use]
    pub fn server(message: impl Into<String>) -> Self {
        Self {
            code: JsonRpcErrorCode::ServerError,
            message: message.into(),
            error_code: ErrorCode::Failed,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RpcRequest {
    pub id: Value,
    pub method: String,
    params: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
pub struct RpcResponse {
    #[ts(type = "\"2.0\"")]
    pub jsonrpc: &'static str,
    #[ts(type = "string | number | null")]
    pub id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcErrorBody>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RpcHandshake {
    #[serde(rename = "type")]
    #[ts(rename = "type", type = "\"reshape.rpc.handshake\"")]
    pub frame_type: String,
    #[ts(type = "\"1.0\"")]
    pub protocol_version: String,
    pub client: RpcClient,
    #[serde(default)]
    pub tab: RpcTabContext,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, TS)]
pub struct RpcClient {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, TS)]
pub struct RpcTabContext {
    pub id: Option<u32>,
    pub url: Option<String>,
    pub title: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RpcHandshakeAck {
    #[serde(rename = "type")]
    #[ts(rename = "type", type = "\"reshape.rpc.handshake_ack\"")]
    pub frame_type: String,
    #[ts(type = "\"1.0\"")]
    pub protocol_version: String,
    #[ts(type = "\"1.0\"")]
    pub schema_version: String,
    #[ts(type = "\"local:main\"")]
    pub session_key: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
pub struct RpcResultBody {
    #[serde(rename = "schemaVersion")]
    pub schema_version: String,
    #[serde(rename = "messageId")]
    pub message_id: String,
    #[serde(rename = "traceId")]
    pub trace_id: String,
    #[serde(rename = "sessionKey")]
    pub session_key: String,
    pub output: RpcOutput,
    #[ts(type = "Record<string, unknown>")]
    pub metadata: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
pub struct RpcProgressNotification {
    #[ts(type = "\"2.0\"")]
    pub jsonrpc: &'static str,
    #[ts(type = "\"reshape.progress\"")]
    pub method: &'static str,
    pub params: TurnProgressEvent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RpcHistoryBody {
    #[serde(rename = "schemaVersion")]
    #[ts(type = "\"1.0\"")]
    pub schema_version: String,
    #[ts(type = "\"local:main\"")]
    pub session_key: String,
    pub messages: Vec<RpcHistoryMessage>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
pub struct RpcHistoryMessage {
    pub role: RpcHistoryRole,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum RpcHistoryRole {
    User,
    Reshape,
    System,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
pub struct RpcErrorBody {
    #[ts(type = "number")]
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<RpcErrorData>,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
pub struct RpcErrorData {
    #[serde(rename = "errorCode")]
    pub error_code: ErrorCode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RpcOutput {
    FinalMessage { text: String },
    StreamChunk { text: String },
    ToolProgress { tool_name: String, message: String },
    WorkspaceFileChanged { path: String },
    Error { code: ErrorCode, message: String },
    Completed { summary: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ReshapeInputRequest {
    #[ts(type = "\"2.0\"")]
    pub jsonrpc: String,
    pub id: String,
    #[ts(type = "\"reshape.input\"")]
    pub method: String,
    pub params: ReshapeInputParams,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ReshapeInputParams {
    #[ts(type = "\"local:main\"")]
    pub session_key: String,
    #[ts(type = "\"1.0\"")]
    pub schema_version: String,
    pub metadata: RpcRequestMetadata,
    pub input: ReshapeInputPayload,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RpcRequestMetadata {
    pub client: String,
    pub tab_id: Option<u32>,
    pub url: Option<String>,
    pub title: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[serde(tag = "type")]
pub enum ReshapeInputPayload {
    UserText { text: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
pub struct RpcSuccessResponse {
    #[ts(type = "\"2.0\"")]
    pub jsonrpc: String,
    #[ts(type = "string | number | null")]
    pub id: Value,
    pub result: RpcResultBody,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
pub struct RpcWireErrorResponse {
    #[ts(type = "\"2.0\"")]
    pub jsonrpc: String,
    #[ts(type = "string | number | null")]
    pub id: Value,
    pub error: RpcErrorBody,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(untagged)]
pub enum RpcWireResponse {
    Success(RpcSuccessResponse),
    Error(RpcWireErrorResponse),
}

#[derive(Debug, Deserialize)]
struct JsonRpcRequestBody {
    jsonrpc: String,
    id: Option<Value>,
    method: Option<String>,
    #[serde(default)]
    params: Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct InputParams {
    session_key: Option<String>,
    schema_version: Option<String>,
    #[serde(default)]
    metadata: BTreeMap<String, Value>,
    input: Option<RpcInput>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum RpcInput {
    UserText { text: String },
}

pub fn parse_rpc_request(text: &str) -> RpcResult<RpcRequest> {
    tracing::debug!(bytes = text.len(), "parsing json-rpc request");
    let value = serde_json::from_str(text).map_err(|err| {
        tracing::warn!(error = %err, "failed to parse json-rpc request");
        RpcError::parse(err.to_string())
    })?;
    RpcRequest::from_json_value(value)
}

pub fn parse_rpc_handshake(text: &str) -> RpcResult<RpcHandshake> {
    tracing::debug!(bytes = text.len(), "parsing rpc handshake frame");
    let handshake: RpcHandshake = serde_json::from_str(text).map_err(|err| {
        tracing::warn!(error = %err, "invalid rpc handshake frame");
        RpcError::invalid_request(format!("rpc handshake required: {err}"))
    })?;
    if handshake.frame_type != "reshape.rpc.handshake" {
        tracing::warn!(
            frame_type = %handshake.frame_type,
            "unsupported rpc handshake frame type"
        );
        return Err(RpcError::invalid_request("rpc handshake required"));
    }
    if handshake.protocol_version != DEFAULT_SCHEMA_VERSION {
        tracing::warn!(
            protocol_version = %handshake.protocol_version,
            "unsupported rpc protocol version"
        );
        return Err(RpcError::invalid_params(format!(
            "unsupported rpc protocolVersion: {}",
            handshake.protocol_version
        )));
    }
    if handshake.client.name.trim().is_empty() || handshake.client.version.trim().is_empty() {
        tracing::warn!("rpc handshake missing client identity");
        return Err(RpcError::invalid_params(
            "rpc client name and version are required",
        ));
    }
    tracing::debug!(
        client_name = %handshake.client.name,
        client_version = %handshake.client.version,
        "rpc handshake accepted"
    );
    Ok(handshake)
}

#[must_use]
pub fn rpc_handshake_ack() -> RpcHandshakeAck {
    RpcHandshakeAck {
        frame_type: "reshape.rpc.handshake_ack".to_string(),
        protocol_version: DEFAULT_SCHEMA_VERSION.to_string(),
        schema_version: DEFAULT_SCHEMA_VERSION.to_string(),
        session_key: DEFAULT_SESSION_KEY.to_string(),
    }
}

pub fn export_ts_bindings(output: &std::path::Path) -> std::result::Result<(), ExportError> {
    let mut content =
        String::from("// This file was generated by ts-rs. Do not edit this file manually.\n\n");
    append_ts::<ErrorCode>(&mut content);
    append_ts::<RpcClient>(&mut content);
    append_ts::<RpcTabContext>(&mut content);
    append_ts::<RpcHandshake>(&mut content);
    append_ts::<RpcHandshakeAck>(&mut content);
    append_ts::<RpcRequestMetadata>(&mut content);
    append_ts::<ReshapeInputPayload>(&mut content);
    append_ts::<ReshapeInputParams>(&mut content);
    append_ts::<ReshapeInputRequest>(&mut content);
    append_ts::<RpcOutput>(&mut content);
    append_ts::<TurnProgressKind>(&mut content);
    append_ts::<TurnProgressEvent>(&mut content);
    append_ts::<RpcProgressNotification>(&mut content);
    append_ts::<RpcHistoryRole>(&mut content);
    append_ts::<RpcHistoryMessage>(&mut content);
    append_ts::<RpcHistoryBody>(&mut content);
    append_ts::<RpcErrorData>(&mut content);
    append_ts::<RpcErrorBody>(&mut content);
    append_ts::<RpcResultBody>(&mut content);
    append_ts::<RpcSuccessResponse>(&mut content);
    append_ts::<RpcWireErrorResponse>(&mut content);
    append_ts::<RpcWireResponse>(&mut content);

    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent).map_err(ExportError::Io)?;
    }
    std::fs::write(output, content).map_err(ExportError::Io)
}

fn append_ts<T: TS>(content: &mut String) {
    let config = Config::default();
    content.push_str("export ");
    content.push_str(&T::decl(&config));
    content.push_str("\n\n");
}

impl RpcRequest {
    pub fn from_json_value(value: Value) -> RpcResult<Self> {
        if !value.is_object() {
            return Err(RpcError::invalid_request("request must be a JSON object"));
        }

        let body: JsonRpcRequestBody = serde_json::from_value(value).map_err(|err| {
            tracing::warn!(error = %err, "invalid json-rpc request shape");
            RpcError::invalid_request(err.to_string())
        })?;
        if body.jsonrpc != "2.0" {
            tracing::warn!(jsonrpc = %body.jsonrpc, "unsupported json-rpc version");
            return Err(RpcError::invalid_request("jsonrpc must be 2.0"));
        }

        let id = body
            .id
            .ok_or_else(|| RpcError::invalid_request("id is required"))?;
        if !is_valid_id(&id) {
            tracing::warn!(id = %id, "invalid json-rpc id type");
            return Err(RpcError::invalid_request(
                "id must be a string, number, or null",
            ));
        }
        let method = body
            .method
            .ok_or_else(|| RpcError::invalid_request("method is required"))?;
        tracing::debug!(method = %method, id = %id, "json-rpc request decoded");

        Ok(Self {
            id,
            method,
            params: body.params,
        })
    }

    pub fn into_input_envelope(self) -> RpcResult<Envelope<InputEvent>> {
        if self.method != "reshape.input" {
            tracing::warn!(method = %self.method, "json-rpc method not found");
            return Err(RpcError::method_not_found(&self.method));
        }

        let params: InputParams = serde_json::from_value(self.params).map_err(|err| {
            let message = input_params_error(err);
            tracing::warn!(reason = %message, "invalid reshape.input params");
            RpcError::invalid_params(message)
        })?;
        let session_key = params
            .session_key
            .unwrap_or_else(|| DEFAULT_SESSION_KEY.to_string());
        if session_key != DEFAULT_SESSION_KEY {
            tracing::warn!(session_key, "unsupported json-rpc session key");
            return Err(RpcError::invalid_params(format!(
                "unsupported sessionKey: {session_key}"
            )));
        }

        if let Some(schema_version) = params.schema_version
            && schema_version != DEFAULT_SCHEMA_VERSION
        {
            tracing::warn!(schema_version, "unsupported json-rpc schema version");
            return Err(RpcError::invalid_params(format!(
                "unsupported schemaVersion: {schema_version}"
            )));
        }

        let input = params
            .input
            .ok_or_else(|| RpcError::invalid_params("input is required"))?;
        let event = input.into_input_event();
        let mut envelope = Envelope::for_session(session_key, event);
        envelope.metadata = params.metadata;
        envelope
            .metadata
            .insert("jsonrpc_id".to_string(), metadata_id(&self.id));
        tracing::debug!(
            message_id = %envelope.header.message_id,
            trace_id = %envelope.header.trace_id,
            "json-rpc request converted to input envelope"
        );
        Ok(envelope)
    }
}

impl RpcResponse {
    #[must_use]
    pub fn success(id: impl Into<Value>, envelope: Envelope<OutputEvent>) -> Self {
        let result = RpcResultBody {
            schema_version: envelope.header.schema_version,
            message_id: envelope.header.message_id.to_string(),
            trace_id: envelope.header.trace_id.to_string(),
            session_key: envelope.header.session_key,
            output: RpcOutput::from(envelope.payload),
            metadata: envelope.metadata,
        };
        Self {
            jsonrpc: "2.0",
            id: id.into(),
            result: serde_json::to_value(result).ok(),
            error: None,
        }
    }

    #[must_use]
    pub fn raw_success(id: impl Into<Value>, result: Value) -> Self {
        Self {
            jsonrpc: "2.0",
            id: id.into(),
            result: Some(result),
            error: None,
        }
    }

    #[must_use]
    pub fn history(id: impl Into<Value>, session: &Session) -> Self {
        Self::raw_success(
            id,
            serde_json::to_value(RpcHistoryBody::from(session)).unwrap_or_else(|error| {
                serde_json::json!({
                    "schemaVersion": DEFAULT_SCHEMA_VERSION,
                    "sessionKey": DEFAULT_SESSION_KEY,
                    "messages": [],
                    "serializationError": error.to_string(),
                })
            }),
        )
    }

    #[must_use]
    pub fn error(id: Option<Value>, error: RpcError) -> Self {
        Self {
            jsonrpc: "2.0",
            id: id.unwrap_or(Value::Null),
            result: None,
            error: Some(RpcErrorBody {
                code: error.code.as_i64(),
                message: error.message,
                data: Some(RpcErrorData {
                    error_code: error.error_code,
                }),
            }),
        }
    }

    #[must_use]
    pub fn ping(id: impl Into<Value>) -> Self {
        Self::raw_success(
            id,
            serde_json::json!({
                "ok": true,
                "schemaVersion": DEFAULT_SCHEMA_VERSION,
            }),
        )
    }
}

impl RpcProgressNotification {
    #[must_use]
    pub fn new(params: TurnProgressEvent) -> Self {
        Self {
            jsonrpc: "2.0",
            method: "reshape.progress",
            params,
        }
    }
}

impl From<&Session> for RpcHistoryBody {
    fn from(session: &Session) -> Self {
        Self {
            schema_version: DEFAULT_SCHEMA_VERSION.to_string(),
            session_key: session.session_key.clone(),
            messages: session
                .history
                .iter()
                .filter_map(RpcHistoryMessage::from_session_message)
                .collect(),
        }
    }
}

impl RpcHistoryMessage {
    fn from_session_message(message: &SessionMessage) -> Option<Self> {
        match message {
            SessionMessage::Input(InputEvent::UserText { text, .. })
            | SessionMessage::Input(InputEvent::PluginMessage { text }) => Some(Self {
                role: RpcHistoryRole::User,
                text: text.clone(),
            }),
            SessionMessage::Output(OutputEvent::FinalMessage { text }) => Some(Self {
                role: RpcHistoryRole::Reshape,
                text: text.clone(),
            }),
            SessionMessage::Output(OutputEvent::Completed { summary }) => Some(Self {
                role: RpcHistoryRole::Reshape,
                text: summary.clone(),
            }),
            SessionMessage::Output(OutputEvent::Error { message, .. }) => Some(Self {
                role: RpcHistoryRole::System,
                text: message.clone(),
            }),
            _ => None,
        }
    }
}

impl RpcInput {
    fn into_input_event(self) -> InputEvent {
        match self {
            Self::UserText { text } => InputEvent::UserText {
                text,
                source: InputSource::WebSocket,
            },
        }
    }
}

impl From<OutputEvent> for RpcOutput {
    fn from(output: OutputEvent) -> Self {
        match output {
            OutputEvent::FinalMessage { text } => Self::FinalMessage { text },
            OutputEvent::StreamChunk { text } => Self::StreamChunk { text },
            OutputEvent::ToolProgress { tool_name, message } => {
                Self::ToolProgress { tool_name, message }
            }
            OutputEvent::WorkspaceFileChanged { path } => Self::WorkspaceFileChanged {
                path: path.to_string_lossy().to_string(),
            },
            OutputEvent::Error { code, message } => Self::Error { code, message },
            OutputEvent::Completed { summary } => Self::Completed { summary },
        }
    }
}

fn is_valid_id(value: &Value) -> bool {
    matches!(value, Value::String(_) | Value::Number(_) | Value::Null)
}

fn input_params_error(error: serde_json::Error) -> String {
    let message = error.to_string();
    if message.contains("missing field `text`") {
        "input.text is required".to_string()
    } else if message.contains("unknown variant") {
        format!("invalid params: unsupported input.type ({message})")
    } else {
        format!("invalid params: {message}")
    }
}

fn metadata_id(value: &Value) -> Value {
    match value {
        Value::String(value) => Value::String(value.clone()),
        Value::Number(value) => Value::String(value.to_string()),
        Value::Null => Value::Null,
        _ => Value::Null,
    }
}

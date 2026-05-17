use std::collections::BTreeMap;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;
use uuid::Uuid;

pub const DEFAULT_SCHEMA_VERSION: &str = "1.0";
pub const DEFAULT_SESSION_KEY: &str = "local:main";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct Envelope<T> {
    pub header: EnvelopeHeader,
    pub metadata: BTreeMap<String, Value>,
    pub payload: T,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct EnvelopeHeader {
    pub message_id: Uuid,
    pub trace_id: Uuid,
    pub session_key: String,
    pub timestamp: DateTime<Utc>,
    pub attempt: u32,
    pub schema_version: String,
}

impl<T> Envelope<T> {
    pub fn new(payload: T) -> Self {
        Self::for_session(DEFAULT_SESSION_KEY, payload)
    }

    pub fn for_session(session_key: impl Into<String>, payload: T) -> Self {
        Self {
            header: EnvelopeHeader {
                message_id: Uuid::new_v4(),
                trace_id: Uuid::new_v4(),
                session_key: session_key.into(),
                timestamp: Utc::now(),
                attempt: 1,
                schema_version: DEFAULT_SCHEMA_VERSION.to_string(),
            },
            metadata: BTreeMap::new(),
            payload,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub enum InputEvent {
    UserText {
        text: String,
        source: InputSource,
    },
    CdpUserEvent {
        event: CdpUserEvent,
    },
    PluginMessage {
        text: String,
    },
    WorkspaceChanged {
        #[ts(type = "string")]
        path: PathBuf,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum InputSource {
    Cli,
    Cdp,
    Plugin,
    WebSocket,
    Test,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct CdpUserEvent {
    pub event_type: String,
    pub selector_hint: Option<String>,
    pub text: Option<String>,
    pub metadata: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub enum OutputEvent {
    FinalMessage {
        text: String,
    },
    StreamChunk {
        text: String,
    },
    ToolProgress {
        tool_name: String,
        message: String,
    },
    WorkspaceFileChanged {
        #[ts(type = "string")]
        path: PathBuf,
    },
    Error {
        code: ErrorCode,
        message: String,
    },
    Completed {
        summary: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TurnProgressEvent {
    pub turn_id: String,
    pub sequence: u32,
    pub kind: TurnProgressKind,
    pub tool_name: Option<String>,
    pub arguments_preview: Option<String>,
    pub result_preview: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum TurnProgressKind {
    TurnStarted,
    AssistantMessage,
    ToolStarted,
    ToolFinished,
    ToolFailed,
    TurnCompleted,
    TurnFailed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum ErrorCode {
    InvalidSchema,
    ValidationFailed,
    DuplicateMessage,
    AgentTimeout,
    ToolTimeout,
    ProviderUnavailable,
    ProviderResponseInvalid,
    ToolBudgetExceeded,
    Failed,
}

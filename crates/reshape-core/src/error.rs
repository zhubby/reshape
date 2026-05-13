use std::path::PathBuf;

use thiserror::Error;

pub type Result<T> = std::result::Result<T, ReshapeError>;

#[derive(Debug, Error)]
pub enum ReshapeError {
    #[error("workspace does not exist: {0}")]
    WorkspaceMissing(PathBuf),

    #[error("workspace path is not a directory: {0}")]
    WorkspaceNotDirectory(PathBuf),

    #[error("workspace path escapes configured root: {0}")]
    WorkspacePathEscapesRoot(PathBuf),

    #[error("unsupported workspace file type: {0}")]
    UnsupportedWorkspaceFile(PathBuf),

    #[error("invalid session key: expected {expected}, got {actual}")]
    InvalidSessionKey { expected: String, actual: String },

    #[error("unknown tool: {0}")]
    UnknownTool(String),

    #[error("tool budget exceeded: {0}")]
    ToolBudgetExceeded(String),

    #[error("invalid state transition from {from} to {to}")]
    InvalidStateTransition { from: String, to: String },

    #[error("provider error: {0}")]
    Provider(String),

    #[error("configuration error: {0}")]
    Config(String),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("notify error: {0}")]
    Notify(#[from] notify::Error),
}

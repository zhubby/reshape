use std::path::{Path, PathBuf};

use crate::error::{ReshapeError, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppConfig {
    pub workspace: WorkspaceConfig,
    pub runtime: RuntimeConfig,
    pub llm: LlmConfig,
    pub session_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceConfig {
    pub root: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeConfig {
    pub max_tool_iterations: usize,
    pub max_tool_calls: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LlmConfig {
    pub model: Option<String>,
    pub use_mock: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            workspace: WorkspaceConfig {
                root: PathBuf::from("."),
            },
            runtime: RuntimeConfig {
                max_tool_iterations: 8,
                max_tool_calls: 32,
            },
            llm: LlmConfig {
                model: None,
                use_mock: true,
            },
            session_key: "local:main".to_string(),
        }
    }
}

impl AppConfig {
    pub fn for_workspace(path: impl AsRef<Path>) -> Result<Self> {
        let root = validate_workspace(path.as_ref())?;

        Ok(Self {
            workspace: WorkspaceConfig { root },
            ..Self::default()
        })
    }

    pub fn with_model(mut self, model: Option<String>) -> Self {
        self.llm.model = model;
        self
    }

    pub fn with_mock_llm(mut self, use_mock: bool) -> Self {
        self.llm.use_mock = use_mock;
        self
    }
}

pub fn validate_workspace(path: &Path) -> Result<PathBuf> {
    if !path.exists() {
        return Err(ReshapeError::WorkspaceMissing(path.to_path_buf()));
    }

    if !path.is_dir() {
        return Err(ReshapeError::WorkspaceNotDirectory(path.to_path_buf()));
    }

    Ok(path.to_path_buf())
}

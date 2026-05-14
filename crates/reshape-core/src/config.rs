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
    pub provider: LlmProviderKind,
    pub openai: OpenAiConfig,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LlmProviderKind {
    OpenAi,
}

impl LlmProviderKind {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
        }
    }
}

impl TryFrom<&str> for LlmProviderKind {
    type Error = ReshapeError;

    fn try_from(value: &str) -> Result<Self> {
        match value {
            "openai" => Ok(Self::OpenAi),
            other => Err(ReshapeError::Config(format!(
                "unsupported llm provider: {other}"
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenAiConfig {
    pub model: String,
    pub base_url: String,
    pub api_key_env: String,
    pub stream: bool,
    pub timeout_secs: u64,
    pub organization: Option<String>,
    pub project: Option<String>,
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
                provider: LlmProviderKind::OpenAi,
                openai: OpenAiConfig::default(),
            },
            session_key: "local:main".to_string(),
        }
    }
}

impl Default for OpenAiConfig {
    fn default() -> Self {
        Self {
            model: "gpt-5.5".to_string(),
            base_url: "https://api.openai.com/v1".to_string(),
            api_key_env: "OPENAI_API_KEY".to_string(),
            stream: true,
            timeout_secs: 120,
            organization: None,
            project: None,
        }
    }
}

impl AppConfig {
    pub fn for_workspace(path: impl AsRef<Path>) -> Result<Self> {
        let root = validate_workspace(path.as_ref())?;
        tracing::debug!(workspace = %root.display(), "building app config for workspace");

        Ok(Self {
            workspace: WorkspaceConfig { root },
            ..Self::default()
        })
    }

    pub fn with_model(mut self, model: String) -> Self {
        self.llm.openai.model = model;
        self
    }
}

pub fn validate_workspace(path: &Path) -> Result<PathBuf> {
    tracing::debug!(path = %path.display(), "validating workspace path");
    if !path.exists() {
        tracing::warn!(path = %path.display(), "workspace validation failed: missing");
        return Err(ReshapeError::WorkspaceMissing(path.to_path_buf()));
    }

    if !path.is_dir() {
        tracing::warn!(path = %path.display(), "workspace validation failed: not a directory");
        return Err(ReshapeError::WorkspaceNotDirectory(path.to_path_buf()));
    }

    tracing::debug!(path = %path.display(), "workspace path validated");
    Ok(path.to_path_buf())
}

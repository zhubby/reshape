use std::path::{Path, PathBuf};

use crate::error::{ReshapeError, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppConfig {
    pub workspace: WorkspaceConfig,
    pub runtime: RuntimeConfig,
    pub llm: LlmConfig,
    pub tools: ToolsConfig,
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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolsConfig {
    pub web_search: WebSearchConfig,
    pub web_fetch: WebFetchConfig,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebSearchConfig {
    pub enabled: bool,
    pub provider: String,
    pub tavily: TavilyConfig,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TavilyConfig {
    pub api_key: String,
    pub env_key: String,
    pub base_url: String,
    pub search_depth: String,
    pub max_results: usize,
    pub topic: String,
    pub include_answer: bool,
    pub include_images: bool,
    pub include_favicon: bool,
    pub timeout_secs: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebFetchConfig {
    pub enabled: bool,
    pub max_bytes: usize,
    pub timeout_secs: u64,
    pub max_redirects: usize,
    pub download_dir: String,
    pub allowed_content_types: Vec<String>,
    pub ssrf_allowlist: Vec<String>,
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
    pub api_key: String,
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
            tools: ToolsConfig::default(),
            session_key: "local:main".to_string(),
        }
    }
}

impl Default for OpenAiConfig {
    fn default() -> Self {
        Self {
            model: "gpt-5.5".to_string(),
            base_url: "https://api.openai.com/v1".to_string(),
            api_key: String::new(),
            stream: true,
            timeout_secs: 120,
            organization: None,
            project: None,
        }
    }
}

impl Default for WebSearchConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: "tavily".to_string(),
            tavily: TavilyConfig::default(),
        }
    }
}

impl Default for TavilyConfig {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            env_key: "TAVILY_API_KEY".to_string(),
            base_url: "https://api.tavily.com".to_string(),
            search_depth: "basic".to_string(),
            max_results: 5,
            topic: "general".to_string(),
            include_answer: false,
            include_images: false,
            include_favicon: true,
            timeout_secs: 15,
        }
    }
}

impl Default for WebFetchConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_bytes: 52_428_800,
            timeout_secs: 60,
            max_redirects: 5,
            download_dir: "assets/downloads".to_string(),
            allowed_content_types: vec![
                "image/png".to_string(),
                "image/jpeg".to_string(),
                "image/gif".to_string(),
                "image/webp".to_string(),
                "video/mp4".to_string(),
                "video/webm".to_string(),
                "audio/mpeg".to_string(),
                "audio/wav".to_string(),
                "application/pdf".to_string(),
            ],
            ssrf_allowlist: Vec::new(),
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

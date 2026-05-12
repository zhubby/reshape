use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::Result;
use crate::workspace::Workspace;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

#[derive(Clone)]
pub struct ToolContext {
    pub workspace: Arc<dyn Workspace>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolResult {
    pub success: bool,
    pub content_for_model: String,
    pub content_for_user: Option<String>,
    pub should_continue: bool,
}

impl ToolResult {
    pub fn success(content: impl Into<String>) -> Self {
        Self {
            success: true,
            content_for_model: content.into(),
            content_for_user: None,
            should_continue: true,
        }
    }

    pub fn complete(summary: impl Into<String>) -> Self {
        let summary = summary.into();
        Self {
            success: true,
            content_for_model: summary.clone(),
            content_for_user: Some(summary),
            should_continue: false,
        }
    }

    pub fn error(content: impl Into<String>) -> Self {
        Self {
            success: false,
            content_for_model: content.into(),
            content_for_user: None,
            should_continue: true,
        }
    }
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn definition(&self) -> ToolDefinition;
    async fn execute(&self, args: Value, context: &ToolContext) -> Result<ToolResult>;
}

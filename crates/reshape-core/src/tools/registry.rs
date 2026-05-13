use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::error::{ReshapeError, Result};

use super::types::{Tool, ToolContext, ToolDefinition, ToolResult};

#[async_trait]
pub trait ToolRegistry: Send + Sync {
    fn definitions(&self) -> Vec<ToolDefinition>;
    async fn execute(
        &self,
        name: &str,
        arguments: Value,
        context: &ToolContext,
    ) -> Result<ToolResult>;
}

#[derive(Default, Clone)]
pub struct InMemoryToolRegistry {
    tools: BTreeMap<String, Arc<dyn Tool>>,
}

impl InMemoryToolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register<T>(mut self, tool: T) -> Self
    where
        T: Tool + 'static,
    {
        let definition = tool.definition();
        tracing::debug!(
            tool_name = %definition.name,
            tool_count = self.tools.len(),
            "registering tool"
        );
        self.tools.insert(definition.name, Arc::new(tool));
        self
    }
}

#[async_trait]
impl ToolRegistry for InMemoryToolRegistry {
    fn definitions(&self) -> Vec<ToolDefinition> {
        self.tools.values().map(|tool| tool.definition()).collect()
    }

    async fn execute(
        &self,
        name: &str,
        arguments: Value,
        context: &ToolContext,
    ) -> Result<ToolResult> {
        tracing::debug!(tool_name = name, "looking up tool");
        let tool = self.tools.get(name).ok_or_else(|| {
            tracing::warn!(tool_name = name, "unknown tool requested");
            ReshapeError::UnknownTool(name.to_string())
        })?;

        tracing::debug!(tool_name = name, "executing tool");
        tool.execute(arguments, context).await
    }
}

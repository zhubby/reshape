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
        self.tools
            .insert(tool.definition().name.clone(), Arc::new(tool));
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
        let tool = self
            .tools
            .get(name)
            .ok_or_else(|| ReshapeError::UnknownTool(name.to_string()))?;

        tool.execute(arguments, context).await
    }
}

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::Result;

use super::types::{Tool, ToolContext, ToolDefinition, ToolResult};

#[derive(Debug, Default, Clone)]
pub struct CompleteTaskTool;

#[derive(Debug, Deserialize)]
struct CompleteArgs {
    summary: String,
}

#[async_trait]
impl Tool for CompleteTaskTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "complete_task".to_string(),
            description: "Signal that the current agent turn is complete.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "summary": { "type": "string" }
                },
                "required": ["summary"]
            }),
        }
    }

    async fn execute(&self, args: Value, _context: &ToolContext) -> Result<ToolResult> {
        let args: CompleteArgs = serde_json::from_value(args)?;
        Ok(ToolResult::complete(args.summary))
    }
}

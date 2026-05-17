use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::Result;

use super::html::normalize_html_write;
use super::types::{Tool, ToolContext, ToolDefinition, ToolResult};

#[derive(Debug, Clone, Copy)]
enum FileToolKind {
    List,
    Read,
    Write,
    Delete,
}

#[derive(Debug, Clone)]
pub struct FileTool {
    kind: FileToolKind,
}

impl FileTool {
    pub fn list_files() -> Self {
        Self {
            kind: FileToolKind::List,
        }
    }

    pub fn read_file() -> Self {
        Self {
            kind: FileToolKind::Read,
        }
    }

    pub fn write_file() -> Self {
        Self {
            kind: FileToolKind::Write,
        }
    }

    pub fn delete_file() -> Self {
        Self {
            kind: FileToolKind::Delete,
        }
    }
}

#[derive(Debug, Deserialize)]
struct PathArgs {
    path: String,
}

#[derive(Debug, Deserialize)]
struct WriteArgs {
    path: String,
    content: String,
}

#[async_trait]
impl Tool for FileTool {
    fn definition(&self) -> ToolDefinition {
        match self.kind {
            FileToolKind::List => ToolDefinition {
                name: "list_files".to_string(),
                description: "List files in the configured workspace.".to_string(),
                parameters: json!({"type": "object", "properties": {}}),
            },
            FileToolKind::Read => ToolDefinition {
                name: "read_file".to_string(),
                description: "Read a text file from the configured workspace.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {"path": {"type": "string"}},
                    "required": ["path"]
                }),
            },
            FileToolKind::Write => ToolDefinition {
                name: "write_file".to_string(),
                description: "Write a text file inside the configured workspace.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "path": {"type": "string"},
                        "content": {"type": "string"}
                    },
                    "required": ["path", "content"]
                }),
            },
            FileToolKind::Delete => ToolDefinition {
                name: "delete_file".to_string(),
                description: "Delete a text file inside the configured workspace.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {"path": {"type": "string"}},
                    "required": ["path"]
                }),
            },
        }
    }

    async fn execute(&self, args: Value, context: &ToolContext) -> Result<ToolResult> {
        match self.kind {
            FileToolKind::List => {
                tracing::debug!("file tool listing workspace files");
                let files = context.workspace.list_files().await?;
                tracing::debug!(file_count = files.len(), "file tool listed workspace files");
                Ok(ToolResult::success(
                    files
                        .into_iter()
                        .map(|path| path.to_string_lossy().to_string())
                        .collect::<Vec<_>>()
                        .join("\n"),
                ))
            }
            FileToolKind::Read => {
                let args: PathArgs = serde_json::from_value(args)?;
                tracing::debug!(path = %args.path, "file tool reading workspace file");
                Ok(ToolResult::success(
                    context.workspace.read_text(&args.path).await?,
                ))
            }
            FileToolKind::Write => {
                let args: WriteArgs = serde_json::from_value(args)?;
                let content = match normalize_html_write(&args.path, &args.content) {
                    Ok(Some(content)) => content,
                    Ok(None) => args.content,
                    Err(error) => return Ok(ToolResult::error(error.message().to_string())),
                };
                tracing::debug!(
                    path = %args.path,
                    content_len = content.len(),
                    "file tool writing workspace file"
                );
                let path = context.workspace.write_text(&args.path, &content).await?;
                tracing::info!(path = %path.display(), "file tool wrote workspace file");
                Ok(ToolResult::success(format!(
                    "wrote {}",
                    path.to_string_lossy()
                )))
            }
            FileToolKind::Delete => {
                let args: PathArgs = serde_json::from_value(args)?;
                tracing::debug!(path = %args.path, "file tool deleting workspace file");
                let path = context.workspace.delete_file(&args.path).await?;
                tracing::info!(path = %path.display(), "file tool deleted workspace file");
                Ok(ToolResult::success(format!(
                    "deleted {}",
                    path.to_string_lossy()
                )))
            }
        }
    }
}

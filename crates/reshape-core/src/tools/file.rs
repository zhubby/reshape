use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::Result;

use super::html::{default_stylesheet_href, is_html_path, normalize_html_write};
use super::types::{Tool, ToolContext, ToolDefinition, ToolResult};

const DEFAULT_READ_LIMIT: usize = 2_000;

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
    #[serde(default)]
    offset: Option<usize>,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct WriteArgs {
    path: String,
    content: String,
}

impl FileTool {
    fn json_error(
        tool: &str,
        action: &str,
        path: Option<&str>,
        error: impl Into<String>,
        retry_hint: impl Into<String>,
    ) -> Result<ToolResult> {
        let mut content = json!({
            "tool": tool,
            "action": action,
            "success": false,
            "recoverable": true,
            "error": error.into(),
            "retry_hint": retry_hint.into(),
        });
        if let Some(path) = path {
            content["path"] = json!(path);
        }
        ToolResult::json(content, None)
    }

    fn parse_path_args(
        tool: &str,
        action: &str,
        args: Value,
    ) -> Result<std::result::Result<PathArgs, ToolResult>> {
        Ok(match serde_json::from_value(args) {
            Ok(args) => Ok(args),
            Err(error) => Err(Self::json_error(
                tool,
                action,
                None,
                format!("invalid arguments: {error}"),
                format!("Call {tool} with the required schema fields."),
            )?),
        })
    }

    fn parse_write_args(args: Value) -> Result<std::result::Result<WriteArgs, ToolResult>> {
        Ok(match serde_json::from_value(args) {
            Ok(args) => Ok(args),
            Err(error) => Err(Self::json_error(
                "write_file",
                "write",
                None,
                format!("invalid arguments: {error}"),
                "Call write_file with string fields `path` and `content`.",
            )?),
        })
    }

    fn validate_read_window(args: &PathArgs) -> Result<Option<ToolResult>> {
        if args.offset.is_some_and(|offset| offset == 0) {
            return Ok(Some(Self::json_error(
                "read_file",
                "read",
                Some(&args.path),
                "`offset` must be greater than 0",
                "Retry with `offset` set to a 1-indexed line number.",
            )?));
        }
        if args.limit.is_some_and(|limit| limit == 0) {
            return Ok(Some(Self::json_error(
                "read_file",
                "read",
                Some(&args.path),
                "`limit` must be greater than 0",
                "Retry with `limit` set to the number of lines to read.",
            )?));
        }
        Ok(None)
    }
}

#[async_trait]
impl Tool for FileTool {
    fn definition(&self) -> ToolDefinition {
        match self.kind {
            FileToolKind::List => ToolDefinition {
                name: "list_files".to_string(),
                description: "List workspace files and return structured metadata the model can use for follow-up read_file calls.".to_string(),
                parameters: json!({
                    "type": "object",
                    "description": "List all files currently visible in the configured workspace.",
                    "properties": {},
                    "additionalProperties": false
                }),
            },
            FileToolKind::Read => ToolDefinition {
                name: "read_file".to_string(),
                description: "Read a text file from the configured workspace with line-numbered, paginated, structured output.".to_string(),
                parameters: json!({
                    "type": "object",
                    "description": "Read a UTF-8 text file. Use offset and limit to continue after truncated output.",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Workspace-relative path returned by list_files or chosen for a known file.",
                            "examples": ["index.html", "assets/site.css"]
                        },
                        "offset": {
                            "type": "integer",
                            "description": "1-indexed line number to start reading from. Defaults to 1.",
                            "minimum": 1,
                            "default": 1
                        },
                        "limit": {
                            "type": "integer",
                            "description": "Maximum number of lines to return. Defaults to 2000.",
                            "minimum": 1,
                            "default": DEFAULT_READ_LIMIT
                        }
                    },
                    "required": ["path"],
                    "additionalProperties": false
                }),
            },
            FileToolKind::Write => ToolDefinition {
                name: "write_file".to_string(),
                description: "Write a text file inside the configured workspace and return structured write/HTML-normalization metadata.".to_string(),
                parameters: json!({
                    "type": "object",
                    "description": "Write UTF-8 text content to a workspace-relative path. HTML files are normalized and validated before writing.",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Workspace-relative destination path.",
                            "examples": ["index.html", "pages/product-roadmap.html", "assets/site.css"]
                        },
                        "content": {
                            "type": "string",
                            "description": "Full file content to write."
                        }
                    },
                    "required": ["path", "content"],
                    "additionalProperties": false
                }),
            },
            FileToolKind::Delete => ToolDefinition {
                name: "delete_file".to_string(),
                description: "Delete a file inside the configured workspace and return structured deletion metadata.".to_string(),
                parameters: json!({
                    "type": "object",
                    "description": "Delete one workspace-relative file.",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Workspace-relative file path to delete.",
                            "examples": ["old.html", "assets/unused.css"]
                        }
                    },
                    "required": ["path"],
                    "additionalProperties": false
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
                let files = files
                    .into_iter()
                    .map(|path| path.to_string_lossy().to_string())
                    .collect::<Vec<_>>();
                let file_count = files.len();
                ToolResult::json(
                    json!({
                        "tool": "list_files",
                        "action": "list",
                        "success": true,
                        "file_count": file_count,
                        "files": files,
                    }),
                    Some(format!(
                        "Listed {file_count} workspace {}",
                        if file_count == 1 { "file" } else { "files" }
                    )),
                )
            }
            FileToolKind::Read => {
                let args = match Self::parse_path_args("read_file", "read", args)? {
                    Ok(args) => args,
                    Err(result) => return Ok(result),
                };
                if let Some(result) = Self::validate_read_window(&args)? {
                    return Ok(result);
                }
                tracing::debug!(path = %args.path, "file tool reading workspace file");
                let content = context.workspace.read_text(&args.path).await?;
                let lines = content.lines().collect::<Vec<_>>();
                let total_lines = lines.len();
                let offset = args.offset.unwrap_or(1);
                let limit = args.limit.unwrap_or(DEFAULT_READ_LIMIT);
                let start_index = offset.saturating_sub(1);
                let selected = if start_index >= total_lines {
                    Vec::new()
                } else {
                    lines
                        .iter()
                        .enumerate()
                        .skip(start_index)
                        .take(limit)
                        .map(|(index, line)| format!("{}: {line}", index + 1))
                        .collect::<Vec<_>>()
                };
                let returned_lines = selected.len();
                let next_offset_value = offset + returned_lines;
                let truncated = next_offset_value <= total_lines;
                let next_offset = truncated.then_some(next_offset_value);

                ToolResult::json(
                    json!({
                        "tool": "read_file",
                        "action": "read",
                        "success": true,
                        "path": args.path,
                        "offset": offset,
                        "limit": limit,
                        "total_lines": total_lines,
                        "returned_lines": returned_lines,
                        "truncated": truncated,
                        "next_offset": next_offset,
                        "total_bytes": content.len(),
                        "content": selected.join("\n"),
                    }),
                    Some(format!(
                        "Read {}: {returned_lines} of {total_lines} lines",
                        args.path
                    )),
                )
            }
            FileToolKind::Write => {
                let args = match Self::parse_write_args(args)? {
                    Ok(args) => args,
                    Err(result) => return Ok(result),
                };
                let html_path = is_html_path(&args.path);
                let stylesheet_href = html_path.then(|| default_stylesheet_href(&args.path));
                let content = match normalize_html_write(&args.path, &args.content) {
                    Ok(Some(content)) => content,
                    Ok(None) => args.content,
                    Err(error) => {
                        return Self::json_error(
                            "write_file",
                            "write",
                            Some(&args.path),
                            error.message().to_string(),
                            "Fix the HTML and call write_file again.",
                        );
                    }
                };
                let bytes_written = content.len();
                tracing::debug!(
                    path = %args.path,
                    content_len = content.len(),
                    "file tool writing workspace file"
                );
                let path = context.workspace.write_text(&args.path, &content).await?;
                tracing::info!(path = %path.display(), "file tool wrote workspace file");
                let path = path.to_string_lossy().to_string();
                ToolResult::json(
                    json!({
                        "tool": "write_file",
                        "action": "write",
                        "success": true,
                        "path": path,
                        "bytes_written": bytes_written,
                        "normalized_html": html_path,
                        "stylesheet_href": stylesheet_href,
                        "validation": if html_path { "valid" } else { "not_applicable" },
                    }),
                    Some(format!("Wrote {path}")),
                )
            }
            FileToolKind::Delete => {
                let args = match Self::parse_path_args("delete_file", "delete", args)? {
                    Ok(args) => args,
                    Err(result) => return Ok(result),
                };
                tracing::debug!(path = %args.path, "file tool deleting workspace file");
                let path = context.workspace.delete_file(&args.path).await?;
                tracing::info!(path = %path.display(), "file tool deleted workspace file");
                let path = path.to_string_lossy().to_string();
                ToolResult::json(
                    json!({
                        "tool": "delete_file",
                        "action": "delete",
                        "success": true,
                        "path": path,
                    }),
                    Some(format!("Deleted {path}")),
                )
            }
        }
    }
}

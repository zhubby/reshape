use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

use async_trait::async_trait;
use ironhtml_parser::parse;
use serde::Serialize;
use serde_json::{Value, json};

use crate::error::Result;

use super::html::is_html_path;
use super::types::{Tool, ToolContext, ToolDefinition, ToolResult};

#[derive(Debug, Clone, Copy, Default)]
pub struct WorkspaceMapTool;

#[derive(Debug, Clone, Serialize)]
struct FileNode {
    path: String,
    extension: Option<String>,
    kind: &'static str,
    is_hub_candidate: bool,
}

#[derive(Debug, Clone, Serialize)]
struct LinkNode {
    source: String,
    href: String,
    target: Option<String>,
    status: &'static str,
    kind: &'static str,
}

#[derive(Debug, Clone, Serialize)]
struct BacklinkNode {
    path: String,
    sources: Vec<String>,
}

#[async_trait]
impl Tool for WorkspaceMapTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "map_workspace".to_string(),
            description: "Map the configured wiki-like workspace: files, entrypoints, local links, backlinks, missing links, and orphan HTML/Markdown pages so the model can choose focused read_file calls.".to_string(),
            parameters: json!({
                "type": "object",
                "description": "Inspect the current workspace file graph without modifying files.",
                "properties": {},
                "additionalProperties": false
            }),
        }
    }

    async fn execute(&self, args: Value, context: &ToolContext) -> Result<ToolResult> {
        if !args.as_object().is_some_and(serde_json::Map::is_empty) {
            return ToolResult::json(
                json!({
                    "tool": "map_workspace",
                    "action": "map",
                    "success": false,
                    "recoverable": true,
                    "error": "invalid arguments: map_workspace does not accept arguments",
                    "retry_hint": "Call map_workspace with an empty object: {}."
                }),
                None,
            );
        }

        tracing::debug!("mapping workspace files and links");
        let paths = context.workspace.list_files().await?;
        let path_strings = paths
            .iter()
            .map(|path| path_to_workspace_string(path))
            .collect::<Vec<_>>();
        let path_set = path_strings.iter().cloned().collect::<BTreeSet<_>>();

        let files = path_strings
            .iter()
            .map(|path| FileNode {
                path: path.clone(),
                extension: extension(path),
                kind: file_kind(path),
                is_hub_candidate: is_hub_candidate(path),
            })
            .collect::<Vec<_>>();

        let mut links = Vec::new();
        for path in &path_strings {
            if is_html_path(path) || is_markdown_path(path) {
                let content = context.workspace.read_text(path).await?;
                links.extend(extract_links(path, &content, &path_set));
            }
        }

        let backlinks = backlinks(&links);
        let entrypoints = entrypoints(&path_strings);
        let orphans = orphans(&path_strings, &backlinks);
        let file_count = files.len();
        let link_count = links.len();

        ToolResult::json(
            json!({
                "tool": "map_workspace",
                "action": "map",
                "success": true,
                "file_count": file_count,
                "link_count": link_count,
                "entrypoints": entrypoints,
                "files": files,
                "links": links,
                "backlinks": backlinks,
                "orphans": orphans,
            }),
            Some(format!(
                "Mapped {file_count} workspace {} and {link_count} {}",
                if file_count == 1 { "file" } else { "files" },
                if link_count == 1 { "link" } else { "links" }
            )),
        )
    }
}

fn extract_links(source: &str, content: &str, path_set: &BTreeSet<String>) -> Vec<LinkNode> {
    if is_html_path(source) {
        return extract_html_links(source, content, path_set);
    }

    extract_markdown_links(source, content, path_set)
}

fn extract_html_links(source: &str, content: &str, path_set: &BTreeSet<String>) -> Vec<LinkNode> {
    let document = parse(content);
    let mut links = Vec::new();

    for (tag, attribute) in [
        ("a", "href"),
        ("link", "href"),
        ("script", "src"),
        ("img", "src"),
    ] {
        for element in document.root.find_all_elements(tag) {
            if let Some(href) = element.get_attribute(attribute) {
                links.push(link_node(source, href, path_set, tag));
            }
        }
    }

    links
}

fn extract_markdown_links(
    source: &str,
    content: &str,
    path_set: &BTreeSet<String>,
) -> Vec<LinkNode> {
    let mut links = markdown_inline_links(source, content, path_set);
    links.extend(markdown_wiki_links(source, content, path_set));
    links
}

fn markdown_inline_links(
    source: &str,
    content: &str,
    path_set: &BTreeSet<String>,
) -> Vec<LinkNode> {
    let bytes = content.as_bytes();
    let mut links = Vec::new();
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index] != b'[' {
            index += 1;
            continue;
        }

        if index + 1 < bytes.len() && bytes[index + 1] == b'[' {
            index += 2;
            continue;
        }

        let Some(label_end) = content[index + 1..].find(']') else {
            break;
        };
        let open_paren = index + 1 + label_end + 1;
        if bytes.get(open_paren) != Some(&b'(') {
            index += 1;
            continue;
        }

        let Some(close_paren_offset) = content[open_paren + 1..].find(')') else {
            break;
        };
        let href_start = open_paren + 1;
        let href_end = href_start + close_paren_offset;
        let href = content[href_start..href_end].trim();
        if !href.is_empty() {
            links.push(link_node(source, href, path_set, "markdown"));
        }
        index = href_end + 1;
    }

    links
}

fn markdown_wiki_links(source: &str, content: &str, path_set: &BTreeSet<String>) -> Vec<LinkNode> {
    let mut links = Vec::new();
    let mut remainder = content;

    while let Some(start) = remainder.find("[[") {
        let after_start = &remainder[start + 2..];
        let Some(end) = after_start.find("]]") else {
            break;
        };
        let raw = after_start[..end].trim();
        let href = raw.split('|').next().unwrap_or_default().trim();
        if !href.is_empty() {
            links.push(link_node(source, href, path_set, "wikilink"));
        }
        remainder = &after_start[end + 2..];
    }

    links
}

fn link_node(
    source: &str,
    href: &str,
    path_set: &BTreeSet<String>,
    kind: &'static str,
) -> LinkNode {
    match classify_href(source, href, path_set) {
        HrefTarget::External => LinkNode {
            source: source.to_string(),
            href: href.to_string(),
            target: None,
            status: "external",
            kind,
        },
        HrefTarget::Anchor => LinkNode {
            source: source.to_string(),
            href: href.to_string(),
            target: None,
            status: "anchor",
            kind,
        },
        HrefTarget::Unsupported => LinkNode {
            source: source.to_string(),
            href: href.to_string(),
            target: None,
            status: "unsupported",
            kind,
        },
        HrefTarget::Local(target) => {
            let status = if path_set.contains(&target) {
                "resolved"
            } else {
                "missing"
            };
            LinkNode {
                source: source.to_string(),
                href: href.to_string(),
                target: Some(target),
                status,
                kind,
            }
        }
    }
}

enum HrefTarget {
    Local(String),
    External,
    Anchor,
    Unsupported,
}

fn classify_href(source: &str, href: &str, path_set: &BTreeSet<String>) -> HrefTarget {
    let trimmed = href.trim();
    if trimmed.is_empty() {
        return HrefTarget::Unsupported;
    }
    if trimmed.starts_with('#') {
        return HrefTarget::Anchor;
    }
    if is_external_href(trimmed) {
        return HrefTarget::External;
    }
    if trimmed.starts_with('/') {
        return HrefTarget::Unsupported;
    }

    let without_fragment = trimmed.split('#').next().unwrap_or_default();
    let path_part = without_fragment.split('?').next().unwrap_or_default();
    if path_part.is_empty() {
        return HrefTarget::Anchor;
    }

    let Some(target) = resolve_relative_target(source, path_part) else {
        return HrefTarget::Unsupported;
    };

    if path_set.contains(&target) {
        return HrefTarget::Local(target);
    }

    if extension(&target).is_none() {
        let markdown_target = format!("{target}.md");
        if path_set.contains(&markdown_target) {
            return HrefTarget::Local(markdown_target);
        }
    }

    HrefTarget::Local(target)
}

fn is_external_href(href: &str) -> bool {
    href.starts_with("//")
        || href
            .split_once(':')
            .is_some_and(|(scheme, _)| !scheme.contains('/') && !scheme.contains('\\'))
}

fn resolve_relative_target(source: &str, href: &str) -> Option<String> {
    let mut combined = PathBuf::new();
    if let Some(parent) = Path::new(source).parent() {
        combined.push(parent);
    }
    combined.push(href);
    normalize_workspace_path(&combined)
}

fn normalize_workspace_path(path: &Path) -> Option<String> {
    let mut parts = Vec::new();

    for component in path.components() {
        match component {
            Component::Normal(value) => parts.push(value.to_string_lossy().to_string()),
            Component::CurDir => {}
            Component::ParentDir => {
                parts.pop()?;
            }
            Component::RootDir | Component::Prefix(_) => return None,
        }
    }

    if parts.is_empty() {
        None
    } else {
        Some(parts.join("/"))
    }
}

fn backlinks(links: &[LinkNode]) -> Vec<BacklinkNode> {
    let mut backlinks = BTreeMap::<String, BTreeSet<String>>::new();
    for link in links {
        if link.status == "resolved"
            && let Some(target) = &link.target
        {
            backlinks
                .entry(target.clone())
                .or_default()
                .insert(link.source.clone());
        }
    }

    backlinks
        .into_iter()
        .map(|(path, sources)| BacklinkNode {
            path,
            sources: sources.into_iter().collect(),
        })
        .collect()
}

fn entrypoints(paths: &[String]) -> Vec<String> {
    if paths.iter().any(|path| path == "index.html") {
        return vec!["index.html".to_string()];
    }

    paths
        .iter()
        .filter(|path| is_root_document(path))
        .cloned()
        .collect()
}

fn orphans(paths: &[String], backlinks: &[BacklinkNode]) -> Vec<String> {
    let linked = backlinks
        .iter()
        .map(|backlink| backlink.path.as_str())
        .collect::<BTreeSet<_>>();

    paths
        .iter()
        .filter(|path| path.as_str() != "index.html")
        .filter(|path| is_page_document(path))
        .filter(|path| !linked.contains(path.as_str()))
        .cloned()
        .collect()
}

fn path_to_workspace_string(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn extension(path: &str) -> Option<String> {
    Path::new(path)
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
}

fn file_kind(path: &str) -> &'static str {
    match extension(path).as_deref() {
        Some("html" | "htm") => "html",
        Some("md") => "markdown",
        Some("css") => "stylesheet",
        Some("js") => "script",
        Some("json") => "data",
        Some("txt") => "text",
        Some("png" | "jpg" | "jpeg" | "gif" | "webp") => "image",
        Some("mp4" | "webm") => "video",
        Some("mp3" | "wav") => "audio",
        Some("pdf") => "document",
        _ => "unknown",
    }
}

fn is_markdown_path(path: &str) -> bool {
    matches!(extension(path).as_deref(), Some("md"))
}

fn is_page_document(path: &str) -> bool {
    is_html_path(path) || is_markdown_path(path)
}

fn is_root_document(path: &str) -> bool {
    !path.contains('/') && is_page_document(path)
}

fn is_hub_candidate(path: &str) -> bool {
    path == "index.html" || is_root_document(path)
}

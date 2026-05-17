use std::path::{Component, Path};

use ironhtml_parser::{parse, validate};
use lol_html::{RewriteStrSettings, element, html_content::ContentType, rewrite_str};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HtmlWriteError {
    message: String,
}

impl HtmlWriteError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

pub fn normalize_html_write(path: &str, content: &str) -> Result<Option<String>, HtmlWriteError> {
    if !is_html_path(path) {
        return Ok(None);
    }

    let stylesheet_href = default_stylesheet_href(path);
    let document = complete_document(content, &stylesheet_href);
    validate_document(path, &document)?;

    if has_default_stylesheet(&document, &stylesheet_href) {
        return Ok(Some(document));
    }

    let link = format!(r#"<link rel="stylesheet" href="{stylesheet_href}">"#);
    let rewritten = rewrite_str(
        &document,
        RewriteStrSettings {
            element_content_handlers: vec![element!("head", move |head| {
                head.append(&link, ContentType::Html);
                Ok(())
            })],
            ..RewriteStrSettings::new()
        },
    )
    .map_err(|error| HtmlWriteError::new(format!("could not rewrite HTML: {error}")))?;

    validate_document(path, &rewritten)?;
    Ok(Some(rewritten))
}

fn is_html_path(path: &str) -> bool {
    matches!(
        Path::new(path).extension().and_then(|value| value.to_str()),
        Some("html" | "htm")
    )
}

fn complete_document(content: &str, stylesheet_href: &str) -> String {
    let normalized = content.trim_start().to_ascii_lowercase();
    if normalized.contains("<html") && normalized.contains("<head") {
        return content.to_string();
    }

    format!(
        r#"<!doctype html><html lang="zh-CN"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><link rel="stylesheet" href="{stylesheet_href}"></head><body>{content}</body></html>"#
    )
}

fn default_stylesheet_href(path: &str) -> String {
    let parent_count = Path::new(path)
        .parent()
        .map(|parent| {
            parent
                .components()
                .filter(|component| matches!(component, Component::Normal(_)))
                .count()
        })
        .unwrap_or_default();

    if parent_count == 0 {
        "assets/site.css".to_string()
    } else {
        format!("{}assets/site.css", "../".repeat(parent_count))
    }
}

fn has_default_stylesheet(html: &str, href: &str) -> bool {
    parse(html)
        .root
        .find_all_elements("link")
        .into_iter()
        .any(|element| {
            element.get_attribute("rel").is_some_and(|rel| {
                rel.split_ascii_whitespace()
                    .any(|item| item == "stylesheet")
            }) && element.get_attribute("href") == Some(href)
        })
}

fn validate_document(path: &str, html: &str) -> Result<(), HtmlWriteError> {
    let document = parse(html);
    let errors = validate(&document);
    if errors.is_empty() {
        return Ok(());
    }

    let summary = errors
        .iter()
        .take(3)
        .map(|error| format!("<{}>: {}", error.element, error.message))
        .collect::<Vec<_>>()
        .join("; ");

    Err(HtmlWriteError::new(format!(
        "invalid HTML for {path}: {summary}"
    )))
}

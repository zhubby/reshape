use std::sync::Arc;

use reshape_core::tools::file::FileTool;
use reshape_core::tools::{Tool, ToolContext};
use reshape_core::workspace::local::LocalWorkspace;

async fn write_file(
    dir: &tempfile::TempDir,
    path: &str,
    content: &str,
) -> reshape_core::tools::ToolResult {
    let workspace = Arc::new(LocalWorkspace::new(dir.path()).unwrap());
    FileTool::write_file()
        .execute(
            serde_json::json!({
                "path": path,
                "content": content,
            }),
            &ToolContext { workspace },
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn html_write_adds_default_stylesheet_to_root_page() {
    let dir = tempfile::tempdir().unwrap();

    let result = write_file(
        &dir,
        "index.html",
        "<!doctype html><html><head></head><body><h1>Hello</h1></body></html>",
    )
    .await;

    assert!(result.success);
    let html = tokio::fs::read_to_string(dir.path().join("index.html"))
        .await
        .unwrap();
    assert!(html.contains(r#"href="assets/site.css""#));
    assert!(html.contains("<h1>Hello</h1>"));
}

#[tokio::test]
async fn html_write_adds_depth_aware_stylesheet_to_nested_page() {
    let dir = tempfile::tempdir().unwrap();

    let result = write_file(
        &dir,
        "pages/topic.html",
        "<!doctype html><html><head></head><body><h1>Topic</h1></body></html>",
    )
    .await;

    assert!(result.success);
    let html = tokio::fs::read_to_string(dir.path().join("pages").join("topic.html"))
        .await
        .unwrap();
    assert!(html.contains(r#"href="../assets/site.css""#));
}

#[tokio::test]
async fn html_write_does_not_duplicate_existing_default_stylesheet() {
    let dir = tempfile::tempdir().unwrap();

    let result = write_file(
        &dir,
        "index.html",
        r#"<!doctype html><html><head><link rel="stylesheet" href="assets/site.css"></head><body><h1>Hello</h1></body></html>"#,
    )
    .await;

    assert!(result.success);
    let html = tokio::fs::read_to_string(dir.path().join("index.html"))
        .await
        .unwrap();
    assert_eq!(html.matches(r#"href="assets/site.css""#).count(), 1);
}

#[tokio::test]
async fn html_write_wraps_body_fragment_in_complete_document() {
    let dir = tempfile::tempdir().unwrap();

    let result = write_file(&dir, "index.html", "<h1>Hello</h1>").await;

    assert!(result.success);
    let html = tokio::fs::read_to_string(dir.path().join("index.html"))
        .await
        .unwrap();
    assert!(html.contains("<!doctype html>"));
    assert!(html.contains(r#"<html lang="zh-CN">"#));
    assert!(html.contains(r#"href="assets/site.css""#));
    assert!(html.contains("<body><h1>Hello</h1></body>"));
}

#[tokio::test]
async fn invalid_html_write_returns_tool_error_and_does_not_write_file() {
    let dir = tempfile::tempdir().unwrap();

    let result = write_file(
        &dir,
        "index.html",
        "<!doctype html><html><head></head><body><img></body></html>",
    )
    .await;

    assert!(!result.success);
    assert!(result.content_for_model.contains("invalid HTML"));
    assert!(!dir.path().join("index.html").exists());
}

#[tokio::test]
async fn non_html_write_keeps_content_unchanged() {
    let dir = tempfile::tempdir().unwrap();

    let result = write_file(&dir, "assets/site.css", "body { color: red; }").await;

    assert!(result.success);
    assert_eq!(
        tokio::fs::read_to_string(dir.path().join("assets").join("site.css"))
            .await
            .unwrap(),
        "body { color: red; }"
    );
}

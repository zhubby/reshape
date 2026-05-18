use std::sync::Arc;

use reshape_core::tools::map::WorkspaceMapTool;
use reshape_core::tools::{Tool, ToolContext, ToolResult};
use reshape_core::workspace::local::LocalWorkspace;
use serde_json::Value;

fn context(dir: &tempfile::TempDir) -> ToolContext {
    ToolContext {
        workspace: Arc::new(LocalWorkspace::new(dir.path()).unwrap()),
    }
}

fn model_json(result: &ToolResult) -> Value {
    serde_json::from_str(&result.content_for_model).expect("tool output should be JSON")
}

async fn write(dir: &tempfile::TempDir, path: &str, content: &str) {
    let full_path = dir.path().join(path);
    if let Some(parent) = full_path.parent() {
        tokio::fs::create_dir_all(parent).await.unwrap();
    }
    tokio::fs::write(full_path, content).await.unwrap();
}

fn array_strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn map_workspace_returns_files_and_entrypoints() {
    let dir = tempfile::tempdir().unwrap();
    write(&dir, "index.html", "<a href=\"pages/topic.html\">Topic</a>").await;
    write(&dir, "pages/topic.html", "<h1>Topic</h1>").await;
    write(&dir, "assets/site.css", "body { color: black; }").await;
    let context = context(&dir);

    let result = WorkspaceMapTool
        .execute(serde_json::json!({}), &context)
        .await
        .unwrap();
    let json = model_json(&result);

    assert_eq!(json["tool"], "map_workspace");
    assert_eq!(json["action"], "map");
    assert_eq!(json["success"], true);
    assert_eq!(json["file_count"], 3);
    assert_eq!(array_strings(&json["entrypoints"]), vec!["index.html"]);
    assert!(
        json["files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|file| file["path"] == "index.html"
                && file["extension"] == "html"
                && file["kind"] == "html"
                && file["is_hub_candidate"] == true)
    );
    assert!(
        json["files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|file| file["path"] == "assets/site.css" && file["kind"] == "stylesheet")
    );
}

#[tokio::test]
async fn map_workspace_resolves_html_links_and_backlinks() {
    let dir = tempfile::tempdir().unwrap();
    write(&dir, "index.html", "<a href=\"pages/topic.html\">Topic</a>").await;
    write(
        &dir,
        "pages/topic.html",
        "<a href=\"../index.html\">Home</a>",
    )
    .await;
    let context = context(&dir);

    let result = WorkspaceMapTool
        .execute(serde_json::json!({}), &context)
        .await
        .unwrap();
    let json = model_json(&result);

    assert!(
        json["links"]
            .as_array()
            .unwrap()
            .iter()
            .any(|link| link["source"] == "index.html"
                && link["href"] == "pages/topic.html"
                && link["target"] == "pages/topic.html"
                && link["status"] == "resolved")
    );
    assert!(
        json["backlinks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|backlink| backlink["path"] == "pages/topic.html"
                && array_strings(&backlink["sources"]) == vec!["index.html"])
    );
}

#[tokio::test]
async fn map_workspace_reports_missing_links() {
    let dir = tempfile::tempdir().unwrap();
    write(
        &dir,
        "index.html",
        "<a href=\"pages/missing.html\">Missing</a>",
    )
    .await;
    let context = context(&dir);

    let result = WorkspaceMapTool
        .execute(serde_json::json!({}), &context)
        .await
        .unwrap();
    let json = model_json(&result);

    assert!(
        json["links"]
            .as_array()
            .unwrap()
            .iter()
            .any(|link| link["source"] == "index.html"
                && link["href"] == "pages/missing.html"
                && link["target"] == "pages/missing.html"
                && link["status"] == "missing")
    );
}

#[tokio::test]
async fn map_workspace_reports_orphan_pages() {
    let dir = tempfile::tempdir().unwrap();
    write(&dir, "index.html", "<a href=\"pages/topic.html\">Topic</a>").await;
    write(&dir, "pages/topic.html", "<h1>Topic</h1>").await;
    write(&dir, "pages/orphan.html", "<h1>Orphan</h1>").await;
    let context = context(&dir);

    let result = WorkspaceMapTool
        .execute(serde_json::json!({}), &context)
        .await
        .unwrap();
    let json = model_json(&result);

    assert_eq!(array_strings(&json["orphans"]), vec!["pages/orphan.html"]);
}

#[tokio::test]
async fn map_workspace_parses_markdown_links_and_wikilinks() {
    let dir = tempfile::tempdir().unwrap();
    write(
        &dir,
        "notes.md",
        "[Topic](pages/topic.html)\n[[notes/topic.md]]\n[[notes/other.md|Other]]",
    )
    .await;
    write(&dir, "pages/topic.html", "<h1>Topic</h1>").await;
    write(&dir, "notes/topic.md", "# Topic").await;
    write(&dir, "notes/other.md", "# Other").await;
    let context = context(&dir);

    let result = WorkspaceMapTool
        .execute(serde_json::json!({}), &context)
        .await
        .unwrap();
    let json = model_json(&result);

    for target in ["pages/topic.html", "notes/topic.md", "notes/other.md"] {
        assert!(
            json["links"]
                .as_array()
                .unwrap()
                .iter()
                .any(|link| link["source"] == "notes.md"
                    && link["target"] == target
                    && link["status"] == "resolved"),
            "missing resolved markdown link to {target}"
        );
    }
}

#[tokio::test]
async fn map_workspace_marks_external_and_anchor_links() {
    let dir = tempfile::tempdir().unwrap();
    write(
        &dir,
        "index.html",
        r##"<a href="https://example.com">External</a><a href="#section">Section</a>"##,
    )
    .await;
    let context = context(&dir);

    let result = WorkspaceMapTool
        .execute(serde_json::json!({}), &context)
        .await
        .unwrap();
    let json = model_json(&result);

    assert!(
        json["links"]
            .as_array()
            .unwrap()
            .iter()
            .any(|link| link["href"] == "https://example.com"
                && link["target"].is_null()
                && link["status"] == "external")
    );
    assert!(
        json["links"]
            .as_array()
            .unwrap()
            .iter()
            .any(|link| link["href"] == "#section"
                && link["target"].is_null()
                && link["status"] == "anchor")
    );
    assert_eq!(array_strings(&json["orphans"]), Vec::<String>::new());
}

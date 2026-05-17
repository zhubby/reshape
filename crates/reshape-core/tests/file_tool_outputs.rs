use std::sync::Arc;

use reshape_core::tools::file::FileTool;
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

#[tokio::test]
async fn list_files_returns_structured_json() {
    let dir = tempfile::tempdir().unwrap();
    let context = context(&dir);
    FileTool::write_file()
        .execute(
            serde_json::json!({"path": "index.html", "content": "<h1>Hello</h1>"}),
            &context,
        )
        .await
        .unwrap();

    let result = FileTool::list_files()
        .execute(serde_json::json!({}), &context)
        .await
        .unwrap();
    let json = model_json(&result);

    assert_eq!(json["tool"], "list_files");
    assert_eq!(json["action"], "list");
    assert_eq!(json["success"], true);
    assert_eq!(json["file_count"], 1);
    assert_eq!(json["files"][0], "index.html");
    assert_eq!(
        result.content_for_user.as_deref(),
        Some("Listed 1 workspace file")
    );
}

#[tokio::test]
async fn read_file_returns_line_numbered_structured_json() {
    let dir = tempfile::tempdir().unwrap();
    let context = context(&dir);
    FileTool::write_file()
        .execute(
            serde_json::json!({"path": "notes.txt", "content": "alpha\nbeta\ngamma"}),
            &context,
        )
        .await
        .unwrap();

    let result = FileTool::read_file()
        .execute(
            serde_json::json!({"path": "notes.txt", "offset": 2, "limit": 1}),
            &context,
        )
        .await
        .unwrap();
    let json = model_json(&result);

    assert_eq!(json["tool"], "read_file");
    assert_eq!(json["action"], "read");
    assert_eq!(json["success"], true);
    assert_eq!(json["path"], "notes.txt");
    assert_eq!(json["total_lines"], 3);
    assert_eq!(json["returned_lines"], 1);
    assert_eq!(json["truncated"], true);
    assert_eq!(json["next_offset"], 3);
    assert_eq!(json["content"], "2: beta");
    assert_eq!(
        result.content_for_user.as_deref(),
        Some("Read notes.txt: 1 of 3 lines")
    );
}

#[tokio::test]
async fn read_file_reports_empty_and_out_of_range_states() {
    let dir = tempfile::tempdir().unwrap();
    let context = context(&dir);
    FileTool::write_file()
        .execute(
            serde_json::json!({"path": "empty.txt", "content": ""}),
            &context,
        )
        .await
        .unwrap();

    let empty = FileTool::read_file()
        .execute(serde_json::json!({"path": "empty.txt"}), &context)
        .await
        .unwrap();
    let empty_json = model_json(&empty);
    assert_eq!(empty_json["total_lines"], 0);
    assert_eq!(empty_json["returned_lines"], 0);
    assert_eq!(empty_json["truncated"], false);
    assert!(empty_json["next_offset"].is_null());

    let out_of_range = FileTool::read_file()
        .execute(
            serde_json::json!({"path": "empty.txt", "offset": 10}),
            &context,
        )
        .await
        .unwrap();
    let out_of_range_json = model_json(&out_of_range);
    assert_eq!(out_of_range_json["returned_lines"], 0);
    assert_eq!(out_of_range_json["truncated"], false);
    assert_eq!(out_of_range_json["content"], "");
}

#[tokio::test]
async fn write_file_reports_html_normalization_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let context = context(&dir);

    let result = FileTool::write_file()
        .execute(
            serde_json::json!({"path": "pages/topic.html", "content": "<h1>Topic</h1>"}),
            &context,
        )
        .await
        .unwrap();
    let json = model_json(&result);

    assert_eq!(json["tool"], "write_file");
    assert_eq!(json["action"], "write");
    assert_eq!(json["success"], true);
    assert_eq!(json["path"], "pages/topic.html");
    assert_eq!(json["normalized_html"], true);
    assert_eq!(json["stylesheet_href"], "../assets/site.css");
    assert_eq!(json["validation"], "valid");
    assert!(json["bytes_written"].as_u64().unwrap() > 0);
    assert_eq!(
        result.content_for_user.as_deref(),
        Some("Wrote pages/topic.html")
    );
}

#[tokio::test]
async fn invalid_html_write_returns_structured_recoverable_error() {
    let dir = tempfile::tempdir().unwrap();
    let context = context(&dir);

    let result = FileTool::write_file()
        .execute(
            serde_json::json!({
                "path": "index.html",
                "content": "<!doctype html><html><head></head><body><img></body></html>"
            }),
            &context,
        )
        .await
        .unwrap();
    let json = model_json(&result);

    assert!(!result.success);
    assert_eq!(json["tool"], "write_file");
    assert_eq!(json["action"], "write");
    assert_eq!(json["success"], false);
    assert_eq!(json["path"], "index.html");
    assert_eq!(json["recoverable"], true);
    assert_eq!(
        json["retry_hint"],
        "Fix the HTML and call write_file again."
    );
    assert!(json["error"].as_str().unwrap().contains("invalid HTML"));
    assert!(!dir.path().join("index.html").exists());
}

#[tokio::test]
async fn delete_file_returns_structured_json() {
    let dir = tempfile::tempdir().unwrap();
    let context = context(&dir);
    FileTool::write_file()
        .execute(
            serde_json::json!({"path": "old.txt", "content": "remove me"}),
            &context,
        )
        .await
        .unwrap();

    let result = FileTool::delete_file()
        .execute(serde_json::json!({"path": "old.txt"}), &context)
        .await
        .unwrap();
    let json = model_json(&result);

    assert_eq!(json["tool"], "delete_file");
    assert_eq!(json["action"], "delete");
    assert_eq!(json["success"], true);
    assert_eq!(json["path"], "old.txt");
    assert_eq!(result.content_for_user.as_deref(), Some("Deleted old.txt"));
}

use std::path::PathBuf;
use std::time::Duration;

use reshape_core::workspace::local::{LocalWorkspace, LocalWorkspaceWatcher};
use reshape_core::workspace::{Workspace, WorkspaceWatcher};

#[tokio::test]
async fn writes_and_reads_workspace_text_file() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = LocalWorkspace::new(dir.path()).unwrap();

    workspace
        .write_text("index.html", "<main>Hello</main>")
        .await
        .unwrap();

    assert_eq!(
        workspace.read_text("index.html").await.unwrap(),
        "<main>Hello</main>"
    );
    assert_eq!(
        workspace.list_files().await.unwrap(),
        vec![PathBuf::from("index.html")]
    );
}

#[tokio::test]
async fn rejects_path_escape_attempts() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = LocalWorkspace::new(dir.path()).unwrap();

    let error = workspace
        .write_text("../outside.html", "bad")
        .await
        .unwrap_err();

    assert!(error.to_string().contains("escapes configured root"));
}

#[tokio::test]
async fn rejects_symlink_escape_attempts() {
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let outside_file = outside.path().join("outside.md");
    tokio::fs::write(&outside_file, "secret").await.unwrap();
    std::os::unix::fs::symlink(&outside_file, dir.path().join("link.md")).unwrap();
    std::os::unix::fs::symlink(outside.path(), dir.path().join("assets")).unwrap();
    let workspace = LocalWorkspace::new(dir.path()).unwrap();

    let read_error = workspace.read_text("link.md").await.unwrap_err();
    let symlink_write_error = workspace
        .write_text("link.md", "overwritten")
        .await
        .unwrap_err();
    let write_error = workspace
        .write_text("assets/page.md", "escaped")
        .await
        .unwrap_err();

    assert!(read_error.to_string().contains("escapes configured root"));
    assert!(
        symlink_write_error
            .to_string()
            .contains("escapes configured root")
    );
    assert!(write_error.to_string().contains("escapes configured root"));
}

#[tokio::test]
async fn list_files_recursively_returns_relative_paths() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = LocalWorkspace::new(dir.path()).unwrap();

    workspace
        .write_text("assets/page.md", "nested")
        .await
        .unwrap();

    assert_eq!(
        workspace.list_files().await.unwrap(),
        vec![PathBuf::from("assets/page.md")]
    );
}

#[tokio::test]
async fn watcher_observes_workspace_file_changes() {
    let dir = tempfile::tempdir().unwrap();
    let mut watcher = LocalWorkspaceWatcher::new(dir.path()).unwrap();
    tokio::fs::write(dir.path().join("index.html"), "<h1>Changed</h1>")
        .await
        .unwrap();

    let changed = tokio::time::timeout(Duration::from_secs(3), watcher.next_change())
        .await
        .unwrap()
        .unwrap()
        .unwrap();

    assert!(changed.ends_with("index.html"));
}

pub mod local;

use std::path::PathBuf;

use async_trait::async_trait;

use crate::error::Result;

#[async_trait]
pub trait Workspace: Send + Sync {
    fn root(&self) -> PathBuf;
    async fn list_files(&self) -> Result<Vec<PathBuf>>;
    async fn read_text(&self, path: &str) -> Result<String>;
    async fn write_text(&self, path: &str, content: &str) -> Result<PathBuf>;
    async fn write_bytes(&self, path: &str, content: &[u8]) -> Result<PathBuf>;
    async fn delete_file(&self, path: &str) -> Result<PathBuf>;
}

#[async_trait]
pub trait WorkspaceWatcher: Send + Sync {
    async fn next_change(&mut self) -> Result<Option<PathBuf>>;
}

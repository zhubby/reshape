use std::path::{Component, Path, PathBuf};

use async_trait::async_trait;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;

use crate::error::{ReshapeError, Result};

use super::{Workspace, WorkspaceWatcher};

#[derive(Debug, Clone)]
pub struct LocalWorkspace {
    root: PathBuf,
}

impl LocalWorkspace {
    pub fn new(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        if !root.exists() {
            return Err(ReshapeError::WorkspaceMissing(root));
        }
        if !root.is_dir() {
            return Err(ReshapeError::WorkspaceNotDirectory(root));
        }

        Ok(Self {
            root: root.canonicalize()?,
        })
    }

    fn relative_path<'a>(&self, path: &'a str) -> Result<&'a Path> {
        let relative = Path::new(path);
        if relative
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::RootDir))
        {
            return Err(ReshapeError::WorkspacePathEscapesRoot(
                relative.to_path_buf(),
            ));
        }

        let extension = relative.extension().and_then(|value| value.to_str());
        let supported = matches!(
            extension,
            Some("html" | "htm" | "js" | "css" | "json" | "md" | "txt")
        );
        if !supported {
            return Err(ReshapeError::UnsupportedWorkspaceFile(
                relative.to_path_buf(),
            ));
        }

        Ok(relative)
    }

    fn resolve_existing(&self, path: &str) -> Result<PathBuf> {
        let relative = self.relative_path(path)?;
        let candidate = self.root.join(relative);
        let canonical = candidate.canonicalize()?;
        if !canonical.starts_with(&self.root) {
            return Err(ReshapeError::WorkspacePathEscapesRoot(
                relative.to_path_buf(),
            ));
        }

        Ok(canonical)
    }

    async fn resolve_for_write(&self, path: &str) -> Result<PathBuf> {
        let relative = self.relative_path(path)?;
        let candidate = self.root.join(relative);
        if let Ok(metadata) = tokio::fs::symlink_metadata(&candidate).await {
            if metadata.file_type().is_symlink() {
                return Err(ReshapeError::WorkspacePathEscapesRoot(
                    relative.to_path_buf(),
                ));
            }

            let canonical = candidate.canonicalize()?;
            if !canonical.starts_with(&self.root) {
                return Err(ReshapeError::WorkspacePathEscapesRoot(
                    relative.to_path_buf(),
                ));
            }
        }

        if let Some(parent) = candidate.parent() {
            tokio::fs::create_dir_all(parent).await?;
            let canonical_parent = parent.canonicalize()?;
            if !canonical_parent.starts_with(&self.root) {
                return Err(ReshapeError::WorkspacePathEscapesRoot(
                    relative.to_path_buf(),
                ));
            }
        }

        Ok(candidate)
    }

    fn strip_root(&self, path: PathBuf) -> PathBuf {
        path.strip_prefix(&self.root)
            .map(Path::to_path_buf)
            .unwrap_or(path)
    }
}

#[async_trait]
impl Workspace for LocalWorkspace {
    fn root(&self) -> PathBuf {
        self.root.clone()
    }

    async fn list_files(&self) -> Result<Vec<PathBuf>> {
        let mut files = Vec::new();
        let mut dirs = vec![self.root.clone()];

        while let Some(dir) = dirs.pop() {
            let mut entries = tokio::fs::read_dir(dir).await?;

            while let Some(entry) = entries.next_entry().await? {
                let file_type = entry.file_type().await?;
                if file_type.is_dir() {
                    let canonical = entry.path().canonicalize()?;
                    if canonical.starts_with(&self.root) {
                        dirs.push(canonical);
                    }
                } else if file_type.is_file() {
                    let canonical = entry.path().canonicalize()?;
                    if canonical.starts_with(&self.root) {
                        files.push(self.strip_root(canonical));
                    }
                }
            }
        }

        files.sort();
        Ok(files)
    }

    async fn read_text(&self, path: &str) -> Result<String> {
        Ok(tokio::fs::read_to_string(self.resolve_existing(path)?).await?)
    }

    async fn write_text(&self, path: &str, content: &str) -> Result<PathBuf> {
        let path = self.resolve_for_write(path).await?;
        tokio::fs::write(&path, content).await?;
        Ok(self.strip_root(path))
    }

    async fn delete_file(&self, path: &str) -> Result<PathBuf> {
        let path = self.resolve_existing(path)?;
        tokio::fs::remove_file(&path).await?;
        Ok(self.strip_root(path))
    }
}

pub struct LocalWorkspaceWatcher {
    rx: mpsc::Receiver<PathBuf>,
    _watcher: RecommendedWatcher,
}

impl LocalWorkspaceWatcher {
    pub fn new(root: impl AsRef<Path>) -> Result<Self> {
        let (tx, rx) = mpsc::channel(64);
        let mut watcher =
            notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                if let Ok(event) = event {
                    for path in event.paths {
                        let _ = tx.blocking_send(path);
                    }
                }
            })?;

        watcher.watch(root.as_ref(), RecursiveMode::Recursive)?;

        Ok(Self {
            rx,
            _watcher: watcher,
        })
    }
}

#[async_trait]
impl WorkspaceWatcher for LocalWorkspaceWatcher {
    async fn next_change(&mut self) -> Result<Option<PathBuf>> {
        while let Some(path) = self.rx.recv().await {
            if path.is_file() {
                return Ok(Some(path));
            }
        }

        Ok(None)
    }
}

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
        tracing::debug!(root = %root.display(), "initializing local workspace");
        if !root.exists() {
            tracing::warn!(root = %root.display(), "workspace root is missing");
            return Err(ReshapeError::WorkspaceMissing(root));
        }
        if !root.is_dir() {
            tracing::warn!(root = %root.display(), "workspace root is not a directory");
            return Err(ReshapeError::WorkspaceNotDirectory(root));
        }

        let canonical = root.canonicalize()?;
        tracing::info!(root = %canonical.display(), "local workspace initialized");
        Ok(Self { root: canonical })
    }

    fn relative_path<'a>(&self, path: &'a str) -> Result<&'a Path> {
        let relative = Path::new(path);
        if relative
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::RootDir))
        {
            tracing::warn!(path, "workspace path escapes root");
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
            tracing::warn!(path, "unsupported workspace file type");
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
            tracing::warn!(path, "existing workspace path resolves outside root");
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
                tracing::warn!(path, "refusing to write through workspace symlink");
                return Err(ReshapeError::WorkspacePathEscapesRoot(
                    relative.to_path_buf(),
                ));
            }

            let canonical = candidate.canonicalize()?;
            if !canonical.starts_with(&self.root) {
                tracing::warn!(path, "workspace write path resolves outside root");
                return Err(ReshapeError::WorkspacePathEscapesRoot(
                    relative.to_path_buf(),
                ));
            }
        }

        if let Some(parent) = candidate.parent() {
            tokio::fs::create_dir_all(parent).await?;
            let canonical_parent = parent.canonicalize()?;
            if !canonical_parent.starts_with(&self.root) {
                tracing::warn!(path, "workspace write parent resolves outside root");
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
        tracing::debug!(root = %self.root.display(), "listing workspace files");
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
        tracing::debug!(file_count = files.len(), "listed workspace files");
        Ok(files)
    }

    async fn read_text(&self, path: &str) -> Result<String> {
        tracing::debug!(path, "reading workspace text file");
        let content = tokio::fs::read_to_string(self.resolve_existing(path)?).await?;
        tracing::debug!(
            path,
            content_len = content.len(),
            "read workspace text file"
        );
        Ok(content)
    }

    async fn write_text(&self, path: &str, content: &str) -> Result<PathBuf> {
        tracing::debug!(
            path,
            content_len = content.len(),
            "writing workspace text file"
        );
        let path = self.resolve_for_write(path).await?;
        tokio::fs::write(&path, content).await?;
        let relative = self.strip_root(path);
        tracing::info!(path = %relative.display(), "wrote workspace text file");
        Ok(relative)
    }

    async fn delete_file(&self, path: &str) -> Result<PathBuf> {
        tracing::debug!(path, "deleting workspace file");
        let path = self.resolve_existing(path)?;
        tokio::fs::remove_file(&path).await?;
        let relative = self.strip_root(path);
        tracing::info!(path = %relative.display(), "deleted workspace file");
        Ok(relative)
    }
}

pub struct LocalWorkspaceWatcher {
    rx: mpsc::Receiver<PathBuf>,
    _watcher: RecommendedWatcher,
}

impl LocalWorkspaceWatcher {
    pub fn new(root: impl AsRef<Path>) -> Result<Self> {
        tracing::debug!(root = %root.as_ref().display(), "initializing workspace watcher");
        let (tx, rx) = mpsc::channel(64);
        let mut watcher =
            notify::recommended_watcher(move |event: notify::Result<notify::Event>| match event {
                Ok(event) => {
                    for path in event.paths {
                        let _ = tx.blocking_send(path);
                    }
                }
                Err(error) => {
                    tracing::warn!(%error, "workspace watcher received notify error");
                }
            })?;

        watcher.watch(root.as_ref(), RecursiveMode::Recursive)?;
        tracing::info!(root = %root.as_ref().display(), "workspace watcher initialized");

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
                tracing::debug!(path = %path.display(), "workspace watcher observed file change");
                return Ok(Some(path));
            }
        }

        tracing::debug!("workspace watcher stream ended");
        Ok(None)
    }
}

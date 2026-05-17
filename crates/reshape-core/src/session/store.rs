use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::Mutex;

use crate::error::{ReshapeError, Result};

use super::Session;

#[async_trait]
pub trait SessionStore: Send + Sync {
    async fn load(&self) -> Result<Session>;
    async fn save(&self, session: Session) -> Result<()>;
}

#[derive(Debug, Clone, Default)]
pub struct InMemorySessionStore {
    session: Arc<Mutex<Session>>,
}

#[derive(Debug, Clone)]
pub struct FileSessionStore {
    path: PathBuf,
}

impl FileSessionStore {
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

#[async_trait]
impl SessionStore for InMemorySessionStore {
    async fn load(&self) -> Result<Session> {
        let session = self.session.lock().await.clone();
        tracing::debug!(
            session_key = %session.session_key,
            turn_index = session.turn_index,
            history_len = session.history.len(),
            "loaded session"
        );
        Ok(session)
    }

    async fn save(&self, session: Session) -> Result<()> {
        tracing::debug!(
            session_key = %session.session_key,
            turn_index = session.turn_index,
            history_len = session.history.len(),
            "saving session"
        );
        *self.session.lock().await = session;
        Ok(())
    }
}

#[async_trait]
impl SessionStore for FileSessionStore {
    async fn load(&self) -> Result<Session> {
        match tokio::fs::read_to_string(&self.path).await {
            Ok(content) => serde_json::from_str(&content).map_err(|error| {
                ReshapeError::Config(format!(
                    "failed to parse session store {}: {error}",
                    self.path.display()
                ))
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Session::default()),
            Err(error) => Err(error.into()),
        }
    }

    async fn save(&self, session: Session) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let content = serde_json::to_string_pretty(&session)?;
        let tmp_path = self.path.with_extension("json.tmp");
        tokio::fs::write(&tmp_path, content).await?;
        tokio::fs::rename(&tmp_path, &self.path).await?;
        Ok(())
    }
}

use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::Mutex;

use crate::error::Result;

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

#[async_trait]
impl SessionStore for InMemorySessionStore {
    async fn load(&self) -> Result<Session> {
        Ok(self.session.lock().await.clone())
    }

    async fn save(&self, session: Session) -> Result<()> {
        *self.session.lock().await = session;
        Ok(())
    }
}

pub mod cli_stdin;

use async_trait::async_trait;

use crate::error::Result;
use crate::protocol::InputEvent;

#[async_trait]
pub trait IngressSource: Send + Sync {
    fn name(&self) -> &str;
    async fn next_event(&mut self) -> Result<Option<InputEvent>>;
}

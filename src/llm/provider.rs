use async_trait::async_trait;

use crate::error::Result;
use crate::tools::types::ToolDefinition;

use super::types::{ChatMessage, ChatOptions, LlmResponse};

#[async_trait]
pub trait LlmProvider: Send + Sync {
    fn name(&self) -> &str;
    fn default_model(&self) -> &str;

    async fn chat(
        &self,
        messages: Vec<ChatMessage>,
        tools: Vec<ToolDefinition>,
        options: ChatOptions,
    ) -> Result<LlmResponse>;
}

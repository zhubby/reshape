use std::collections::VecDeque;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;
use tokio::sync::Mutex;

use crate::error::Result;
use crate::tools::types::ToolDefinition;

use super::provider::LlmProvider;
use super::types::{ChatMessage, ChatOptions, ChatRole, LlmResponse, ToolCall};

#[derive(Debug, Clone)]
pub struct MockLlmProvider {
    responses: Arc<Mutex<VecDeque<LlmResponse>>>,
    fallback_turns: Arc<Mutex<usize>>,
    default: LlmResponse,
}

impl MockLlmProvider {
    pub fn new(responses: impl IntoIterator<Item = LlmResponse>) -> Self {
        Self {
            responses: Arc::new(Mutex::new(responses.into_iter().collect())),
            fallback_turns: Arc::new(Mutex::new(0)),
            default: LlmResponse {
                content: "Mock response complete".to_string(),
                tool_calls: Vec::new(),
            },
        }
    }
}

impl Default for MockLlmProvider {
    fn default() -> Self {
        Self::new([])
    }
}

#[async_trait]
impl LlmProvider for MockLlmProvider {
    fn name(&self) -> &str {
        "mock"
    }

    fn default_model(&self) -> &str {
        "mock-model"
    }

    async fn chat(
        &self,
        messages: Vec<ChatMessage>,
        tools: Vec<ToolDefinition>,
        _options: ChatOptions,
    ) -> Result<LlmResponse> {
        tracing::debug!(
            provider = self.name(),
            message_count = messages.len(),
            tool_count = tools.len(),
            "mock llm chat requested"
        );
        if let Some(response) = self.responses.lock().await.pop_front() {
            tracing::debug!(
                provider = self.name(),
                tool_call_count = response.tool_calls.len(),
                "mock llm returning queued response"
            );
            return Ok(response);
        }

        if !tools.iter().any(|tool| tool.name == "write_file") {
            tracing::debug!(
                provider = self.name(),
                "mock llm returning default response without file tool"
            );
            return Ok(self.default.clone());
        }

        let saw_tool_result = messages
            .iter()
            .any(|message| matches!(message.role, ChatRole::Tool));

        if saw_tool_result {
            tracing::debug!(
                provider = self.name(),
                "mock llm returning completion tool call"
            );
            return Ok(LlmResponse {
                content: "Completing mock page generation".to_string(),
                tool_calls: vec![ToolCall {
                    id: "mock-complete".to_string(),
                    name: "complete_task".to_string(),
                    arguments: json!({"summary": "Mock page generated in index.html"}),
                }],
            });
        }

        let mut turns = self.fallback_turns.lock().await;
        *turns += 1;
        tracing::debug!(
            provider = self.name(),
            turn = *turns,
            "mock llm returning write_file tool call"
        );
        Ok(LlmResponse {
            content: "Creating mock page".to_string(),
            tool_calls: vec![ToolCall {
                id: format!("mock-write-{turns}"),
                name: "write_file".to_string(),
                arguments: json!({
                    "path": "index.html",
                    "content": "<!doctype html><html><body><h1>Reshape Mock Page</h1></body></html>"
                }),
            }],
        })
    }
}

use std::collections::BTreeMap;
use std::time::Duration;

use async_trait::async_trait;
use futures_util::StreamExt;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::OpenAiConfig;
use crate::error::{ReshapeError, Result};
use crate::tools::types::ToolDefinition;

use super::provider::LlmProvider;
use super::types::{ChatMessage, ChatOptions, ChatRole, LlmResponse, ToolCall};

const CHAT_COMPLETIONS_PATH: &str = "chat/completions";

#[derive(Debug, Clone)]
pub struct OpenAiChatCompletionProvider {
    config: OpenAiConfig,
    api_key: String,
    client: reqwest::Client,
}

impl OpenAiChatCompletionProvider {
    pub fn from_config(config: OpenAiConfig) -> Result<Self> {
        let api_key = config.api_key.clone();
        Self::new(config, api_key)
    }

    pub fn new(config: OpenAiConfig, api_key: impl Into<String>) -> Result<Self> {
        let api_key = api_key.into();
        if api_key.trim().is_empty() {
            return Err(ReshapeError::Provider(
                "OpenAI API key must not be empty".to_string(),
            ));
        }

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(config.timeout_secs))
            .no_proxy()
            .build()
            .map_err(|error| ReshapeError::Provider(error.to_string()))?;

        Ok(Self {
            config,
            api_key,
            client,
        })
    }

    fn endpoint(&self) -> String {
        format!(
            "{}/{}",
            self.config.base_url.trim_end_matches('/'),
            CHAT_COMPLETIONS_PATH
        )
    }

    fn request_body(
        &self,
        messages: Vec<ChatMessage>,
        tools: Vec<ToolDefinition>,
        options: ChatOptions,
        stream: bool,
    ) -> Result<OpenAiChatCompletionRequest> {
        let tools = (!tools.is_empty()).then(|| {
            tools
                .into_iter()
                .map(OpenAiTool::from)
                .collect::<Vec<OpenAiTool>>()
        });

        Ok(OpenAiChatCompletionRequest {
            model: options.model.unwrap_or_else(|| self.config.model.clone()),
            messages: messages
                .into_iter()
                .map(OpenAiMessage::try_from)
                .collect::<Result<Vec<_>>>()?,
            tools,
            stream,
        })
    }

    async fn post(&self, body: OpenAiChatCompletionRequest) -> Result<reqwest::Response> {
        let response = self
            .client
            .post(self.endpoint())
            .headers(self.headers()?)
            .json(&body)
            .send()
            .await
            .map_err(|error| ReshapeError::Provider(error.to_string()))?;

        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }

        let body = response
            .text()
            .await
            .unwrap_or_else(|error| error.to_string());
        Err(ReshapeError::Provider(format!(
            "OpenAI chat completions request failed with status {status}: {body}"
        )))
    }

    fn headers(&self) -> Result<HeaderMap> {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", self.api_key))
                .map_err(|error| ReshapeError::Provider(error.to_string()))?,
        );
        if let Some(organization) = &self.config.organization {
            headers.insert(
                "OpenAI-Organization",
                HeaderValue::from_str(organization)
                    .map_err(|error| ReshapeError::Provider(error.to_string()))?,
            );
        }
        if let Some(project) = &self.config.project {
            headers.insert(
                "OpenAI-Project",
                HeaderValue::from_str(project)
                    .map_err(|error| ReshapeError::Provider(error.to_string()))?,
            );
        }
        Ok(headers)
    }

    async fn chat_once(
        &self,
        messages: Vec<ChatMessage>,
        tools: Vec<ToolDefinition>,
        options: ChatOptions,
    ) -> Result<LlmResponse> {
        let request = self.request_body(messages, tools, options, false)?;
        let response = self.post(request).await?;
        let response = response
            .json::<OpenAiChatCompletionResponse>()
            .await
            .map_err(|error| ReshapeError::Provider(error.to_string()))?;
        response.into_llm_response()
    }

    async fn chat_streaming(
        &self,
        messages: Vec<ChatMessage>,
        tools: Vec<ToolDefinition>,
        options: ChatOptions,
    ) -> Result<LlmResponse> {
        let request = self.request_body(messages, tools, options, true)?;
        let response = self.post(request).await?;
        aggregate_stream(response).await
    }
}

#[async_trait]
impl LlmProvider for OpenAiChatCompletionProvider {
    fn name(&self) -> &str {
        "openai"
    }

    fn default_model(&self) -> &str {
        &self.config.model
    }

    async fn chat(
        &self,
        messages: Vec<ChatMessage>,
        tools: Vec<ToolDefinition>,
        options: ChatOptions,
    ) -> Result<LlmResponse> {
        if self.config.stream {
            return self.chat_streaming(messages, tools, options).await;
        }
        self.chat_once(messages, tools, options).await
    }
}

#[derive(Debug, Serialize)]
struct OpenAiChatCompletionRequest {
    model: String,
    messages: Vec<OpenAiMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<OpenAiTool>>,
    stream: bool,
}

#[derive(Debug, Serialize)]
struct OpenAiMessage {
    role: &'static str,
    #[serde(skip_serializing_if = "String::is_empty")]
    content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<OpenAiToolCall>>,
}

impl TryFrom<ChatMessage> for OpenAiMessage {
    type Error = ReshapeError;

    fn try_from(message: ChatMessage) -> Result<Self> {
        let role = match message.role {
            ChatRole::System => "system",
            ChatRole::User => "user",
            ChatRole::Assistant => "assistant",
            ChatRole::Tool => "tool",
        };

        if matches!(message.role, ChatRole::Tool) && message.tool_call_id.is_none() {
            return Err(ReshapeError::Provider(
                "OpenAI tool messages require tool_call_id".to_string(),
            ));
        }

        let tool_calls = (!message.tool_calls.is_empty()).then(|| {
            message
                .tool_calls
                .into_iter()
                .map(OpenAiToolCall::from)
                .collect()
        });

        Ok(Self {
            role,
            content: message.content,
            tool_call_id: message.tool_call_id,
            tool_calls,
        })
    }
}

#[derive(Debug, Serialize)]
struct OpenAiTool {
    #[serde(rename = "type")]
    tool_type: &'static str,
    function: OpenAiFunctionDefinition,
}

impl From<ToolDefinition> for OpenAiTool {
    fn from(tool: ToolDefinition) -> Self {
        Self {
            tool_type: "function",
            function: OpenAiFunctionDefinition {
                name: tool.name,
                description: tool.description,
                parameters: tool.parameters,
            },
        }
    }
}

#[derive(Debug, Serialize)]
struct OpenAiFunctionDefinition {
    name: String,
    description: String,
    parameters: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OpenAiToolCall {
    id: String,
    #[serde(rename = "type")]
    tool_type: String,
    function: OpenAiFunctionCall,
}

impl From<ToolCall> for OpenAiToolCall {
    fn from(call: ToolCall) -> Self {
        Self {
            id: call.id,
            tool_type: "function".to_string(),
            function: OpenAiFunctionCall {
                name: call.name,
                arguments: call.arguments.to_string(),
            },
        }
    }
}

impl TryFrom<OpenAiToolCall> for ToolCall {
    type Error = ReshapeError;

    fn try_from(call: OpenAiToolCall) -> Result<Self> {
        let arguments = serde_json::from_str(&call.function.arguments).map_err(|error| {
            ReshapeError::Provider(format!(
                "invalid OpenAI tool call arguments for {}: {error}",
                call.function.name
            ))
        })?;
        Ok(Self {
            id: call.id,
            name: call.function.name,
            arguments,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OpenAiFunctionCall {
    name: String,
    arguments: String,
}

#[derive(Debug, Deserialize)]
struct OpenAiChatCompletionResponse {
    choices: Vec<OpenAiChoice>,
}

impl OpenAiChatCompletionResponse {
    fn into_llm_response(self) -> Result<LlmResponse> {
        let choice = self.choices.into_iter().next().ok_or_else(|| {
            ReshapeError::Provider("OpenAI response contained no choices".to_string())
        })?;
        Ok(LlmResponse {
            content: choice.message.content.unwrap_or_default(),
            tool_calls: choice
                .message
                .tool_calls
                .unwrap_or_default()
                .into_iter()
                .map(ToolCall::try_from)
                .collect::<Result<Vec<_>>>()?,
        })
    }
}

#[derive(Debug, Deserialize)]
struct OpenAiChoice {
    message: OpenAiResponseMessage,
}

#[derive(Debug, Deserialize)]
struct OpenAiResponseMessage {
    content: Option<String>,
    tool_calls: Option<Vec<OpenAiToolCall>>,
}

#[derive(Debug, Deserialize)]
struct OpenAiStreamChunk {
    choices: Vec<OpenAiStreamChoice>,
}

#[derive(Debug, Deserialize)]
struct OpenAiStreamChoice {
    delta: OpenAiStreamDelta,
}

#[derive(Debug, Deserialize)]
struct OpenAiStreamDelta {
    content: Option<String>,
    tool_calls: Option<Vec<OpenAiStreamToolCall>>,
}

#[derive(Debug, Deserialize)]
struct OpenAiStreamToolCall {
    index: usize,
    id: Option<String>,
    function: Option<OpenAiStreamFunctionCall>,
}

#[derive(Debug, Deserialize)]
struct OpenAiStreamFunctionCall {
    name: Option<String>,
    arguments: Option<String>,
}

#[derive(Debug, Default)]
struct PartialToolCall {
    id: Option<String>,
    name: Option<String>,
    arguments: String,
}

async fn aggregate_stream(response: reqwest::Response) -> Result<LlmResponse> {
    let mut content = String::new();
    let mut tool_calls = BTreeMap::<usize, PartialToolCall>::new();
    let mut pending = String::new();
    let mut stream = response.bytes_stream();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| ReshapeError::Provider(error.to_string()))?;
        pending.push_str(&String::from_utf8_lossy(&chunk));

        while let Some(newline) = pending.find('\n') {
            let line = pending[..newline].trim_end_matches('\r').to_string();
            pending.replace_range(..=newline, "");
            handle_sse_line(&line, &mut content, &mut tool_calls)?;
        }
    }

    if !pending.is_empty() {
        let line = pending.trim_end_matches('\r').to_string();
        handle_sse_line(&line, &mut content, &mut tool_calls)?;
    }

    Ok(LlmResponse {
        content,
        tool_calls: finalize_tool_calls(tool_calls)?,
    })
}

fn handle_sse_line(
    line: &str,
    content: &mut String,
    tool_calls: &mut BTreeMap<usize, PartialToolCall>,
) -> Result<()> {
    let Some(data) = line.strip_prefix("data:") else {
        return Ok(());
    };
    let data = data.trim_start();
    if data == "[DONE]" || data.is_empty() {
        return Ok(());
    }

    let chunk: OpenAiStreamChunk = serde_json::from_str(data).map_err(|error| {
        ReshapeError::Provider(format!("invalid OpenAI streaming chunk: {error}"))
    })?;
    for choice in chunk.choices {
        if let Some(delta) = choice.delta.content {
            content.push_str(&delta);
        }
        for call in choice.delta.tool_calls.unwrap_or_default() {
            let partial = tool_calls.entry(call.index).or_default();
            if let Some(id) = call.id {
                partial.id = Some(id);
            }
            let Some(function) = call.function else {
                continue;
            };
            if let Some(name) = function.name {
                partial.name = Some(name);
            }
            if let Some(arguments) = function.arguments {
                partial.arguments.push_str(&arguments);
            }
        }
    }
    Ok(())
}

fn finalize_tool_calls(tool_calls: BTreeMap<usize, PartialToolCall>) -> Result<Vec<ToolCall>> {
    tool_calls
        .into_values()
        .map(|call| {
            let id = call.id.ok_or_else(|| {
                ReshapeError::Provider("streamed OpenAI tool call missing id".to_string())
            })?;
            let name = call.name.ok_or_else(|| {
                ReshapeError::Provider(
                    "streamed OpenAI tool call missing function name".to_string(),
                )
            })?;
            let arguments = serde_json::from_str(&call.arguments).map_err(|error| {
                ReshapeError::Provider(format!(
                    "invalid OpenAI streamed tool call arguments for {name}: {error}"
                ))
            })?;
            Ok(ToolCall {
                id,
                name,
                arguments,
            })
        })
        .collect()
}

pub mod deps;
pub mod state;

use crate::error::{ReshapeError, Result};
use crate::llm::{ChatMessage, ChatOptions};
use crate::protocol::{
    DEFAULT_SESSION_KEY, Envelope, InputEvent, OutputEvent, TurnProgressEvent, TurnProgressKind,
};
use crate::session::Session;
use crate::tools::ToolContext;

pub use deps::RuntimeDeps;
pub use state::RuntimeState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeLimits {
    pub max_tool_iterations: usize,
    pub max_tool_calls: usize,
}

impl Default for RuntimeLimits {
    fn default() -> Self {
        Self {
            max_tool_iterations: 8,
            max_tool_calls: 32,
        }
    }
}

pub struct AgentRuntime {
    deps: RuntimeDeps,
    limits: RuntimeLimits,
}

impl AgentRuntime {
    pub fn new(deps: RuntimeDeps, limits: RuntimeLimits) -> Self {
        Self { deps, limits }
    }

    pub async fn session(&self) -> Result<Session> {
        self.deps.sessions.load().await
    }

    pub async fn reset_session(&self) -> Result<Session> {
        let session = Session::default();
        self.deps.sessions.save(session.clone()).await?;
        Ok(session)
    }

    pub async fn process(&self, event: Envelope<InputEvent>) -> Result<Envelope<OutputEvent>> {
        self.process_with_optional_progress(event, None).await
    }

    pub async fn process_with_progress<F>(
        &self,
        event: Envelope<InputEvent>,
        progress: F,
    ) -> Result<Envelope<OutputEvent>>
    where
        F: FnMut(TurnProgressEvent) + Send,
    {
        let mut progress = progress;
        self.process_with_optional_progress(event, Some(&mut progress))
            .await
    }

    async fn process_with_optional_progress(
        &self,
        event: Envelope<InputEvent>,
        mut progress: Option<&mut (dyn FnMut(TurnProgressEvent) + Send)>,
    ) -> Result<Envelope<OutputEvent>> {
        tracing::debug!(
            message_id = %event.header.message_id,
            trace_id = %event.header.trace_id,
            session_key = %event.header.session_key,
            "runtime received inbound event"
        );
        if event.header.session_key != DEFAULT_SESSION_KEY {
            tracing::warn!(
                expected = DEFAULT_SESSION_KEY,
                actual = %event.header.session_key,
                "runtime rejected invalid session key"
            );
            return Err(ReshapeError::InvalidSessionKey {
                expected: DEFAULT_SESSION_KEY.to_string(),
                actual: event.header.session_key,
            });
        }

        self.deps.telemetry.record_event("inbound_received").await;

        let turn_id = turn_id(&event);
        let mut progress_sequence = 0u32;
        emit_progress(
            &mut progress,
            &turn_id,
            &mut progress_sequence,
            TurnProgressKind::TurnStarted,
            None,
            None,
            None,
            "Agent turn started".to_string(),
        );

        let mut session = self.deps.sessions.load().await?;
        session.record_input(event.payload.clone());
        tracing::debug!(
            turn_index = session.turn_index,
            history_len = session.history.len(),
            "runtime recorded input event"
        );

        let mut messages = vec![
            ChatMessage::system(crate::prompt::system_prompt()),
            ChatMessage::user(input_text(&event.payload)),
        ];
        let tool_context = ToolContext {
            workspace: self.deps.workspace.clone(),
        };
        let tool_defs = self.deps.tools.definitions();
        let mut tool_calls = 0usize;

        for iteration in 0..=self.limits.max_tool_iterations {
            tracing::debug!(
                iteration,
                max_tool_iterations = self.limits.max_tool_iterations,
                message_count = messages.len(),
                tool_count = tool_defs.len(),
                "runtime requesting llm response"
            );
            let response = match self
                .deps
                .llm
                .chat(messages.clone(), tool_defs.clone(), ChatOptions::default())
                .await
            {
                Ok(response) => response,
                Err(error) => {
                    emit_turn_failed(&mut progress, &turn_id, &mut progress_sequence, &error);
                    return Err(error);
                }
            };
            tracing::debug!(
                iteration,
                tool_call_count = response.tool_calls.len(),
                content_len = response.content.len(),
                "runtime received llm response"
            );

            if !response.content.trim().is_empty() {
                emit_progress(
                    &mut progress,
                    &turn_id,
                    &mut progress_sequence,
                    TurnProgressKind::AssistantMessage,
                    None,
                    None,
                    Some(preview_text(&response.content)),
                    "Assistant message received".to_string(),
                );
            }

            messages.push(ChatMessage::assistant_with_tool_calls(
                response.content.clone(),
                response.tool_calls.clone(),
            ));

            if response.tool_calls.is_empty() {
                let result_preview = preview_text(&response.content);
                let output = OutputEvent::FinalMessage {
                    text: response.content,
                };
                session.record_output(output.clone());
                self.deps.sessions.save(session).await?;
                emit_progress(
                    &mut progress,
                    &turn_id,
                    &mut progress_sequence,
                    TurnProgressKind::TurnCompleted,
                    None,
                    None,
                    Some(result_preview),
                    "Agent turn completed".to_string(),
                );
                tracing::info!("runtime completed turn with final message");
                return Ok(Envelope::for_session(DEFAULT_SESSION_KEY, output));
            }

            if iteration == self.limits.max_tool_iterations {
                tracing::warn!(
                    iteration,
                    max_tool_iterations = self.limits.max_tool_iterations,
                    "runtime exceeded max tool iterations"
                );
                let error =
                    ReshapeError::ToolBudgetExceeded("max tool iterations reached".to_string());
                emit_turn_failed(&mut progress, &turn_id, &mut progress_sequence, &error);
                return Err(error);
            }

            for call in response.tool_calls {
                tool_calls += 1;
                if tool_calls > self.limits.max_tool_calls {
                    tracing::warn!(
                        tool_calls,
                        max_tool_calls = self.limits.max_tool_calls,
                        "runtime exceeded max tool calls"
                    );
                    let error =
                        ReshapeError::ToolBudgetExceeded("max tool calls reached".to_string());
                    emit_turn_failed(&mut progress, &turn_id, &mut progress_sequence, &error);
                    return Err(error);
                }

                tracing::debug!(
                    tool_call_id = %call.id,
                    tool_name = %call.name,
                    tool_calls,
                    "runtime executing tool call"
                );
                let arguments_preview = preview_value(&call.arguments);
                emit_progress(
                    &mut progress,
                    &turn_id,
                    &mut progress_sequence,
                    TurnProgressKind::ToolStarted,
                    Some(call.name.clone()),
                    Some(arguments_preview.clone()),
                    None,
                    format!("Running {}", call.name),
                );
                let result = match self
                    .deps
                    .tools
                    .execute(&call.name, call.arguments, &tool_context)
                    .await
                {
                    Ok(result) => result,
                    Err(error) => {
                        emit_progress(
                            &mut progress,
                            &turn_id,
                            &mut progress_sequence,
                            TurnProgressKind::ToolFailed,
                            Some(call.name.clone()),
                            Some(arguments_preview),
                            Some(preview_text(&error.to_string())),
                            format!("{} failed", call.name),
                        );
                        emit_turn_failed(&mut progress, &turn_id, &mut progress_sequence, &error);
                        return Err(error);
                    }
                };
                tracing::debug!(
                    tool_name = %call.name,
                    should_continue = result.should_continue,
                    content_len = result.content_for_model.len(),
                    "runtime received tool result"
                );
                messages.push(ChatMessage::tool_for_call(
                    call.id.clone(),
                    result.content_for_model.clone(),
                ));
                emit_progress(
                    &mut progress,
                    &turn_id,
                    &mut progress_sequence,
                    TurnProgressKind::ToolFinished,
                    Some(call.name.clone()),
                    Some(arguments_preview),
                    Some(preview_text(&result.content_for_model)),
                    format!("{} finished", call.name),
                );

                if !result.should_continue {
                    let result_preview = preview_text(&result.content_for_model);
                    let output = OutputEvent::Completed {
                        summary: result.content_for_model,
                    };
                    session.record_output(output.clone());
                    self.deps.sessions.save(session).await?;
                    emit_progress(
                        &mut progress,
                        &turn_id,
                        &mut progress_sequence,
                        TurnProgressKind::TurnCompleted,
                        None,
                        None,
                        Some(result_preview),
                        "Agent turn completed".to_string(),
                    );
                    tracing::info!(
                        tool_name = %call.name,
                        "runtime completed turn via tool signal"
                    );
                    return Ok(Envelope::for_session(DEFAULT_SESSION_KEY, output));
                }
            }
        }

        tracing::error!("runtime tool loop ended unexpectedly");
        let error = ReshapeError::ToolBudgetExceeded("tool loop ended unexpectedly".to_string());
        emit_turn_failed(&mut progress, &turn_id, &mut progress_sequence, &error);
        Err(error)
    }
}

fn turn_id(event: &Envelope<InputEvent>) -> String {
    event
        .metadata
        .get("jsonrpc_id")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| event.header.message_id.to_string())
}

fn emit_turn_failed(
    progress: &mut Option<&mut (dyn FnMut(TurnProgressEvent) + Send)>,
    turn_id: &str,
    sequence: &mut u32,
    error: &ReshapeError,
) {
    emit_progress(
        progress,
        turn_id,
        sequence,
        TurnProgressKind::TurnFailed,
        None,
        None,
        Some(preview_text(&error.to_string())),
        "Agent turn failed".to_string(),
    );
}

#[allow(clippy::too_many_arguments)]
fn emit_progress(
    progress: &mut Option<&mut (dyn FnMut(TurnProgressEvent) + Send)>,
    turn_id: &str,
    sequence: &mut u32,
    kind: TurnProgressKind,
    tool_name: Option<String>,
    arguments_preview: Option<String>,
    result_preview: Option<String>,
    message: String,
) {
    let Some(progress) = progress.as_deref_mut() else {
        return;
    };
    *sequence += 1;
    progress(TurnProgressEvent {
        turn_id: turn_id.to_string(),
        sequence: *sequence,
        kind,
        tool_name,
        arguments_preview,
        result_preview,
        message,
    });
}

fn preview_value(value: &serde_json::Value) -> String {
    serde_json::to_string(&redact_preview_value(value))
        .map(|text| preview_text(&text))
        .unwrap_or_else(|_| "<unserializable arguments>".to_string())
}

fn redact_preview_value(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => serde_json::Value::Object(
            map.iter()
                .map(|(key, value)| {
                    let value = if key == "content" {
                        value.as_str().map_or_else(
                            || redact_preview_value(value),
                            |text| serde_json::Value::String(format!("<{} chars>", text.len())),
                        )
                    } else {
                        redact_preview_value(value)
                    };
                    (key.clone(), value)
                })
                .collect(),
        ),
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.iter().map(redact_preview_value).collect())
        }
        serde_json::Value::String(text) => serde_json::Value::String(preview_text(text)),
        _ => value.clone(),
    }
}

fn preview_text(text: &str) -> String {
    const MAX_CHARS: usize = 500;
    let mut preview: String = text.chars().take(MAX_CHARS).collect();
    if text.chars().count() > MAX_CHARS {
        preview.push_str("...");
    }
    preview
}

fn input_text(event: &InputEvent) -> String {
    match event {
        InputEvent::UserText { text, .. } => text.clone(),
        InputEvent::CdpUserEvent { event } => {
            format!("CDP event: {}", event.event_type)
        }
        InputEvent::PluginMessage { text } => text.clone(),
        InputEvent::WorkspaceChanged { path } => {
            format!("Workspace changed: {}", path.to_string_lossy())
        }
    }
}

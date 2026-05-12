pub mod deps;
pub mod state;

use crate::error::{ReshapeError, Result};
use crate::llm::{ChatMessage, ChatOptions};
use crate::protocol::{DEFAULT_SESSION_KEY, Envelope, InputEvent, OutputEvent};
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

    pub async fn process(&self, event: Envelope<InputEvent>) -> Result<Envelope<OutputEvent>> {
        if event.header.session_key != DEFAULT_SESSION_KEY {
            return Err(ReshapeError::InvalidSessionKey {
                expected: DEFAULT_SESSION_KEY.to_string(),
                actual: event.header.session_key,
            });
        }

        self.deps.telemetry.record_event("inbound_received").await;

        let mut session = self.deps.sessions.load().await?;
        session.record_input(event.payload.clone());

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
            let response = self
                .deps
                .llm
                .chat(messages.clone(), tool_defs.clone(), ChatOptions::default())
                .await?;

            messages.push(ChatMessage::assistant(response.content.clone()));

            if response.tool_calls.is_empty() {
                let output = OutputEvent::FinalMessage {
                    text: response.content,
                };
                session.record_output(output.clone());
                self.deps.sessions.save(session).await?;
                return Ok(Envelope::for_session(DEFAULT_SESSION_KEY, output));
            }

            if iteration == self.limits.max_tool_iterations {
                return Err(ReshapeError::ToolBudgetExceeded(
                    "max tool iterations reached".to_string(),
                ));
            }

            for call in response.tool_calls {
                tool_calls += 1;
                if tool_calls > self.limits.max_tool_calls {
                    return Err(ReshapeError::ToolBudgetExceeded(
                        "max tool calls reached".to_string(),
                    ));
                }

                let result = self
                    .deps
                    .tools
                    .execute(&call.name, call.arguments, &tool_context)
                    .await?;
                messages.push(ChatMessage::tool(result.content_for_model.clone()));

                if !result.should_continue {
                    let output = OutputEvent::Completed {
                        summary: result.content_for_model,
                    };
                    session.record_output(output.clone());
                    self.deps.sessions.save(session).await?;
                    return Ok(Envelope::for_session(DEFAULT_SESSION_KEY, output));
                }
            }
        }

        Err(ReshapeError::ToolBudgetExceeded(
            "tool loop ended unexpectedly".to_string(),
        ))
    }
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

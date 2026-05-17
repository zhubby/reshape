use std::sync::Arc;

use async_trait::async_trait;
use reshape_core::error::Result;
use reshape_core::llm::{ChatMessage, ChatOptions, LlmProvider, LlmResponse, ToolCall};
use reshape_core::observability::NoopTelemetry;
use reshape_core::protocol::{
    Envelope, InputEvent, InputSource, OutputEvent, TurnProgressEvent, TurnProgressKind,
};
use reshape_core::runtime::{AgentRuntime, RuntimeDeps, RuntimeLimits};
use reshape_core::session::store::InMemorySessionStore;
use reshape_core::tools::InMemoryToolRegistry;
use reshape_core::tools::complete::CompleteTaskTool;
use reshape_core::tools::file::FileTool;
use reshape_core::workspace::local::LocalWorkspace;
use tokio::sync::Mutex;

fn tool_runtime(provider: ScriptedLlmProvider, workspace: Arc<LocalWorkspace>) -> AgentRuntime {
    let tools = InMemoryToolRegistry::new()
        .register(FileTool::write_file())
        .register(CompleteTaskTool);

    AgentRuntime::new(
        RuntimeDeps {
            llm: Arc::new(provider),
            tools: Arc::new(tools),
            sessions: Arc::new(InMemorySessionStore::default()),
            workspace,
            telemetry: Arc::new(NoopTelemetry),
        },
        RuntimeLimits {
            max_tool_iterations: 4,
            max_tool_calls: 4,
        },
    )
}

#[tokio::test]
async fn tool_loop_writes_file_then_completes() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = Arc::new(LocalWorkspace::new(dir.path()).unwrap());
    let provider = ScriptedLlmProvider::new([
        LlmResponse {
            content: "Writing page".to_string(),
            tool_calls: vec![ToolCall {
                id: "call-write".to_string(),
                name: "write_file".to_string(),
                arguments: serde_json::json!({
                    "path": "index.html",
                    "content": "<h1>Hello</h1>"
                }),
            }],
        },
        LlmResponse {
            content: "Completing".to_string(),
            tool_calls: vec![ToolCall {
                id: "call-complete".to_string(),
                name: "complete_task".to_string(),
                arguments: serde_json::json!({
                    "summary": "Created index.html"
                }),
            }],
        },
    ]);
    let runtime = tool_runtime(provider, workspace);

    let output = runtime
        .process(Envelope::new(InputEvent::UserText {
            text: "create a page".to_string(),
            source: InputSource::Test,
        }))
        .await
        .unwrap();

    assert_eq!(
        output.payload,
        OutputEvent::Completed {
            summary: "Created index.html".to_string()
        }
    );
    let html = tokio::fs::read_to_string(dir.path().join("index.html"))
        .await
        .unwrap();
    assert!(html.contains("<h1>Hello</h1>"));
    assert!(html.contains(r#"href="assets/site.css""#));
}

#[tokio::test]
async fn tool_loop_reports_ordered_progress_events() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = Arc::new(LocalWorkspace::new(dir.path()).unwrap());
    let provider = ScriptedLlmProvider::new([
        LlmResponse {
            content: "Writing page".to_string(),
            tool_calls: vec![ToolCall {
                id: "call-write".to_string(),
                name: "write_file".to_string(),
                arguments: serde_json::json!({
                    "path": "index.html",
                    "content": "<h1>Hello</h1>"
                }),
            }],
        },
        LlmResponse {
            content: "Completing".to_string(),
            tool_calls: vec![ToolCall {
                id: "call-complete".to_string(),
                name: "complete_task".to_string(),
                arguments: serde_json::json!({
                    "summary": "Created index.html"
                }),
            }],
        },
    ]);
    let runtime = tool_runtime(provider, workspace);
    let mut progress = Vec::<TurnProgressEvent>::new();

    let output = runtime
        .process_with_progress(
            Envelope::new(InputEvent::UserText {
                text: "create a page".to_string(),
                source: InputSource::Test,
            }),
            |event| progress.push(event),
        )
        .await
        .unwrap();

    assert!(matches!(output.payload, OutputEvent::Completed { .. }));
    assert_eq!(
        progress
            .iter()
            .map(|event| event.kind.clone())
            .collect::<Vec<_>>(),
        vec![
            TurnProgressKind::TurnStarted,
            TurnProgressKind::AssistantMessage,
            TurnProgressKind::ToolStarted,
            TurnProgressKind::ToolFinished,
            TurnProgressKind::AssistantMessage,
            TurnProgressKind::ToolStarted,
            TurnProgressKind::ToolFinished,
            TurnProgressKind::TurnCompleted,
        ]
    );
    assert_eq!(progress[2].tool_name.as_deref(), Some("write_file"));
    assert!(
        progress[2]
            .arguments_preview
            .as_ref()
            .unwrap()
            .contains("index.html")
    );
    assert!(
        progress[2]
            .arguments_preview
            .as_ref()
            .unwrap()
            .contains("<14 chars>")
    );
    assert_eq!(
        progress[3].result_preview.as_deref(),
        Some("wrote index.html")
    );
    assert_eq!(
        progress[7].result_preview.as_deref(),
        Some("Created index.html")
    );
}

#[tokio::test]
async fn tool_loop_stops_when_iteration_budget_is_exceeded() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = Arc::new(LocalWorkspace::new(dir.path()).unwrap());
    let provider = ScriptedLlmProvider::new(std::iter::repeat_n(
        LlmResponse {
            content: "Still writing".to_string(),
            tool_calls: vec![ToolCall {
                id: "call-write".to_string(),
                name: "write_file".to_string(),
                arguments: serde_json::json!({
                    "path": "index.html",
                    "content": "<h1>Loop</h1>"
                }),
            }],
        },
        8,
    ));
    let runtime = AgentRuntime::new(
        RuntimeDeps {
            llm: Arc::new(provider),
            tools: Arc::new(InMemoryToolRegistry::new().register(FileTool::write_file())),
            sessions: Arc::new(InMemorySessionStore::default()),
            workspace,
            telemetry: Arc::new(NoopTelemetry),
        },
        RuntimeLimits {
            max_tool_iterations: 1,
            max_tool_calls: 8,
        },
    );

    let error = runtime
        .process(Envelope::new(InputEvent::UserText {
            text: "loop".to_string(),
            source: InputSource::Test,
        }))
        .await
        .unwrap_err();

    assert!(error.to_string().contains("max tool iterations"));
}

#[derive(Debug)]
struct ScriptedLlmProvider {
    responses: Mutex<std::collections::VecDeque<LlmResponse>>,
}

impl ScriptedLlmProvider {
    fn new(responses: impl IntoIterator<Item = LlmResponse>) -> Self {
        Self {
            responses: Mutex::new(responses.into_iter().collect()),
        }
    }
}

#[async_trait]
impl LlmProvider for ScriptedLlmProvider {
    fn name(&self) -> &str {
        "scripted"
    }

    fn default_model(&self) -> &str {
        "scripted-model"
    }

    async fn chat(
        &self,
        _messages: Vec<ChatMessage>,
        _tools: Vec<reshape_core::tools::types::ToolDefinition>,
        _options: ChatOptions,
    ) -> Result<LlmResponse> {
        self.responses.lock().await.pop_front().ok_or_else(|| {
            reshape_core::error::ReshapeError::Provider("no scripted response".to_string())
        })
    }
}

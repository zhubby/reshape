use std::sync::Arc;

use reshape_core::llm::mock::MockLlmProvider;
use reshape_core::llm::{LlmResponse, ToolCall};
use reshape_core::observability::NoopTelemetry;
use reshape_core::protocol::{Envelope, InputEvent, InputSource, OutputEvent};
use reshape_core::runtime::{AgentRuntime, RuntimeDeps, RuntimeLimits};
use reshape_core::session::store::InMemorySessionStore;
use reshape_core::tools::InMemoryToolRegistry;
use reshape_core::tools::complete::CompleteTaskTool;
use reshape_core::tools::file::FileTool;
use reshape_core::workspace::local::LocalWorkspace;

fn tool_runtime(provider: MockLlmProvider, workspace: Arc<LocalWorkspace>) -> AgentRuntime {
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
    let provider = MockLlmProvider::new([
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
    assert_eq!(
        tokio::fs::read_to_string(dir.path().join("index.html"))
            .await
            .unwrap(),
        "<h1>Hello</h1>"
    );
}

#[tokio::test]
async fn tool_loop_stops_when_iteration_budget_is_exceeded() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = Arc::new(LocalWorkspace::new(dir.path()).unwrap());
    let provider = MockLlmProvider::new(std::iter::repeat_n(
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

use std::sync::Arc;

use reshape::llm::mock::MockLlmProvider;
use reshape::observability::NoopTelemetry;
use reshape::protocol::{Envelope, InputEvent, InputSource, OutputEvent};
use reshape::runtime::{AgentRuntime, RuntimeDeps, RuntimeLimits};
use reshape::session::store::{InMemorySessionStore, SessionStore};
use reshape::tools::InMemoryToolRegistry;
use reshape::tools::complete::CompleteTaskTool;
use reshape::workspace::local::LocalWorkspace;

fn runtime(
    provider: MockLlmProvider,
    tools: InMemoryToolRegistry,
    workspace: Arc<LocalWorkspace>,
    sessions: Arc<InMemorySessionStore>,
) -> AgentRuntime {
    AgentRuntime::new(
        RuntimeDeps {
            llm: Arc::new(provider),
            tools: Arc::new(tools),
            sessions,
            workspace,
            telemetry: Arc::new(NoopTelemetry),
        },
        RuntimeLimits::default(),
    )
}

#[tokio::test]
async fn natural_language_message_produces_final_output() {
    let dir = tempfile::tempdir().unwrap();
    let sessions = Arc::new(InMemorySessionStore::default());
    let runtime = runtime(
        MockLlmProvider::default(),
        InMemoryToolRegistry::new(),
        Arc::new(LocalWorkspace::new(dir.path()).unwrap()),
        sessions.clone(),
    );

    let output = runtime
        .process(Envelope::new(InputEvent::UserText {
            text: "hello".to_string(),
            source: InputSource::Test,
        }))
        .await
        .unwrap();

    assert_eq!(
        output.payload,
        OutputEvent::FinalMessage {
            text: "Mock response complete".to_string()
        }
    );
    assert_eq!(sessions.load().await.unwrap().turn_index, 1);
}

#[tokio::test]
async fn consecutive_messages_increment_single_session_turns() {
    let dir = tempfile::tempdir().unwrap();
    let sessions = Arc::new(InMemorySessionStore::default());
    let runtime = runtime(
        MockLlmProvider::default(),
        InMemoryToolRegistry::new(),
        Arc::new(LocalWorkspace::new(dir.path()).unwrap()),
        sessions.clone(),
    );

    for text in ["first", "second"] {
        runtime
            .process(Envelope::new(InputEvent::UserText {
                text: text.to_string(),
                source: InputSource::Test,
            }))
            .await
            .unwrap();
    }

    assert_eq!(sessions.load().await.unwrap().turn_index, 2);
}

#[tokio::test]
async fn unknown_tool_error_is_reported_without_panic() {
    let dir = tempfile::tempdir().unwrap();
    let sessions = Arc::new(InMemorySessionStore::default());
    let provider = MockLlmProvider::new([reshape::llm::LlmResponse {
        content: "Need a tool".to_string(),
        tool_calls: vec![reshape::llm::ToolCall {
            id: "call-1".to_string(),
            name: "missing_tool".to_string(),
            arguments: serde_json::json!({}),
        }],
    }]);
    let runtime = runtime(
        provider,
        InMemoryToolRegistry::new().register(CompleteTaskTool),
        Arc::new(LocalWorkspace::new(dir.path()).unwrap()),
        sessions,
    );

    let error = runtime
        .process(Envelope::new(InputEvent::UserText {
            text: "trigger tool".to_string(),
            source: InputSource::Test,
        }))
        .await
        .unwrap_err();

    assert!(error.to_string().contains("unknown tool"));
}

#[tokio::test]
async fn runtime_rejects_non_local_session_key() {
    let dir = tempfile::tempdir().unwrap();
    let sessions = Arc::new(InMemorySessionStore::default());
    let runtime = runtime(
        MockLlmProvider::default(),
        InMemoryToolRegistry::new(),
        Arc::new(LocalWorkspace::new(dir.path()).unwrap()),
        sessions,
    );

    let error = runtime
        .process(Envelope::for_session(
            "plugin:other",
            InputEvent::UserText {
                text: "hello".to_string(),
                source: InputSource::Test,
            },
        ))
        .await
        .unwrap_err();

    assert!(error.to_string().contains("invalid session key"));
}

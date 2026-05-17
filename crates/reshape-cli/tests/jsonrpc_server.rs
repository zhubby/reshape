use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;

use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use reshape_browser::{BrowserRenderError, BrowserRenderer};
use reshape_cli::rpc_server::{
    RpcRenderLoop, serve_rpc_listener, serve_rpc_listener_with_render_loop,
};
use reshape_cli::{
    CliArgs, build_runtime_with_provider, build_runtime_with_provider_and_session_store,
};
use reshape_core::error::Result;
use reshape_core::llm::{ChatMessage, ChatOptions, LlmProvider, LlmResponse, ToolCall};
use reshape_core::session::store::FileSessionStore;
use serde_json::Value;
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use tracing_subscriber::fmt::MakeWriter;

#[test]
fn rpc_server_startup_log_includes_listening_address() {
    let addr: SocketAddr = "127.0.0.1:7331".parse().unwrap();
    let logs = Arc::new(StdMutex::new(Vec::new()));
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_writer(CapturedWriter::new(logs.clone()))
        .finish();

    tracing::subscriber::with_default(subscriber, || {
        reshape_cli::rpc_server::log_server_listening(addr);
    });
    let logs = String::from_utf8(logs.lock().unwrap().clone()).unwrap();

    assert!(logs.contains("json-rpc websocket server listening"));
    assert!(logs.contains("127.0.0.1:7331"));
}

#[tokio::test]
async fn websocket_requires_rpc_handshake_before_ping() {
    let (addr, _workspace, task) = spawn_server().await;
    let (mut socket, _) = connect_async(format!("ws://{addr}/v1/rpc")).await.unwrap();

    socket
        .send(Message::Text(
            r#"{"jsonrpc":"2.0","id":"ping-1","method":"reshape.ping","params":{}}"#.into(),
        ))
        .await
        .unwrap();

    let response = next_json(&mut socket).await;

    assert_eq!(response["error"]["code"], -32600);
    assert!(
        response["error"]["message"]
            .as_str()
            .unwrap()
            .contains("rpc handshake required")
    );
    task.abort();
}

#[tokio::test]
async fn websocket_input_runs_agent_turn_after_rpc_handshake() {
    let (addr, workspace, task) = spawn_server().await;
    let (mut socket, _) = connect_async(format!("ws://{addr}/v1/rpc")).await.unwrap();

    send_handshake(&mut socket).await;
    let ack = next_json(&mut socket).await;
    assert_eq!(ack["type"], "reshape.rpc.handshake_ack");
    assert_eq!(ack["sessionKey"], "local:main");

    socket
        .send(Message::Text(
            r#"{
                "jsonrpc": "2.0",
                "id": "turn-1",
                "method": "reshape.input",
                "params": {
                    "input": {
                        "type": "user_text",
                        "text": "create a page"
                    }
                }
            }"#
            .into(),
        ))
        .await
        .unwrap();

    let response = next_response(&mut socket, "turn-1").await;

    assert_eq!(response["jsonrpc"], "2.0");
    assert_eq!(response["id"], "turn-1");
    assert_eq!(response["result"]["output"]["type"], "completed");
    assert_eq!(response["result"]["output"]["summary"], "RPC page created");
    assert_eq!(
        response["result"]["metadata"]["changedFiles"],
        serde_json::json!(["index.html"])
    );
    assert_eq!(
        response["result"]["metadata"]["toolEvents"][0]["kind"],
        "turn_started"
    );
    assert!(workspace.path().join("index.html").exists());
    task.abort();
}

#[tokio::test]
async fn websocket_input_streams_progress_before_final_response() {
    let (addr, _workspace, task) = spawn_server().await;
    let (mut socket, _) = connect_async(format!("ws://{addr}/v1/rpc")).await.unwrap();
    send_handshake(&mut socket).await;
    let _ack = next_json(&mut socket).await;

    socket
        .send(Message::Text(
            r#"{
                "jsonrpc": "2.0",
                "id": "turn-1",
                "method": "reshape.input",
                "params": {
                    "input": {
                        "type": "user_text",
                        "text": "create a page"
                    }
                }
            }"#
            .into(),
        ))
        .await
        .unwrap();

    let first = next_json(&mut socket).await;
    assert_eq!(first["jsonrpc"], "2.0");
    assert_eq!(first["method"], "reshape.progress");
    assert_eq!(first["params"]["turnId"], "turn-1");
    assert_eq!(first["params"]["sequence"], 1);
    assert_eq!(first["params"]["kind"], "turn_started");

    let response = next_response(&mut socket, "turn-1").await;
    let events = response["result"]["metadata"]["toolEvents"]
        .as_array()
        .unwrap();
    assert!(events.iter().any(|event| event["kind"] == "tool_started"));
    assert!(events.iter().any(|event| event["kind"] == "tool_finished"));
    assert!(events.iter().any(|event| event["kind"] == "turn_completed"));
    assert!(events.iter().any(|event| {
        event["toolName"] == "write_file"
            && event["argumentsPreview"]
                .as_str()
                .unwrap()
                .contains("index.html")
    }));
    task.abort();
}

#[tokio::test]
async fn websocket_input_reports_tool_failure_progress_before_error_response() {
    let provider = Arc::new(ScriptedLlmProvider::new([LlmResponse {
        content: "Using a missing tool".to_string(),
        tool_calls: vec![ToolCall {
            id: "missing".to_string(),
            name: "missing_tool".to_string(),
            arguments: serde_json::json!({"path": "index.html"}),
        }],
    }]));
    let (addr, _workspace, task) = spawn_server_with_provider(provider).await;
    let (mut socket, _) = connect_async(format!("ws://{addr}/v1/rpc")).await.unwrap();
    send_handshake(&mut socket).await;
    let _ack = next_json(&mut socket).await;

    socket
        .send(Message::Text(
            r#"{
                "jsonrpc": "2.0",
                "id": "turn-1",
                "method": "reshape.input",
                "params": {
                    "input": {
                        "type": "user_text",
                        "text": "create a page"
                    }
                }
            }"#
            .into(),
        ))
        .await
        .unwrap();

    let mut saw_tool_failed = false;
    loop {
        let value = next_json(&mut socket).await;
        if value["method"] == "reshape.progress" && value["params"]["kind"] == "tool_failed" {
            saw_tool_failed = true;
            assert_eq!(value["params"]["toolName"], "missing_tool");
        }
        if value["id"] == "turn-1" {
            assert!(saw_tool_failed);
            assert_eq!(value["error"]["code"], -32000);
            assert!(
                value["error"]["message"]
                    .as_str()
                    .unwrap()
                    .contains("unknown tool")
            );
            break;
        }
    }
    task.abort();
}

#[tokio::test]
async fn completed_rpc_turn_refreshes_browser_preview() {
    let renderer = Arc::new(FakeBrowserRenderer::default());
    let events = renderer.events.clone();
    let (addr, _workspace, task) =
        spawn_server_with_renderer(renderer, "http://127.0.0.1:7331/").await;
    let response = send_create_page_turn(addr).await;

    assert_eq!(response["result"]["output"]["type"], "completed");
    assert_eq!(
        response["result"]["metadata"]["changedFiles"],
        serde_json::json!(["index.html"])
    );
    assert_eq!(
        response["result"]["metadata"]["renderedUrl"],
        "http://127.0.0.1:7331/"
    );
    assert_eq!(events.lock().unwrap().as_slice(), ["reload"]);
    task.abort();
}

#[tokio::test]
async fn completed_rpc_turn_falls_back_to_open_url_when_reload_fails() {
    let renderer = Arc::new(FakeBrowserRenderer::reload_fails());
    let events = renderer.events.clone();
    let (addr, _workspace, task) =
        spawn_server_with_renderer(renderer, "http://127.0.0.1:7331/").await;
    let response = send_create_page_turn(addr).await;

    assert_eq!(
        response["result"]["metadata"]["changedFiles"],
        serde_json::json!(["index.html"])
    );
    assert_eq!(
        response["result"]["metadata"]["renderedUrl"],
        "http://127.0.0.1:7331/"
    );
    assert_eq!(
        events.lock().unwrap().as_slice(),
        ["reload", "open:http://127.0.0.1:7331/"]
    );
    task.abort();
}

#[tokio::test]
async fn render_failure_is_reported_as_metadata_warning() {
    let renderer = Arc::new(FakeBrowserRenderer::all_rendering_fails());
    let (addr, _workspace, task) =
        spawn_server_with_renderer(renderer, "http://127.0.0.1:7331/").await;
    let response = send_create_page_turn(addr).await;

    assert_eq!(response["result"]["output"]["type"], "completed");
    assert_eq!(
        response["result"]["metadata"]["changedFiles"],
        serde_json::json!(["index.html"])
    );
    assert!(
        response["result"]["metadata"]["renderWarning"]
            .as_str()
            .unwrap()
            .contains("browser render failed")
    );
    task.abort();
}

#[tokio::test]
async fn completed_rpc_turn_without_index_does_not_refresh_browser() {
    let renderer = Arc::new(FakeBrowserRenderer::default());
    let events = renderer.events.clone();
    let provider = Arc::new(ScriptedLlmProvider::new([LlmResponse {
        content: "Done".to_string(),
        tool_calls: vec![ToolCall {
            id: "complete".to_string(),
            name: "complete_task".to_string(),
            arguments: serde_json::json!({"summary": "Nothing to render"}),
        }],
    }]));
    let (addr, _workspace, task) = spawn_server_with_provider_and_render_loop(
        provider,
        Some(RpcRenderLoop {
            browser: renderer,
            local_url: "http://127.0.0.1:7331/".to_string(),
        }),
    )
    .await;
    let response = send_create_page_turn(addr).await;

    assert_eq!(response["result"]["output"]["type"], "completed");
    assert!(response["result"]["metadata"]["renderedUrl"].is_null());
    assert!(events.lock().unwrap().is_empty());
    task.abort();
}

#[tokio::test]
async fn websocket_rejects_wrong_path() {
    let (addr, _workspace, task) = spawn_server().await;

    let result = connect_async(format!("ws://{addr}/wrong")).await;

    assert!(result.is_err());
    task.abort();
}

#[tokio::test]
async fn websocket_binary_frame_returns_invalid_request_error() {
    let (addr, _workspace, task) = spawn_server().await;
    let (mut socket, _) = connect_async(format!("ws://{addr}/v1/rpc")).await.unwrap();

    socket.send(Message::Binary(vec![1, 2, 3])).await.unwrap();

    let response = next_json(&mut socket).await;

    assert_eq!(response["error"]["code"], -32600);
    task.abort();
}

#[tokio::test]
async fn plugin_path_is_not_a_separate_websocket_endpoint() {
    let (addr, _workspace, task) = spawn_server().await;

    let result = connect_async(format!("ws://{addr}/v1/plugin")).await;

    assert!(result.is_err());
    task.abort();
}

#[tokio::test]
async fn rpc_websocket_accepts_ping_after_rpc_handshake() {
    let (addr, _workspace, task) = spawn_server().await;
    let (mut socket, _) = connect_async(format!("ws://{addr}/v1/rpc")).await.unwrap();

    send_handshake(&mut socket).await;
    let ack = next_json(&mut socket).await;
    assert_eq!(ack["type"], "reshape.rpc.handshake_ack");
    assert_eq!(ack["sessionKey"], "local:main");

    socket
        .send(Message::Text(
            r#"{"jsonrpc":"2.0","id":"ping-1","method":"reshape.ping","params":{}}"#.into(),
        ))
        .await
        .unwrap();

    let response = next_json(&mut socket).await;

    assert_eq!(response["jsonrpc"], "2.0");
    assert_eq!(response["id"], "ping-1");
    assert_eq!(response["result"]["ok"], true);
    assert_eq!(response["result"]["schemaVersion"], "1.0");
    task.abort();
}

#[tokio::test]
async fn rpc_history_returns_messages_after_agent_turn() {
    let (addr, _workspace, task) = spawn_server().await;
    let (mut socket, _) = connect_async(format!("ws://{addr}/v1/rpc")).await.unwrap();
    send_handshake(&mut socket).await;
    let _ack = next_json(&mut socket).await;

    socket
        .send(Message::Text(
            r#"{
                "jsonrpc": "2.0",
                "id": "turn-1",
                "method": "reshape.input",
                "params": {
                    "input": {
                        "type": "user_text",
                        "text": "create a page"
                    }
                }
            }"#
            .into(),
        ))
        .await
        .unwrap();
    let _turn = next_response(&mut socket, "turn-1").await;

    socket
        .send(Message::Text(
            r#"{"jsonrpc":"2.0","id":"history-1","method":"reshape.history","params":{}}"#.into(),
        ))
        .await
        .unwrap();
    let history = next_json(&mut socket).await;

    assert_eq!(history["jsonrpc"], "2.0");
    assert_eq!(history["id"], "history-1");
    assert_eq!(history["result"]["schemaVersion"], "1.0");
    assert_eq!(history["result"]["sessionKey"], "local:main");
    assert_eq!(history["result"]["messages"][0]["role"], "user");
    assert_eq!(history["result"]["messages"][0]["text"], "create a page");
    assert_eq!(history["result"]["messages"][1]["role"], "reshape");
    assert_eq!(history["result"]["messages"][1]["text"], "RPC page created");
    task.abort();
}

#[tokio::test]
async fn rpc_history_survives_runtime_restart_with_file_session_store() {
    let session_dir = tempfile::tempdir().unwrap();
    let session_path = session_dir.path().join("session.json");
    let (addr, _workspace, task) =
        spawn_server_with_file_session_store(scripted_page_provider(), session_path.clone()).await;
    let _turn = send_create_page_turn(addr).await;
    task.abort();

    let (addr, _workspace, task) =
        spawn_server_with_file_session_store(Arc::new(ScriptedLlmProvider::new([])), session_path)
            .await;
    let history = send_history_request(addr).await;

    assert_eq!(history["result"]["messages"][0]["text"], "create a page");
    assert_eq!(history["result"]["messages"][1]["text"], "RPC page created");
    task.abort();
}

#[tokio::test]
async fn root_serves_workspace_index_html() {
    let (addr, workspace, task) = spawn_server().await;
    tokio::fs::write(
        workspace.path().join("index.html"),
        "<!doctype html><title>Reshape</title>",
    )
    .await
    .unwrap();

    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let response = client.get(format!("http://{addr}/")).send().await.unwrap();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let body = response.text().await.unwrap();

    assert!(content_type.starts_with("text/html"));
    assert!(body.contains("<title>Reshape</title>"));
    task.abort();
}

#[tokio::test]
async fn static_route_serves_workspace_asset() {
    let (addr, workspace, task) = spawn_server().await;
    tokio::fs::write(workspace.path().join("style.css"), "body { color: red; }")
        .await
        .unwrap();

    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let response = client
        .get(format!("http://{addr}/style.css"))
        .send()
        .await
        .unwrap();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let body = response.text().await.unwrap();

    assert!(content_type.starts_with("text/css"));
    assert_eq!(body, "body { color: red; }");
    task.abort();
}

#[tokio::test]
async fn static_route_returns_not_found_for_missing_file() {
    let (addr, _workspace, task) = spawn_server().await;

    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let response = client
        .get(format!("http://{addr}/missing.js"))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
    task.abort();
}

async fn spawn_server() -> (SocketAddr, tempfile::TempDir, tokio::task::JoinHandle<()>) {
    spawn_server_with_provider(scripted_page_provider()).await
}

async fn spawn_server_with_renderer(
    renderer: Arc<dyn BrowserRenderer>,
    local_url: &str,
) -> (SocketAddr, tempfile::TempDir, tokio::task::JoinHandle<()>) {
    spawn_server_with_provider_and_render_loop(
        scripted_page_provider(),
        Some(RpcRenderLoop {
            browser: renderer,
            local_url: local_url.to_string(),
        }),
    )
    .await
}

async fn spawn_server_with_provider(
    provider: Arc<dyn LlmProvider>,
) -> (SocketAddr, tempfile::TempDir, tokio::task::JoinHandle<()>) {
    spawn_server_with_provider_and_render_loop(provider, None).await
}

async fn spawn_server_with_provider_and_render_loop(
    provider: Arc<dyn LlmProvider>,
    render_loop: Option<RpcRenderLoop>,
) -> (SocketAddr, tempfile::TempDir, tokio::task::JoinHandle<()>) {
    let workspace = tempfile::tempdir().unwrap();
    let config =
        CliArgs::parse_from(["reshape", "--workspace", workspace.path().to_str().unwrap()])
            .into_config()
            .unwrap();
    let runtime = build_runtime_with_provider(config, provider).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let workspace_root = workspace.path().to_path_buf();
    let task = tokio::spawn(async move {
        match render_loop {
            Some(render_loop) => {
                serve_rpc_listener_with_render_loop(listener, runtime, workspace_root, render_loop)
                    .await
                    .unwrap();
            }
            None => {
                serve_rpc_listener(listener, runtime, workspace_root)
                    .await
                    .unwrap();
            }
        }
    });
    (addr, workspace, task)
}

async fn spawn_server_with_file_session_store(
    provider: Arc<dyn LlmProvider>,
    session_path: std::path::PathBuf,
) -> (SocketAddr, tempfile::TempDir, tokio::task::JoinHandle<()>) {
    let workspace = tempfile::tempdir().unwrap();
    let config =
        CliArgs::parse_from(["reshape", "--workspace", workspace.path().to_str().unwrap()])
            .into_config()
            .unwrap();
    let runtime = build_runtime_with_provider_and_session_store(
        config,
        provider,
        Arc::new(FileSessionStore::new(session_path)),
    )
    .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let workspace_root = workspace.path().to_path_buf();
    let task = tokio::spawn(async move {
        serve_rpc_listener(listener, runtime, workspace_root)
            .await
            .unwrap();
    });
    (addr, workspace, task)
}

fn scripted_page_provider() -> Arc<dyn LlmProvider> {
    Arc::new(ScriptedLlmProvider::new([
        LlmResponse {
            content: "Writing".to_string(),
            tool_calls: vec![ToolCall {
                id: "write".to_string(),
                name: "write_file".to_string(),
                arguments: serde_json::json!({
                    "path": "index.html",
                    "content": "<h1>RPC</h1>"
                }),
            }],
        },
        LlmResponse {
            content: "Done".to_string(),
            tool_calls: vec![ToolCall {
                id: "complete".to_string(),
                name: "complete_task".to_string(),
                arguments: serde_json::json!({"summary": "RPC page created"}),
            }],
        },
    ]))
}

async fn send_create_page_turn(addr: SocketAddr) -> Value {
    let (mut socket, _) = connect_async(format!("ws://{addr}/v1/rpc")).await.unwrap();
    send_handshake(&mut socket).await;
    let _ack = next_json(&mut socket).await;

    socket
        .send(Message::Text(
            r#"{
                "jsonrpc": "2.0",
                "id": "turn-1",
                "method": "reshape.input",
                "params": {
                    "input": {
                        "type": "user_text",
                        "text": "create a page"
                    }
                }
            }"#
            .into(),
        ))
        .await
        .unwrap();

    next_response(&mut socket, "turn-1").await
}

async fn send_history_request(addr: SocketAddr) -> Value {
    let (mut socket, _) = connect_async(format!("ws://{addr}/v1/rpc")).await.unwrap();
    send_handshake(&mut socket).await;
    let _ack = next_json(&mut socket).await;

    socket
        .send(Message::Text(
            r#"{"jsonrpc":"2.0","id":"history-1","method":"reshape.history","params":{}}"#.into(),
        ))
        .await
        .unwrap();

    next_json(&mut socket).await
}

async fn next_json(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> Value {
    let message = socket.next().await.unwrap().unwrap();
    let text = message.to_text().unwrap();
    serde_json::from_str(text).unwrap()
}

async fn next_response(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    id: &str,
) -> Value {
    loop {
        let value = next_json(socket).await;
        if value["id"] == id {
            return value;
        }
    }
}

async fn send_handshake(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) {
    socket
        .send(Message::Text(
            r#"{
                "type": "reshape.rpc.handshake",
                "protocolVersion": "1.0",
                "client": {
                    "name": "reshape-plasmo-extension",
                    "version": "0.1.0"
                },
                "tab": {
                    "id": 123,
                    "url": "http://127.0.0.1:7331/",
                    "title": "Reshape"
                }
            }"#
            .into(),
        ))
        .await
        .unwrap();
}

#[derive(Clone)]
struct CapturedWriter {
    logs: Arc<StdMutex<Vec<u8>>>,
}

impl CapturedWriter {
    fn new(logs: Arc<StdMutex<Vec<u8>>>) -> Self {
        Self { logs }
    }
}

impl<'a> MakeWriter<'a> for CapturedWriter {
    type Writer = CapturedLogWriter;

    fn make_writer(&'a self) -> Self::Writer {
        CapturedLogWriter {
            logs: self.logs.clone(),
        }
    }
}

struct CapturedLogWriter {
    logs: Arc<StdMutex<Vec<u8>>>,
}

impl std::io::Write for CapturedLogWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.logs.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[derive(Debug)]
struct ScriptedLlmProvider {
    responses: Mutex<std::collections::VecDeque<LlmResponse>>,
}

#[derive(Default)]
struct FakeBrowserRenderer {
    events: Arc<StdMutex<Vec<String>>>,
    fail_reload: bool,
    fail_open: bool,
}

impl FakeBrowserRenderer {
    fn reload_fails() -> Self {
        Self {
            fail_reload: true,
            ..Self::default()
        }
    }

    fn all_rendering_fails() -> Self {
        Self {
            fail_reload: true,
            fail_open: true,
            ..Self::default()
        }
    }
}

impl BrowserRenderer for FakeBrowserRenderer {
    fn open_url(&self, url: &str) -> std::result::Result<(), BrowserRenderError> {
        self.events.lock().unwrap().push(format!("open:{url}"));
        if self.fail_open {
            return Err(BrowserRenderError::NotAFile("open-failed".into()));
        }
        Ok(())
    }

    fn open_workspace_entry(
        &self,
        path: &std::path::Path,
    ) -> std::result::Result<(), BrowserRenderError> {
        self.events
            .lock()
            .unwrap()
            .push(format!("open_workspace:{}", path.display()));
        Ok(())
    }

    fn reload(&self) -> std::result::Result<(), BrowserRenderError> {
        self.events.lock().unwrap().push("reload".to_string());
        if self.fail_reload {
            return Err(BrowserRenderError::NotAFile("reload-failed".into()));
        }
        Ok(())
    }

    fn snapshot(&self) -> std::result::Result<(), BrowserRenderError> {
        Ok(())
    }

    fn screenshot(
        &self,
        _path: Option<&std::path::Path>,
    ) -> std::result::Result<(), BrowserRenderError> {
        Ok(())
    }

    fn close(&self) -> std::result::Result<(), BrowserRenderError> {
        Ok(())
    }
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

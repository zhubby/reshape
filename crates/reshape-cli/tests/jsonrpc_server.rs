use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;

use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use reshape_cli::rpc_server::serve_rpc_listener;
use reshape_cli::{CliArgs, build_runtime_with_provider};
use reshape_core::error::Result;
use reshape_core::llm::{ChatMessage, ChatOptions, LlmProvider, LlmResponse, ToolCall};
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

    let response = next_json(&mut socket).await;

    assert_eq!(response["jsonrpc"], "2.0");
    assert_eq!(response["id"], "turn-1");
    assert_eq!(response["result"]["output"]["type"], "completed");
    assert_eq!(response["result"]["output"]["summary"], "RPC page created");
    assert!(workspace.path().join("index.html").exists());
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
    let workspace = tempfile::tempdir().unwrap();
    let config =
        CliArgs::parse_from(["reshape", "--workspace", workspace.path().to_str().unwrap()])
            .into_config()
            .unwrap();
    let runtime = build_runtime_with_provider(
        config,
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
        ])),
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

async fn next_json(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> Value {
    let message = socket.next().await.unwrap().unwrap();
    let text = message.to_text().unwrap();
    serde_json::from_str(text).unwrap()
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

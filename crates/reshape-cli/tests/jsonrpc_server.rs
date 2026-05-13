use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;

use futures_util::{SinkExt, StreamExt};
use reshape_cli::rpc_server::serve_rpc_listener;
use reshape_cli::{CliArgs, build_runtime};
use reshape_core::llm::mock::MockLlmProvider;
use serde_json::Value;
use tokio::net::TcpListener;
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
async fn websocket_ping_returns_jsonrpc_result() {
    let (addr, _workspace, task) = spawn_server().await;
    let (mut socket, _) = connect_async(format!("ws://{addr}/v1/rpc")).await.unwrap();

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
async fn websocket_input_runs_agent_turn_and_returns_completed_output() {
    let (addr, workspace, task) = spawn_server().await;
    let (mut socket, _) = connect_async(format!("ws://{addr}/v1/rpc")).await.unwrap();

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
    assert_eq!(
        response["result"]["output"]["summary"],
        "Mock page generated in index.html"
    );
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

async fn spawn_server() -> (SocketAddr, tempfile::TempDir, tokio::task::JoinHandle<()>) {
    let workspace = tempfile::tempdir().unwrap();
    let config =
        CliArgs::parse_from(["reshape", "--workspace", workspace.path().to_str().unwrap()])
            .into_config()
            .unwrap();
    let runtime = build_runtime(config, Arc::new(MockLlmProvider::default())).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        serve_rpc_listener(listener, runtime).await.unwrap();
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

use std::net::SocketAddr;
use std::sync::Arc;

use reshape_core::config::AppConfig;
use reshape_core::tools::web_fetch::WebFetchTool;
use reshape_core::tools::web_search::WebSearchTool;
use reshape_core::tools::{Tool, ToolContext, ToolResult};
use reshape_core::workspace::local::LocalWorkspace;
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

fn model_json(result: &ToolResult) -> Value {
    serde_json::from_str(&result.content_for_model).expect("tool output should be JSON")
}

#[tokio::test]
async fn tavily_search_sends_expected_request_and_returns_structured_json() {
    let (addr, received, task) = spawn_http_server(
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\n\r\n",
        r#"{
            "answer": "Rust is a systems language.",
            "response_time": 0.12,
            "request_id": "req-1",
            "results": [{
                "title": "Rust",
                "url": "https://www.rust-lang.org/",
                "content": "Rust language",
                "score": 0.9,
                "favicon": "https://www.rust-lang.org/favicon.ico"
            }],
            "images": ["https://example.test/rust.png"],
            "usage": {"searches": 1}
        }"#,
    )
    .await;
    let mut config = AppConfig::default();
    config.tools.web_search.enabled = true;
    config.tools.web_search.tavily.api_key = "tvly-test".to_string();
    config.tools.web_search.tavily.base_url = format!("http://{addr}");
    let tool = WebSearchTool::new(config.tools.web_search.clone()).unwrap();

    let result = tool
        .execute(
            serde_json::json!({
                "query": "rust language",
                "max_results": 3,
                "include_answer": true,
                "include_images": true
            }),
            &ToolContext {
                workspace: Arc::new(
                    LocalWorkspace::new(tempfile::tempdir().unwrap().path()).unwrap(),
                ),
            },
        )
        .await
        .unwrap();
    let json = model_json(&result);
    let request = task.await.unwrap();

    assert!(request.starts_with("POST /search HTTP/1.1"));
    assert!(request.contains("authorization: Bearer tvly-test"));
    assert!(request.contains(r#""query":"rust language""#));
    assert!(request.contains(r#""max_results":3"#));
    assert_eq!(received, addr);
    assert_eq!(json["tool"], "web_search");
    assert_eq!(json["provider"], "tavily");
    assert_eq!(json["success"], true);
    assert_eq!(json["answer"], "Rust is a systems language.");
    assert_eq!(json["results"][0]["title"], "Rust");
    assert_eq!(json["images"][0], "https://example.test/rust.png");
    assert_eq!(json["request_id"], "req-1");
}

#[tokio::test]
async fn tavily_search_http_error_returns_recoverable_structured_error() {
    let (addr, _received, _task) = spawn_http_server(
        "HTTP/1.1 429 Too Many Requests\r\ncontent-type: application/json\r\n\r\n",
        r#"{"error":"rate limited"}"#,
    )
    .await;
    let mut config = AppConfig::default();
    config.tools.web_search.enabled = true;
    config.tools.web_search.tavily.api_key = "tvly-test".to_string();
    config.tools.web_search.tavily.base_url = format!("http://{addr}");
    let tool = WebSearchTool::new(config.tools.web_search.clone()).unwrap();

    let result = tool
        .execute(
            serde_json::json!({"query": "rust"}),
            &ToolContext {
                workspace: Arc::new(
                    LocalWorkspace::new(tempfile::tempdir().unwrap().path()).unwrap(),
                ),
            },
        )
        .await
        .unwrap();
    let json = model_json(&result);

    assert_eq!(json["tool"], "web_search");
    assert_eq!(json["success"], false);
    assert_eq!(json["status"], 429);
    assert_eq!(json["recoverable"], true);
}

#[tokio::test]
async fn web_fetch_downloads_media_to_workspace_with_metadata() {
    let png = vec![0x89, b'P', b'N', b'G', 1, 2, 3];
    let (addr, _received, _task) = spawn_binary_server(
        "HTTP/1.1 200 OK\r\ncontent-type: image/png\r\n\r\n",
        png.clone(),
    )
    .await;
    let dir = tempfile::tempdir().unwrap();
    let mut config = AppConfig::default();
    config.tools.web_fetch.enabled = true;
    config.tools.web_fetch.ssrf_allowlist = vec!["127.0.0.1/32".to_string()];
    let tool = WebFetchTool::new(config.tools.web_fetch.clone()).unwrap();
    let context = ToolContext {
        workspace: Arc::new(LocalWorkspace::new(dir.path()).unwrap()),
    };

    let result = tool
        .execute(
            serde_json::json!({
                "url": format!("http://{addr}/image.png"),
                "path": "assets/downloads/custom.png"
            }),
            &context,
        )
        .await
        .unwrap();
    let json = model_json(&result);

    assert_eq!(json["tool"], "web_fetch");
    assert_eq!(json["success"], true);
    assert_eq!(json["content_type"], "image/png");
    assert_eq!(json["path"], "assets/downloads/custom.png");
    assert_eq!(json["bytes_written"], png.len());
    assert!(json["sha256"].as_str().unwrap().len() == 64);
    assert_eq!(
        tokio::fs::read(dir.path().join("assets/downloads/custom.png"))
            .await
            .unwrap(),
        png
    );
}

#[tokio::test]
async fn web_fetch_rejects_non_media_content_type() {
    let (addr, _received, _task) = spawn_http_server(
        "HTTP/1.1 200 OK\r\ncontent-type: text/html\r\n\r\n",
        "<html></html>",
    )
    .await;
    let dir = tempfile::tempdir().unwrap();
    let mut config = AppConfig::default();
    config.tools.web_fetch.enabled = true;
    config.tools.web_fetch.ssrf_allowlist = vec!["127.0.0.1/32".to_string()];
    let tool = WebFetchTool::new(config.tools.web_fetch.clone()).unwrap();

    let result = tool
        .execute(
            serde_json::json!({"url": format!("http://{addr}/index.html")}),
            &ToolContext {
                workspace: Arc::new(LocalWorkspace::new(dir.path()).unwrap()),
            },
        )
        .await
        .unwrap();
    let json = model_json(&result);

    assert_eq!(json["tool"], "web_fetch");
    assert_eq!(json["success"], false);
    assert_eq!(json["recoverable"], true);
    assert!(json["error"].as_str().unwrap().contains("content type"));
}

#[tokio::test]
async fn web_fetch_blocks_private_network_without_allowlist() {
    let dir = tempfile::tempdir().unwrap();
    let config = AppConfig::default();
    let tool = WebFetchTool::new(config.tools.web_fetch.clone()).unwrap();

    let result = tool
        .execute(
            serde_json::json!({"url": "http://127.0.0.1/image.png"}),
            &ToolContext {
                workspace: Arc::new(LocalWorkspace::new(dir.path()).unwrap()),
            },
        )
        .await
        .unwrap();
    let json = model_json(&result);

    assert_eq!(json["success"], false);
    assert!(json["error"].as_str().unwrap().contains("SSRF blocked"));
}

async fn spawn_http_server(
    headers: &'static str,
    body: &'static str,
) -> (SocketAddr, SocketAddr, tokio::task::JoinHandle<String>) {
    spawn_binary_server(headers, body.as_bytes().to_vec()).await
}

async fn spawn_binary_server(
    headers: &'static str,
    body: Vec<u8>,
) -> (SocketAddr, SocketAddr, tokio::task::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buf = vec![0; 8192];
        let n = stream.read(&mut buf).await.unwrap();
        let request = String::from_utf8_lossy(&buf[..n]).to_string();
        let mut response = headers.as_bytes().to_vec();
        response.extend_from_slice(&body);
        stream.write_all(&response).await.unwrap();
        request
    });
    (addr, addr, task)
}

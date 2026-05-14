use std::collections::BTreeMap;
use std::sync::Arc;

use reshape_core::config::OpenAiConfig;
use reshape_core::llm::{ChatMessage, ChatOptions, LlmProvider, OpenAiChatCompletionProvider};
use reshape_core::tools::types::ToolDefinition;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Mutex;

#[tokio::test]
async fn non_streaming_request_maps_messages_tools_and_headers() {
    let (base_url, requests, task) = spawn_server([http_json(
        200,
        json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": "hello"
                }
            }]
        }),
    )])
    .await;
    let provider = provider(base_url, false).unwrap();

    let response = provider
        .chat(
            vec![ChatMessage::system("system"), ChatMessage::user("user")],
            vec![weather_tool()],
            ChatOptions {
                model: Some("override-model".to_string()),
            },
        )
        .await
        .unwrap();

    assert_eq!(response.content, "hello");
    assert!(response.tool_calls.is_empty());

    let requests = requests.lock().await;
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.path, "/v1/chat/completions");
    assert_eq!(request.headers["authorization"], "Bearer test-key");
    assert_eq!(request.body["model"], "override-model");
    assert_eq!(request.body["stream"], false);
    assert_eq!(request.body["messages"][0]["role"], "system");
    assert_eq!(request.body["messages"][1]["content"], "user");
    assert_eq!(request.body["tools"][0]["type"], "function");
    assert_eq!(request.body["tools"][0]["function"]["name"], "get_weather");
    task.abort();
}

#[tokio::test]
async fn non_streaming_tool_call_parses_json_arguments() {
    let (base_url, _requests, task) = spawn_server([http_json(
        200,
        json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_1",
                        "type": "function",
                        "function": {
                            "name": "get_weather",
                            "arguments": "{\"city\":\"Paris\"}"
                        }
                    }]
                }
            }]
        }),
    )])
    .await;
    let provider = provider(base_url, false).unwrap();

    let response = provider
        .chat(
            vec![ChatMessage::user("weather")],
            vec![],
            ChatOptions::default(),
        )
        .await
        .unwrap();

    assert_eq!(response.content, "");
    assert_eq!(response.tool_calls[0].id, "call_1");
    assert_eq!(response.tool_calls[0].name, "get_weather");
    assert_eq!(response.tool_calls[0].arguments, json!({"city": "Paris"}));
    task.abort();
}

#[tokio::test]
async fn streaming_text_chunks_aggregate_into_final_response() {
    let (base_url, requests, task) = spawn_server([http_sse(
        200,
        [
            r#"data: {"choices":[{"delta":{"role":"assistant","content":"hel"}}]}"#,
            r#"data: {"choices":[{"delta":{"content":"lo"}}]}"#,
            "data: [DONE]",
        ]
        .join("\n\n"),
    )])
    .await;
    let provider = provider(base_url, true).unwrap();

    let response = provider
        .chat(
            vec![ChatMessage::user("hello")],
            vec![],
            ChatOptions::default(),
        )
        .await
        .unwrap();

    assert_eq!(response.content, "hello");
    assert!(response.tool_calls.is_empty());
    assert_eq!(requests.lock().await[0].body["stream"], true);
    task.abort();
}

#[tokio::test]
async fn streaming_tool_call_chunks_aggregate_arguments_by_index() {
    let (base_url, _requests, task) = spawn_server([http_sse(
        200,
        [
            r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"write_file","arguments":"{\"path\""}}]}}]}"#,
            r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":":\"index.html\",\"content\":\"hi\"}"}}]}}]}"#,
            "data: [DONE]",
        ]
        .join("\n\n"),
    )])
    .await;
    let provider = provider(base_url, true).unwrap();

    let response = provider
        .chat(
            vec![ChatMessage::user("write")],
            vec![],
            ChatOptions::default(),
        )
        .await
        .unwrap();

    assert_eq!(response.tool_calls.len(), 1);
    assert_eq!(response.tool_calls[0].id, "call_1");
    assert_eq!(response.tool_calls[0].name, "write_file");
    assert_eq!(
        response.tool_calls[0].arguments,
        json!({"path": "index.html", "content": "hi"})
    );
    task.abort();
}

#[tokio::test]
async fn http_error_body_becomes_provider_error() {
    let (base_url, _requests, task) = spawn_server([http_json(
        429,
        json!({"error": {"message": "rate limited"}}),
    )])
    .await;
    let provider = provider(base_url, false).unwrap();

    let error = provider
        .chat(
            vec![ChatMessage::user("hello")],
            vec![],
            ChatOptions::default(),
        )
        .await
        .unwrap_err();

    assert!(error.to_string().contains("429"));
    assert!(error.to_string().contains("rate limited"));
    task.abort();
}

#[test]
fn empty_api_key_is_rejected_before_request() {
    let error = OpenAiChatCompletionProvider::new(OpenAiConfig::default(), "").unwrap_err();

    assert!(error.to_string().contains("must not be empty"));
}

#[tokio::test]
async fn invalid_tool_call_arguments_become_provider_error() {
    let (base_url, _requests, task) = spawn_server([http_json(
        200,
        json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_1",
                        "type": "function",
                        "function": {
                            "name": "get_weather",
                            "arguments": "{not-json"
                        }
                    }]
                }
            }]
        }),
    )])
    .await;
    let provider = provider(base_url, false).unwrap();

    let error = provider
        .chat(
            vec![ChatMessage::user("weather")],
            vec![],
            ChatOptions::default(),
        )
        .await
        .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("invalid OpenAI tool call arguments")
    );
    task.abort();
}

fn provider(
    base_url: String,
    stream: bool,
) -> reshape_core::error::Result<OpenAiChatCompletionProvider> {
    OpenAiChatCompletionProvider::new(
        OpenAiConfig {
            model: "test-model".to_string(),
            base_url,
            stream,
            ..OpenAiConfig::default()
        },
        "test-key",
    )
}

fn weather_tool() -> ToolDefinition {
    ToolDefinition {
        name: "get_weather".to_string(),
        description: "Get the weather".to_string(),
        parameters: json!({
            "type": "object",
            "properties": {
                "city": {"type": "string"}
            },
            "required": ["city"]
        }),
    }
}

#[derive(Debug)]
struct RecordedRequest {
    path: String,
    headers: BTreeMap<String, String>,
    body: Value,
}

#[derive(Debug, Clone)]
struct TestResponse {
    status: u16,
    content_type: &'static str,
    body: String,
}

fn http_json(status: u16, body: Value) -> TestResponse {
    TestResponse {
        status,
        content_type: "application/json",
        body: body.to_string(),
    }
}

fn http_sse(status: u16, body: String) -> TestResponse {
    TestResponse {
        status,
        content_type: "text/event-stream",
        body,
    }
}

async fn spawn_server(
    responses: impl IntoIterator<Item = TestResponse>,
) -> (
    String,
    Arc<Mutex<Vec<RecordedRequest>>>,
    tokio::task::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let responses = Arc::new(Mutex::new(
        responses
            .into_iter()
            .collect::<std::collections::VecDeque<_>>(),
    ));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let task_requests = requests.clone();
    let task = tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            let response = responses.lock().await.pop_front();
            let Some(response) = response else {
                break;
            };

            let mut raw = Vec::new();
            let mut buffer = [0_u8; 1024];
            loop {
                let read = stream.read(&mut buffer).await.unwrap();
                if read == 0 {
                    break;
                }
                raw.extend_from_slice(&buffer[..read]);
                if request_is_complete(&raw) {
                    break;
                }
            }
            task_requests.lock().await.push(parse_request(&raw));
            write_response(&mut stream, response).await;
        }
    });

    (format!("http://{addr}/v1"), requests, task)
}

fn request_is_complete(raw: &[u8]) -> bool {
    let Some(header_end) = find_header_end(raw) else {
        return false;
    };
    let headers = String::from_utf8_lossy(&raw[..header_end]);
    let content_length = headers
        .lines()
        .find_map(|line| line.strip_prefix("Content-Length: "))
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or_default();
    raw.len() >= header_end + 4 + content_length
}

fn parse_request(raw: &[u8]) -> RecordedRequest {
    let header_end = find_header_end(raw).unwrap();
    let headers = String::from_utf8_lossy(&raw[..header_end]);
    let mut lines = headers.lines();
    let request_line = lines.next().unwrap();
    let path = request_line.split_whitespace().nth(1).unwrap().to_string();
    let headers = lines
        .filter_map(|line| {
            let (name, value) = line.split_once(": ")?;
            Some((name.to_ascii_lowercase(), value.to_string()))
        })
        .collect();
    let body = serde_json::from_slice(&raw[header_end + 4..]).unwrap();
    RecordedRequest {
        path,
        headers,
        body,
    }
}

fn find_header_end(raw: &[u8]) -> Option<usize> {
    raw.windows(4).position(|window| window == b"\r\n\r\n")
}

async fn write_response(stream: &mut tokio::net::TcpStream, response: TestResponse) {
    let status_text = match response.status {
        200 => "OK",
        429 => "Too Many Requests",
        _ => "Error",
    };
    let response = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        response.status,
        status_text,
        response.content_type,
        response.body.len(),
        response.body
    );
    stream.write_all(response.as_bytes()).await.unwrap();
}

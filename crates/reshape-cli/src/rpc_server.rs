use axum::Router;
use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::Response;
use axum::routing::get;
use futures_util::{SinkExt, StreamExt};
use reshape_core::error::Result;
use reshape_core::protocol::{DEFAULT_SCHEMA_VERSION, Envelope, InputEvent, OutputEvent};
use reshape_core::runtime::AgentRuntime;
use tokio::net::TcpListener;
use tokio::sync::{mpsc, oneshot};

use crate::rpc_protocol::{
    RpcError, RpcResponse, parse_rpc_handshake, parse_rpc_request, rpc_handshake_ack,
};

const AGENT_QUEUE_CAPACITY: usize = 32;

#[derive(Clone)]
struct RpcServerState {
    agent_tx: mpsc::Sender<AgentRequest>,
}

struct AgentRequest {
    envelope: Envelope<InputEvent>,
    response_tx: oneshot::Sender<std::result::Result<Envelope<OutputEvent>, RpcError>>,
}

pub async fn serve_rpc_listener(listener: TcpListener, runtime: AgentRuntime) -> Result<()> {
    let addr = listener.local_addr()?;
    log_server_listening(addr);
    let app = rpc_router(runtime);
    axum::serve(listener, app).await.map_err(|error| {
        tracing::error!(%error, "json-rpc websocket server stopped with error");
        std::io::Error::other(error).into()
    })
}

pub fn log_server_listening(addr: std::net::SocketAddr) {
    tracing::info!("json-rpc websocket server listening on {addr}");
}

pub fn rpc_router(runtime: AgentRuntime) -> Router {
    let (agent_tx, agent_rx) = mpsc::channel(AGENT_QUEUE_CAPACITY);
    tracing::debug!(
        capacity = AGENT_QUEUE_CAPACITY,
        "starting json-rpc agent worker"
    );
    tokio::spawn(run_agent_worker(runtime, agent_rx));
    Router::new()
        .route("/v1/rpc", get(ws_handler))
        .with_state(RpcServerState { agent_tx })
}

async fn ws_handler(ws: WebSocketUpgrade, State(state): State<RpcServerState>) -> Response {
    tracing::debug!("websocket upgrade accepted for json-rpc endpoint");
    ws.on_upgrade(|socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: RpcServerState) {
    let (mut sender, mut receiver) = socket.split();
    let mut is_handshake_complete = false;
    tracing::debug!("json-rpc websocket connection opened");
    while let Some(message) = receiver.next().await {
        let response = match message {
            Ok(Message::Text(text)) => {
                tracing::debug!(bytes = text.len(), "received json-rpc websocket text frame");
                if !is_handshake_complete {
                    match parse_rpc_handshake(text.as_str()) {
                        Ok(_) => {
                            if sender
                                .send(Message::Text(rpc_handshake_ack_to_text().into()))
                                .await
                                .is_err()
                            {
                                tracing::warn!("failed to send rpc handshake ack");
                                break;
                            }
                            is_handshake_complete = true;
                            tracing::debug!("rpc handshake ack sent on json-rpc websocket");
                            None
                        }
                        Err(error) => Some(RpcResponse::error(None, error)),
                    }
                } else {
                    handle_text_frame(&state, text.as_str()).await
                }
            }
            Ok(Message::Binary(_)) => {
                tracing::debug!("rejecting unsupported json-rpc websocket binary frame");
                Some(RpcResponse::error(
                    None,
                    RpcError::invalid_request("binary frames are not supported"),
                ))
            }
            Ok(Message::Close(frame)) => {
                tracing::debug!(?frame, "json-rpc websocket connection closing");
                break;
            }
            Ok(Message::Ping(_)) | Ok(Message::Pong(_)) => None,
            Err(error) => {
                tracing::warn!(%error, "websocket frame error");
                Some(RpcResponse::error(
                    None,
                    RpcError::server(error.to_string()),
                ))
            }
        };

        let Some(response) = response else {
            continue;
        };
        if sender
            .send(Message::Text(response_to_text(&response).into()))
            .await
            .is_err()
        {
            tracing::warn!("failed to send json-rpc websocket response");
            break;
        }
        tracing::debug!("sent json-rpc websocket response");
    }
    tracing::debug!("json-rpc websocket connection closed");
}

async fn handle_text_frame(state: &RpcServerState, text: &str) -> Option<RpcResponse> {
    let request = match parse_rpc_request(text) {
        Ok(request) => request,
        Err(error) => return Some(RpcResponse::error(None, error)),
    };

    let id = request.id.clone();
    tracing::debug!(method = %request.method, id = %id, "parsed json-rpc request");
    if request.method == "reshape.ping" {
        tracing::debug!(id = %id, "responding to json-rpc ping");
        return Some(ping_response(id));
    }

    let envelope = match request.into_input_envelope() {
        Ok(envelope) => envelope,
        Err(error) => return Some(RpcResponse::error(Some(id), error)),
    };

    let (response_tx, response_rx) = oneshot::channel();
    if state
        .agent_tx
        .send(AgentRequest {
            envelope,
            response_tx,
        })
        .await
        .is_err()
    {
        tracing::error!("agent worker queue is unavailable");
        return Some(RpcResponse::error(
            Some(id.clone()),
            RpcError::server("agent worker is not available"),
        ));
    }
    tracing::debug!(id = %id, "queued json-rpc agent request");

    match response_rx.await {
        Ok(Ok(envelope)) => {
            tracing::debug!(id = %id, "json-rpc agent turn completed");
            Some(RpcResponse::success(id, envelope))
        }
        Ok(Err(error)) => {
            tracing::warn!(id = %id, message = %error.message, "json-rpc agent turn failed");
            Some(RpcResponse::error(Some(id.clone()), error))
        }
        Err(error) => {
            tracing::error!(id = %id, %error, "json-rpc response channel closed");
            Some(RpcResponse::error(
                Some(id),
                RpcError::server(error.to_string()),
            ))
        }
    }
}

async fn run_agent_worker(runtime: AgentRuntime, mut agent_rx: mpsc::Receiver<AgentRequest>) {
    tracing::debug!("json-rpc agent worker started");
    while let Some(request) = agent_rx.recv().await {
        tracing::debug!(
            message_id = %request.envelope.header.message_id,
            trace_id = %request.envelope.header.trace_id,
            "json-rpc agent worker processing request"
        );
        let response = runtime
            .process(request.envelope)
            .await
            .map_err(|error| RpcError::server(error.to_string()));
        if request.response_tx.send(response).is_err() {
            tracing::warn!("json-rpc client dropped before agent response was delivered");
        }
    }
    tracing::warn!("json-rpc agent worker stopped");
}

fn ping_response(id: serde_json::Value) -> RpcResponse {
    RpcResponse::raw_success(
        id,
        serde_json::json!({
            "ok": true,
            "schemaVersion": DEFAULT_SCHEMA_VERSION,
        }),
    )
}

fn response_to_text(response: &RpcResponse) -> String {
    serde_json::to_string(response).unwrap_or_else(|error| {
        format!(
            r#"{{"jsonrpc":"2.0","error":{{"code":-32000,"message":"failed to serialize response: {error}"}}}}"#
        )
    })
}

fn rpc_handshake_ack_to_text() -> String {
    serde_json::to_string(&rpc_handshake_ack()).unwrap_or_else(|error| {
        format!(
            r#"{{"jsonrpc":"2.0","error":{{"code":-32000,"message":"failed to serialize rpc handshake ack: {error}"}}}}"#
        )
    })
}

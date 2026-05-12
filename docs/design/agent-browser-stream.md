# Agent Browser Stream Server

The `native/stream/` module implements a real-time browser streaming system — WebSocket/HTTP server, screencast frame broadcasting, embedded dashboard UI, AI chat integration, and session discovery/proxying.

```
native/stream/
├── mod.rs        — StreamServer core, frame/status broadcast (~486 lines)
├── cdp_loop.rs   — CDP event loop, screencast auto-start/stop (~325 lines)
├── websocket.rs  — WebSocket accept loop, client handler (~338 lines)
├── http.rs       — HTTP handler, dashboard assets, API endpoints (~315 lines)
├── dashboard.rs  — Standalone dashboard server, session proxy (~960 lines)
├── discovery.rs  — Session discovery by socket scanning (~118 lines)
├── chat.rs       — AI chat integration with tool execution (~970 lines)
```

## StreamServer Core (`mod.rs`)

### Key Structs

- **`FrameMetadata`** — Screencast frame metadata from CDP `Page.screencastFrame`:
  - `offset_top`, `page_scale_factor`, `device_width`, `device_height`
  - `scroll_offset_x`, `scroll_offset_y`, `timestamp`

- **`StreamServer`** — Full server state:
  - `port: u16` — Listening port
  - `session_name: String` — Browser session identifier
  - `frame_tx: broadcast::Sender<Value>` — Frame broadcast channel
  - `client_count: Arc<AtomicU64>` — Connected viewer count
  - `client_slot: Arc<RwLock<Option<Arc<CdpClient>>>>` — Shared CDP client (injectable)
  - `cdp_session_id: Option<String>` — Active CDP target session
  - `client_notify: Arc<Notify>` — Notifies CDP loop of client changes
  - `screencasting: bool` — Whether screencast is active
  - `viewport: (u32, u32)` — Current viewport dimensions
  - Cached: `tabs`, `engine`, `last_frame` — For new client initialization
  - `recording: bool` — Recording state flag
  - `shutdown_rx/tx` — Shutdown signaling
  - `_accept_task`, `_cdp_task` — Background task handles

### Server Startup

- **`start(preferred_port, client, session_id)`** — Start with an existing CDP client. Binds TCP listener, spawns accept loop and CDP event loop.
- **`start_without_client(preferred_port, session_name, allow_port_fallback)`** — Start without client (daemon startup). Returns shared slot for later client injection. Supports port fallback if preferred port is occupied.

### Broadcast Methods

- **`broadcast_frame(frame_json)`** / **`broadcast_screencast_frame(base64_data, metadata)`** — Send frame data to all connected clients.
- **`broadcast_status(connected, screencasting, viewport, engine)`** — Send connection/screencasting/recording state updates.
- **`broadcast_error(msg)`** / **`broadcast_console(level, text, args)`** / **`broadcast_page_error(text, line, column)`** — Error and console event broadcasting.
- **`broadcast_command(action, id, params)`** / **`broadcast_result(id, action, success, data, duration_ms)`** — Command execution lifecycle events for dashboard UI.
- **`broadcast_tabs(tabs)`** — Tab list updates with caching for new clients.

### State Mutations

- **`set_viewport(width, height)`** — Updates viewport and restarts screencast.
- **`set_cdp_session_id(session_id)`** — Updates active CDP session.
- **`notify_client_changed()`** — Triggers CDP loop to re-evaluate screencast state.
- **`set_screencasting(active)`** / **`set_recording(active)`** — State flags.

### Security

- **`is_allowed_origin(origin)`** — Only allows:
  - `file://` URLs
  - `localhost` (any port)
  - `127.0.0.1` (any port)
  - `::1` (any port)

- **`shutdown()`** — Stops accept loop and CDP task, releases port.

## CDP Event Loop (`cdp_loop.rs`)

Background task that subscribes to CDP events and broadcasts screencast frames, console output, and page errors in real-time. Auto-starts/stops screencast based on client count.

### Main Loop Structure

```
Outer loop: wait for client changes (client_notify) or shutdown
  When clients > 0 and CDP client exists:
    Subscribe to CDP events
    Start Page.startScreencast (JPEG, quality 80, max viewport size)
    Inner loop:
      Page.screencastFrame → ack frame → broadcast frame data with metadata
      Page.frameNavigated → update tab URL → broadcast URL event
      Runtime.consoleAPICalled → format and broadcast console output
      Runtime.exceptionThrown → broadcast page error with line/column
      Client/session/viewport changes → restart screencast at new settings
      Client count → 0 → stop screencast
  Shutdown → stop screencast, exit
```

### Exported Functions

- **`start_screencast(client, session_id, format, quality, max_width, max_height)`** — Explicit screencast start.
- **`stop_screencast(client, session_id)`** — Explicit screencast stop.
- **`ack_screencast_frame(client, session_id, screencast_session_id)`** — Acknowledge a specific screencast frame.

## WebSocket Handler (`websocket.rs`)

### Key Functions

- **`accept_loop(...)`** — TCP accept loop with shutdown support. Spawns per-connection handlers via `tokio::spawn`.
- **`is_websocket_upgrade(request_bytes)`** — Checks for `Upgrade: websocket` header in peeked TCP bytes.
- **`handle_connection(...)`** — Peeks TCP stream, routes to WebSocket or HTTP handler based on upgrade header.

### WebSocket Client Handler (`handle_ws_client`)

1. **Origin validation** via `accept_hdr_async` with callback checking `is_allowed_origin`.
2. **Client count increment** — atomic increment + notify CDP loop.
3. **Initial burst** — sends cached status, tab list, and last frame to new client.
4. **Inner loop** — forwards broadcast frames to WebSocket, reads client messages.
5. **Client message dispatch** (`handle_client_message`):
   - `input_mouse` → `Input.dispatchMouseEvent`
   - `input_keyboard` → `Input.dispatchKeyEvent`
   - `input_touch` → `Input.dispatchTouchEvent`
6. **Disconnect** — decrements client count, notifies CDP loop.

## HTTP Handler (`http.rs`)

### Embedded Dashboard

- **`DashboardAssets`** — `rust_embed::Embed` struct that embeds `packages/dashboard/out/` (compiled Next.js dashboard). This is the local modification from upstream (path changed from `../packages/dashboard/out` to `packages/dashboard/out`).

### CORS Handling

- **`CORS_HEADERS`** — Universal CORS headers for standard endpoints.
- **`cors_headers_for_origin(origin)`** — Reflects allowed origins for sensitive endpoints (chat, models).

### HTTP Request Routing (`handle_http_request`)

| Route | Method | Handler |
|-------|--------|---------|
| `OPTIONS *` | OPTIONS | 204 with CORS headers |
| `POST /api/sessions` | POST | `spawn_session` — dashboard-managed browser session creation |
| `POST /api/command` | POST | `relay_command_to_daemon` — Unix socket/TCP pipe to running daemon |
| `POST /api/chat` | POST | `handle_chat_request` — AI gateway chat |
| `GET /api/models` | GET | `handle_models_request` — available AI models |
| `GET /api/sessions` | GET | `discover_sessions` JSON — active session list |
| `GET /api/tabs` | GET | cached tab list |
| `GET /api/status` | GET | engine name JSON |
| `GET /api/chat/status` | GET | chat enabled status |
| `Other paths` | GET | `serve_embedded_file` — dashboard SPA with fallback to `index.html` |

### Command Relay (`relay_command_to_daemon`)

Connects to daemon Unix socket (or TCP on Windows), sends JSON command, reads newline-delimited response. This is how the dashboard UI executes browser commands — it proxies through the daemon's IPC channel.

### Body Reading (`read_full_body`)

Reads POST body from peeked buffer + remaining TCP stream. Respects `Content-Length` header. Maximum body size: **10 MB**.

## Dashboard Server (`dashboard.rs`)

A standalone multi-session web UI that can list, create, and interact with browser sessions. Serves as the web-based management interface for agent-browser.

### Key Types

- **`SessionProxyEndpoint`** enum — `Tabs`, `Status`, `Stream` — identifies which proxy route to use for a session.
- **`DashboardProxyError`** — `{status: u16, message: String}` with `not_found()` and `bad_gateway()` constructors.

### Session Routing

- `/session/<name>/stream` → WebSocket proxy to session's stream server
- `/session/<name>/tabs` → HTTP proxy to session's `/api/tabs`
- `/session/<name>/status` → HTTP proxy to session's `/api/status`

### Security

- **Same-origin validation** for both WebSocket and HTTP requests:
  - `is_same_origin_ws_request` — compares Origin header against Host
  - `is_same_origin_http_request` — compares Origin/Referer against Host
  - `normalize_origin_authority` / `normalize_host_authority` — handles default ports, HTTPS

### API Endpoints

| Endpoint | Method | Purpose |
|----------|--------|---------|
| `GET /api/sessions` | GET | List active sessions |
| `POST /api/sessions` | POST | Spawn new browser session |
| `POST /api/command` | POST | Relay command to daemon |
| `POST /api/kill` | POST | Kill session by PID |

### Session Spawning (`spawn_session`)

Creates a new browser session by executing the agent-browser CLI binary with appropriate arguments (session name, headed mode, etc.) as a detached child process.

### Session Discovery (`discover_sessions`)

Scans the socket directory for `*.stream` files, checks corresponding `*.pid` files for process liveness, reads `*.engine`, `*.provider`, and `*.extensions` metadata. Cleans up stale entries.

## AI Chat Integration (`chat.rs`)

Connects to an AI gateway for browser automation via natural language. Handles `/api/chat` and `/api/models` endpoints.

### Key Components

- **`DEFAULT_AI_GATEWAY_URL`** — `https://ai-gateway.vercel.sh`
- **`HTTP_CLIENT`** — `OnceLock<reqwest::Client>` singleton for gateway requests.
- **`is_chat_enabled()`** — Checks `AI_GATEWAY_API_KEY` env var presence.
- **`chat_status_json()`** — Returns `{enabled: bool, model: String}`.

### Tool Definitions

- **`CHAT_TOOLS`** — Defines browser action tools available to the AI model:
  - Navigation, clicking, typing, screenshots, scrolling, waiting
  - Snapshot, cookie, storage, tab management
  - Each tool has a `name`, `description`, and `input_schema` (JSON Schema)

- **`ALLOWED_COMMANDS`** — Whitelist of ~80 CLI commands the chat tool can execute. Prevents arbitrary command injection. Includes: `navigate`, `click`, `screenshot`, `scroll`, `wait`, `react tree`, `snapshot`, `cookies`, etc.

### Chat Request Handler (`handle_chat_request`)

1. Creates AI gateway request with system prompt + tool definitions
2. Streams response via SSE (Server-Sent Events)
3. Handles tool calls in a loop: execute → append result → continue
4. Compacts conversation history when it exceeds `COMPACT_THRESHOLD_CHARS`
5. Enriches tool output: compresses screenshots, formats structured data

### Context Compaction

When conversation history exceeds `COMPACT_THRESHOLD_CHARS`, old messages are summarized via `summarize_for_compaction` to maintain context window limits while preserving key information.

### System Prompt

Loaded lazily from skills directory via `get_system_prompt()`. Contains browser automation instructions, available tools, and interaction guidelines.
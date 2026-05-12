# Agent Browser Facade

> Module: `crates/agent-browser/src/facade.rs`

## 1. Purpose

The `facade` module provides a **local, additive Rust API** for embedding `agent-browser` capabilities directly into other Rust applications — most notably, `reshape`. It deliberately exposes a small, stable surface area that hides the complexities of daemon lifecycle management, Unix/TCP transport, and JSON command framing behind a handful of ergonomic structs and methods.

Key design goals:

- **Zero IPC ceremony for callers.** A consumer constructs a `BrowserSession`, calls `.open(url)` or `.snapshot()`, and never deals with socket paths, PID files, or command JSON directly.
- **Additive, not replacing.** The facade does not wrap the full CLI command surface — only the operations reshape needs (open, reload, snapshot, screenshot, close). Additional commands can be added later without breaking existing callers.
- **Self-healing daemon startup.** `BrowserSession::ensure()` automatically launches the browser daemon if it is not already running, cleans stale filesystem artifacts, applies environment variables, and polls until the daemon is ready — or returns a structured error.
- **Command builders as pure functions.** The `_command` methods (`open_command`, `reload_command`, etc.) return `serde_json::Value` objects without sending them, enabling inspection, logging, or batch composition before transmission.

---

## 2. `BrowserOptions`

`BrowserOptions` configures session identity and daemon behaviour. All fields have sensible defaults via `Default::default()`.

```crates/agent-browser/src/facade.rs#L12-28
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserOptions {
    pub session: String,
    pub headed: bool,
    pub debug: bool,
    pub allow_file_access: bool,
    pub idle_timeout: Option<String>,
    pub default_timeout: Option<u64>,
    pub no_auto_dialog: bool,
}
```

### Fields

| Field | Type | Default | Description |
|---|---|---|---|
| `session` | `String` | `"default"` | Daemon session name. Determines socket/PID file paths, isolates multiple concurrent daemons on the same machine. |
| `headed` | `bool` | `false` | When `true`, the browser launches with a visible window instead of headless mode. Sets `AGENT_BROWSER_HEADED=1`. |
| `debug` | `bool` | `false` | Enables verbose daemon logging. Sets `AGENT_BROWSER_DEBUG=1`. |
| `allow_file_access` | `bool` | `false` | Permits the browser to access `file://` URLs and local filesystem resources. Sets `AGENT_BROWSER_ALLOW_FILE_ACCESS=1`. Reshape sets this to `true` for its default renderer. |
| `idle_timeout` | `Option<String>` | `None` | Duration string (e.g. `"30000"` milliseconds) the daemon waits before auto-shutting down when idle. Sets `AGENT_BROWSER_IDLE_TIMEOUT_MS`. |
| `default_timeout` | `Option<u64>` | `None` | Default command execution timeout in milliseconds. Propagates to all `wait_*` operations and snapshot/screenshot calls. Sets `AGENT_BROWSER_DEFAULT_TIMEOUT`. |
| `no_auto_dialog` | `bool` | `false` | Disables automatic dismissal of browser dialog boxes (alerts, confirms, prompts). Sets `AGENT_BROWSER_NO_AUTO_DIALOG=1`. |

### `Default` implementation

```crates/agent-browser/src/facade.rs#L30-42
impl Default for BrowserOptions {
    fn default() -> Self {
        Self {
            session: "default".to_string(),
            headed: false,
            debug: false,
            allow_file_access: false,
            idle_timeout: None,
            default_timeout: None,
            no_auto_dialog: false,
        }
    }
}
```

---

## 3. `BrowserSession`

`BrowserSession` is the primary entry point. It holds a `BrowserOptions` and exposes methods that **ensure the daemon is alive** before sending commands.

```crates/agent-browser/src/facade.rs#L44-46
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserSession {
    options: BrowserOptions,
}
```

### Construction

```rust
let options = BrowserOptions {
    session: "reshape-main".to_string(),
    allow_file_access: true,
    ..BrowserOptions::default()
};
let session = BrowserSession::new(options);
```

`BrowserSession::new(options)` simply wraps the options. No I/O or daemon startup happens at construction time — that is deferred to the first `.ensure()` or action call.

### `options()`

Returns a shared reference to the inner `BrowserOptions`, enabling inspection after construction.

### `ensure()`

Ensures the daemon process is running and accepting connections. If the daemon is already ready (`daemon_ready()` succeeds), returns `Ok(())` immediately. Otherwise, it:

1. Calls `cleanup_stale_files(&self.options.session)` to remove orphaned PID, version, socket, and stream files from previous runs.
2. Calls `apply_daemon_environment()` to set environment variables the daemon process will inherit.
3. Spawns a background thread that creates a dedicated Tokio runtime and runs `crate::native::daemon::run_daemon(&session)` inside it.
4. Polls for readiness 25 times, sleeping 200 ms between each attempt (total: up to 5 seconds).
5. If the daemon never becomes ready, returns `BrowserError::DaemonStart { session }`.

```crates/agent-browser/src/facade.rs#L54-73
pub fn ensure(&self) -> Result<(), BrowserError> {
    if self.daemon_ready() {
        return Ok(());
    }

    cleanup_stale_files(&self.options.session);
    self.apply_daemon_environment();

    let session = self.options.session.clone();
    thread::spawn(move || {
        let runtime = tokio::runtime::Runtime::new()
            .expect("failed to create daemon runtime");
        runtime.block_on(
            crate::native::daemon::run_daemon(&session)
        );
    });

    for _ in 0..25 {
        thread::sleep(Duration::from_millis(200));
        if self.daemon_ready() {
            return Ok(());
        }
    }

    Err(BrowserError::DaemonStart {
        session: self.options.session.clone(),
    })
}
```

### `send()`

Sends a raw `serde_json::Value` command to the daemon for the configured session, returning a `BrowserResponse` (alias for `crate::connection::Response`). Transport errors are wrapped in `BrowserError::Transport`.

```crates/agent-browser/src/facade.rs#L75-78
pub fn send(&self, command: Value)
    -> Result<BrowserResponse, BrowserError>
{
    send_command(command, &self.options.session)
        .map_err(BrowserError::Transport)
}
```

Internally, `send_command` retries up to 5 times on transient errors (EAGAIN, connection reset, broken pipe) with exponential backoff.

### Action methods

Each action method calls `self.ensure()` (except `close`) to guarantee the daemon is alive, then delegates to `self.send()` with the corresponding command builder.

| Method | Ensures daemon? | Command builder | Returns |
|---|---|---|---|
| `open(url)` | Yes | `open_command(url)` | `BrowserResponse` |
| `reload()` | Yes | `reload_command()` | `BrowserResponse` |
| `snapshot()` | Yes | `snapshot_command()` | `BrowserResponse` |
| `screenshot(path)` | Yes | `screenshot_command(path)` | `BrowserResponse` |
| `close()` | No | `close_command()` | `BrowserResponse` |

`close()` intentionally skips `ensure()` — the daemon should accept a close command even on its final run, and re-launching a daemon just to close it would be wasteful.

```crates/agent-browser/src/facade.rs#L80-101
pub fn open(&self, url: impl AsRef<str>)
    -> Result<BrowserResponse, BrowserError>
{
    self.ensure()?;
    self.send(self.open_command(url))
}

pub fn reload(&self) -> Result<BrowserResponse, BrowserError> {
    self.ensure()?;
    self.send(self.reload_command())
}

pub fn snapshot(&self) -> Result<BrowserResponse, BrowserError> {
    self.ensure()?;
    self.send(self.snapshot_command())
}

pub fn screenshot(
    &self,
    path: Option<impl AsRef<str>>,
) -> Result<BrowserResponse, BrowserError> {
    self.ensure()?;
    self.send(self.screenshot_command(path))
}

pub fn close(&self) -> Result<BrowserResponse, BrowserError> {
    self.send(self.close_command())
}
```

---

## 4. Command Builders

Each `_command` method produces a `serde_json::Value` representing the JSON payload that would be sent to the daemon. The `id` field is generated by `gen_id()`, which produces a microsecond-derived identifier like `"r428571"`.

### `open_command(url)`

Produces a **navigate** action with a normalized URL:

```json
{
  "id": "r428571",
  "action": "navigate",
  "url": "https://example.com"
}
```

URL normalization via `normalize_url` is applied automatically (see section 6).

### `reload_command()`

```json
{
  "id": "r428571",
  "action": "reload"
}
```

### `snapshot_command()`

```json
{
  "id": "r428571",
  "action": "snapshot"
}
```

Requests an accessibility-tree snapshot of the current page state. The daemon returns structured DOM data that reshape uses to understand page content without rendering pixels.

### `screenshot_command(path)`

```json
{
  "id": "r428571",
  "action": "screenshot",
  "path": "/tmp/output.png",
  "selector": null
}
```

- `path` — optional file path to write the screenshot to. When `None`, the value becomes `null` in JSON, and the daemon returns the image data inline.
- `selector` — always `null` in the facade builder (captures full page). The full CLI parser supports targeting a specific element, but the facade intentionally omits this to keep the surface small.

### `close_command()`

```json
{
  "id": "r428571",
  "action": "close"
}
```

Instructs the daemon to shut down its browser instance for the session.

---

## 5. `BrowserError`

```crates/agent-browser/src/facade.rs#L134-137
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrowserError {
    DaemonStart { session: String },
    Transport(String),
}
```

| Variant | Fields | Meaning |
|---|---|---|
| `DaemonStart` | `session: String` | The daemon failed to become ready within the 25-retry polling window (5 seconds). The `session` field identifies which session name failed. |
| `Transport` | `String` | A communication error with an already-running daemon — socket connection failure, timeout, malformed response, etc. Wraps the error string from `send_command`. |

### `Display` implementation

```crates/agent-browser/src/facade.rs#L139-150
impl std::fmt::Display for BrowserError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>)
        -> std::fmt::Result
    {
        match self {
            BrowserError::DaemonStart { session } => {
                write!(f,
                    "failed to start agent-browser daemon \
                     for session {session}")
            }
            BrowserError::Transport(error) => {
                write!(f,
                    "agent-browser transport failed: {error}")
            }
        }
    }
}
```

`BrowserError` also implements `std::error::Error`, making it compatible with the `thiserror` `#[from]` derivation used in reshape's `BrowserRenderError`.

---

## 6. URL Normalization — `normalize_url`

```crates/agent-browser/src/facade.rs#L154-167
fn normalize_url(url: &str) -> String {
    let lower = url.to_lowercase();
    if lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("about:")
        || lower.starts_with("data:")
        || lower.starts_with("file:")
        || lower.starts_with("chrome-extension://")
        || lower.starts_with("chrome://")
    {
        url.to_string()
    } else {
        format!("https://{url}")
    }
}
```

### Recognized protocols

| Protocol | Example | Behaviour |
|---|---|---|
| `http://` | `http://localhost:3000` | Passed through unchanged |
| `https://` | `https://example.com` | Passed through unchanged |
| `about:` | `about:blank` | Passed through unchanged |
| `data:` | `data:text/html,<h1>Hello</h1>` | Passed through unchanged |
| `file:` | `file:///home/user/doc.html` | Passed through unchanged |
| `chrome-extension://` | `chrome-extension://abc...` | Passed through unchanged |
| `chrome://` | `chrome://version` | Passed through unchanged |
| *(anything else)* | `example.com` | Prepended with `https://` → `https://example.com` |

The comparison is case-insensitive (`url.to_lowercase()`), so `HTTP://example.com` is still recognized and preserved as-is (not re-normalized to lowercase). The default prefix is always `https://`, not `http://`, reflecting modern web security expectations.

---

## 7. Daemon Startup Flow

The `ensure()` method orchestrates daemon startup through a deterministic sequence:

```
ensure()
  ├── daemon_ready()?  ── Yes → return Ok(())
  │
  ├── cleanup_stale_files(session)
  │     ├── remove PID file     (e.g. /run/user/1000/agent-browser/default.pid)
  │     ├── remove version file (e.g. /run/user/1000/agent-browser/default.version)
  │     ├── remove stream file  (e.g. /run/user/1000/agent-browser/default.stream)
  │     ├── [unix] remove Unix socket
  │     └── [windows] remove TCP port file
  │
  ├── apply_daemon_environment()
  │     ├── set AGENT_BROWSER_SESSION = session name
  │     ├── set AGENT_BROWSER_EMBEDDED_DAEMON = "1"
  │     ├── set AGENT_BROWSER_HEADED = "1" (if headed)
  │     ├── set AGENT_BROWSER_DEBUG = "1" (if debug)
  │     ├── set AGENT_BROWSER_ALLOW_FILE_ACCESS = "1" (if allow_file_access)
  │     ├── set AGENT_BROWSER_IDLE_TIMEOUT_MS = value (if idle_timeout set)
  │     ├── set AGENT_BROWSER_DEFAULT_TIMEOUT = value (if default_timeout set)
  │     └── set AGENT_BROWSER_NO_AUTO_DIALOG = "1" (if no_auto_dialog)
  │
  ├── thread::spawn → tokio::Runtime::new() → block_on(run_daemon(session))
  │
  └── Poll loop: 25 retries × 200 ms sleep
        │   each retry: send stream_status → is_ok()? → return Ok(())
        │   25th retry fails → return Err(DaemonStart { session })
```

### Polling details

- **Retry count**: 25
- **Sleep interval**: 200 ms per attempt
- **Total maximum wait**: 5 seconds
- **Readiness probe**: sends a `stream_status` command via `send_command`; success means the daemon socket is listening and responding.
- The daemon thread runs `crate::native::daemon::run_daemon` in a dedicated Tokio runtime, completely independent of the caller's async context.

### `daemon_ready()` implementation

```crates/agent-browser/src/facade.rs#L103-107
fn daemon_ready(&self) -> bool {
    let command = json!({
        "id": gen_id(),
        "action": "stream_status"
    });
    send_command(command, &self.options.session).is_ok()
}
```

### `cleanup_stale_files()` — from `connection.rs`

```crates/agent-browser/src/connection.rs#L131-150
pub fn cleanup_stale_files(session: &str) {
    let pid_path = get_pid_path(session);
    let _ = fs::remove_file(&pid_path);
    let version_path = get_version_path(session);
    let _ = fs::remove_file(&version_path);
    let stream_path = get_socket_dir()
        .join(format!("{}.stream", session));
    let _ = fs::remove_file(&stream_path);

    #[cfg(unix)]
    {
        let socket_path = get_socket_path(session);
        let _ = fs::remove_file(&socket_path);
    }

    #[cfg(windows)]
    {
        let port_path = get_port_path(session);
        let _ = fs::remove_file(&port_path);
    }
}
```

All removals use `let _ =` (ignore `Err`) because stale files may or may not exist — the goal is a clean slate, not precise error reporting.

---

## 8. Environment Variables Set by `apply_daemon_environment`

The daemon process inherits environment variables set by `apply_daemon_environment()` before it is spawned. These variables are consumed by `crate::native::daemon::run_daemon` and the broader `agent-browser` startup logic.

```crates/agent-browser/src/facade.rs#L109-130
fn apply_daemon_environment(&self) {
    std::env::set_var("AGENT_BROWSER_SESSION", &self.options.session);
    std::env::set_var("AGENT_BROWSER_EMBEDDED_DAEMON", "1");
    if self.options.headed {
        std::env::set_var("AGENT_BROWSER_HEADED", "1");
    }
    if self.options.debug {
        std::env::set_var("AGENT_BROWSER_DEBUG", "1");
    }
    if self.options.allow_file_access {
        std::env::set_var("AGENT_BROWSER_ALLOW_FILE_ACCESS", "1");
    }
    if let Some(idle_timeout) = &self.options.idle_timeout {
        std::env::set_var("AGENT_BROWSER_IDLE_TIMEOUT_MS", idle_timeout);
    }
    if let Some(default_timeout) = self.options.default_timeout {
        std::env::set_var(
            "AGENT_BROWSER_DEFAULT_TIMEOUT",
            default_timeout.to_string(),
        );
    }
    if self.options.no_auto_dialog {
        std::env::set_var("AGENT_BROWSER_NO_AUTO_DIALOG", "1");
    }
}
```

| Variable | Source field | Value | Set condition |
|---|---|---|---|
| `AGENT_BROWSER_SESSION` | `session` | Session name (e.g. `"reshape-main"`) | Always |
| `AGENT_BROWSER_EMBEDDED_DAEMON` | — | `"1"` | Always — signals the daemon was spawned by a Rust host, not the CLI |
| `AGENT_BROWSER_HEADED` | `headed` | `"1"` | Only when `headed == true` |
| `AGENT_BROWSER_DEBUG` | `debug` | `"1"` | Only when `debug == true` |
| `AGENT_BROWSER_ALLOW_FILE_ACCESS` | `allow_file_access` | `"1"` | Only when `allow_file_access == true` |
| `AGENT_BROWSER_IDLE_TIMEOUT_MS` | `idle_timeout` | Timeout string (e.g. `"30000"`) | Only when `idle_timeout` is `Some` |
| `AGENT_BROWSER_DEFAULT_TIMEOUT` | `default_timeout` | Timeout as string (e.g. `"5000"`) | Only when `default_timeout` is `Some` |
| `AGENT_BROWSER_NO_AUTO_DIALOG` | `no_auto_dialog` | `"1"` | Only when `no_auto_dialog == true` |

**Important**: `apply_daemon_environment` sets **process-global** environment variables via `std::env::set_var`. This is intentional — the daemon thread spawned immediately after inherits these vars. However, this means the variables persist for the entire lifetime of the host process. In multi-session scenarios, only the most-recently-ensured session's variables remain in the environment.

---

## 9. How Reshape Uses This

The `reshape-browser` crate (`crates/reshape-browser`) wraps `BrowserSession` into reshape's domain-specific rendering interface. It re-exports the core types and provides two traits that abstract browser operations for the reshape runtime.

### Re-export

```crates/reshape-browser/src/agent_browser.rs#L1
pub use ::agent_browser::{BrowserOptions, BrowserSession};
```

### `BrowserSessionClient` trait

A thin adapter that maps `BrowserSession` methods into reshape's `Result<()>` return type (discarding response data, preserving errors):

```crates/reshape-browser/src/lib.rs
pub trait BrowserSessionClient {
    fn open(&self, url: &str) -> Result<()>;
    fn reload(&self) -> Result<()>;
    fn snapshot(&self) -> Result<()>;
    fn screenshot(&self, path: Option<&str>) -> Result<()>;
    fn close(&self) -> Result<()>;
}
```

`BrowserSession` implements `BrowserSessionClient` directly, converting `BrowserError` into `BrowserRenderError::AgentBrowser` via the `#[from]` thiserror derivation.

### `BrowserRenderer` trait

The higher-level interface reshape's runtime calls. It adds workspace-awareness:

```crates/reshape-browser/src/lib.rs
pub trait BrowserRenderer {
    fn open_workspace_entry(&self, path: &Path) -> Result<()>;
    fn reload(&self) -> Result<()>;
    fn snapshot(&self) -> Result<()>;
    fn screenshot(&self, path: Option<&Path>) -> Result<()>;
    fn close(&self) -> Result<()>;
}
```

### `AgentBrowserRenderer<C>` — the concrete implementation

```crates/reshape-browser/src/lib.rs
#[derive(Debug, Clone)]
pub struct AgentBrowserRenderer<C = BrowserSession> {
    session: C,
}
```

- Generic over `C: BrowserSessionClient`, defaulting to `BrowserSession`.
- `Default` implementation constructs `BrowserSession` with reshape-specific options:

```rust
impl Default for AgentBrowserRenderer<BrowserSession> {
    fn default() -> Self {
        let options = BrowserOptions {
            session: "reshape-main".to_string(),
            allow_file_access: true,  // reshape needs file:// for workspace entries
            ..BrowserOptions::default()
        };
        Self::new(BrowserSession::new(options))
    }
}
```

- `open_workspace_entry(path)` canonicalizes the filesystem path, converts it to a `file://` URL via `Url::from_file_path`, then delegates to `C::open(url)`.

### `BrowserRenderError`

```crates/reshape-browser/src/lib.rs
#[derive(Debug, thiserror::Error)]
pub enum BrowserRenderError {
    #[error("workspace entry is not a file: {0}")]
    NotAFile(PathBuf),

    #[error("failed to convert workspace entry to file URL: {path}")]
    FileUrl { path: PathBuf, reason: Option<String> },

    #[error(transparent)]
    AgentBrowser(#[from] ::agent_browser::BrowserError),
}
```

The `#[from]` attribute means any `BrowserError` (whether `DaemonStart` or `Transport`) is automatically convertible into `BrowserRenderError::AgentBrowser`, keeping reshape's error handling clean.

### Integration flow

When reshape needs to render a workspace file in the browser, the call chain is:

```
reshape runtime
  → AgentBrowserRenderer::open_workspace_entry(path)
    → workspace_entry_url(path)  // canonicalize → file:// URL
    → BrowserSessionClient::open(url)
      → BrowserSession::open(url)
        → ensure()               // auto-start daemon if needed
        → send(open_command(url)) // JSON over Unix socket / TCP
```

This layered design means reshape's core runtime never imports `agent-browser` directly — it only sees the `BrowserRenderer` trait. Swapping the implementation (e.g. for a remote browser service) requires only a different `BrowserSessionClient` adapter, with no changes to the agent loop or tool registry.
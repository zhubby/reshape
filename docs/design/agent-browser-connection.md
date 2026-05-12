# Agent Browser Connection — Daemon IPC Layer

> Module: `crates/agent-browser/src/connection.rs`

The `connection` module is the IPC backbone between the CLI client and the
headless browser daemon. It covers every step of the daemon lifecycle — from
socket directory resolution and stale-file cleanup through process spawning
and readiness polling, down to the low-level command send/receive protocol
with automatic retry on transient errors.

---

## 1. Purpose

The browser daemon runs as a long-lived background process. The CLI (and any
other client) communicates with it over **Unix domain sockets** on macOS/Linux
and **TCP loopback** on Windows. This module:

- **Resolves the socket directory** where all sidecar files (`.sock`, `.pid`,
  `.port`, `.version`, `.stream`) live.
- **Manages daemon lifecycle**: starts the daemon if it isn't running, kills
  stale instances after upgrades, and cleans up orphaned files.
- **Sends commands**: serialises a JSON `Request`, writes it to the socket with
  a newline delimiter, reads a JSON `Response` line back, and retries on
  transient errors (EAGAIN, connection reset, etc.).

All IPC is **synchronous and blocking** — the module uses `std::thread` and
`std::net`/`std::os::unix::net` rather than Tokio, because the daemon
protocol is short-lived request/response pairs that don't need an async
runtime.

---

## 2. Request / Response Structs

### `Request`

```reshape/crates/agent-browser/src/connection.rs#L22-27
pub struct Request {
    pub id: String,
    pub action: String,
    #[serde(flatten)]
    pub extra: Value,
}
```

| Field   | Type   | Purpose                                              |
|---------|--------|------------------------------------------------------|
| `id`    | `String` | Unique correlation ID for the request.             |
| `action`| `String` | The daemon command name (e.g. `"navigate"`, `"click"`). |
| `extra` | `Value`  | Additional parameters flattened into the same JSON object via `#[serde(flatten)]`. |

Serialised as a single JSON object; `extra`'s keys are merged alongside
`id` and `action` so the daemon receives one flat envelope.

### `Response`

```reshape/crates/agent-browser/src/connection.rs#L30-36
pub struct Response {
    pub success: bool,
    pub data: Option<Value>,
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}
```

| Field      | Type            | Purpose                                           |
|------------|-----------------|---------------------------------------------------|
| `success`  | `bool`          | Whether the command completed without error.      |
| `data`     | `Option<Value>` | Result payload on success (screenshots, DOM, etc).|
| `error`    | `Option<String>`| Error message on failure.                          |
| `warning`  | `Option<String>`| Non-fatal warning; omitted from serialisation when `None`. |

---

## 3. `Connection` Enum

```reshape/crates/agent-browser/src/connection.rs#L39 43
pub enum Connection {
    Unix(UnixStream),   // #[cfg(unix)]
    Tcp(TcpStream),     // #[cfg(all)]
}
```

`Connection` abstracts over the platform transport. On Unix it wraps
`std::os::unix::net::UnixStream`; on Windows it wraps `std::net::TcpStream`.

### `Read` / `Write` impls

The enum implements `std::io::Read` and `std::io::Write` by delegating to the
inner stream variant, so callers can treat it as a single opaque I/O handle
regardless of platform.

```reshape/crates/agent-browser/src/connection.rs#L73 89
impl Connection {
    pub fn set_read_timeout(&self, dur: Option<Duration>) -> std::io::Result<()>;
    pub fn set_write_timeout(&self, dur: Option<Duration>) -> std::io::Result<()>;
}
```

Both timeout methods likewise delegate to the underlying stream. `send_command`
sets a **30-second read timeout** and a **5-second write timeout** before
sending data.

---

## 4. Socket Directory Resolution

```reshape/crates/agent-browser/src/connection.rs#L93 115
pub fn get_socket_dir() -> PathBuf
```

The function resolves the base directory for all sidecar files using this
priority chain (empty strings are **ignored** at every level):

| Priority | Source                           | Result path                                     |
|----------|----------------------------------|-------------------------------------------------|
| 1        | `AGENT_BROWSER_SOCKET_DIR` env   | The value itself (absolute or relative).        |
| 2        | `XDG_RUNTIME_DIR` env            | `$XDG_RUNTIME_DIR/agent-browser`                |
| 3        | Home directory                   | `~/.agent-browser`                              |
| 4        | System temp directory            | `$TMPDIR/agent-browser` (last resort)           |

The XDG and home fallbacks mirror Docker Desktop's `~/.docker/run/` pattern.
Empty env-var values are skipped so that `AGENT_BROWSER_SOCKET_DIR=""` does
not override a valid `XDG_RUNTIME_DIR`.

### Derived paths

Three private helpers build session-specific sidecar filenames from the
socket directory and session name:

| Helper            | File                      | Purpose                                     |
|-------------------|---------------------------|---------------------------------------------|
| `get_socket_path` | `{dir}/{session}.sock`   | Unix domain socket for IPC (Unix only).     |
| `get_pid_path`    | `{dir}/{session}.pid`    | PID of the daemon process.                  |
| `get_version_path`| `{dir}/{session}.version`| CLI version that started the daemon.        |

---

## 5. Daemon Lifecycle

### `cleanup_stale_files`

```reshape/crates/agent-browser/src/connection.rs#L131 150
pub fn cleanup_stale_files(session: &str)
```

Removes all sidecar files for a session: `.pid`, `.version`, `.stream`, and
the `.sock` file (Unix) or `.port` file (Windows). Errors are silently
ignored — the files may already be gone.

### `is_pid_alive`

```reshape/crates/agent-browser/src/connection.rs#L157 175
pub fn is_pid_alive(pid: u32) -> bool
```

Checks whether a process is still running:

- **Unix**: sends signal 0 via `libc::kill`. Returns `true` on success (pid
  exists and we can signal it) **or** when the error is `EPERM` (pid exists
  but owned by a different UID). Only `ESRCH` ("no such process") returns
  `false`.
- **Windows**: calls `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)`. A
  non-zero handle means the process exists; the handle is immediately closed.

### `ActiveSession`

```reshape/crates/agent-browser/src/connection.rs#L179 184
pub struct ActiveSession {
    pub name: String,
    pub pid: u32,
    pub version: Option<String>,
}
```

Represents a live daemon discovered by `walk_daemons`. `version` holds the
content of the `.version` sidecar if it exists and is non-empty.

### `CleanReason`

```reshape/crates/agent-browser/src/connection.rs#L188 197
pub enum CleanReason {
    ProcessGone,          // .pid references a dead process
    UnreadablePidFile,    // .pid file could not be parsed as a u32
    OrphanedSocket,       // .sock exists with no .pid (Unix only)
    DashboardGone,        // dashboard.pid references a dead process
}
```

### `CleanedSession`

```reshape/crates/agent-browser/src/connection.rs#L201 204
pub struct CleanedSession {
    pub name: String,
    pub reason: CleanReason,
}
```

### `DashboardInfo`

```reshape/crates/agent-browser/src/connection.rs#L208 211
pub struct DashboardInfo {
    pub pid: u32,
    pub alive: bool,
}
```

### `DaemonInventory`

```reshape/crates/agent-browser/src/connection.rs#L216 220
pub struct DaemonInventory {
    pub sessions: Vec<ActiveSession>,
    pub cleaned: Vec<CleanedSession>,
    pub dashboard: Option<DashboardInfo>,
}
```

A snapshot of the socket directory after a walk. Stale files are cleaned as a
side effect and recorded in `cleaned`.

### `walk_daemons`

```reshape/crates/agent-browser/src/connection.rs#L242 330
pub fn walk_daemons() -> DaemonInventory
```

Iterates over all entries in `get_socket_dir()` and classifies each:

1. **`dashboard.pid`** — parsed and checked for liveness. If dead, the file
   is removed and a `DashboardGone` entry is added to `cleaned`.
2. **`*.pid`** — each is parsed; unreadable files trigger `UnreadablePidFile`
   cleanup, dead processes trigger `ProcessGone` cleanup, live processes are
   added to `sessions` with their `.version` content.
3. **Orphaned `*.sock`** (Unix only) — sockets without a matching `.pid` are
   cleaned and recorded as `OrphanedSocket`.

If the socket directory doesn't exist, returns an empty `DaemonInventory` with
no side effects.

---

## 6. Port Management (Windows)

On Windows, the daemon communicates over TCP loopback rather than Unix sockets.
Three functions manage port assignment:

### `get_port_path`

```reshape/crates/agent-browser/src/connection.rs#L333 335
fn get_port_path(session: &str) -> PathBuf
```

Returns `{socket_dir}/{session}.port` — the file the daemon writes its
actual listening port into after binding.

### `get_port_for_session`

```reshape/crates/agent-browser/src/connection.rs#L338 346
#[cfg(windows)]
pub fn get_port_for_session(session: &str) -> u16
```

Deterministically derives a port from the session name using a hash-based
algorithm (DJB2-style `hash << 5 - hash + char`). The result is mapped into
the IANA dynamic/private range **49152–65534** (`49152 + hash % 16383`),
avoiding conflicts with well-known ports. This is the fallback port used
before the daemon has written its `.port` file.

### `resolve_port`

```reshape/crates/agent-browser/src/connection.rs#L352 358
#[cfg(windows)]
pub fn resolve_port(session: &str) -> u16
```

Reads the `.port` sidecar file for the actual daemon port. Falls back to
`get_port_for_session` if the file is missing or unreadable (daemon not yet
started).

---

## 7. Daemon Readiness

```reshape/crates/agent-browser/src/connection.rs#L360 375
pub fn daemon_ready(session: &str) -> bool
```

Attempts a real connection to the daemon:

- **Unix**: tries `UnixStream::connect` to `{session}.sock`.
- **Windows**: tries `TcpStream::connect_timeout` to `127.0.0.1:{port}`
  with a **50 ms** timeout.

Returns `true` only if the connection succeeds, meaning the daemon process is
alive and its transport endpoint is bound. This is the sole liveness probe —
no PID check is used, so callers in different PID namespaces (e.g. `unshare`)
can still detect a reachable daemon.

---

## 8. `DaemonOptions` Struct

```reshape/crates/agent-browser/src/connection.rs#L387 417
pub struct DaemonOptions<'a>
```

`DaemonOptions` carries every browser configuration field the CLI forwards to
the daemon process as environment variables. All string fields use `Option<&'a str>`
to avoid allocations — the struct is ephemeral and only lives during the
`ensure_daemon` call.

| Field                  | Type                    | Env variable                       | Purpose                                              |
|------------------------|-------------------------|------------------------------------|------------------------------------------------------|
| `headed`               | `bool`                  | `AGENT_BROWSER_HEADED`             | Run browser with a visible window.                   |
| `debug`                | `bool`                  | `AGENT_BROWSER_DEBUG`              | Enable verbose daemon logging.                       |
| `executable_path`      | `Option<&'a str>`       | `AGENT_BROWSER_EXECUTABLE_PATH`    | Path to a custom Chromium executable.                |
| `extensions`           | `&'a [String]`          | `AGENT_BROWSER_EXTENSIONS`         | Comma-separated Chrome extensions to load.           |
| `init_scripts`         | `&'a [String]`          | `AGENT_BROWSER_INIT_SCRIPTS`       | Comma-separated JS scripts to inject on page load.   |
| `enable`               | `&'a [String]`          | `AGENT_BROWSER_ENABLE`             | Comma-separated feature flags to enable.             |
| `args`                 | `Option<&'a str>`       | `AGENT_BROWSER_ARGS`               | Extra Chromium flags (e.g. `--disable-gpu`).         |
| `user_agent`           | `Option<&'a str>`       | `AGENT_BROWSER_USER_AGENT`         | Override the browser User-Agent string.              |
| `proxy`                | `Option<&'a str>`       | `AGENT_BROWSER_PROXY`              | Proxy server URL.                                    |
| `proxy_bypass`         | `Option<&'a str>`       | `AGENT_BROWSER_PROXY_BYPASS`       | Hosts that bypass the proxy.                         |
| `proxy_username`       | `Option<&'a str>`       | `AGENT_BROWSER_PROXY_USERNAME`     | Proxy auth username.                                 |
| `proxy_password`       | `Option<&'a str>`       | `AGENT_BROWSER_PROXY_PASSWORD`     | Proxy auth password.                                 |
| `ignore_https_errors`  | `bool`                  | `AGENT_BROWSER_IGNORE_HTTPS_ERRORS`| Skip TLS certificate validation.                     |
| `allow_file_access`    | `bool`                  | `AGENT_BROWSER_ALLOW_FILE_ACCESS`  | Enable `file://` URL access in the browser.          |
| `profile`              | `Option<&'a str>`       | `AGENT_BROWSER_PROFILE`            | Browser profile directory path.                      |
| `state`                | `Option<&'a str>`       | `AGENT_BROWSER_STATE`              | Persistent state directory path.                     |
| `provider`             | `Option<&'a str>`       | `AGENT_BROWSER_PROVIDER`           | AI provider for autonomous actions (e.g. `"openai"`).|
| `device`               | `Option<&'a str>`       | `AGENT_BROWSER_IOS_DEVICE`         | iOS device name for WebKit inspector targets.        |
| `session_name`         | `Option<&'a str>`       | `AGENT_BROWSER_SESSION_NAME`       | Human-readable session label.                        |
| `download_path`        | `Option<&'a str>`       | `AGENT_BROWSER_DOWNLOAD_PATH`      | Directory for auto-downloaded files.                 |
| `allowed_domains`      | `Option<&'a [String]>`  | `AGENT_BROWSER_ALLOWED_DOMAINS`    | Comma-separated domain whitelist for navigation.     |
| `action_policy`        | `Option<&'a str>`       | `AGENT_BROWSER_ACTION_POLICY`      | Policy restricting which browser actions are allowed.|
| `confirm_actions`      | `Option<&'a str>`       | `AGENT_BROWSER_CONFIRM_ACTIONS`    | Action categories requiring user confirmation.       |
| `engine`               | `Option<&'a str>`       | `AGENT_BROWSER_ENGINE`             | Browser engine choice (`"chromium"`, `"webkit"`, etc).|
| `auto_connect`         | `bool`                  | `AGENT_BROWSER_AUTO_CONNECT`       | Automatically connect CDP on browser launch.         |
| `idle_timeout`         | `Option<&'a str>`       | `AGENT_BROWSER_IDLE_TIMEOUT_MS`    | Milliseconds before idle daemon shuts down.          |
| `default_timeout`      | `Option<u64>`           | `AGENT_BROWSER_DEFAULT_TIMEOUT`    | Default navigation/action timeout in ms.             |
| `cdp`                  | `Option<&'a str>`       | `AGENT_BROWSER_CDP`                | CDP endpoint override for external Chrome instances. |
| `no_auto_dialog`       | `bool`                  | `AGENT_BROWSER_NO_AUTO_DIALOG`     | Suppress automatic dialog dismissal.                 |

> **Note**: `confirm_interactive` is intentionally absent from `DaemonOptions`.
> It is a CLI-side UX concern (prompting the user on stdin) and not a daemon
> configuration knob. The daemon only receives `confirm_actions` to gate
> action categories.

### `apply_daemon_env`

```reshape/crates/agent-browser/src/connection.rs#L419 510
fn apply_daemon_env(cmd: &mut Command, session: &str, opts: &DaemonOptions)
```

Sets every applicable environment variable on the `Command` that will spawn
the daemon. Boolean fields are only set when `true`; `Option` fields are only
set when `Some`; slice fields are comma-joined and only set when non-empty.
The `AGENT_BROWSER_DAEMON=1` and `AGENT_BROWSER_SESSION` variables are always
set (they tell the child process it is running in daemon mode and which
session it owns).

---

## 9. `ensure_daemon` — Full Startup Flow

```reshape/crates/agent-browser/src/connection.rs#L574 755
pub fn ensure_daemon(session: &str, opts: &DaemonOptions) -> Result<DaemonResult, String>
```

`ensure_daemon` is the main entry point for starting or reusing a browser
daemon. It follows this sequence:

### Step 1 — Check for existing daemon

Calls `daemon_ready(session)`. If the socket is reachable, waits **150 ms**
and checks again (to handle the race where the daemon is mid-shutdown — the
daemon has a 100 ms shutdown delay). If still ready:

- **Version check**: reads the `.version` sidecar and compares it to
  `CARGO_PKG_VERSION`. If the versions differ (e.g. after a CLI upgrade),
  prints a warning, calls `kill_stale_daemon`, and falls through to spawn
  a fresh instance.
- **Version matches**: returns `Ok(DaemonResult { already_running: true })`.

### Step 2 — Cleanup stale files

Calls `cleanup_stale_files(session)` to remove any leftover `.pid`, `.sock`,
`.version`, `.stream`, or `.port` files from previous runs.

### Step 3 — Ensure socket directory exists

Creates `get_socket_dir()` via `fs::create_dir_all` if it doesn't already
exist.

### Step 4 — Pre-flight: socket path length (Unix only)

Unix domain socket paths must be ≤ **103 bytes** (104 including the null
terminator). If the session name plus directory prefix exceeds this limit,
returns an error suggesting a shorter session name or a shorter
`AGENT_BROWSER_SOCKET_DIR`.

### Step 5 — Pre-flight: socket directory writable

Writes and deletes a `.write_test` file in the socket directory. If the write
fails, returns an error identifying the unwritable directory.

### Step 6 — Spawn the daemon process

Locates the current executable via `env::current_exe()` + `canonicalize()`,
then spawns it as a child:

- **Unix**: sets `AGENT_BROWSER_DAEMON=1`, applies all `DaemonOptions` env
  vars, calls `setsid()` via `pre_exec` to detach the child into its own
  session, and redirects stdin/stdout to null + stderr to piped.
- **Windows**: uses `CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS` creation
  flags to detach the child, similarly redirects I/O.

### Step 7 — Retry loop (up to 50 iterations)

Polls `daemon_ready(session)` in a loop with **100 ms** sleep between
iterations (max ~5 seconds total):

- If the daemon becomes ready, returns `Ok(DaemonResult { already_running: false })`.
- If the child exits early, reads its stderr to surface the real error:
  - If stderr contains `"Address already in use"` or `"Failed to bind"`,
    sleeps 200 ms and checks `daemon_ready` once more — another process may
    have won the bind race, and we can piggyback on it.
  - Otherwise, returns the truncated stderr (max 500 chars) or a generic
    "no error output" message suggesting `--debug`.

### Step 8 — Timeout failure

If no readiness is detected after 50 iterations, returns an error including
the endpoint info (socket path on Unix, port on Windows).

### `kill_stale_daemon`

```reshape/crates/agent-browser/src/connection.rs#L527 572
fn kill_stale_daemon(session: &str)
```

Called when a version mismatch is detected or a bind race occurs. It:

1. Removes the socket file first (Unix) so no new connections reach the old
   daemon.
2. Reads the `.pid` file and sends `SIGTERM` (Unix) or `taskkill /F` (Windows).
3. On Unix, polls up to 10 × 100 ms for the process to exit gracefully;
   sends `SIGKILL` if it survives.
4. Calls `cleanup_stale_files(session)` to remove remaining sidecar files.

---

## 10. `send_command` — Retry Logic

```reshape/crates/agent-browser/src/connection.rs#L774 803
pub fn send_command(cmd: Value, session: &str) -> Result<Response, String>
```

Sends a JSON command to the daemon and deserialises the response. The retry
loop handles transient IPC failures:

| Constant         | Value | Purpose                                              |
|------------------|-------|------------------------------------------------------|
| `MAX_RETRIES`    | `5`   | Maximum attempts before giving up.                    |
| `RETRY_DELAY_MS` | `200` | Base delay between retries, **scaled by attempt number** (`RETRY_DELAY_MS * attempt`). |

The delay grows linearly: 0 ms on the first attempt, 200 ms on the second,
400 ms on the third, etc.

**Flow per attempt:**

1. Sleep if `attempt > 0` (scaled delay).
2. Call `send_command_once(&cmd, session)`.
3. On success, return the response immediately.
4. On error, call `is_transient_error(&e)`. If transient, record the error
   and continue the loop. If non-transient, fail immediately.

If all retries are exhausted, returns an error message including the last
transient error and the retry count.

### `send_command_once`

```reshape/crates/agent-browser/src/connection.rs#L829 849
fn send_command_once(cmd: &Value, session: &str) -> Result<Response, String>
```

The single-attempt implementation:

1. `connect(session)` — opens a `Connection` to the daemon.
2. Sets read timeout to **30 s** and write timeout to **5 s**.
3. Serialises `cmd` to JSON, appends `\n`, writes it to the stream.
4. Reads one line from a `BufReader` wrapping the stream.
5. Deserialises the line as `Response`.

The newline delimiter makes the protocol simple and line-oriented — each
request and response is exactly one JSON line.

---

## 11. Transient Error Detection

```reshape/crates/agent-browser/src/connection.rs#L811 827
fn is_transient_error(error: &str) -> bool
```

Classifies an error string as transient (worth retrying) or permanent (fail
immediately). The function checks for substring matches against known OS and
IO error patterns:

| Pattern                       | Meaning                                                    |
|-------------------------------|------------------------------------------------------------|
| `"os error 35"`               | `EAGAIN` on macOS                                          |
| `"os error 11"`               | `EAGAIN` on Linux                                          |
| `"WouldBlock"`                | Rust IO `WouldBlock` error kind                            |
| `"Resource temporarily unavailable"` | `EAGAIN`/`EWOULDBLOCK human-readable form         |
| `"EOF"`                       | Daemon closed the connection before sending a response     |
| `"line 1 column 0"`          | Empty JSON response (serde parse failure at position 0)    |
| `"Connection reset"`          | Peer reset the TCP connection                              |
| `"Broken pipe"`               | Write to a closed socket/pipe                              |
| `"os error 54"`               | `ECONNRESET` — connection reset by peer on macOS           |
| `"os error 104"`              | `ECONNRESET` — connection reset by peer on Linux           |
| `"os error 2"`                | `ENOENT` — socket file not found (daemon not started)      |
| `"os error 61"`               | `ECONNREFUSED` on macOS                                    |
| `"os error 111"`              | `ECONNREFUSED` on Linux                                    |
| `"os error 10061"`            | `ECONNREFUSED` on Windows                                  |
| `"os error 10054"`            | `ECONNRESET` on Windows                                    |

These cover the full spectrum of "daemon might recover" scenarios:

- **Socket not yet bound**: `ENOENT` (Unix socket missing), `ECONNREFUSED`
  (daemon still starting up).
- **Concurrent pressure**: `EAGAIN`/`EWOULDBLOCK` (kernel buffer full,
  retry after short wait).
- **Daemon crash/restart**: `ECONNRESET`, `EPIPE`, `EOF` (process died mid-
  connection; a restart may succeed).

Non-matching errors (e.g. permission denied, malformed JSON that isn't the
empty-string case) are treated as permanent and surfaced to the caller
immediately.

---

## Cross-Platform Summary

| Aspect              | Unix (macOS/Linux)                     | Windows                               |
|---------------------|----------------------------------------|----------------------------------------|
| Transport           | `UnixStream` → `.sock` file           | `TcpStream` → `127.0.0.1:{port}`      |
| Port derivation     | Not applicable                         | DJB2 hash → 49152–65534 range          |
| Port resolution     | Not applicable                         | `.port` sidecar file → hash fallback   |
| Readiness check     | `UnixStream::connect`                  | `TcpStream::connect_timeout` (50 ms)   |
| Process detach      | `setsid()` via `pre_exec`              | `CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS` |
| Process kill        | `SIGTERM` → 1 s grace → `SIGKILL`     | `taskkill /PID /F`                    |
| PID liveness        | `kill(pid, 0)` — EPERM = alive        | `OpenProcess` → non-zero handle = alive|
| Orphan detection    | `.sock` without `.pid`                 | Not applicable                         |
| Socket path limit   | ≤ 103 bytes (including null terminator)| Not applicable                         |
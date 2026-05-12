# Native Browser Engine Modules

> Architecture documentation for the in-process browser automation layer in
> `crates/agent-browser/src/native/`.

This document covers the three core modules that form the browser automation
engine's foundation: **browser lifecycle** (`browser.rs`), **command
dispatch & daemon state** (`actions.rs`), and **in-process daemon** (`daemon.rs`).

---

## 1. `browser.rs` — Browser Lifecycle Management

`browser.rs` owns the `BrowserManager` struct — the single point of control
for launching, connecting to, navigating within, and managing tabs on a
Chromium-compatible browser process via the Chrome DevTools Protocol (CDP).

### Key Types

#### `PageInfo`

```reshape/crates/agent-browser/src/native/browser.rs#L160-173
pub struct PageInfo {
    pub tab_id: u32,
    pub label: Option<String>,
    pub target_id: String,
    pub session_id: String,
    pub url: String,
    pub title: String,
    pub target_type: String,
}
```

Each `PageInfo` represents one open browser tab. The `tab_id` is a stable,
monotonically-increasing identifier formatted as `t1`, `t2`, … via
`format_tab_id()`. The `label` is a user-assigned, unique-within-session name
(e.g., `"docs"`, `"app"`). Labels are never auto-generated and never rewritten
on navigation — agents use them for readable multi-tab workflows.

#### `TabRef` — Tab Reference Enum

```reshape/crates/agent-browser/src/native/browser.rs#L185-188
pub enum TabRef {
    Id(u32),
    Label(String),
}
```

`TabRef::parse()` accepts strings like `t2` (stable id) or `docs` (label). It
rejects bare positional integers with a teaching error so agents and scripts
don't silently confuse stable ids with positional indices. Labels must start
with a letter and contain only letters, digits, `-`, and `_` (enforced by
`is_valid_label()`).

#### `WaitUntil` — Navigation Wait Strategy

```reshape/crates/agent-browser/src/native/browser.rs#L247-252
pub enum WaitUntil {
    Load,
    DomContentLoaded,
    NetworkIdle,
    None,
}
```

Controls how `navigate()` waits after issuing a `Page.navigate` CDP command:

- **Load** — wait for `Page.loadEventFired`.
- **DomContentLoaded** — wait for `Page.domContentEventFired`.
- **NetworkIdle** — wait for `poll_network_idle()` (no in-flight requests for
  ≥500 ms within a configurable overall timeout).
- **None** — return immediately after the CDP navigate response.

`WaitUntil::from_str()` maps string inputs to variants; unrecognized values
default to `Load`.

#### `BrowserProcess` — Process Handle

```reshape/crates/agent-browser/src/native/browser.rs#L265-268
pub enum BrowserProcess {
    Chrome(ChromeProcess),
    Lightpanda(LightpandaProcess),
}
```

Wraps the underlying browser process. `kill()` terminates it; `wait_or_kill()`
waits up to a timeout then forces termination; `has_exited()` is a
non-blocking check whether the process has crashed or been killed. For
Lightpanda, `has_exited()` always returns `false` (Lightpanda uses a
different lifecycle model).

#### `BrowserManager` — The Central Manager

```reshape/crates/agent-browser/src/native/browser.rs#L294-308
pub struct BrowserManager {
    pub client: Arc<CdpClient>,
    browser_process: Option<BrowserProcess>,
    ws_url: String,
    pages: Vec<PageInfo>,
    active_page_index: usize,
    default_timeout_ms: u64,
    pub download_path: Option<String>,
    pub ignore_https_errors: bool,
    visited_origins: HashSet<String>,
    next_tab_id: u32,
}
```

Holds the CDP WebSocket client, optional process handle, list of open pages,
and configuration inherited from launch options. The `visited_origins` set
tracks origins navigated to during the session (used by `state_save` for
cross-origin localStorage collection).

### Launch Validation

#### `validate_launch_options`

```reshape/crates/agent-browser/src/native/browser.rs#L21-59
pub fn validate_launch_options(
    extensions, has_cdp, profile, storage_state,
    allow_file_access, executable_path,
) -> Result<(), String>
```

Checks for incompatible option combinations before launch:

| Combination | Error |
|---|---|
| Extensions + CDP URL | Extensions require local launch |
| Profile + CDP URL | Profile requires local launch |
| Storage state + Profile | Cannot combine |
| Storage state + Extensions | Cannot combine |
| `allow_file_access` + non-Chromium | Not supported |

#### `validate_lightpanda_options`

```reshape/crates/agent-browser/src/native/browser.rs#L62-89
fn validate_lightpanda_options(options: &LaunchOptions) -> Result<(), String>
```

Lightpanda is a lightweight, headless-only CDP engine. This function rejects
Chrome-only features: extensions, profiles, storage state, file access,
headed mode, and custom Chrome arguments.

### Launch & Connect Flows

#### `launch`

```reshape/crates/agent-browser/src/native/browser.rs#L315-432
pub async fn launch(options: LaunchOptions, engine: Option<&str>) -> Result<Self, String>
```

The primary entry point for creating a `BrowserManager`. Flow:

1. **Validate options** — calls `validate_launch_options` for Chrome or
   `validate_lightpanda_options` for Lightpanda. Rejects unknown engine names.
2. **Start browser process** — for Chrome, `launch_chrome()` runs in a
   `spawn_blocking` task (Chrome startup is synchronous). For Lightpanda,
   `launch_lightpanda()` is async.
3. **Initialize manager** — for Lightpanda, calls `initialize_lightpanda_manager()`
   which handles the specialized CDP connect-with-retry and target discovery.
   For Chrome, creates a `CdpClient`, calls `discover_and_attach_targets()`.
4. **Apply launch-time overrides** — HTTPS error ignoring, user agent override,
   color scheme emulation, and download behavior are set on the active session.

#### `connect_cdp` / `connect_cdp_direct` / `connect_cdp_with_headers`

Three variants for connecting to an already-running browser:

- **`connect_cdp(url)`** — standard connection, discovers and attaches targets.
- **`connect_cdp_direct(url)`** — for provider CDP proxies where the WebSocket
  IS the page session; skips `Target.*` commands that most proxies don't
  support. Uses `enable_domains_direct()` instead of session-scoped domain
  enables.
- **`connect_cdp_with_headers(url, headers)`** — adds custom WebSocket headers
  (used by AgentCore provider).

All three call `connect_cdp_inner()` which resolves the CDP URL (supports
`http://` → WebSocket upgrade, plain `ws://` URLs, and port-only inputs)
before connecting.

#### `connect_auto`

Uses `auto_connect_cdp()` to discover a running Chrome instance's debug port
(via `~/chrome-debug-port` or well-known debug flags), then calls
`connect_cdp()`.

#### `discover_and_attach_targets`

```reshape/crates/agent-browser/src/native/browser.rs#L495-586
async fn discover_and_attach_targets(&mut self) -> Result<(), String>
```

After establishing a CDP connection, this function:

1. Sends `Target.setDiscoverTargets` to start receiving target lifecycle events.
2. Calls `Target.getTargets` to list existing targets.
3. Filters targets via `should_track_target()` — only `page` and `webview`
   types, excluding `chrome://`, `chrome-extension://`, and `devtools://`
   internal pages.
4. If no page targets exist, creates a new `about:blank` tab via
   `Target.createTarget`.
5. For each tracked target, calls `Target.attachToTarget` with `flatten: true`
   to get a dedicated session ID, then calls `enable_domains()` on each
   session.

### Domain Enable Sequence

```reshape/crates/agent-browser/src/native/browser.rs#L592-625
async fn enable_domains(&self, session_id: &str) -> Result<(), String>
```

Enables the minimal set of CDP domains needed for automation:

- `Page.enable` — lifecycle events (load, domContentLoaded)
- `Runtime.enable` — JS evaluation and console API
- `Runtime.runIfWaitingForDebugger` — resume targets paused after attach
  (required for Chrome 144+)
- `Network.enable` — request/response tracking
- `Target.setAutoAttach` with `flatten: true` — cross-origin iframe support

### Navigation

#### `navigate`

```reshape/crates/agent-browser/src/native/browser.rs#L652-697
pub async fn navigate(&mut self, url: &str, wait_until: WaitUntil) -> Result<Value, String>
```

1. Sends `Page.navigate` with the URL.
2. If `loader_id` is present (full navigation, not same-document), waits for
   the specified lifecycle event via `wait_for_lifecycle()`.
3. Retrieves updated URL and title, updates the active `PageInfo`.
4. Adds the page's origin to `visited_origins` for future state save.

Same-document navigations (hash routing, pushState) produce no `loader_id`,
so lifecycle waiting is skipped — the navigation is already complete.

#### `wait_for_lifecycle` & `poll_network_idle`

`wait_for_lifecycle()` subscribes to CDP events and waits for the matching
event name. For `NetworkIdle`, it delegates to `poll_network_idle()`.

`poll_network_idle()` tracks request IDs from `Network.requestWillBeSent`
and removes them on `Network.loadingFinished` / `Network.loadingFailed`. It
requires 500 ms of zero in-flight requests before declaring idle, with an
overall timeout. A `Page.loadEventFired` with zero pending requests also
starts the idle timer. The 600 ms per-event recv timeout prevents false
positives on cached pages where the subscription starts after load.

### Tab Management

#### `tab_new`

Creates a new browser target via `Target.createTarget`, attaches it, enables
domains, and pushes a new `PageInfo` with an assigned `tab_id`. Accepts an
optional `url` (defaults to `about:blank`) and optional `label` (validated
for uniqueness and format).

#### `tab_switch`

Switches `active_page_index`, re-enables domains on the new session, and
calls `Page.bringToFront`. Updates URL/title in the `PageInfo`.

#### `tab_close`

Removes the `PageInfo`, calls `Target.closeTarget`, adjusts
`active_page_index` (shifts back if the removed tab was before the active
one, clamps if it was the last), and re-enables domains on the now-active
session. Refuses to close the last tab.

### Emulation Methods

#### `set_viewport`

Sends `Emulation.setDeviceMetricsOverride` with width, height,
`deviceScaleFactor`, and `mobile` flag. Then attempts to resize the actual
browser window content area via `Browser.setContentsSize` (experimental CDP)
so screencast captures match the desired dimensions.

#### `set_user_agent`

Sends `Emulation.setUserAgentOverride` on the active session.

#### `set_emulated_media`

Sends `Emulation.setEmulatedMedia` with optional `media` type and/or
`features` array (e.g., `prefers-color-scheme` overrides).

### Error Formatting

#### `to_ai_friendly_error`

```reshape/crates/agent-browser/src/native/browser.rs#L135-157
pub fn to_ai_friendly_error(error: &str) -> String
```

Converts raw CDP/browser error messages into actionable, AI-friendly
descriptions. Recognized patterns:

| Raw Pattern | Friendly Message |
|---|---|
| "strict mode violation" | Use a more specific selector |
| "element is not visible" | Wait for visibility or scroll |
| "intercept" | Another element is covering — scroll or close overlays |
| "timeout" | Page may still be loading or element may not exist |
| "element not found" / "no element" | Verify selector is correct |

Unrecognized errors pass through unchanged.

---

## 2. `actions.rs` — Command Dispatcher & Daemon State

`actions.rs` is the largest module in the native layer. It defines the
per-session `DaemonState`, the central `execute_command()` dispatcher, and
all action handlers. It also manages background tasks for Fetch interception,
dialog auto-dismissal, HAR recording, and network request tracking.

### Key Structs

#### `DaemonState`

```reshape/crates/agent-browser/src/native/actions.rs#L202-263
pub struct DaemonState {
    pub browser: Option<BrowserManager>,
    pub appium: Option<AppiumManager>,
    pub safari_driver: Option<safari::SafariDriverProcess>,
    pub webdriver_backend: Option<WebDriverBackend>,
    pub backend_type: BackendType,
    pub ref_map: RefMap,
    pub domain_filter: Arc<RwLock<Option<DomainFilter>>>,
    pub event_tracker: EventTracker,
    pub session_name: Option<String>,
    pub session_id: String,
    pub tracing_state: TracingState,
    pub recording_state: RecordingState,
    event_rx: Option<broadcast::Receiver<CdpEvent>>,
    pub screencasting: bool,
    pub policy: Option<ActionPolicy>,
    pub pending_confirmation: Option<PendingConfirmation>,
    pub har_recording: bool,
    pub har_entries: Vec<HarEntry>,
    pub confirm_actions: Option<ConfirmActions>,
    pub inspect_server: Option<InspectServer>,
    pub routes: Arc<RwLock<Vec<RouteEntry>>>,
    pub tracked_requests: Vec<TrackedRequest>,
    pub request_tracking: bool,
    pub active_frame_id: Option<String>,
    pub iframe_sessions: HashMap<String, String>,
    pub origin_headers: Arc<RwLock<HashMap<String, HashMap<String, String>>>>,
    pub proxy_credentials: Arc<RwLock<Option<(String, String)>>>,
    fetch_handler_task: Option<JoinHandle<()>>,
    dialog_handler_task: Option<JoinHandle<()>>,
    pub mouse_state: MouseState,
    pub pending_dialog: Option<PendingDialog>,
    pub auto_dialog: bool,
    pub stream_client: Option<Arc<RwLock<Option<Arc<CdpClient>>>>>,
    pub stream_server: Option<Arc<StreamServer>>,
    launch_hash: Option<u64>,
    pub engine: String,
    pub default_timeout_ms: u64,
    pub viewport: Option<(i32, i32, f64, bool)>,
}
```

`DaemonState` is the **per-session state bag** that lives inside the daemon's
`Arc<Mutex<DaemonState>>` and is shared across all connection handlers. Key
field groups:

| Group | Fields | Purpose |
|---|---|---|
| **Browser backends** | `browser`, `appium`, `safari_driver`, `webdriver_backend`, `backend_type` | Supports CDP (Chrome/Lightpanda) and WebDriver (iOS Safari, desktop Safari) backends. `backend_type` selects the code path in handlers. |
| **Element references** | `ref_map` | Maps `@e<N>` element refs from accessibility snapshots to DOM object IDs. Cleared on navigation and tab switch. |
| **Network policy** | `domain_filter`, `routes`, `origin_headers`, `proxy_credentials` | Domain-level request blocking, URL-pattern route interception, origin-scoped extra headers, and proxy auth credentials. All wrapped in `Arc<RwLock<>>` so the background Fetch handler can read them concurrently. |
| **HAR / request tracking** | `har_recording`, `har_entries`, `tracked_requests`, `request_tracking` | Captures CDP `Network.*` events into HAR 1.2 entries or simpler `TrackedRequest` objects. |
| **Dialog / confirmation** | `pending_dialog`, `pending_confirmation`, `auto_dialog`, `confirm_actions`, `policy` | Auto-dismisses `alert` and `beforeunload` dialogs; surfaces `confirm`/`prompt` to the agent; action policy and confirmation gating. |
| **Background tasks** | `fetch_handler_task`, `dialog_handler_task` | Spawned tokio tasks that process CDP events in real-time (Fetch interception, dialog auto-dismiss). Aborted and restarted on browser relaunch. |
| **Stream / screencast** | `stream_client`, `stream_server`, `screencasting` | WebSocket stream server for live browser view. `stream_client` is a shared slot so the stream server receives the CDP client when the browser launches. |
| **Session identity** | `session_id`, `session_name`, `engine`, `launch_hash` | Used for socket directory paths, storage state files, and relaunch detection (hash of `LaunchOptions`). |
| **Emulation state** | `mouse_state`, `viewport` | Tracks mouse position/button state for dispatch; last viewport settings re-applied to new contexts. |

#### `HarEntry`

```reshape/crates/agent-browser/src/native/actions.rs#L67-93
pub struct HarEntry {
    pub request_id: String,
    pub wall_time: f64,
    pub method: String,
    pub url: String,
    pub request_headers: Vec<(String, String)>,
    pub post_data: Option<String>,
    pub request_body_size: i64,
    pub resource_type: String,
    pub status: Option<i64>,
    pub status_text: String,
    pub http_version: String,
    pub response_headers: Vec<(String, String)>,
    pub mime_type: String,
    pub redirect_url: String,
    pub response_body_size: i64,
    pub cdp_timing: Option<Value>,
    pub loading_finished_timestamp: Option<f64>,
}
```

Populated incrementally from CDP events: `Network.requestWillBeSent` fills
request fields, `Network.responseReceived` fills response fields and CDP
timing, `Network.loadingFinished` updates `response_body_size` and timestamp.
Serialized to HAR 1.2 format via `har_entry_to_json()`.

#### `RouteEntry` & `RouteResponse`

```reshape/crates/agent-browser/src/native/actions.rs#L95-110
pub struct RouteEntry {
    pub url_pattern: String,
    pub response: Option<RouteResponse>,
    pub abort: bool,
    pub resource_types: Vec<String>,
}
pub struct RouteResponse {
    pub status: Option<u16>,
    pub body: Option<String>,
    pub content_type: Option<String>,
    pub headers: Option<HashMap<String, String>>,
}
```

Routes intercept matching requests via CDP `Fetch.requestPaused`. A route can
either **abort** the request (`Fetch.failRequest`) or **fulfill** it with a
synthetic response (`Fetch.fulfillRequest`). The `resource_types` field
filters which CDP resource types the route matches (case-insensitive).

#### `TrackedRequest`

Lightweight request tracking struct used by the `requests` and
`request_detail` commands. Populated from `Network.requestWillBeSent` and
`Network.responseReceived`.

#### `FetchPausedRequest`

Captures a paused request from `Fetch.requestPaused`, including original
request headers. Needed because `Fetch.continueRequest` replaces (not merges)
headers, so the background handler must reconstruct the full header set when
injecting origin-scoped headers.

#### `BackendType`

```reshape/crates/agent-browser/src/native/actions.rs#L142-145
pub enum BackendType {
    Cdp,
    WebDriver,
}
```

Selects which code path handlers use. CDP-backed actions use
`BrowserManager` + `CdpClient`; WebDriver-backed actions use
`WebDriverBackend`. Some actions (annotations, React inspection) are not
supported on WebDriver and return errors.

#### `PendingDialog` & `MouseState`

- `PendingDialog` — tracks the currently open JavaScript dialog type, message,
  URL, and default prompt value.
- `MouseState` — tracks current mouse position (`x`, `y`) and button mask for
  CDP `Input.dispatchMouseEvent` calls.

#### `PendingConfirmation`

```reshape/crates/agent-browser/src/native/actions.rs#L61-64
pub struct PendingConfirmation {
    pub action: String,
    pub cmd: Value,
}
```

Set when an action requires confirmation (via `ActionPolicy` or
`ConfirmActions`). The agent must then send `confirm` or `deny` before the
original action proceeds.

#### `DrainedEvents` (internal)

```reshape/crates/agent-browser/src/native/actions.rs#L163-172
struct DrainedEvents {
    pending_acks: Vec<i64>,
    new_targets: Vec<TargetCreatedEvent>,
    changed_targets: Vec<TargetInfoChangedEvent>,
    destroyed_targets: Vec<String>,
    attached_iframe_sessions: Vec<(String, String)>,
    detached_iframe_sessions: Vec<String>,
}
```

Collected by `drain_cdp_events()` from the CDP broadcast channel. Applied
by `apply_drained_events()` — acks screencast frames, attaches new targets
(with domain enable + domain filter installation), updates target info,
removes destroyed targets, and tracks cross-origin iframe sessions.

### `execute_command` — The Central Dispatcher

```reshape/crates/agent-browser/src/native/actions.rs#L1153-1496
pub async fn execute_command(cmd: &Value, state: &mut DaemonState) -> Value
```

The **core dispatch function** that every daemon command passes through. Its
pipeline:

1. **Extract action** — reads `cmd["action"]` as the handler name.
2. **Broadcast to stream** — if a stream server is active, broadcasts the
   command name and ID to WebSocket clients.
3. **Drain CDP events** — calls `drain_cdp_events_background()` to process
   pending browser events before acting on the command.
4. **Policy check** — if an `ActionPolicy` file exists, hot-reloads it and
   checks the action. `Deny` returns an error; `RequiresConfirmation` sets
   `pending_confirmation` and returns `{ confirmation_required: true }`.
5. **Confirmation check** — `ConfirmActions` (env-var-based) may also gate
   actions.
6. **Auto-launch** — for actions that require a browser (not in the
   `skip_launch` list), checks if the existing browser is alive. If the
   process has exited or the CDP connection is dead, closes the stale
   browser and calls `auto_launch()` to create a fresh one. Also ensures at
   least one page exists (`ensure_page()`).
7. **WebDriver rejection** — CDP-only actions are rejected if the backend is
   `WebDriver`.
8. **Dispatch** — a large `match` on the action string, calling the
   corresponding `handle_*` function. Returns `Ok(data)` or `Err(message)`.
9. **Post-processing** — converts errors to AI-friendly messages via
   `to_ai_friendly_error()`. If a pending JavaScript dialog exists, appends
   a `warning` field to the response. Broadcasts the result, duration, and
   updated tab list to the stream server.

The `skip_launch` set includes actions that don't need a browser (`launch`,
`close`, `har_stop`, `credentials_*`, `auth_*`, `state_*`, `stream_*`,
`device_list`).

### Auto-Launch

```reshape/crates/agent-browser/src/native/actions.rs#L1515-1637
async fn auto_launch(state: &mut DaemonState) -> Result<(), String>
```

Called when a command requires a browser but none is active. Tries these
strategies in order:

1. **`AGENT_BROWSER_CDP`** — connect to an explicit CDP URL.
2. **`AGENT_BROWSER_AUTO_CONNECT`** — discover and connect to a running Chrome
   with `connect_auto_with_fresh_tab()`.
3. **`AGENT_BROWSER_PROVIDER`** — connect to a cloud provider (AgentCore,
   Browserbase, etc.) via `providers::connect_provider()`.
4. **Local launch** — build `LaunchOptions` from env vars via
   `launch_options_from_env()`, then `BrowserManager::launch()`.

Each strategy: creates the `BrowserManager`, subscribes to events, starts the
Fetch handler and dialog handler, updates the stream client, applies init
scripts, and loads storage state.

### Key Handlers

#### `handle_launch`

```reshape/crates/agent-browser/src/native/actions.rs#L1806-2112
async fn handle_launch(cmd: &Value, state: &mut DaemonState) -> Result<Value, String>
```

The most complex handler. Builds `LaunchOptions` from the command JSON,
computes a `launch_hash` of options that require a browser relaunch, and
decides whether to reuse the existing browser or relaunch:

- **Reuse** — if the hash matches, no storage-state clean launch is needed,
  and the connection is alive, just loads storage state and returns
  `{ launched: true, reused: true }`.
- **Relaunch** — if options changed, storage state requires a clean launch,
  or the connection is stale, closes the old browser and launches fresh.

Supports multiple connection modes: CDP URL, CDP port, auto-connect, cloud
provider, and local launch (Chrome or Lightpanda engine). For providers,
special handling for iOS/Safari (WebDriver) and AgentCore (WebSocket headers).

After launch: subscribes to events, starts Fetch/dialog handlers, installs
domain filter and proxy auth interception, loads storage state, applies init
scripts.

#### `handle_navigate`

```reshape/crates/agent-browser/src/native/actions.rs#L2205-2283
async fn handle_navigate(cmd: &Value, state: &mut DaemonState) -> Result<Value, String>
```

1. Checks the domain filter — if the URL's hostname isn't allowed, returns an
   error.
2. If WebDriver backend is active (and no CDP browser), delegates to
   `WebDriverBackend::navigate()`.
3. Parses `waitUntil` from the command.
4. If `headers` are provided, stores them keyed by the target origin in
   `origin_headers`. On first use, enables `Fetch.enable` with a wildcard
   pattern so the background handler can inject headers in real-time.
5. Clears `ref_map`, `iframe_sessions`, `active_frame_id`.
6. Calls `BrowserManager::navigate()`.

#### `handle_snapshot`

```reshape/crates/agent-browser/src/native/actions.rs#L2451-2501
async fn handle_snapshot(cmd: &Value, state: &mut DaemonState) -> Result<Value, String>
```

Takes an accessibility snapshot via `snapshot::take_snapshot()`. Options:
`selector` (scoped snapshot), `interactive` (only interactive elements),
`compact` (trimmed output), `maxDepth`, `urls`. Clears `ref_map` first so
fresh element refs are assigned. Returns `{ snapshot, origin, refs }`.

#### `handle_screenshot`

```reshape/crates/agent-browser/src/native/actions.rs#L2503-2609
async fn handle_screenshot(cmd: &Value, state: &mut DaemonState) -> Result<Value, String>
```

Captures a screenshot via `screenshot::take_screenshot()`. Options: `format`
(png/jpeg), `fullPage`, `quality`, `selector` (element-only), `path`,
`annotate` (overlay element refs on the image), `screenshotDir`. When
`annotate` is true, first takes an interactive snapshot to populate `ref_map`,
then captures and annotates.

#### `handle_click`

```reshape/crates/agent-browser/src/native/actions.rs#L2611-2686
async fn handle_click(cmd: &Value, state: &mut DaemonState) -> Result<Value, String>
```

Delegates to `interaction::click()` with `selector`, `button` (left/right/middle),
and `clickCount`. Special `--new-tab` mode: resolves the element's `href`,
opens a new tab with that URL instead of clicking in-page.

#### `handle_fill` / `handle_type` / `handle_press`

- **`fill`** — clears the input and sets the value atomically via
  `interaction::fill()`.
- **`type`** — types text character-by-character via `interaction::type_text()`
  with optional `clear` and `delay` parameters.
- **`press`** — sends a key event via `interaction::press_key_with_modifiers()`.
  Supports chord notation like `"Control+a"` parsed by `parse_key_chord()`
  into a key name and CDP modifier bitmask (Alt=1, Control=2, Meta=4, Shift=8).

#### `handle_wait`

```reshape/crates/agent-browser/src/native/actions.rs#L2946-2985
async fn handle_wait(cmd: &Value, state: &mut DaemonState) -> Result<Value, String>
```

Polymorphic wait based on which parameter is present:

| Parameter | Behavior |
|---|---|
| `text` | Wait for text to appear in the DOM |
| `selector` | Wait for element to match state (`visible`, `hidden`, `attached`, `detached`) |
| `url` | Wait for URL pattern match |
| `function` | Wait for JS function to return truthy |
| `loadState` | Wait for lifecycle event (Load, DomContentLoaded, NetworkIdle) |
| None | Sleep for `timeout_ms` |

Uses `state.timeout_ms(cmd)` which falls back to `AGENT_BROWSER_DEFAULT_TIMEOUT`
(default 30 seconds).

#### `handle_auth_login`

```reshape/crates/agent-browser/src/native/actions.rs#L7430-7629
async fn handle_auth_login(cmd: &Value, state: &mut DaemonState) -> Result<Value, String>
```

Automated login flow using saved credentials:

1. Navigates to the credential URL with `AUTH_LOGIN_WAIT_UNTIL = Load`
   (not NetworkIdle — many apps keep background requests active indefinitely).
2. Finds the username field using a tiered selector strategy:
   - **Preferred selectors** (email-specific, autocomplete-based) for
     `AUTH_LOGIN_PREFERRED_SELECTOR_WINDOW_MS` (default 5 s).
   - **Fallback selectors** (generic text inputs) for the remaining timeout.
3. Fills username via `interaction::fill()`.
4. Finds and fills the password field (default selector:
   `input[type=password]`).
5. Finds and clicks the submit button using `wait_for_any_selector()` with
   common submit selectors.
6. Waits for post-submit navigation (`Page.frameNavigated` or
   `Page.loadEventFired`) up to 10 seconds, with a 2 s fallback sleep.

#### `handle_har_start` / `handle_har_stop`

- **`har_start`** — enables `Network.enable` on the active session and all
  iframe sessions, sets `har_recording = true`, clears `har_entries`.
- **`har_stop`** — stops recording, serializes all `HarEntry` objects to
  HAR 1.2 format via `har_entry_to_json()`, writes to disk (default path:
  `~/.agent-browser/tmp/har/har-<timestamp>.har`). Includes browser metadata
  from `Browser.getVersion`.

#### `handle_route` / `handle_unroute`

- **`route`** — adds a `RouteEntry` to `state.routes`, then rebuilds and sends
  `Fetch.enable` with updated patterns. The background `fetch_handler_task`
  matches paused requests against routes in `resolve_fetch_paused()`.
- **`unroute`** — removes routes matching a URL pattern (or clears all),
  then either updates `Fetch.enable` patterns or disables Fetch entirely
  if no patterns remain.

#### `handle_stream_enable` / `handle_stream_disable`

- **`stream_enable`** — creates a `StreamServer` on the requested port,
  writes the `.stream` file, sets `request_tracking = true`, updates the
  stream client slot.
- **`stream_disable`** — shuts down the stream server, removes `.stream`,
  `.engine`, `.provider` files.

### `drain_cdp_events`

```reshape/crates/agent-browser/src/native/actions.rs#L692-1137
fn drain_cdp_events(&mut self) -> DrainedEvents
```

Non-blocking drain of the CDP broadcast channel. Processes events by method:

| Method | Action |
|---|---|
| `Target.targetCreated` | Queue for attachment (if not already tracked) |
| `Target.targetInfoChanged` | Queue for update, or promote to new target if previously filtered |
| `Target.targetDestroyed` | Queue for removal |
| `Target.attachedToTarget` | Track iframe `(frame_id, session_id)` pairs |
| `Target.detachedFromTarget` | Remove iframe session mapping |
| `Runtime.consoleAPICalled` | Add to `event_tracker`, broadcast to stream |
| `Runtime.exceptionThrown` | Add error to `event_tracker`, broadcast to stream |
| `Network.requestWillBeSent` | Create `HarEntry` / `TrackedRequest` |
| `Network.responseReceived` | Update `HarEntry` / `TrackedRequest` with response data |
| `Network.loadingFinished` | Update `HarEntry` body size and timestamp |
| `Network.loadingFailed` | Mark `HarEntry` as failed |
| `Page.screencastFrame` | Collect ack session ID (fallback, stream server handles primary) |
| `Page.javascriptDialogOpening` | Set `pending_dialog` (unless auto-handled) |
| `Page.javascriptDialogClosed` | Clear `pending_dialog` |

Events are filtered by active session ID, with an exception for iframe
`Network.*` events when HAR recording or request tracking is active.

### Background Tasks

#### `start_fetch_handler`

```reshape/crates/agent-browser/src/native/actions.rs#L359-466
fn start_fetch_handler(&mut self)
```

Spawns a tokio task that subscribes to CDP events and processes:

- **`Fetch.authRequired`** — if proxy credentials are set, responds with
  `Fetch.continueWithAuth` providing the credentials; otherwise cancels auth.
- **`Fetch.requestPaused`** — delegates to `resolve_fetch_paused()` which
  checks domain filter (blocks disallowed domains), matches routes (abort or
  fulfill), and injects origin-scoped headers via `Fetch.continueRequest`.

#### `start_dialog_handler`

```reshape/crates/agent-browser/src/native/actions.rs#L471-525
fn start_dialog_handler(&mut self)
```

Spawns a tokio task that auto-dismisses `alert` and `beforeunload` dialogs
via `Page.handleJavaScriptDialog` with `accept: true`. Only active when
`auto_dialog` is enabled (default true, disabled by
`AGENT_BROWSER_NO_AUTO_DIALOG=1`). `confirm` and `prompt` dialogs are left
for the agent to handle explicitly.

---

## 3. `daemon.rs` — In-Process Daemon

`daemon.rs` implements the long-running daemon process that accepts commands
over a Unix domain socket (or TCP on Windows) and routes them through
`execute_command()`.

### `run_daemon` — Top-Level Entry Point

```reshape/crates/agent-browser/src/native/daemon.rs#L19-155
pub async fn run_daemon(session: &str)
```

The daemon lifecycle:

1. **Create socket directory** — `get_daemon_socket_dir()` resolves from
   `AGENT_BROWSER_SOCKET_DIR`, `XDG_RUNTIME_DIR`, `~/.agent-browser`, or
   `/tmp/agent-browser`.
2. **Log setup** — In embedded mode, stderr is untouched (library caller
   owns it). In debug mode (`AGENT_BROWSER_DEBUG`), redirects stderr to
   `<socket_dir>/<session>.log`. Otherwise redirects to `/dev/null` to
   prevent SIGPIPE when the parent CLI drops the piped stderr handle.
3. **Write pid and version files** — `<session>.pid` and `<session>.version`.
4. **Clean stale files** — removes `.sock`, `.stream`, `.engine`, `.provider`,
   `.extensions` files from previous runs.
5. **State expiry** — if `AGENT_BROWSER_STATE_EXPIRE_DAYS` is set, runs
   `state::state_clean()` to purge old saved states.
6. **Start stream server** — creates a `StreamServer` on the preferred port
   (from `AGENT_BROWSER_STREAM_PORT`, default 0 = OS-assigned). Writes the
   `.stream` file with the actual port.
7. **Read idle timeout** — `AGENT_BROWSER_IDLE_TIMEOUT_MS` (0 or unset = no
   timeout).
8. **Run socket server** — delegates to `run_socket_server()`.
9. **Cleanup** — removes all filesystem artifacts (`.sock`/`.port`, `.pid`,
   `.version`, `.stream`, `.engine`, `.provider`, `.extensions`).

### `run_socket_server` (Unix)

```reshape/crates/agent-browser/src/native/daemon.rs#L158-260
async fn run_socket_server(socket_path, session, stream_client, stream_server, idle_timeout_ms)
```

The main event loop:

1. **Bind Unix socket** — creates a `UnixListener` on `<session>.sock`.
2. **Initialize `DaemonState`** — wrapped in `Arc<Mutex<DaemonState>>` for
   shared access across connection handlers.
3. **Set up channels** — `reset_tx`/`reset_rx` for idle timeout resets;
   `close_notify` (`Arc<Notify>`) for graceful shutdown after `close` command.
4. **Event loop** — `tokio::select!` over:

| Branch | Behavior |
|---|---|
| `listener.accept()` | Spawn `handle_connection()` for each new client |
| `drain_interval.tick()` (100 ms) | Drain CDP events; detect browser process exit |
| `idle_sleep_pin` (if idle timeout) | Auto-shutdown after inactivity |
| `reset_rx.recv()` (if idle timeout) | Reset idle timer on command activity |
| `close_notify.notified()` | Graceful exit after `close` command |
| `shutdown_signal()` | Graceful exit on SIGINT/SIGTERM/SIGHUP |

The 100 ms drain interval ensures CDP events (console logs, errors, target
changes) are processed promptly even when no commands arrive.

### `run_socket_server` (Windows)

```reshape/crates/agent-browser/src/native/daemon.rs#L263-360
async fn run_socket_server(socket_path, session, stream_client, stream_server, idle_timeout_ms)
```

Same structure as Unix, but uses `TcpListener` on `127.0.0.1:<port>`. The port
is derived from the session name via `get_port_for_session()` (a djb2 hash
mapped into the ephemeral range 49152–65535). If that port is unavailable
(e.g., Windows Hyper-V excluded range), falls back to `0` (OS-assigned).
Writes `<session>.port` file with the actual port.

### `handle_connection` — Per-Connection Handler

```reshape/crates/agent-browser/src/native/daemon.rs#L362-435
async fn handle_connection(stream, state, idle_reset_tx, stream_file_cleanup, close_notify)
```

1. **Split stream** into reader/writer, wrap reader in `BufReader`.
2. **Read lines** in a loop. Each line is a JSON command.
3. **Reject HTTP** — if the line looks like an HTTP method (`GET `, `POST `,
   etc.), breaks the loop (prevents HTTP clients from connecting to the
  socket).
4. **Parse JSON** — invalid JSON returns an error response immediately.
5. **Reset idle timer** — sends `try_send(())` on `idle_reset_tx`.
6. **Detect `close`** — if action is `"close", sets a flag.
7. **Execute command** — locks `DaemonState`, calls `execute_command()`.
8. **Write response** — JSON + newline.
9. **After `close`** — removes the `.stream` file, sleeps 100 ms (lets the
   response flush), then notifies `close_notify` to break the daemon loop.
   This graceful exit ensures destructors fire and Chrome processes are
   properly cleaned up (issue #1113 — previously `process::exit()` skipped
   destructors and orphaned Chrome).

### `shutdown_signal`

```reshape/crates/agent-browser/src/native/daemon.rs#L444-487
async fn shutdown_signal()
```

Platform-specific signal handling:

- **Unix** — installs handlers for SIGINT, SIGTERM, and SIGHUP via
  `tokio::signal::unix`. Waits for any of them in a `tokio::select!`.
- **Windows** — uses `tokio::signal::ctrl_c()`.

When a signal is received, the daemon loop breaks, the browser is closed,
and filesystem artifacts are cleaned up.

### Idle Timeout Handling

The daemon can auto-shutdown after a configurable period of inactivity:

- `AGENT_BROWSER_IDLE_TIMEOUT_MS` — milliseconds with no commands received.
- Each command resets the timer via the `reset_rx` channel.
- The timer uses `tokio::time::sleep` pinned in the `tokio::select!` loop.
- When the timer fires, the browser is closed and the loop exits.

### Embedded Daemon Mode

When `AGENT_BROWSER_EMBEDDED_DAEMON` is set, the daemon runs as an in-process
background task rather than a separate process. In this mode:

- stderr is not redirected (the host CLI or library owns it).
- The socket server still runs, accepting commands from the same process or
  external clients.
- This enables library-style integration where the caller spawns the daemon
  as a tokio task rather than via `std::process::Command`.

### Filesystem Artifacts

The daemon writes several files to `<socket_dir>/` for inter-process
coordination:

| File | Purpose |
|---|---|
| `<session>.pid` | Process ID for health checks |
| `<session>.version` | Cargo package version |
| `<session>.sock` | Unix domain socket path (Unix only) |
| `<session>.port` | TCP port number (Windows only) |
| `<session>.stream` | WebSocket stream server port |
| `<session>.engine` | Browser engine name (`chrome`, `lightpanda`, etc.) |
| `<session>.provider` | Cloud provider name (when using a provider) |
| `<session>.extensions` | Marker file for extension-loaded sessions |

All files are cleaned up on daemon exit (normal shutdown, signal, or close
command).

---

## Architecture Summary

The three modules form a layered architecture:

```
┌─────────────────────────────────────────────────┐
│  daemon.rs                                       │
│  Socket server, connection handler, signal/idle  │
│  Orchestrates DaemonState lifecycle              │
├─────────────────────────────────────────────────┤
│  actions.rs                                      │
│  DaemonState + execute_command dispatcher         │
│  Background tasks (Fetch, dialog)                │
│  All action handlers                            │
│  CDP event drain & apply                        │
├─────────────────────────────────────────────────┤
│  browser.rs                                      │
│  BrowserManager — CDP lifecycle & tab management │
│  Launch, connect, navigate, emulation            │
│  Validation, error formatting                    │
└─────────────────────────────────────────────────┘
```

**Key flows:**

- **Command flow**: Socket → `handle_connection` → `execute_command` →
  `handle_*` → `BrowserManager` method → CDP command → response.
- **Event flow**: CDP WebSocket → `CdpClient` broadcast →
  `drain_cdp_events` (100 ms interval) → `apply_drained_events` →
  state updates; or background tasks (Fetch handler, dialog handler)
  consuming events in real-time.
- **Launch flow**: `execute_command` detects stale/missing browser →
  `auto_launch` → strategy selection (CDP URL / auto-connect / provider /
  local) → `BrowserManager::launch` / `connect_cdp` → event subscription →
  background task startup → stream client update → init scripts → storage
  state.
- **Shutdown flow**: Signal / idle timeout / close command → browser close
  → cleanup filesystem artifacts → exit.
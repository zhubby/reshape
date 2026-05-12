# Agent Browser State Persistence, Auth, and Supporting Modules

This document covers the state management, authentication, and supporting utility modules in `native/`.

## State Persistence (`state.rs`)

Browser state persistence — saves and loads cookies + localStorage/sessionStorage per origin to/from JSON files, with optional AES-256-GCM encryption.

### Key Structs

| Struct | Fields | Purpose |
|--------|--------|---------|
| `StorageState` | `cookies: Vec<Cookie>`, `origins: Vec<OriginStorage>` | Top-level persisted state |
| `OriginStorage` | `origin`, `local_storage: Vec<StorageEntry>`, `session_storage: Vec<StorageEntry>` | Per-origin storage data |
| `StorageEntry` | `name`, `value` | Single key-value pair |

### Key Functions

- **`save_state(client, session, options)`** — Collects cookies via `Network.getAllCookies`, evaluates localStorage/sessionStorage via JS, merges visited origins with frame tree origins, collects cross-origin storage via temporary CDP targets, optionally encrypts, writes to disk.
- **`load_state(client, session, state_file)`** — Reads JSON (or decrypts `.enc` files), sets cookies via `Network.setCookies`, navigates to each origin to restore localStorage/sessionStorage.
- **`collect_storage_via_temp_target(client, origin, domain_filter)`** — Creates a temporary CDP target, navigates to origin with Fetch interception serving blank HTML, collects storage, closes target. Avoids real network requests.
- **`collect_storage_in_target(client, session_id, origin)`** — Evaluates storage JS and fulfills intercepted requests within a temp target session.
- **`encrypt_data(data, key)` / `decrypt_data(data, key)`** — AES-256-GCM encryption/decryption using SHA256-hashed key and random 12-byte nonce.
- **`find_auto_state_file(session_name)`** — Finds most recent state file matching session name pattern.
- **`dispatch_state_command(json)`** — Routes state subcommands from JSON payloads.

### State CRUD Operations

| Operation | Function | Description |
|-----------|----------|-------------|
| List | `state_list(json)` | Lists all state files in `~/.agent-browser/sessions/` with metadata |
| Show | `state_show(json)` | Displays contents and metadata of a state file |
| Clear | `state_clear(json)` | Deletes a specific state file |
| Clean | `state_clean(json)` | Deletes state files older than `AGENT_BROWSER_STATE_EXPIRE_DAYS` |
| Rename | `state_rename(json)` | Renames a state file |

### Directory Paths

- `get_state_dir()` → `~/.agent-browser`
- `get_sessions_dir()` → `~/.agent-browser/sessions`

## Authentication (`auth.rs`)

Encrypted credential storage for browser authentication profiles. Uses AES-256-GCM encryption with a Node.js-compatible JSON envelope format.

### Key Structs

| Struct | Fields | Purpose |
|--------|--------|---------|
| `AuthProfile` | `name`, `url`, `username`, `password`, `username_selector`, `password_selector`, `submit_selector`, `created_at`, `last_login_at` | Full auth profile with CSS selectors for automated login |
| `EncryptedPayload` | `version`, `encrypted`, `iv`, `auth_tag`, `data` | Node.js-compatible JSON envelope (all base64) |

### Key Functions

- **`validate_profile_name(name)`** — Ensures names match `[a-zA-Z0-9_-]+` (prevents path traversal).
- **`get_encryption_key()`** — Reads from `AGENT_BROWSER_ENCRYPTION_KEY` (64-char hex) or `~/.agent-browser/.encryption-key` file.
- **`ensure_encryption_key()`** — Auto-generates 256-bit key via `getrandom`, writes to `.encryption-key` with `0o600` permissions on Unix.
- **`encrypt_profile(profile, key)`** — Serializes to JSON, encrypts with AES-256-GCM (random 12-byte IV), builds JSON envelope `{version:1, encrypted:true, iv, authTag, data}`.
- **`decrypt_profile(data, key)`** — Parses JSON envelope or falls back to plain JSON. Base64-decodes IV/authTag/ciphertext. AES-GCM decrypts. Falls back to unencrypted JSON if envelope parsing fails.
- **`save_profile(profile, key)`** — Encrypts and writes to `~/.agent-browser/auth/{name}.json` with `0o600`.
- **`load_profile(name, key)`** — Reads and decrypts from file.
- **`credentials_set(name, username, password, url)`** — Quick save: name + username + password + optional URL.
- **`auth_save(name, url, username, password, selectors)`** — Full save: includes URL and all CSS selectors.
- **`credentials_get(name)`** — Returns `{name, username, url, hasPassword}` (never exposes password).
- **`credentials_get_full(name)`** — Returns full `AuthProfile` (including password).
- **`credentials_delete(name)`** — Deletes the profile file.
- **`credentials_list(key)`** — Enumerates all `.json` files in auth dir, attempts decryption.
- **`auth_show(name)`** — Returns metadata (name, url, username, selectors) without password.

## Network Control (`network.rs`)

Network-level control and event tracking via CDP.

### Domain Filter

- **`DomainFilter`** — Holds `allowed_domains: Vec<String>`. Supports wildcard patterns (`*.example.com`). Empty list = allow all.
- **`is_allowed(hostname)`** — Checks hostname against allowed patterns.
- **`check_url(url)`** — Validates full URL against filter, returns `Err` for blocked domains.
- **`sanitize_existing_pages(client, filter)`** — Navigates blocked pages to `about:blank`.

### Dual-Layer Enforcement

- **`install_domain_filter_script(client)`** — Injects JS that patches `WebSocket`, `EventSource`, and `navigator.sendBeacon` for client-side restriction.
- **`install_domain_filter_fetch(client)`** — Enables CDP `Fetch.enable` for server-side interception.
- **`install_domain_filter(client)`** — Convenience: installs both layers.

### Network Functions

| Function | CDP Command | Purpose |
|----------|-------------|---------|
| `set_extra_headers(client, headers)` | `Network.setExtraHTTPHeaders` | Inject custom headers into all requests |
| `set_offline(client, offline)` | `Network.emulateNetworkConditions` | Toggle offline mode (zero throughput) |
| `set_content(client, html)` | `Page.setDocumentContent` | Replace current page HTML |

### Console/Error Event Tracking

- **`ConsoleEntry`** — `{level, text, args: Vec<RemoteObject>}`
- **`ErrorEntry`** — `{text, url, line, column}`
- **`EventTracker`** — Circular buffer (1000 entries max) for console and error events. Overflow evicts oldest entries.
- **`add_console/add_error`** — Append entries with overflow eviction.
- **`get_console_json/get_errors_json`** — Serialize tracked events.
- **`format_console_arg(obj)`** — Converts CDP `RemoteObject` to human-readable string. Priority: `value` → `preview` → `description`. Handles arrays, objects, Maps, Sets.

## Action Policy (`policy.rs`)

Action-level policy engine governing which browser actions are allowed, denied, or require human confirmation.

### Key Types

- **`PolicyResult`** enum — `Allow`, `Deny(String)`, `RequiresConfirmation`
- **`ActionPolicy`** — JSON-loaded policy with `default`, `allow`, `deny`, `confirm` lists
- **`ConfirmActions`** — Runtime confirmation set from `AGENT_BROWSER_CONFIRM_ACTIONS` env var

### Decision Logic (`ActionPolicy::check`)

Priority: **deny > confirm > allow**

1. If action is in `deny` list → `Deny(reason)`
2. If action is in `confirm` list → `RequiresConfirmation`
3. If `allow` list is non-empty and action is in it → `Allow`
4. If `allow` list is non-empty and action is NOT in it → `Deny` (when default=deny)
5. If `allow` list is empty → `Allow` (allow all by default)

- **`ActionPolicy::reload()`** — Re-reads the JSON file for hot-reload without daemon restart.

## Remote Browser Providers (`providers.rs`)

Multi-provider browser connection manager.

### Key Structs

| Struct | Fields | Purpose |
|--------|--------|---------|
| `ProviderSession` | `provider`, `session_id` | Lightweight tracking for cleanup |
| `ProviderConnection` | `ws_url`, `session`, `direct_page` | Result of successful connection |
| `AgentCoreSessionInfo` | `session_id`, `browser_identifier`, `region`, `live_view_url` | Stored in thread-local RefCell |

### Provider Connections

| Provider | Env Key | Connection Method |
|----------|---------|-------------------|
| Browserbase | `BROWSERBASE_API_KEY` | POST to API → extract `connectUrl` + `id` |
| Browserless | `BROWSERLESS_API_KEY` | POST to session endpoint with TTL/stealth/browser type → `connectUrl` |
| Browser Use | `BROWSER_USE_API_KEY` | Construct WS URL from env var |
| Kernel | `KERNEL_API_KEY` | POST to API with headless/stealth/timeout config → extract CDP WS URL |
| AgentCore (AWS) | AWS credentials | Full AWS SigV4-signed PUT → `/browsers/{id}/sessions/start` → CDP WS URL |

### AgentCore AWS SigV4 Signing

Manual implementation (no AWS SDK dependency):

- `get_aws_credentials()` — env vars first, falls back to `aws configure export-credentials` CLI
- `sign_request(method, url, headers, body, credentials, region, service)` — Complete AWS4-HMAC-SHA256 signing:
  1. Canonical request (method + path + query + headers + signed headers + payload hash)
  2. String-to-sign (algorithm + timestamp + scope + canonical request hash)
  3. HMAC key derivation chain (date key → region key → service key → signing key)
  4. Authorization header construction
- Session info stored in `thread_local!` RefCell for cross-function access

### Cleanup (`close_provider_session`)

Sends provider-specific HTTP requests (POST/DELETE/PATCH) to each provider's session termination API.

## Recording (`recording.rs`)

Browser session recording via CDP screenshot capture piped through ffmpeg.

### Key Struct

- **`RecordingState`** — `{active, output_path, frame_count, capture_task, shared_frame_count, cancel_tx}`
  - `shared_frame_count: Arc<AtomicU64>` — cross-task frame counter
  - `cancel_tx: oneshot::Sender` — stop signal for capture loop

### Key Functions

- **`recording_start(state, path)`** — Marks active, sets output path. Rejects if already active.
- **`recording_stop(state)`** — Stops recording. Rejects if not active or zero frames.
- **`recording_restart(state, path)`** — Stops any active recording, starts new one.
- **`build_ffmpeg_command(path)`** — Constructs ffmpeg args: MJPEG pipe at 10fps, `image2pipe` format, pad filter for odd dimensions. Codec selection: `.webm` → libvpx, else → libx264.
- **`spawn_recording_task(client, session_id, state, cancel_rx)`** — Spawns tokio task that:
  - Loops on 100ms intervals
  - Captures screenshots via CDP `Page.captureScreenshot` (JPEG quality 80)
  - Base64-decodes → writes raw bytes to ffmpeg stdin
  - Uses `tokio::select!` with cancel channel
  - `kill_on_drop` on ffmpeg process
- **`stop_recording_task(state)`** — Sends cancel signal, awaits capture task, reads final frame count.

## Diff (`diff.rs`)

Visual and textual diff for screenshots and DOM snapshots.

### Key Structs

| Struct | Fields | Purpose |
|--------|--------|---------|
| `ScreenshotDiffResult` | `total_pixels`, `different_pixels`, `mismatch_percentage`, `matched`, `diff_image`, `dimension_mismatch` | Pixel comparison result |
| `SnapshotDiffResult` | `diff`, `additions`, `removals`, `unchanged`, `changed` | Text diff result |

### Key Functions

- **`diff_screenshot(a, b, threshold)`** — Decodes two images, checks dimension match, computes per-pixel Euclidean color distance against threshold. Builds diff image (red pixels for mismatches, dimmed grayscale for matches).
- **`diff_snapshots(before, after)`** — Myers diff via `similar` crate on line granularity. Fast path for identical inputs (avoids TextDiff construction). Returns unified diff with 3-line context radius.
- **`diff_text(before, after)`** — JSON output wrapper: `{identical, additions, removals, deletions, unchanged, changed}`.
- **`diff_unified(before, after)`** — Returns just the unified diff string.

## Storage (`storage.rs`)

Browser Web Storage access via CDP `Runtime.evaluate`.

| Function | JS Call | Returns |
|----------|---------|---------|
| `storage_get(client, session_id, storage_type, key)` | `storage.getItem(key)` or iterate all | `{key, value}` or `{data: {...}}` |
| `storage_set(client, session_id, storage_type, key, value)` | `storage.setItem(key, value)` | Success |
| `storage_clear(client, session_id, storage_type)` | `storage.clear()` | Success |
| `storage_js_name(type)` | — | `"session"` → `"sessionStorage"`, else → `"localStorage"` |

- **`eval_simple(client, session_id, expression)`** — Helper: sends `Runtime.evaluate` with `return_by_value=true`, checks for exceptions.

## Cookies (`cookies.rs`)

Browser cookie management via CDP `Network.*` commands.

### Cookie Struct

- **`Cookie`** — `{name, value, domain, path, expires, size, http_only, secure, session, same_site}`. Serialized with `camelCase` rename and skip-serializing-if annotations.

| Function | CDP Command | Description |
|----------|-------------|-------------|
| `get_all_cookies(client)` | `Network.getAllCookies` | Get all browser cookies |
| `get_cookies(client, urls)` | `Network.getCookies` | Get cookies for specified URLs |
| `set_cookies(client, cookies, current_url)` | `Network.setCookies` | Set cookies; auto-fills `url` from `current_url` if no domain/path |
| `clear_cookies(client)` | `Network.clearBrowserCookies` | Wipe all cookies |

## Tracing (`tracing.rs`)

CDP tracing and CPU profiling for browser sessions.

### Key Struct

- **`TracingState`** — `{active: bool, events: Vec<Value>, events_dropped: bool}`
- **`MAX_PROFILE_EVENTS`** = 5,000,000 — caps profile event collection

### Key Functions

| Function | CDP Commands | Description |
|----------|--------------|-------------|
| `trace_start(client, session_id)` | `Tracing.start` (recordContinuously, ReturnAsStream) | Start continuous trace |
| `trace_stop(client, session_id)` | `Tracing.end` → collect `Tracing.dataCollected` + `Tracing.tracingComplete` (30s timeout) → `IO.read` stream | Stop and save trace JSON |
| `profiler_start(client, session_id, categories)` | `Tracing.start` (includedCategories, enableSampling, ReportEvents) | Start CPU profiler with category filtering |
| `profiler_stop(client, session_id)` | Same collection loop, enforces MAX_PROFILE_EVENTS | Stop and save profile JSON |
| `read_io_stream(client, handle)` | `IO.read` in 1MB chunks | Read CDP IO stream until EOF |
| `get_clock_domain()` | — | OS-specific: `LINUX_CLOCK_MONOTONIC` / `MAC_MACH_ABSOLUTE_TIME` |

Output directories: `~/.agent-browser/tmp/traces/` and `~/.agent-browser/tmp/profiles/`.
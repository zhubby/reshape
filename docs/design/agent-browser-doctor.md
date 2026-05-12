# Agent Browser Doctor Diagnostic Module

The doctor subsystem (`crates/agent-browser/src/doctor/`) is a self-diagnostic framework that runs a battery of checks across environment, Chrome install, daemon state, config files, encryption, providers, network reachability, and a live headless browser launch test. It auto-cleans stale daemon sidecar files and offers destructive repairs (reinstalling Chrome, purging expired state files, generating a missing encryption key) gated behind `--fix`.

---

## 1. Core Framework — `doctor/mod.rs`

### `DoctorOptions`

Configuration flags that control doctor behaviour. All fields default to `false` via `#[derive(Default)]`.

| Field     | Type | Default | Purpose |
|-----------|------|---------|---------|
| `offline` | `bool` | `false` | Skip all network probes (Chrome CDN, AI Gateway, provider endpoints). |
| `quick`   | `bool` | `false` | Skip the live launch test — only static checks run. |
| `fix`     | `bool` | `false` | Attempt destructive repair actions on failing checks. |
| `json`    | `bool` | `false` | Emit structured JSON output instead of coloured terminal text. |

### `Status` enum

A `#[repr(u8)]` discriminated enum representing the outcome of a single check.

| Variant | Label | `as_str()` | Colour (text mode) | Meaning |
|---------|-------|------------|--------------------|---------|
| `Pass`  | `pass` | `"pass"` | Green | Check succeeded, no issues. |
| `Warn`  | `warn` | `"warn"` | Yellow | Non-critical issue detected; repair hint available. |
| `Fail`  | `fail` | `"fail"` | Red | Critical issue; agent-browser may not function correctly. |
| `Info`  | `info` | `"info"` | Dimmed | Advisory — no problem, but worth knowing (e.g. optional feature not configured). |

### `Check` struct

The atomic diagnostic result produced by every sub-module.

| Field      | Type             | Purpose |
|------------|------------------|---------|
| `id`       | `String`         | Dot-separated check identifier (e.g. `"chrome.installed"`, `"daemon.session.local:main"`). Unique within a single doctor run. |
| `category` | `&'static str`   | Grouping heading for display (e.g. `"Chrome"`, `"Daemons"`, `"Security"`). |
| `status`   | `Status`         | Outcome of the check. |
| `message`  | `String`         | Human-readable description of the result. |
| `fix`      | `Option<String>` | Suggested repair command or instruction. Populated only for `Warn` and `Fail` statuses (and occasionally `Info` for optional features). |

Constructed via the builder-style `Check::new(id, category, status, message)` and optionally `.with_fix(fix)`.

### `Summary` struct

Aggregated counts across all checks after the run completes.

| Field  | Type   | Counts |
|--------|--------|--------|
| `pass` | `usize` | Number of `Pass` checks. |
| `warn` | `usize` | Number of `Warn` checks. |
| `fail` | `usize` | Number of `Fail` checks. |

`Info` checks are not counted in the summary.

### `run_doctor` flow

```
┌─────────────────────────────────────┐
│         environment::check          │
│         chrome::check               │
│         daemon::check               │
│         config::check               │
│         security::check             │
│         providers::check            │
│  (skip network if --offline)        │
│         network::check              │
│  (skip launch if --quick)           │
│         launch::check               │
├─────────────────────────────────────┤
│  fix::run (only if --fix flag)      │
│  → mutates Check statuses to Pass  │
│  → appends to fixed list            │
├─────────────────────────────────────┤
│         summarize(&checks)          │
│         exit_code = fail > 0 ? 1 : 0│
├─────────────────────────────────────┤
│  json ? print_json : print_text     │
└─────────────────────────────────────┘
```

Each sub-module's `check(&mut Vec<Check>)` function appends its results to the shared vector. Checks run in a deterministic order, which determines the grouping and display order in the output.

### Output modes

#### Text mode (default)

- Prints `agent-browser doctor` header in bold.
- Groups checks by `category`, printing the category heading in bold whenever it changes.
- Each check line: `  <status-label>  <message>`.
- Fix hints (when present): `        fix: <fix>`.
- After all checks, a **Fixed** section lists repairs performed with `  done  <description>` markers.
- A **Summary** line: `"Summary: X pass, Y warn, Z fail"` coloured red (any fail), yellow (any warn), or green (all pass).
- If `--fix` was not run and at least one check has a `fix` hint, a tip line suggests re-running with `--fix`.

#### JSON mode (`--json`)

Emits a single `serde_json` payload to stdout:

```json
{
  "success": true,
  "summary": { "pass": 8, "warn": 2, "fail": 0 },
  "checks": [
    {
      "id": "env.version",
      "category": "Environment",
      "status": "pass",
      "message": "CLI version 0.5.0 (macOS aarch64)",
      "fix": null
    },
    ...
  ],
  "fixed": ["Reinstalled Chrome", "Closed 1 version-mismatched daemon(s)"]
}
```

The `success` field is `true` when `exit_code == 0` (no fails), `false` otherwise. Fix hints are omitted from the JSON when `null`.

---

## 2. Chrome Checks — `doctor/chrome.rs`

Validates the local Chrome/Chromium installation, cache directories, profile availability, and the optional Lightpanda engine.

### Functions

| Function | Purpose |
|----------|---------|
| `check(&mut Vec<Check>)` | Entry point; delegates to `find_chrome`, `query_chrome_version`, and checks for cache/profile/engine. |
| `query_chrome_version(path: &Path) -> Option<String>` | Runs `chrome --version` via `std::process::Command` and returns the trimmed stdout. Returns `None` on failure or empty output. |
| `puppeteer_cache_dir() -> Option<PathBuf>` | Resolves `PUPPETEER_CACHE_DIR` env var, or falls back to `~/.cache/puppeteer`. Returns `None` if home dir is unavailable. |

### Check IDs

| ID | Status | Condition |
|----|--------|-----------|
| `chrome.installed` | **Pass** | Chrome binary found via `find_chrome()`; version string available → `"${version} at ${path}"`. |
| `chrome.installed` | **Pass** | Chrome binary found but version query failed → `"Chrome at ${path} (version unknown)"`. |
| `chrome.installed` | **Fail** | No Chrome binary found. Fix: `"agent-browser install"`. |
| `chrome.cache_dir` | **Info** | The browser download cache (`get_browsers_dir()`) exists. |
| `chrome.puppeteer_cache` | **Info** | A Puppeteer cache directory exists alongside the agent-browser cache; will be used as a fallback. |
| `chrome.user_data_dir` | **Info** | Chrome user-data dir found with zero readable profiles. |
| `chrome.user_data_dir` | **Info** | Chrome user-data dir found with N readable profiles → `"${N} Chrome profile(s) at ${dir}"`. |
| `chrome.engine_lightpanda` | **Pass** | `AGENT_BROWSER_ENGINE=lightpanda` and `lightpanda` binary on PATH. |
| `chrome.engine_lightpanda` | **Fail** | `AGENT_BROWSER_ENGINE=lightpanda` but no `lightpanda` binary on PATH. Fix: `"install lightpanda or unset AGENT_BROWSER_ENGINE"`. |

---

## 3. Config File Checks — `doctor/config.rs`

Validates the three config file locations that agent-browser reads at startup.

### Config file locations

| Priority | Path | Source |
|----------|------|--------|
| User | `~/.agent-browser/config.json` | Resolved via `dirs::home_dir()`. |
| Project | `./agent-browser.json` | Relative to the current working directory. |
| Custom | `$AGENT_BROWSER_CONFIG` | Explicit override via environment variable. |

### Check IDs

| ID | Status | Condition |
|----|--------|-----------|
| `config.user` | **Pass** | User config exists and parses as valid JSON. |
| `config.user` | **Fail** | User config exists but contains invalid JSON. Fix: `"edit ${path}"`. |
| `config.project` | **Pass** | Project config exists and parses as valid JSON. |
| `config.project` | **Fail** | Project config exists but contains invalid JSON. Fix: `"edit ${path}"`. |
| `config.custom` | **Fail** | `AGENT_BROWSER_CONFIG` is set but points to a missing file. Fix: `"update or unset AGENT_BROWSER_CONFIG"`. |
| `config.custom` | **Pass** | `AGENT_BROWSER_CONFIG` file exists and parses as valid JSON. |
| `config.custom` | **Fail** | `AGENT_BROWSER_CONFIG` file exists but contains invalid JSON. Fix: `"edit ${path}"`. |

All parsing uses `parse_json_file` from `helpers.rs`, which only checks syntactic validity (any `serde_json::Value` is accepted, including arrays). Schema-level validation is done separately by the config module at load time.

---

## 4. Daemon Checks — `doctor/daemon.rs`

Inventories running daemon sessions, detects version mismatches with the CLI, and auto-cleans stale sidecar files as a side effect of the walk.

### Functions

The `check` function calls `walk_daemons()` from the `connection` module, which returns a `DaemonInventory` containing:
- `sessions`: active daemon sessions (name, pid, version).
- `cleaned`: stale sidecar files that were removed during the walk.
- `dashboard`: optional dashboard server status (pid, alive flag).

Each `cleaned` entry carries a `CleanReason` enum:

| `CleanReason` variant | Meaning |
|-----------------------|---------|
| `ProcessGone` / `DashboardGone` | The PID in the sidecar file no longer maps to a running process. |
| `UnreadablePidFile` | The `.pid` sidecar file could not be read. |
| `OrphanedSocket` | The `.sock` file exists but no matching PID file. |

Auto-cleanup is **always** performed (not gated by `--fix`) because stale files are harmless to remove and prevent "address already in use" errors on next launch.

### Check IDs

| ID | Status | Condition |
|----|--------|-----------|
| `daemon.cleaned.${name}` | **Warn** | Stale sidecar files removed for session `${name}`; reason string appended. |
| `daemon.active` | **Pass** | No active daemon sessions found. |
| `daemon.session.${name}` | **Pass** | Session is active and its version matches the CLI (`CARGO_PKG_VERSION`). |
| `daemon.session.${name}` | **Warn** | Session is active but its version mismatches the CLI. Fix: `"agent-browser --session ${name} close"`. |
| `daemon.dashboard` | **Pass** | Dashboard server is running with the given PID. |

---

## 5. Environment Checks — `doctor/environment.rs`

Validates the CLI version, home directory, state/socket directory writability, and available disk space.

### Check IDs

| ID | Status | Condition |
|----|--------|-----------|
| `env.version` | **Pass** | Always emitted: `"CLI version ${version} (${OS} ${ARCH})"`. |
| `env.home` | **Pass** | Home directory resolves via `dirs::home_dir()`. |
| `env.home` | **Fail** | Home directory cannot be determined. |
| `env.state_dir` | **Pass** | State dir exists and is writable (or combined `"State and socket directory"` when state and socket dirs coincide). |
| `env.state_dir` | **Fail** | State dir exists but is not writable. Fix: `"chmod u+rwx ${dir}"`. |
| `env.state_dir` | **Info** | State dir does not yet exist; will be created on first use. |
| `env.socket_dir` | **Pass** / **Fail** / **Info** | Same logic as `env.state_dir`, but emitted only when `XDG_RUNTIME_DIR` or `AGENT_BROWSER_SOCKET_DIR` diverts sockets to a different path. |
| `env.disk_free` | **Pass** | Free disk space ≥ 500 MB at state dir path. |
| `env.disk_free` | **Warn** | Free disk space < 500 MB. Fix: `"free up disk space; Chrome installs require ~500 MB"`. |
| `env.disk_free` | **Info** | `disk_free_bytes` is unavailable on the current platform (Windows, or non-standard OS). |

When the state directory and socket directory are the same (the default `~/.agent-browser`), they collapse into a single `env.state_dir` check. When they differ, both `env.state_dir` and `env.socket_dir` are emitted independently.

---

## 6. Repair Actions — `doctor/fix.rs`

Destructive repair actions gated behind the `--fix` flag. The `run` function iterates over all checks and applies targeted repairs for specific check IDs and statuses.

### Repair actions

| Action | Target check ID(s) | Trigger condition | Behaviour |
|--------|---------------------|-------------------|-----------|
| `attempt_chrome_install` | `chrome.installed` | Status == `Fail` | Shells out to `agent-browser install` via the current executable. Uses a subprocess so a failed install doesn't terminate the doctor process. |
| `close_all_sessions` | `daemon.session.*` (first `Warn` encountered) | Status == `Warn` (version mismatch) | Sends a `close` command to every active daemon session, then calls `cleanup_stale_files` for each. Runs only once — subsequent `daemon.session.*` Warn checks piggyback on the same result. |
| `purge_old_state` | `security.state_count` | Status == `Warn` | Deletes all state files in `get_sessions_dir()` whose `mtime` is older than `AGENT_BROWSER_STATE_EXPIRE_DAYS` (default 30 days). Returns the count of removed files. |
| `create_encryption_key` | `security.encryption_key` | Status == `Info` (no key present) | Creates `~/.agent-browser/.encryption-key` with a 64-character hex string generated via `getrandom`. Sets directory perms to `0o700` and file perms to `0o600` on Unix. Idempotent — returns `false` if the key file already exists. |

After a successful repair, the check's `status` is promoted to `Pass`, its `message` is appended with `" (fixed by --fix)"`, and the `fix` hint is cleared. The `fixed` vector collects human-readable summaries of each repair for display.

### `create_encryption_key_at` details

```text
1. mkdir -p ${dir}          (creates parent dirs)
2. chmod 0o700 ${dir}       (Unix only)
3. getrandom(&mut [0u8; 32]) → hex encode → 64-char string
4. write "${hex}\n" to ${dir}/.encryption-key
5. chmod 0o600 ${dir}/.encryption-key (Unix only)
```

---

## 7. Shared Utilities — `doctor/helpers.rs`

Stateless helper functions used across all doctor sub-modules.

### Functions

| Function | Signature | Purpose |
|----------|-----------|---------|
| `is_writable_dir` | `(path: &Path) -> bool` | Checks directory metadata: returns `true` if the directory exists and its permissions are not readonly. Returns `false` for missing or non-writable directories. |
| `human_size` | `(bytes: u64) -> String` | Converts byte counts to human-readable units (`B`, `KB`, `MB`, `GB`, `TB`). Values ≥ 1024 are scaled up; fractional values show one decimal place (e.g. `"1.4 MB"`). Sub-1024 values show raw bytes (e.g. `"512 B"`). |
| `disk_free_bytes` | `(path: &Path) -> Option<u64>` | Returns available disk space in bytes. On Unix, walks up to the first existing ancestor (for paths that don't yet exist), then calls `libc::statvfs`. Returns `None` on Windows and non-standard platforms. |
| `which_exists` | `(name: &str) -> bool` | Checks whether a binary is on PATH by running `which` (Unix) or `where` (Windows). Returns `true` if the command exits with success. |
| `parse_json_file` | `(path: &Path) -> Result<(), String>` | Reads a file and parses it as `serde_json::Value`. Returns `Ok(())` for any valid JSON (objects, arrays, primitives). Returns `Err` with `"read failed: ..."` or `"invalid JSON: ..."` on failure. |
| `new_id` | `() -> String` | Generates a unique `"doctor-${pid}-${micros}-${sequence}"` id for JSON command envelopes. Uses an `AtomicU64` counter for the sequence suffix, ensuring uniqueness across calls in the same process. |

---

## 8. Live Browser Launch Test — `doctor/launch.rs`

An end-to-end test that spawns a scratch daemon session, launches a headless Chrome instance, navigates to `about:blank`, and measures total elapsed time. Skipped under `--quick` or when a remote provider / external CDP connection is configured.

### `LaunchGuard` (RAII cleanup)

A struct that owns the scratch session name and implements `Drop`:

- On drop, sends a `close` command to the session.
- Calls `cleanup_stale_files` for the session name.
- Armed **only after** `ensure_daemon` succeeds — avoids sending stray `close` commands or deleting sidecar files for a daemon that never started.

This ensures cleanup happens even on panic or early return from the launch test.

### E2E launch test flow

```
1. Check AGENT_BROWSER_PROVIDER → skip (Info: "would consume cloud quota")
2. Check AGENT_BROWSER_CDP       → skip (Info: "would attach to a real browser")
3. Generate scratch session name: "doctor-${pid}-${epoch_ms}"
4. ensure_daemon(session, opts)  → Fail if daemon can't start
5. Arm LaunchGuard
6. send launch command (headless=true)
   → Fail if browser launch fails
7. send navigate command (url=about:blank)
   → Fail if navigation fails
8. Measure elapsed time from step 4
   → Warn if > 5s, Pass if ≤ 5s
9. LaunchGuard::drop → close + cleanup (automatic)
```

### Check IDs

| ID | Status | Condition |
|----|--------|-----------|
| `launch.skipped.provider` | **Info** | `AGENT_BROWSER_PROVIDER` is set; launch test skipped to avoid consuming cloud quota. |
| `launch.skipped.cdp` | **Info** | `AGENT_BROWSER_CDP` is set; launch test skipped to avoid interfering with a real browser connection. |
| `launch.daemon` | **Fail** | `ensure_daemon` failed. Fix: `"check Chrome install and re-run with --debug"`. |
| `launch.launch` | **Fail** | Browser launch command returned an error. Fix: `"agent-browser install   # or check --debug output"`. |
| `launch.navigate` | **Fail** | Navigation to `about:blank` failed. Fix: `"re-run with --debug for full launch logs"`. |
| `launch.elapsed` | **Warn** | Launch + navigation took > 5 seconds → `"${secs}s (slow; expected < 5s)"`. |
| `launch.elapsed` | **Pass** | Launch + navigation completed in ≤ 5 seconds → `"${secs}s"`. |

---

## 9. Network Probes — `doctor/network.rs`

Probes reachability of the Chrome for Testing CDN, AI Gateway (if configured), and the currently-selected provider endpoint. All probes use a 3-second timeout.

### Functions

| Function | Purpose |
|----------|---------|
| `check(&mut Vec<Check>)` | Creates a single-threaded tokio runtime and a `reqwest::Client` with 3s connect/read timeouts, then dispatches probes. |
| `probe_url` | Sends an HTTP `HEAD` request to the given URL. Classifies the response: success/redirect/405 → `Pass`; other HTTP status → `Warn`; connection error → `Fail` with a general fix hint. |

### Probe targets

| Target | URL | Condition |
|--------|-----|-----------|
| Chrome CDN | `https://googlechromelabs.github.io/chrome-for-testing/last-known-good-versions-with-downloads.json` | Always probed (unless `--offline`). |
| AI Gateway | `$AI_GATEWAY_URL` (default `https://ai-gateway.vercel.sh`) | Probed only when `AI_GATEWAY_API_KEY` is set. |
| Provider endpoint | Per-provider default URL (see table below) | Probed only when `AGENT_BROWSER_PROVIDER` is set and maps to a known endpoint. |

Provider endpoint defaults:

| Provider | Default URL | Override env var |
|----------|-------------|------------------|
| `browserbase` | `https://api.browserbase.com` | — |
| `browserless` | `https://production-sfo.browserless.io` | `BROWSERLESS_API_URL` |
| `browseruse` | `https://api.browser-use.com` | — |
| `kernel` | `https://api.onkernel.com` | `KERNEL_ENDPOINT` |

### Check IDs

| ID | Status | Condition |
|----|--------|-----------|
| `net.runtime` | **Fail** | Could not create a tokio runtime for async probes. |
| `net.client` | **Fail** | Could not build a `reqwest::Client`. |
| `net.chrome_cdn` | **Pass** | Chrome CDN reachable in `${ms}ms`. |
| `net.chrome_cdn` | **Warn** | Chrome CDN returned non-success HTTP status. |
| `net.chrome_cdn` | **Fail** | Chrome CDN unreachable. Fix: `"check network connectivity / firewall / proxy settings"`. |
| `net.ai_gateway` | **Pass** / **Warn** / **Fail** | Same pattern for the AI Gateway URL. |
| `net.provider` | **Pass** / **Warn** / **Fail** | Same pattern for the active provider endpoint. |

A `HEAD` request returning HTTP 405 (Method Not Allowed) is treated as `Pass` because some endpoints accept `GET` but not `HEAD`.

---

## 10. Provider Credential Checks — `doctor/providers.rs`

Checks API key or credential presence for all seven remote browser providers and the AI Gateway chat key. Results are `Info`-level unless the provider is explicitly selected via `AGENT_BROWSER_PROVIDER`, in which case missing credentials escalate to `Fail`.

### `active_status` logic

```text
active_status(provider, key_present) =
  if AGENT_BROWSER_PROVIDER == provider:
    key_present ? Pass : Fail
  else:
    Info
```

When a provider is **active** (selected via env var) and its credentials are missing, the check becomes `Fail` with a fix hint. When inactive, credentials are merely `Info` — advisory, not actionable.

### Providers

| Provider | Env var(s) for credentials | Check ID | Notes |
|----------|---------------------------|----------|-------|
| Browserless | `BROWSERLESS_API_KEY` | `providers.browserless` | API key check. |
| Browserbase | `BROWSERBASE_API_KEY` | `providers.browserbase` | API key check. |
| Browser Use | `BROWSER_USE_API_KEY` | `providers.browseruse` | API key check. |
| Kernel | `KERNEL_API_KEY` | `providers.kernel` | API key check. |
| AgentCore (AWS) | `AWS_ACCESS_KEY_ID` / `AWS_PROFILE` / `AWS_SESSION_TOKEN` (any one) | `providers.agentcore` | AWS credential resolution; not a single key but a profile/chain. |
| iOS (Appium) | `which_exists("appium")` | `providers.ios` | Only checked when active; verifies `appium` binary on PATH. Fix: `"npm install -g appium && appium driver install xcuitest"`. |
| Chat (AI Gateway) | `AI_GATEWAY_API_KEY` | `providers.chat` | Always `Info`; indicates whether the `chat` command is enabled. |

### Additional check

| ID | Status | Condition |
|----|--------|-----------|
| `providers.active` | **Info** | `AGENT_BROWSER_PROVIDER` is set; echoes the value for visibility. |

---

## 11. Security Posture — `doctor/security.rs`

Checks encryption key presence and permissions, saved state file age, and the optional action policy file validity.

### Encryption key resolution order

1. **Environment variable** (`AGENT_BROWSER_ENCRYPTION_KEY`): Must be a 64-character hex string (32 bytes). If present but malformed, the check fails with a fix hint to generate via `openssl rand -hex 32`.
2. **Key file** (`~/.agent-browser/.encryption-key`): Checked for existence and, on Unix, file permissions. If the file is group/world-readable (mode bits `0o077` != 0), the check warns with a `chmod 600` fix hint.
3. **Neither present**: Info-level with a fix hint. When `--fix` is active, `create_encryption_key` generates the file.

### State file expiry

State files in `get_sessions_dir()` are checked against a configurable expiry threshold:

- Default: **30 days** (`AGENT_BROWSER_STATE_EXPIRE_DAYS`, parsed as `u64`).
- Files whose `mtime` is older than `now - expire_days * 86400` are counted as "old".
- If old files exist: `Warn` with fix `"agent-browser state clean --older-than ${expire_days}"`.
- If no state files: `Info`.
- If all files are within expiry: `Pass` with total count.

### Action policy file

When `AGENT_BROWSER_ACTION_POLICY` is set:

- Missing file → `Fail` with fix `"update or unset AGENT_BROWSER_ACTION_POLICY"`.
- Invalid JSON → `Fail` with fix `"edit ${path}"`.
- Valid JSON → `Pass`.

### Check IDs

| ID | Status | Condition |
|----|--------|-----------|
| `security.encryption_key` | **Pass** | `AGENT_BROWSER_ENCRYPTION_KEY` set and is a valid 64-char hex string. |
| `security.encryption_key` | **Fail** | `AGENT_BROWSER_ENCRYPTION_KEY` set but malformed (not 64-char hex). Fix: `"export AGENT_BROWSER_ENCRYPTION_KEY=$(openssl rand -hex 32)"`. |
| `security.encryption_key` | **Pass** | Key file exists with safe permissions. |
| `security.encryption_key` | **Warn** | Key file exists but permissions are too open (group/world readable on Unix). Fix: `"chmod 600 ${path}"`. |
| `security.encryption_key` | **Info** | No encryption key present; will be auto-generated on first auth save. Fix: `"export AGENT_BROWSER_ENCRYPTION_KEY=$(openssl rand -hex 32)"`. |
| `security.state_count` | **Info** | No saved state files. |
| `security.state_count` | **Warn** | N state files older than the expiry threshold. Fix: `"agent-browser state clean --older-than ${expire_days}"`. |
| `security.state_count` | **Pass** | All state files within expiry threshold. |
| `security.action_policy` | **Fail** | `AGENT_BROWSER_ACTION_POLICY` points to a missing file. Fix: `"update or unset AGENT_BROWSER_ACTION_POLICY"`. |
| `security.action_policy` | **Fail** | `AGENT_BROWSER_ACTION_POLICY` file contains invalid JSON. Fix: `"edit ${path}"`. |
| `security.action_policy` | **Pass** | `AGENT_BROWSER_ACTION_POLICY` file is valid JSON. |

---

## Check ID Reference (Complete)

| ID | Module | Category |
|----|--------|----------|
| `env.version` | environment | Environment |
| `env.home` | environment | Environment |
| `env.state_dir` | environment | Environment |
| `env.socket_dir` | environment | Environment |
| `env.disk_free` | environment | Environment |
| `chrome.installed` | chrome | Chrome |
| `chrome.cache_dir` | chrome | Chrome |
| `chrome.puppeteer_cache` | chrome | Chrome |
| `chrome.user_data_dir` | chrome | Chrome |
| `chrome.engine_lightpanda` | chrome | Chrome |
| `daemon.cleaned.*` | daemon | Daemons |
| `daemon.active` | daemon | Daemons |
| `daemon.session.*` | daemon | Daemons |
| `daemon.dashboard` | daemon | Daemons |
| `config.user` | config | Config |
| `config.project` | config | Config |
| `config.custom` | config | Config |
| `security.encryption_key` | security | Security |
| `security.state_count` | security | Security |
| `security.action_policy` | security | Security |
| `providers.browserless` | providers | Providers |
| `providers.browserbase` | providers | Providers |
| `providers.browseruse` | providers | Providers |
| `providers.kernel` | providers | Providers |
| `providers.agentcore` | providers | Providers |
| `providers.ios` | providers | Providers |
| `providers.chat` | providers | Providers |
| `providers.active` | providers | Providers |
| `net.runtime` | network | Network |
| `net.client` | network | Network |
| `net.chrome_cdn` | network | Network |
| `net.ai_gateway` | network | Network |
| `net.provider` | network | Network |
| `launch.skipped.provider` | launch | Launch test |
| `launch.skipped.cdp` | launch | Launch test |
| `launch.daemon` | launch | Launch test |
| `launch.launch` | launch | Launch test |
| `launch.navigate` | launch | Launch test |
| `launch.elapsed` | launch | Launch test |

---

## Exit Code Semantics

| Exit code | Meaning |
|-----------|---------|
| `0` | All checks passed or only warnings/info (no failures). |
| `1` | At least one `Fail`-status check. |

The exit code is derived from `summary.fail > 0` and is independent of whether `--fix` was run. If repairs promoted all fails to passes, the exit code will be `0`.
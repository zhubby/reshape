# Agent Browser React Introspection, WebDriver, and Top-Level Modules

This document covers the React DevTools introspection, WebDriver/Safari/iOS automation, the inspect server proxy, and the remaining top-level source files.

## React Introspection (`native/react/`)

```
react/
├── mod.rs      — Module root, INSTALL_HOOK_JS constant
├── scripts.rs  — Browser-side JS evaluation scripts (~745 lines)
├── renders.rs  — Fiber render profiler types & formatter (~169 lines)
├── suspense.rs — Suspense boundary analysis (~633 lines)
├── tree.rs     — Component tree formatter (~67 lines)
├── vitals.rs   — Core Web Vitals + hydration timing (~160 lines)
```

### Module Root (`mod.rs`)

Declares submodules (`scripts`, `renders`, `suspense`, `tree`, `vitals`) and re-exports key types. Contains `INSTALL_HOOK_JS` — the React DevTools `installHook.js` vendored from `facebook/react` (MIT license). Injected via `addScriptToEvaluateOnNewDocument` when `--enable react-devtools` is passed.

### Scripts (`scripts.rs`)

Browser-side JavaScript constants injected via `Runtime.evaluate`. Assume `__REACT_DEVTOOLS_GLOBAL_HOOK__` is already installed.

| Constant | Purpose |
|-----------|---------|
| `TREE_SNAPSHOT` | Async IIFE that walks React fiber tree via DevTools hook operations protocol, collecting `{id, type, name, key, parent}` per fiber node |
| `TREE_INSPECT` | Template with `{{ID}}` placeholder — inspects single fiber element returning props, hooks, state, context, owner chain, source location |
| `RENDERS_INIT` | Installs fiber profiler by monkey-patching `hook.onCommitFiberRoot`. Tracks render counts, mounts, re-renders, self/total time, DOM mutations, change diffs. Uses `requestAnimationFrame` for FPS. Idempotent (`__AB_RENDERS_ACTIVE__` flag) |
| `RENDERS_STOP` | Stops profiler, cancels RAF loop, restores original `onCommitFiberRoot`, returns structured JSON profile |
| `SUSPENSE_WALK` | Async IIFE walking operations protocol to find Suspense boundaries, uses `ri.inspectElement` to extract `suspendedBy` metadata |
| `VITALS_INIT` | Installs PerformanceObservers for LCP, CLS, FCP, INP, intercepts `console.timeStamp` for React reconciler timings |
| `VITALS_READ` | Reads observed CWV metrics, React timing data, Navigation Timing TTFB |
| `PUSHSTATE` | SPA navigation: tries `next.router.push()` (Next.js RSC) first, then `history.pushState` + `popstate`/`navigate`. Template with `{{URL}}` |

### Render Profiler (`renders.rs`)

| Struct | Fields |
|--------|--------|
| `RendersData` | elapsed_time, fps_stats, total_renders/mounts/re-renders/components count, `Vec<Component>` |
| `FpsStats` | avg, min, max, drops (frames < 30fps) |
| `Component` | name, count, mounts, re-renders, instance_count, total/self time, DOM mutations, `Vec<Change>`, change_summary HashMap |
| `Change` | type (props/state/context/mount/parent), name, prev/next values |

`format_renders_report(d)` — Markdown-style table with top 50 components by total time, FPS stats, and change details (prev → next for top 15).

### Suspense Boundary Analysis (`suspense.rs`)

**BlockerKind Classification** — classifies what's making a boundary suspend, with actionability scores:

| Kind | Weight | Actionability | Examples |
|------|--------|---------------|----------|
| `ClientHook` | 90 | Highest | `useEffect`, `useLayoutEffect` |
| `RequestApi` | 88 | Very High | `fetch()`, `axios` calls |
| `ServerFetch` | 82 | High | Server-side data fetching |
| `Cache` | 74 | Moderate | Cache reads/misses |
| `Stream` | 60 | Low | Streaming data |
| `Framework` | 18 | Very Low | Next.js RSC, Remix loaders |
| `Unknown` | 35 | Minimal | Unrecognized patterns |

**Key Types:**

| Type | Purpose |
|------|---------|
| `Boundary` | Raw boundary: id, parent_id, name, is_suspended, environments, suspended_by, owners, jsx_source |
| `Suspender` | What's causing suspension: name, description, duration, env, owner/awaiter names + stack frames |
| `ActionableBlocker` | Classified blocker with key, kind, description, actionability score, suggestion |
| `BoundaryInsight` | Enriched boundary with primary_blocker, blockers, actionability, recommendation |
| `RootCauseGroup` | Grouped blockers by kind with count, boundary names, suggestion |
| `AnalysisReport` | Complete analysis: total/static/dynamic counts, holes, statics, root_causes, files_to_read |

**Key Flow:** `analyze_boundaries` → splits into holes (suspended) + statics → `build_insight` per boundary → `classify_blocker` per suspender → `build_root_causes` groups by kind → `suggest_blocker_fix` / `recommend_fix` generate suggestions.

### Component Tree (`tree.rs`)

- `TreeNode` — `{id, node_type, name, key, parent}`
- `format_tree(nodes)` — Depth-indented tree using parent→children HashMap
- `type_name()` — Maps fiber types: 11→Root, 12→Suspense, 13→SuspenseList

### Core Web Vitals (`vitals.rs`)

| Struct | Fields |
|--------|--------|
| `VitalsData` | url, ttfb, lcp (optional), cls, fcp, inp, hydration (optional), phases, hydrated_components |
| `Lcp` | start_time, size, element, url |
| `Cls` | score, entries |
| `HydrationRange` | start_time, end_time, duration |
| `Phase` | label, start/end/duration |
| `HydratedComponent` | name, start/end/duration |

`format_vitals_report(d)` — Markdown-style report with CWV section (TTFB/LCP/CLS/FCP/INP) and React Hydration section (phases table, top 30 hydrated components).

## Inspect Server (`inspect_server.rs`)

Lightweight HTTP + WebSocket server for the `agent-browser inspect` command. Enables using Chrome's built-in DevTools UI to inspect agent-browser sessions.

### Key Struct

- **`InspectServer`** — `{port: u16, accept_handle: JoinHandle<()>}`. Methods: `start(proxy_handle, target_id, chrome_host_port)`, `port()`, `shutdown()`.

### Connection Routing

1. `GET /` → HTTP 302 redirect to Chrome DevTools frontend URL (`devtools://devtools/bundled/inspector.html?ws=...`)
2. `WebSocket /ws` → Bidirectional CDP proxy:
   - Creates per-connection CDP session via `Target.attachToTarget`
   - Two proxy tasks: Chrome→DevTools (`strip_session_id`) and DevTools→Chrome (`inject_session_id`)
   - Uses `tokio::select!` for lifetime management
   - Detaches CDP session on cleanup

### Session ID Manipulation

- **`inject_session_id(message, session_id)`** — Adds `sessionId` to outgoing DevTools→Chrome messages
- **`strip_session_id(message)`** — Removes `sessionId` from incoming Chrome→DevTools messages
- Enables DevTools frontend to see a page-level view while the proxy routes to the correct CDP target session

## WebDriver (`native/webdriver/`)

```
webdriver/
├── mod.rs      — Module root declaring submodules
├── types.rs    — WebDriver protocol types (~97 lines)
├── backend.rs  — BrowserBackend trait + WebDriverBackend (~142 lines)
├── client.rs   — Raw TCP HTTP WebDriver client (~318 lines)
├── appium.rs   — Appium server management (~240 lines)
├── safari.rs   — SafariDriver process management (~80 lines)
├── ios.rs      — iOS simulator/device management (~235 lines)
```

### BrowserBackend Trait (`backend.rs`)

Unified interface so `actions.rs` remains backend-agnostic between CDP (Chromium) and WebDriver (Safari/iOS).

| Method | Purpose |
|--------|---------|
| `navigate(url)` | Go to URL |
| `get_url()` | Current page URL |
| `get_title()` | Page title |
| `get_content()` | Page HTML source |
| `evaluate(script)` | Execute JavaScript |
| `screenshot()` | Capture screenshot |
| `click(selector)` | Click element |
| `fill(selector, value)` | Fill input field |
| `close()` | Close browser |
| `back()` / `forward()` / `reload()` | Navigation |
| `get_cookies()` | Get cookies |
| `backend_type()` | Returns `"cdp"` or `"webdriver"` |
| `supports(feature)` | Feature check: CDP-only features (`screencast`, `tracing`, `network_intercept`, `cdp`) return true only for CDP backend |

### WebDriver Client (`client.rs`)

Raw TCP HTTP client for WebDriver protocol. No external HTTP library — uses `tokio::net::TcpStream` with 10-second connection timeout.

| Method | Endpoint | Purpose |
|--------|----------|---------|
| `new(port)` | — | Constructor with base URL `http://127.0.0.1:{port}` |
| `create_session(capabilities)` | POST `/session` | Create new browser session |
| `delete_session()` | DELETE `/session/{id}` | Close session |
| `navigate(url)` | POST `/session/{id}/url` | Go to URL |
| `find_element(using, value)` | POST `/session/{id}/element` | Find element (W3C + legacy ID formats) |
| `click_element(id)` | POST `/session/{id}/element/{id}/click` | Click element |
| `send_keys(id, text)` | POST `/session/{id}/element/{id}/value` | Type text |
| `execute_script(script, args)` | POST `/session/{id}/execute/sync` | Run JavaScript |
| `screenshot()` | GET `/session/{id}/screenshot` | Take screenshot (base64) |
| `execute_actions(actions)` | POST `/session/{id}/actions` | W3C Actions API (touch/pointer) |

### Appium Manager (`appium.rs`)

- **`AppiumManager`** — `{client, appium_process, device_udid}`. `Drop` kills the Appium process.
- **`connect_or_launch(device_udid)`** — Checks if Appium is running on port 4723, otherwise launches via `npx appium`.
- **`create_ios_session(device_name, platform_version)`** — Creates iOS session with XCUITest automation + Safari browser.
- **`tap(x, y)` / `swipe(...)`** — Touch gestures via W3C Actions API.

### SafariDriver (`safari.rs`)

- **`SafariDriverProcess`** — `{child, port}`. `Drop` kills and waits.
- **`find_safaridriver()`** — Searches `/usr/bin/safaridriver` and PATH.
- **`launch_safaridriver(port)`** — Spawns with `--port`, waits 500ms for readiness.

### iOS Device Management (`ios.rs`)

- **`IosDevice`** — `{name, udid, state, runtime, is_real}`
- **`list_simulators()`** — Parses `xcrun simctl list devices --json`
- **`list_real_devices()`** — Parses `xcrun xctrace list devices`
- **`boot_simulator(udid)` / `shutdown_simulator(udid)`** — `xcrun simctl boot/shutdown`
- **`select_device(device_name, udid)`** — Smart selection: by UDID → by name → default (prefer iPhone Pro)

## Top-Level Source Files

### Flags & Config (`flags.rs`)

Two parallel structs for configuration:

| Struct | Source | Fields |
|--------|--------|--------|
| `Config` | Config files (`~/.agent-browser/config.json`, `./agent-browser.json`) | ~30 fields: headed, json, debug, session, extensions, proxy, provider, etc. |
| `Flags` | CLI arguments | ~30 fields + `cli_*` tracking fields for daemon-relevant args |

Key functions:
- **`load_config()`** — Reads config files with `--config` override support. Merges user config → project config.
- **`parse_flags(args)`** — Parses CLI args into `Flags`. Handles global bool flags and flags-with-value.
- **`clean_args(args)`** — Removes global flags from args before command parsing.

Idle timeout parsing: raw ms (`3000`), seconds (`3s`), minutes (`5m`), hours (`1h`). Rejects `5S` (capital) and unknown units.

### Output (`output.rs`)

- **`OutputOptions`** — `{json, content_boundaries, max_output}`
- **`print_response_with_opts(response, opts)`** — Formats daemon responses for terminal output (~1000 lines). Handles every command type: navigation, snapshot, screenshot, cookies, storage, console, errors, tabs, network, etc.
- **`print_command_help()`** — Comprehensive CLI help text (~2900 lines)
- **`print_help()`** — Main help overview
- **`print_snapshot_diff()` / `print_screenshot_diff()`** — Formatted diff output
- **`print_version()`** — Version string

### Chrome Installation (`install.rs`)

- **`get_browsers_dir()`** → `~/.agent-browser/browsers`
- **`find_installed_chrome()`** — Platform-specific search for installed Chrome binaries
- **`fetch_download_url(platform_key)`** — Fetches Chrome for Testing download URL from CDN
- **`download_bytes(url)`** — Downloads with retry (3 attempts, exponential backoff)
- **`extract_zip(data, dest)`** — Extracts Chrome archive
- **`run_install(with_deps)`** — Main install flow: download → extract → report status
- **`install_linux_deps()`** — Platform-specific Linux dependency installation (~220 lines for apt/dnf/pacman/brew)

### Upgrade (`upgrade.rs`)

- **`InstallMethod`** enum — Npm, Pnpm, Yarn, Bun, Homebrew, Cargo, Unknown
- **`detect_install_method()`** — Multi-strategy detection: marker file → path inference → package manager probing
- **`fetch_latest_version()`** — Fetches from npm registry
- **`run_upgrade()`** — Auto-detect → auto-upgrade. Reports version delta (current → latest).

### Skills (`skills.rs`)

- **`SkillInfo`** — `{name, description, dir, hidden}` — parsed from Markdown frontmatter
- **`discover_skills()`** — Walks skill directories, parses frontmatter from `.md` files
- **`parse_frontmatter(text)`** — Extracts `name`, `description`, `hidden` from YAML-like frontmatter
- **`run_skills(args)`** — Dispatches to `list`, `get`, or `path` subcommands

### Validation (`validation.rs`)

- **`is_valid_session_name(name)`** — Alphanumeric + hyphens + underscores only
- **`session_name_error(name)`** — Error message for invalid names

### Color (`color.rs`)

Terminal color utilities following `NO_COLOR` spec (<https://no-color.org>):

- **Priority:** `NO_COLOR` (presence disables) > `AGENT_BROWSER_COLOR` (truthy enables) > default (off)
- Colors: `red()`, `green()`, `yellow()`, `cyan()`, `bold()`, `dim()`
- Indicators: `error_indicator()` (✗), `success_indicator()` (✓), `warning_indicator()` (⚠)
- `console_level_prefix(level)` — Colorized `[error]`, `[warning]`, `[info]` prefixes

### Test Utils (`test_utils.rs`)

- **`ENV_MUTEX`** — `static Mutex<()>` — global lock for serializing env var mutations in tests
- **`EnvGuard<'a>`** — RAII guard:
  - On construction: locks mutex, snapshots original env var values
  - `set(name, value)` / `remove(name)` — mutate with debug_assert safety checks
  - On drop: restores all vars to original state, releases mutex
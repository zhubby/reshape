# Agent-Browser Commands Module

**Source:** `crates/agent-browser/src/commands.rs`
**~5 100 lines** (implementation + ~2 430 lines of tests)

---

## 1. Purpose

The `commands` module is the **central translator** from human-friendly CLI syntax to the daemon JSON protocol. When a user (or an LLM agent) types something like `click .submit-btn` or `cookies set session_id abc123 --domain example.com`, `parse_command` turns that string into a structured JSON object that the background daemon can execute.

The design philosophy is:

- **CLI arguments are the source of truth.** The daemon speaks JSON; the CLI speaks verbs-and-flags. This module bridges the two worlds.
- **Every command produces a `serde_json::Value`.** The output is always a JSON object with at minimum an `"id"` and `"action"` key, plus whatever parameters the specific action needs.
- **Errors are contextual, not generic.** Every parse failure includes the exact command context and a usage hint so the caller (or an LLM) can self-correct.

---

## 2. ParseError Enum

All parse failures are represented by `ParseError`, a five-variant enum that carries enough context for the caller to understand *what* went wrong and *how* to fix it.

```reshape/crates/agent-browser/src/commands.rs#L11-31
pub enum ParseError {
    UnknownCommand { command: String },
    UnknownSubcommand {
        subcommand: String,
        valid_options: &'static [&'static str],
    },
    MissingArguments {
        context: String,
        usage: &'static str,
    },
    InvalidValue {
        message: String,
        usage: &'static str,
    },
    InvalidSessionName { name: String },
}
```

| Variant | When it fires | Example trigger |
|---|---|---|
| `UnknownCommand` | The top-level verb doesn't match any known command | `foo` |
| `UnknownSubcommand` | The verb is valid but the sub-verb isn't | `tab explode` → lists `new`, `list`, `close`, etc. |
| `MissingArguments` | Required positional args or flag values are absent | `click` (no selector) |
| `InvalidValue` | An argument is present but semantically wrong (wrong type, out of range, bad JSON) | `connect 99999` (port out of range) |
| `InvalidSessionName` | A session name fails path-traversal or character validation (delegates to `validation::session_name_error`) | `state rename ../../etc/passwd new-name` |

`ParseError::format()` renders each variant into a human-readable string with the context and usage hint, prefixed by `agent-browser` so it's clear which tool produced the error.

---

## 3. gen_id Function

Every command JSON object carries a unique `"id"` field used by the daemon to correlate requests and responses. `gen_id()` produces a short, deterministic-ish identifier:

```reshape/crates/agent-browser/src/commands.rs#L63-72
pub fn gen_id() -> String {
    format!(
        "r{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_micros()
            % 1000000
    )
}
```

**Format:** `"r{micros}"` where `micros` is the current microsecond count since the Unix epoch, modulo 1 000 000 (so it fits in 6 digits). The `"r"` prefix marks it as a reshape-generated ID. This is *not* globally unique — it's a correlation tag for a single session's request/response pairing, where ordering is sufficient.

---

## 4. Command Categories and Parse Functions

The top-level entry point is `parse_command(args, flags)`, which calls `parse_command_inner(args, flags)` and then injects the `AGENT_BROWSER_DEFAULT_TIMEOUT` into any `wait`-family command that doesn't already carry an explicit `timeout` field.

`parse_command_inner` is a large `match cmd { … }` block. Below is a catalog of every command category, its CLI verbs, the JSON `action` it produces, and key flags/arguments.

### 4.1 Navigation

| CLI verb(s) | JSON `action` | Notes |
|---|---|---|
| `open`, `goto`, `navigate` | `"navigate"` (or `"launch"` if `open` has no URL) | `open` without a URL launches the browser onto `about:blank` — useful for setting up routes/cookies/init scripts before the first real navigation. `goto` and `navigate` require a URL. URLs without a scheme get `https://` prepended. `chrome://` and `chrome-extension://` URLs are preserved. `--headers` injects custom request headers as JSON. |
| `back` | `"back"` | No arguments. |
| `forward` | `"forward"` | No arguments. |
| `reload` | `"reload"` | No arguments. |

**Auto-scheme behavior:** If the URL doesn't start with `http://`, `https://`, `about:`, `data:`, `file:`, `chrome-extension://`, or `chrome://`, it is automatically prefixed with `https://`.

**`--headers` flag:** Accepts a JSON object string like `'{"X-Custom": "value"}'` and injects it into the navigate command as `"headers"`.

**Provider integration:** When `flags.provider` is set, `"waitUntil": "none"` is injected (the provider handles its own wait logic). For the `"ios"` provider, `flags.device` is forwarded as `"iosDevice"`.

### 4.2 Interaction

| CLI verb | JSON `action` | Required args | Optional flags |
|---|---|---|---|
| `click` | `"click"` | `<selector>` | `--new-tab` |
| `dblclick` | `"dblclick"` | `<selector>` | — |
| `fill` | `"fill"` | `<selector> <text>` (text = rest of args joined) | — |
| `type` | `"type"` | `<selector> <text>` (text = rest of args joined) | — |
| `hover` | `"hover"` | `<selector>` | — |
| `focus` | `"focus"` | `<selector>` | — |
| `check` | `"check"` | `<selector>` | — |
| `uncheck` | `"uncheck"` | `<selector>` | — |
| `select` | `"select"` | `<selector> <value…>` | Single value → `"values": string`; multiple → `"values": array` |
| `drag` | `"drag"` | `<source> <target>` | — |
| `upload` | `"upload"` | `<selector> <files…>` | — |
| `download` | `"download"` | `<selector> <path>` | — |
| `tap` | `"tap"` | `<selector>` | iOS alias for click |
| `swipe` | `"swipe"` | `<direction> [distance]` | Valid directions: `up`, `down`, `left`, `right`; optional numeric distance |

**`click --new-tab`:** Produces `"newTab": true` — tells the daemon to open the link target in a new tab rather than navigating the current one.

**`fill` vs `type`:** Both accept multi-word text via `rest[1..].join(" ")`. `fill` sends `"value"`, `type` sends `"text"` — corresponding to Playwright's `fill()` vs `type()` semantics (clear-first vs append).

**`select` multiple values:** When more than one value is provided after the selector, the JSON `"values"` field becomes an array instead of a single string, supporting multi-select `<select>` elements.

### 4.3 Keyboard

| CLI verb | JSON `action` | Required args |
|---|---|---|
| `press`, `key` | `"press"` | `<key>` (e.g. `Enter`, `Tab`, `ArrowDown`) |
| `keydown` | `"keydown"` | `<key>` |
| `keyup` | `"keyup"` | `<key>` |
| `keyboard type` | `"keyboard"` / `"type"` | `<text>` (rest joined) |
| `keyboard inserttext` / `insertText` | `"keyboard"` / `"insertText"` | `<text>` (rest joined) |

The `keyboard` command is a sub-command dispatcher with two sub-verbs: `type` (character-by-character input) and `inserttext` (bulk insert, Playwright's `insertText()`).

### 4.4 Scroll

| CLI verb | JSON `action` | Defaults |
|---|---|---|
| `scroll` | `"scroll"` | `"direction": "down"`, `"amount": 300` |
| `scrollintoview` / `scrollinto` | `"scrollintoview"` | Requires `<selector>` |

`scroll` supports optional positional `[direction] [amount]` and a `--selector` / `-s` flag. Positional args are consumed in order (direction first, then numeric amount). Flags are intermixed freely — `--selector` is not counted as a positional arg.

### 4.5 Tab Management

| CLI verb | JSON `action` | Args |
|---|---|---|
| `tab new [url]` | `"tab_new"` | Optional URL, optional `--label <name>` |
| `tab new --label <name> [url]` | `"tab_new"` + `"label"` | Label and URL can appear in any order |
| `tab list` (or bare `tab`) | `"tab_list"` | No args — `tab` with no subcommand defaults to `list` |
| `tab switch <ref>` | `"tab_switch"` | `<ref>` can be a tab ID or label string |
| `tab close [ref]` | `"tab_close"` | Optional `tabId` to close a specific tab |

**Tab ref semantics:** When `tab` receives a non-keyword positional arg (not `new`, `list`, `close`), it's treated as a tab reference and routed to `tab_switch`. Unknown flags on `tab new` (e.g. `--foo`) produce `UnknownSubcommand`.

### 4.6 Observation

#### snapshot

| CLI verb | JSON `action` | Flags |
|---|---|---|
| `snapshot` | `"snapshot"` | `-i`/`--interactive`, `-c`/`--compact`, `-C`/`--cursor`, `-u`/`--urls`, `-d`/`--depth <n>`, `-s`/`--selector <sel>` |

The accessibility snapshot command. Flags are additive — any combination can be mixed. `--depth` requires a numeric argument; `--selector` requires a selector string.

#### screenshot

| CLI verb | JSON `action` | Flags |
|---|---|---|
| `screenshot [selector] [path] [--full/-f]` | `"screenshot"` | `--full`/`-f`, `--annotate` (from flags), `--screenshot-format`, `--screenshot-quality`, `--screenshot-dir` |

Smart arg disambiguation: a single positional arg is classified as a selector (starts with `.`, `#`, `@`) or a path (starts with `./`, `../`, contains `/`, or ends with `.png/.jpg/.jpeg/.webp`). Two positional args: first = selector, second = path.

#### wait

The `wait` command is a multi-mode dispatcher. It checks for flag-based modes first, then falls back to selector/timeout.

| Mode | Trigger | JSON `action` | Key fields |
|---|---|---|---|
| Selector wait | `wait <selector>` | `"wait"` | `"selector"` |
| Timeout-only wait | `wait <ms>` (numeric) | `"wait"` | `"timeout"` |
| URL wait | `wait --url` / `-u <pattern>` | `"waitforurl"` | `"url"` |
| Load state wait | `wait --load` / `-l <state>` | `"waitforloadstate"` | `"state"` |
| Function wait | `wait --fn` / `-f <expr>` | `"waitforfunction"` | `"expression"` |
| Text wait | `wait --text` / `-t <text>` | `"wait"` | `"text"`, optional `"timeout"` |
| Download wait | `wait --download` / `-d [path] [--timeout ms]` | `"waitfordownload"` | optional `"path"`, `"timeout"` |

**Default timeout injection:** After `parse_command_inner` returns, `parse_command` checks if the action starts with `"wait"` and the result lacks a `"timeout"` field. If `flags.default_timeout` is set, it injects `"timeout": default_timeout`. This centralization means every new wait variant automatically inherits the default.

#### inspect, get cdp-url

| CLI verb | JSON `action` |
|---|---|
| `inspect` | `"inspect"` |
| `get cdp-url` | `"cdp_url"` (via `parse_get`) |

### 4.7 Window & Frame

| CLI verb | JSON `action` | Args |
|---|---|---|
| `window new` | `"window_new"` | No args |
| `frame main` | `"mainframe"` | No args |
| `frame <selector>` | `"frame"` | Selector for the iframe |

### 4.8 Dialog Handling

| CLI verb | JSON `action` | Args |
|---|---|---|
| `dialog accept [text]` | `"dialog"` / `"accept"` | Optional `promptText` |
| `dialog dismiss [text]` | `"dialog"` / `"dismiss"` | Optional `promptText` |
| `dialog status` | `"dialog"` / `"status"` | No args |

### 4.9 Network

Handled by `parse_network(rest, id)`.

| CLI verb | JSON `action` | Key fields |
|---|---|---|
| `network route <url>` | `"route"` | `--abort`, `--body <json>`, `--resource-type <csv>` |
| `network unroute [url]` | `"unroute"` | Optional URL to remove a specific route |
| `network requests` | `"requests"` | `--clear`, `--filter`, `--type`, `--method`, `--status` |
| `network request <id>` | `"request_detail"` | Required requestId |
| `network har start` | `"har_start"` | No args |
| `network har stop [path]` | `"har_stop"` | Optional output path |

**Route interception:** `--abort` sets `"abort": true` (block the request). `--body` provides a mock response body. `--resource-type` filters which resource types the route applies to (comma-separated, e.g. `stylesheet,image`).

**Request filtering:** The `requests` subcommand supports multiple filter flags that can be combined — `--type`, `--method`, `--status` narrow the returned request list.

### 4.10 Storage

Handled by `parse_storage(rest, id)`.

| CLI verb | JSON `action` | Key fields |
|---|---|---|
| `storage local get [key]` | `"storage_get"` | `"type": "local"` |
| `storage local get <key>` | `"storage_get"` | `"type": "local"`, `"key"` |
| `storage local set <key> <value>` | `"storage_set"` | `"type": "local"` |
| `storage local clear` | `"storage_clear"` | `"type": "local"` |
| `storage session …` | Same actions, `"type": "session"` | Mirror of `local` |
| `storage <type> <key>` (bare) | `"storage_get"` | Implicit get when no explicit `get`/`set`/`clear` keyword |

**Implicit get:** When the second arg after `local`/`session` is not one of `get`, `set`, or `clear`, it's treated as a key for an implicit `get` operation.

### 4.11 Cookies

| CLI verb | JSON `action` | Key fields |
|---|---|---|
| `cookies get` (or bare `cookies`) | `"cookies_get"` | No args |
| `cookies set <name> <value>` | `"cookies_set"` | `"cookies": [{ name, value }]` |
| `cookies set --curl <file>` | `"cookies_set"` | `"cookies": [<parsed array>]` |
| `cookies clear` | `"cookies_clear"` | No args |

**Cookie set flags:**

| Flag | Field | Validation |
|---|---|---|
| `--url <url>` | `"url"` | Per-cookie URL scope |
| `--domain <domain>` | `"domain"` | Per-cookie domain scope (auto-adds `"path": "/"`) |
| `--path <path>` | `"path"` | Per-cookie path scope |
| `--httpOnly` | `"httpOnly": true` | Boolean flag |
| `--secure` | `"secure": true` | Boolean flag |
| `--sameSite <value>` | `"sameSite"` | Must be `Strict`, `Lax`, or `None` |
| `--expires <timestamp>` | `"expires"` | Must parse as `i64` Unix timestamp |

**`--curl` mode:** Reads a file and auto-detects its format (JSON array, cURL dump, or bare Cookie header). See section 6 for details on `parse_curl_cookies`.

### 4.12 Recording

| CLI verb | JSON `action` | Args |
|---|---|---|
| `record start <path> [url]` | `"recording_start"` | Required output path (`.webm`); optional URL (auto-prefixed with `https://` if no scheme) |
| `record stop` | `"recording_stop"` | No args |
| `record restart <path> [url]` | `"recording_restart"` | Required output path; optional URL |

### 4.13 Profiling & Tracing

#### Profiler (CDP)

| CLI verb | JSON `action` | Args |
|---|---|---|
| `profiler start [--categories <csv>]` | `"profiler_start"` | Optional comma-separated CDP category list |
| `profiler stop [path]` | `"profiler_stop"` | Optional output path for the profile dump |

#### Trace (CDP)

| CLI verb | JSON `action` | Args |
|---|---|---|
| `trace start` | `"trace_start"` | No args |
| `trace stop [path]` | `"trace_stop"` | Optional output path |

### 4.14 Diff

Handled by `parse_diff(rest, id)` — a three-branch dispatcher.

#### diff snapshot

| Flag | Short | Field | Notes |
|---|---|---|---|
| `--baseline <file>` | `-b` | `"baseline"` | Baseline snapshot file to compare against |
| `--selector <sel>` | `-s` | `"selector"` | Restrict diff to a subtree |
| `--compact` | `-c` | `"compact": true` | Use compact snapshot mode |
| `--depth <n>` | `-d` | `"maxDepth"` | Must be a non-negative `u32`; negative values produce `InvalidValue` |

JSON `action`: `"diff_snapshot"`. Unexpected positional args or unknown flags produce `InvalidValue`.

#### diff screenshot

| Flag | Short | Field | Notes |
|---|---|---|---|
| `--baseline <file>` | `-b` | `"baseline"` | **Required** — baseline image file |
| `--output <file>` | `-o` | `"output"` | Diff image output path |
| `--threshold <0–1>` | `-t` | `"threshold"` | Float in `[0.0, 1.0]`; out-of-range → `InvalidValue` |
| `--selector <sel>` | `-s` | `"selector"` | Restrict to element |
| `--full` | `-f` | `"fullPage": true` | Full-page screenshot |

JSON `action`: `"diff_screenshot"`. If `--baseline` is missing, returns `MissingArguments`.

#### diff url

Compares two URLs by navigating both and diffing their snapshots.

| Positional | Field | Notes |
|---|---|---|
| `<url1>` | `"url1"` | First URL (required) |
| `<url2>` | `"url2"` | Second URL (required) |

| Flag | Short | Field | Notes |
|---|---|---|---|
| `--screenshot` | — | `"screenshot": true` | Include screenshot diff alongside snapshot diff |
| `--full` | `-f` | `"fullPage": true` | Full-page mode |
| `--wait-until <strategy>` | — | `"waitUntil"` | Navigation wait strategy |
| `--selector <sel>` | `-s` | `"selector"` | Restrict diff scope |
| `--compact` | `-c` | `"compact": true` | Compact snapshot |
| `--depth <n>` | `-d` | `"maxDepth"` | Must be non-negative `u32` |

JSON `action`: `"diff_url"`. Missing either URL produces `MissingArguments`.

### 4.15 React DevTools

Handled by `parse_react(rest, id)`.

| CLI verb | JSON `action` | Notes |
|---|---|---|
| `react tree` | `"react_tree"` | Lists the React fiber tree |
| `react inspect <id>` | `"react_inspect"` | `id` must be numeric (i64); non-numeric → `InvalidValue` |
| `react renders start` | `"react_renders_start"` | Start tracking component renders |
| `react renders stop` | `"react_renders_stop"` | Stop tracking and dump render log |
| `react suspense` | `"react_suspense"` | Lists suspended boundaries; `--only-dynamic` flag filters to dynamic suspenses; `--json` flag adds `"json": true` |

All `react` subcommands accept `--json` to request structured JSON output instead of human-readable text.

### 4.16 Clipboard

| CLI verb | JSON `action` | Key fields |
|---|---|---|
| `clipboard read` (or bare `clipboard`) | `"clipboard"` / `"read"` | Default operation |
| `clipboard write <text>` | `"clipboard"` / `"write"` | Multi-word text via `rest[1..].join(" ")` |
| `clipboard copy` | `"clipboard"` / `"copy"` | Copy current selection to clipboard |
| `clipboard paste` | `"clipboard"` / `"paste"` | Paste clipboard content into focused element |

### 4.17 Find (Locators)

Handled by `parse_find(rest, id)` — Playwright-style semantic locators.

| CLI verb | JSON `action` | Key fields |
|---|---|---|
| `find role <role> [action] [--name <n>] [--exact]` | `"getbyrole"` | `"role"`, `"subaction"`, `"name"`, `"exact"` |
| `find text <text> [action] [--exact]` | `"getbytext"` | `"text"`, `"subaction"`, `"exact"` |
| `find label <label> [action] [text] [--exact]` | `"getbylabel"` | `"label"`, `"subaction"`, `"exact"` |
| `find placeholder <text> [action] [text] [--exact]` | `"getbyplaceholder"` | `"placeholder"`, `"subaction"`, `"exact"` |
| `find alt <text> [action] [--exact]` | `"getbyalttext"` | `"text"`, `"subaction"`, `"exact"` |
| `find title <text> [action] [--exact]` | `"getbytitle"` | `"text"`, `"subaction"`, `"exact"` |
| `find testid <id> [action] [text]` | `"getbytestid"` | `"testId"`, `"subaction"` |
| `find first <selector> [action] [text]` | `"nth"` | `"index": 0` |
| `find last <selector> [action] [text]` | `"nth"` | `"index": -1` |
| `find nth <index> <selector> [action] [text]` | `"nth"` | `"index"` (i32), required selector |

**Subactions:** The `[action]` positional arg defaults to `"click"` and maps to `"subaction"` in the JSON. For `fill`-type subactions, the `[text]` positional (everything after subaction) becomes `"value"`.

**`--exact` flag:** Requests exact string matching instead of substring matching.

**`--name` flag:** Only for `find role` — filters by accessible name.

**Fill values:** Any tokens after the subaction that aren't flags (`--exact`, `--name`) are collected into a fill value string (`rest.join(" ")`) and placed in `"value"`. This allows commands like `find label Email fill hello@example.com`.

### 4.18 Mouse

Handled by `parse_mouse(rest, id)`.

| CLI verb | JSON `action` | Args |
|---|---|---|
| `mouse move <x> <y>` | `"mousemove"` | Two integer coordinates |
| `mouse down [button]` | `"mousedown"` | Optional button name (default `"left"`) |
| `mouse up [button]` | `"mouseup"` | Optional button name (default `"left"`) |
| `mouse wheel [dy] [dx]` | `"wheel"` | `deltaY` default 100, `deltaX` default 0 |

### 4.19 Set (Browser Settings)

Handled by `parse_set(rest, id)`.

| CLI verb | JSON `action` | Args |
|---|---|---|
| `set viewport <w> <h> [scale]` | `"viewport"` | Width and height (integers); optional `deviceScaleFactor` (float) |
| `set device <name>` | `"device"` | Playwright device name |
| `set geo <lat> <lng>` | `"geolocation"` | Latitude and longitude (floats); `geo` and `geolocation` are aliases |
| `set offline [off/false]` | `"offline"` | Boolean; bare `set offline` = true, `set offline off` or `set offline false` = false |
| `set headers <json>` | `"headers"` | Must be valid JSON object |
| `set credentials <user> <pass>` | `"credentials"` | HTTP auth; `credentials` and `auth` are aliases |
| `set media` | `"emulatemedia"` | `dark`/`light` for color scheme; `reduced-motion` for motion preference |

**`set media` logic:** Color scheme defaults to `"no-preference"`. If `dark` or `light` appears in the args, it overrides. Reduced motion defaults to `"no-preference"`; `reduced-motion` arg switches to `"reduce"`.

### 4.20 Get (Property Queries)

Handled by `parse_get(rest, id)`.

| CLI verb | JSON `action` | Args |
|---|---|---|
| `get text <selector>` | `"gettext"` | Required selector |
| `get html <selector>` | `"innerhtml"` | Required selector |
| `get value <selector>` | `"inputvalue"` | Required selector |
| `get attr <selector> <attr>` | `"getattribute"` | Selector + attribute name |
| `get url` | `"url"` | No args — current page URL |
| `get title` | `"title"` | No args — current page title |
| `get count <selector>` | `"count"` | Required selector — element count |
| `get box <selector>` | `"boundingbox"` | Required selector — bounding box |
| `get styles <selector>` | `"styles"` | Required selector — computed styles |
| `get cdp-url` | `"cdp_url"` | No args — Chrome DevTools Protocol websocket URL |

### 4.21 Is (State Checks)

Handled by `parse_is(rest, id)`.

| CLI verb | JSON `action` | Args |
|---|---|---|
| `is visible <selector>` | `"isvisible"` | Required selector |
| `is enabled <selector>` | `"isenabled"` | Required selector |
| `is checked <selector>` | `"ischecked"` | Required selector |

### 4.22 Eval

| CLI verb | JSON `action` | Flags |
|---|---|---|
| `eval <script>` | `"evaluate"` | — |
| `eval -b` / `--base64 <encoded>` | `"evaluate"` | Decodes base64 → UTF-8 script |
| `eval --stdin` | `"evaluate"` | Reads script from stdin (line-by-line) |

**Base64 flow:** The encoded string is decoded with `base64::engine::general_purpose::STANDARD`. Invalid base64 or invalid UTF-8 after decoding produces `InvalidValue`.

**Stdin flow:** Reads all lines from `io::stdin()` and joins them with `\n`. Useful for piping multi-line scripts: `cat script.js | agent-browser eval --stdin`.

### 4.23 Batch

| CLI verb | JSON `action` | Flags |
|---|---|---|
| `batch [commands…] [--bail]` | `"batch"` | `--bail` sets `"bail": true` (stop on first failure) |

**Commands:** Remaining positional args become the `"commands"` array. If no commands are provided, `"commands"` is omitted from the JSON entirely — the daemon interprets this as "process the commands field if present".

### 4.24 Stream (Runtime Stream Control)

| CLI verb | JSON `action` | Args |
|---|---|---|
| `stream enable [--port <port>]` | `"stream_enable"` | Optional port (0–65535, validated) |
| `stream disable` | `"stream_disable"` | No args |
| `stream status` | `"stream_status"` | No args |

Port validation rejects values outside `u16::MAX` and non-integer strings.

### 4.25 Download & Wait-Download

| CLI verb | JSON `action` | Args |
|---|---|---|
| `download <selector> <path>` | `"download"` | Required selector and destination path |
| `wait-download [path] [--timeout ms]` | `"waitfordownload"` | Optional path and timeout (via `wait --download`) |

`wait-download` is actually parsed through the `wait` command's `--download` / `-d` flag path, but it's documented separately because it's a distinct daemon action.

### 4.26 Connect (CDP)

| CLI verb | JSON `action` | Args |
|---|---|---|
| `connect <port>` | `"launch"` + `"cdpPort"` | Port must be 1–65535 |
| `connect <ws-url>` | `"launch"` + `"cdpUrl"` | `ws://`, `wss://`, `http://`, `https://` URLs accepted |

Port 0, ports > 65535, and non-numeric non-URL strings produce `InvalidValue`.

### 4.27 Auth (Authentication Vault)

| CLI verb | JSON `action` | Args |
|---|---|---|
| `auth save <name> --url <url> --username <user> --password <pass>` | `"auth_save"` | `--password-stdin` flag reads password from stdin; `--username-selector`, `--password-selector`, `--submit-selector` for custom login form selectors |
| `auth login <name>` | `"auth_login"` | Saved credential name |
| `auth list` | `"auth_list"` | No args |
| `auth delete <name>` | `"auth_delete"` | Saved credential name |
| `auth show <name>` | `"auth_show"` | Saved credential name |

### 4.28 State (Session State Management)

| CLI verb | JSON `action` | Args |
|---|---|---|
| `state save <path>` | `"state_save"` | Required path |
| `state load <path>` | `"state_load"` | Required path |
| `state list` | `"state_list"` | No args |
| `state clear [--all] [name]` | `"state_clear"` | `--all` clears all sessions; positional `name` clears a specific named session (validated via `is_valid_session_name`) |
| `state show <filename>` | `"state_show"` | Required filename |
| `state clean --older-than <days>` | `"state_clean"` | Required days count |
| `state rename <old> <new>` | `"state_rename"` | Both names validated; `.json` suffix stripped automatically |

### 4.29 Console, Errors, Highlight

| CLI verb | JSON `action` | Flags |
|---|---|---|
| `console [--clear]` | `"console"` | Clear console log buffer |
| `errors [--clear]` | `"errors"` | Clear error buffer |
| `highlight <selector>` | `"highlight"` | Required selector |

### 4.30 Vitals

| CLI verb | JSON `action` | Flags |
|---|---|---|
| `vitals` / `web-vitals` | `"vitals"` | `--json` for structured output; optional positional URL |

### 4.31 Pushstate, RemoveInitScript, Confirm, Deny

| CLI verb | JSON `action` | Args |
|---|---|---|
| `pushstate <url>` | `"pushstate"` | SPA client-side navigation |
| `removeinitscript <id>` | `"removeinitscript"` | Remove a previously injected init script |
| `confirm <confirmation-id>` | `"confirm"` | Approve a pending action |
| `deny <confirmation-id>` | `"deny"` | Reject a pending action |

### 4.32 Device (iOS Simulator)

| CLI verb | JSON `action` | Args |
|---|---|---|
| `device list` (or bare `device`) | `"device_list"` | Lists available iOS simulators |

---

## 5. Special Parsing Helpers

### 5.1 parse_curl_cookies

```reshape/crates/agent-browser/src/commands.rs#L85-127
pub fn parse_curl_cookies(raw: &str) -> Result<Vec<Value>, String>
```

Auto-detects and parses three cookie input formats:

1. **JSON array** — `[{"name":"x","value":"y"}, …]`. Validates that each element has string `"name"` and `"value"` fields. Returns a normalized array of `{ name, value }` objects.

2. **cURL dump** — The output of Chrome DevTools → Network → Copy as cURL. Detected heuristically: starts with `curl` followed by whitespace or a quote. The `Cookie` header is extracted from `-H 'cookie: …'` (case-insensitive) or `-b '…'`/`--cookie '…'` flags. Bash (`\`) and cmd (`^`) line continuations are stripped before parsing.

3. **Bare cookie header** — `name=value; name2=value2`. Splits on `;`, finds `=` in each piece, extracts name and value.

**Security:** Error messages never echo the secret cookie value. If no cookies are found, returns `"no cookies found in input"`. Empty input returns `"cookies file is empty"`.

#### extract_cookie_header_from_curl

Strips line continuations (`\r\n`, `\n` with `\` or `^`), then tries three extraction strategies in order:

1. `-H` flag with a `cookie:` header prefix (case-insensitive)
2. `-b` flag (short cookie flag)
3. `--cookie` flag (long cookie flag)

#### match_quoted_arg

```reshape/crates/agent-browser/src/commands.rs#L151-204
fn match_quoted_arg(haystack: &str, flag: &str, expect_header: Option<&str>) -> Option<String>
```

A byte-level scanner that finds `flag <quote>value<quote>` patterns in a string. Handles both single and double quotes. When `expect_header` is set, the quoted value must start with that header name followed by `:` (case-insensitive), and the prefix is stripped before returning.

Word-boundary checks: the flag must be preceded by whitespace or start-of-string, and followed by whitespace before the quote.

#### parse_cookie_header

Splits a header string on `;`, then on `=` within each piece, to produce `[{ name, value }]` objects. Empty names are skipped.

### 5.2 shell_words_split

```reshape/crates/agent-browser/src/commands.rs#L2645-2674
pub fn shell_words_split(s: &str) -> Vec<String>
```

A shell-like word splitter that respects quoting and escaping:

- **Double quotes** (`"`): Group words; single quotes inside double quotes are literal.
- **Single quotes** (`'`): Group words; no escaping inside single quotes.
- **Backslash escaping** (`\`): Outside single quotes, the next character is taken literally.
- **Whitespace** outside quotes: Splits into separate args.

This is used by the CLI layer to split user input into argument vectors before passing them to `parse_command`.

---

## 6. Test Coverage Overview

The `mod tests` block spans approximately **2 430 lines** (L2677–L5107), covering every command category with behavior-named test functions. Key test groups:

### Test Infrastructure

- **`default_flags()`** (L2680–L2738) — Builds a `Flags` struct with sensible defaults for most tests, including `default_timeout: Some(30000)`.
- **`args()`** (L2740–L2742) — Helper to convert `&str` slices into `Vec<String>`.

### Test Categories by Count

| Category | Test range | Representative tests |
|---|---|---|
| Cookies (basic + curl) | L2747–3089 | `test_cookies_get`, `test_parse_curl_cookies_json_array`, `test_parse_curl_cookies_from_curl_bash`, `test_cookies_set_with_all_flags`, `test_parse_curl_cookies_never_echoes_values_in_errors` |
| React | L2863–2924 | `test_react_tree_command`, `test_react_inspect_requires_numeric_id`, `test_react_suspense_only_dynamic` |
| Network / Storage | L2953–3176 | `test_network_route_resource_type`, `test_network_requests_combined_filters`, `test_storage_local_set`, `test_storage_invalid_type` |
| Navigation | L3181–3272 | `test_open_without_url_launches`, `test_navigate_without_protocol`, `test_navigate_chrome_extension_url` |
| Headers | L3277–3300 | `test_set_headers_parses_json`, `test_set_headers_invalid_json_error` |
| Interaction | L3334–3376 | `test_click`, `test_fill`, `test_select_multiple_values`, `test_frame_main` |
| Tabs | L3381–3507 | `test_tab_new_with_label_and_url`, `test_tab_no_args_defaults_to_list`, `test_tab_non_keyword_treated_as_ref` |
| Network (har/requests) | L3512–3583 | `test_network_har_stop_with_path`, `test_network_request_detail_requires_id` |
| Screenshot | L3588–3654 | `test_screenshot_with_css_class`, `test_screenshot_full_page_shorthand` |
| Snapshot | L3659–3713 | `test_snapshot_interactive_cursor`, `test_snapshot_urls_short` |
| Wait | L3718–3780 | `test_wait_text_with_timeout`, `test_wait_load_missing_state` |
| Clipboard | L3785–3838 | `test_clipboard_write_multi_word`, `test_clipboard_unknown_subcommand` |
| Recording | L3845–3952 | `test_record_start_with_url`, `test_record_restart_missing_path` |
| Profiler | L3957–4019 | `test_profiler_start_with_categories`, `test_profiler_invalid_subcommand` |
| Eval | L4024–4069 | `test_eval_base64_with_special_chars`, `test_eval_base64_invalid` |
| Unknown / empty | L4072–4089 | `test_unknown_command`, `test_empty_args` |
| Get / Mouse | L4094–4129 | `test_get_unknown_subcommand`, `test_mouse_wheel` |
| Set | L4132–4184 | `test_set_viewport_with_fractional_scale`, `test_set_media_reduced_motion` |
| Find | L4187–4223 | `test_find_role_fill_does_not_include_flags_in_value` |
| Download | L4228–4297 | `test_download_missing_selector`, `test_wait_download_with_path_and_timeout` |
| Default timeout inheritance | L4308–4375 | `test_wait_selector_inherits_default_timeout`, `test_wait_no_default_timeout_omits_field` |
| Connect | L4380–4473 | `test_connect_invalid_port`, `test_connect_port_out_of_range` |
| Stream | L4478–4513 | `test_stream_enable_invalid_port` |
| Trace | L4518–4535 | `test_trace_stop_without_path` |
| Diff | L4540–4956 | `test_diff_screenshot_threshold_out_of_range`, `test_diff_url_with_short_snapshot_flags`, `test_diff_snapshot_depth_negative_value` |
| Scroll | L4974–5039 | `test_scroll_selector_before_positional`, `test_scroll_defaults` |
| Misc (inspect, cdp-url, batch) | L5044–5106 | `test_batch_with_args_and_bail`, `test_batch_no_args_no_commands_field` |

### Testing Patterns

- **Enum equality on JSON actions:** Tests construct the expected `json!({...})` and assert `assert_eq!` against `parse_command` output.
- **Error path coverage:** Every `ParseError` variant is tested — `UnknownCommand`, `UnknownSubcommand`, `MissingArguments`, `InvalidValue`, and `InvalidSessionName`.
- **Edge cases:** Out-of-range ports, negative depth values, missing required args, ambiguous positional args (screenshot path vs selector), flag ordering independence, multi-word text joining.
- **Security tests:** `test_parse_curl_cookies_never_echoes_values_in_errors` (L2824–2860) verifies that error messages do not leak secret cookie values.
- **Default timeout propagation:** A dedicated group (L4308–L4375) verifies that `AGENT_BROWSER_DEFAULT_TIMEOUT` is injected into all wait-family commands and omitted when not configured.

---

## 7. Architecture Notes

### Flag injection from `Flags`

The `parse_command` function receives a `Flags` struct (parsed by `flags::parse_flags`) and uses it to inject:

- `flags.headed` → `"headless": !headed` on `open` (launch mode)
- `flags.headers` → `"headers"` on `open`
- `flags.provider` → `"waitUntil": "none"` on `open`, `"iosDevice"` for iOS
- `flags.annotate` → `"annotate"` on `screenshot`; warns if used on non-screenshot commands
- `flags.screenshot_format` / `screenshot_quality` / `screenshot_dir` → corresponding fields on `screenshot`
- `flags.default_timeout` → `"timeout"` on all wait-family commands (post-processing in `parse_command`)

### Command → JSON mapping consistency

All JSON objects share the structure `{ "id": <string>, "action": <string>, … }`. The `"action"` field uses `snake_case` (e.g. `"cookies_set"`, `"diff_snapshot"`, `"react_inspect"`). The CLI verbs use `snake_case` or hyphenated forms (e.g. `cookies set`, `diff snapshot`, `react inspect`).

### Error message format

All `ParseError::format()` output includes the prefix `agent-browser` in usage strings, making it clear which tool produced the error and helping LLM agents self-correct.

---

## Changelog

| Date | Type | Description |
|---|---|---|
| 2025-07-17 | Added | Initial design documentation for `commands.rs` module |
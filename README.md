# Reshape

Reshape is a local, single-session agent foundation for AI-rendered web pages.

The project goal is to let a user describe an interface in natural language, have an agent use an LLM to create or update HTML, CSS, JavaScript, and related files in a watched workspace, and let a browser render the result through the local WebSocket-driven UI.

## Current Scope

This repository currently implements the agent foundation plus the first browser-rendering adapter:

- A Cargo workspace with separate core, CLI, browser adapter, and vendored browser automation crates.
- A CLI process that owns the local runtime.
- A single fixed session: `local:main`.
- A normalized message protocol built around `Envelope<T>`.
- A runtime loop that calls an LLM provider and executes tool calls.
- A trait-based tool system with workspace file tools.
- A safe local workspace abstraction with path escape protection.
- An OpenAI Chat Completions provider with streaming aggregation support.
- A system prompt contract for page-generation behavior.
- A vendored `agent-browser` 0.27.0 CLI crate with an additive Rust façade.
- An optional `reshape-browser` adapter that opens `index.html` through the browser façade after a completed turn.
- Design documents for CDP event normalization, browser plugin protocol, and observability layers.

Browser rendering is wired at the CLI/output-adapter layer. User text enters through the local JSON-RPC WebSocket endpoint and is normalized before it reaches the agent runtime.

## Architecture

Reshape is designed as a set of focused Rust modules. Modules communicate through traits and stable data contracts rather than concrete implementation dependencies.

```text
CLI Process
  -> JSON-RPC WebSocket
  -> Envelope<InputEvent>
  -> AgentRuntime
  -> LlmProvider
  -> ToolRegistry / Tool
  -> Workspace
  -> Envelope<OutputEvent>
```

Workspace crates:

- `crates/reshape-core`: protocol, runtime, session, LLM, tools, workspace, and observability contracts.
- `crates/reshape-cli`: CLI argument parsing, dependency assembly, JSON-RPC WebSocket server, and browser-render output adapter.
- `crates/reshape-browser`: `BrowserRenderer` trait and `AgentBrowserRenderer` implementation backed by the browser façade.
- `crates/agent-browser`: vendored `vercel-labs/agent-browser` 0.27.0 `cli/` source with additive `src/lib.rs` and `src/facade.rs`.

Core modules:

- `protocol`: envelope, input events, output events, schema version, and error codes.
- `session`: the single-session state model, turn state, and `SessionStore` trait.
- `runtime`: agent turn orchestration and tool-loop execution.
- `llm`: provider trait, chat messages, tool-call types, and OpenAI provider.
- `tools`: `Tool`, `ToolRegistry`, `ToolContext`, `ToolResult`, and built-in tools.
- `workspace`: safe local file access and workspace watcher traits.
- `observability`: telemetry trait boundary for logs, audit, metrics, and health.
The core runtime does not import `agent-browser` or `reshape-browser`; browser behavior is composed by `reshape-cli`.

## Agent Runtime Model

Each user input becomes an `Envelope<InputEvent>` and is processed under the fixed session key `local:main`.

The runtime:

1. Validates the session boundary.
2. Loads the current session state.
3. Builds LLM messages from the system prompt and user input.
4. Calls the configured `LlmProvider`.
5. Executes requested tools through `ToolRegistry`.
6. Continues until the model returns a final response or calls `complete_task`.
7. Persists session history and returns an `Envelope<OutputEvent>`.

The runtime receives normalized `InputEvent` values. Runtime callers decide how transport-specific input, currently JSON-RPC WebSocket messages, is converted into protocol events.

## Workspace Contract

The workspace is the shared data surface between the agent and the browser.

The built-in file tools support:

- `list_files`
- `read_file` with optional `offset` / `limit` pagination
- `write_file` with HTML normalization and validation metadata
- `delete_file`
- `complete_task`

The local workspace implementation restricts file access to the configured workspace root and rejects path traversal and symlink escape attempts. File paths exposed to tools are relative workspace paths so the model can safely reuse paths returned by one tool in another tool call.
File tool results are returned to the model as structured JSON with success status, paths, counts, truncation metadata, and recovery hints where applicable.

Optional network tools can be enabled in `~/.reshape/config.toml`:

- `web_search` uses Tavily to search the public web and returns structured result metadata.
- `web_fetch` downloads media and binary resources such as images, videos, audio, and PDFs into the workspace through the same path-safety boundary as file tools.

## Running Locally

Create a workspace directory and start the CLI:

```bash
mkdir -p page
cargo run -p reshape-cli -- --workspace ./page
```

To create the default workspace files without starting the server:

```bash
cargo run -p reshape-cli -- --workspace ./page workspace init
```

The CLI starts the local server and opens the default local URL in the browser
automatically. If the workspace does not have `index.html` yet, Reshape writes a
default static homepage so the browser has a clear starting surface. Send a
natural-language request through the local WebSocket UI; generated pages replace
the default by writing `workspace/index.html`.

Agent turns are prompted to produce HTML artifacts in the workspace rather than
raw HTML in chat. `index.html` acts as the wiki-style hub: generated topic pages
should live under `pages/`, shared assets under `assets/`, and each generated
HTML page should be linked from the hub with relative links.

When the production browser extension build exists at
`extensions/reshape/build/chrome-mv3-prod`, startup loads it automatically into
the managed browser session as an unpacked Chrome extension.

The default provider is OpenAI Chat Completions. Put the API key directly in `~/.reshape/config.toml` as `llm.openai.api_key` before running an agent turn.

Tavily search is disabled by default. To enable it:

```toml
[tools.web_search]
enabled = true
provider = "tavily"

[tools.web_search.tavily]
api_key = "tvly-..."
env_key = "TAVILY_API_KEY"
```

Media downloads are also disabled by default:

```toml
[tools.web_fetch]
enabled = true
download_dir = "assets/downloads"
max_bytes = 52428800
```

To render the completed `index.html` through the embedded browser adapter:

```bash
cargo run -p reshape-cli -- --workspace ./page --render-browser
```

To remove every file and directory inside a workspace while keeping the
workspace directory itself:

```bash
cargo run -p reshape-cli -- --workspace ./page workspace clean
```

Useful browser flags:

- `--browser-session reshape-main`
- `--browser-headed`

## Testing

Run the full verification suite:

```bash
cargo fmt --all --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

The test suite covers:

- CLI parsing and runtime assembly.
- Envelope and single-session protocol behavior.
- Runtime flow and session updates.
- LLM tool-loop execution and budget limits.
- Workspace file safety and watcher behavior.
- System prompt contract.
- Browser façade command construction.
- Browser renderer file URL conversion and CLI output-trigger behavior.

## Design Documents

Additional design notes live in `docs/`:

- `docs/agent-system-prompt.md`: system prompt contract for the page agent.
- `docs/design/cdp-event-ingress.md`: CDP event normalization.
- `docs/design/browser-plugin-websocket.md`: browser-plugin WebSocket protocol.
- `docs/design/browser-render-pipeline.md`: future browser rendering and refresh pipeline.
- `docs/design/agent-browser-source-integration.md`: vendored `agent-browser` source boundaries and sync process.
- `docs/design/runtime-observability.md`: telemetry, audit, metrics, and health design.

## Near-Term Extensions

The next practical milestones are:

- Route browser-extension user input through the JSON-RPC WebSocket protocol.
- Add a CDP adapter that turns user browser actions into `InputEvent::CdpUserEvent`.
- Extend the browser render adapter from completion-triggered `index.html` open to watcher-driven refresh and feedback.
- Replace the no-op telemetry sink with structured logs and local audit persistence.

## Design Principles

- Keep one local session until the product needs more.
- Keep the agent runtime independent from input and rendering adapters.
- Prefer trait contracts over concrete cross-module dependencies.
- Keep tools atomic and composable.
- Treat files in the workspace as the source of truth for rendered output.
- Require explicit completion through `complete_task`, not heuristic loop detection.

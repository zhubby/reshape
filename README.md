# Reshape

Reshape is a local, single-session agent foundation for AI-rendered web pages.

The project goal is to let a user describe an interface in natural language, have an agent use an LLM to create or update HTML, CSS, JavaScript, and related files in a watched workspace, and let a browser render the result through future CDP and browser-extension adapters.

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
- Design documents for future CDP event ingress, browser plugin, and observability layers.

Browser rendering is wired at the CLI/output-adapter layer. CDP event ingress and the floating browser-plugin chat UI remain adapter designs and are not coupled into the agent runtime.

## Architecture

Reshape is designed as a set of focused Rust modules. Modules communicate through traits and stable data contracts rather than concrete implementation dependencies.

```text
CLI Process
  -> IngressSource
  -> Envelope<InputEvent>
  -> AgentRuntime
  -> LlmProvider
  -> ToolRegistry / Tool
  -> Workspace
  -> Envelope<OutputEvent>
```

Workspace crates:

- `crates/reshape-core`: protocol, runtime, session, LLM, tools, workspace, ingress, and observability contracts.
- `crates/reshape-cli`: CLI argument parsing, dependency assembly, stdin loop, and optional browser-render output adapter.
- `crates/reshape-browser`: `BrowserRenderer` trait and `AgentBrowserRenderer` implementation backed by the browser façade.
- `crates/agent-browser`: vendored `vercel-labs/agent-browser` 0.27.0 `cli/` source with additive `src/lib.rs` and `src/facade.rs`.

Core modules:

- `protocol`: envelope, input events, output events, schema version, and error codes.
- `session`: the single-session state model, turn state, and `SessionStore` trait.
- `runtime`: agent turn orchestration and tool-loop execution.
- `llm`: provider trait, chat messages, tool-call types, and OpenAI provider.
- `tools`: `Tool`, `ToolRegistry`, `ToolContext`, `ToolResult`, and built-in tools.
- `workspace`: safe local file access and workspace watcher traits.
- `ingress`: input-source abstraction for CLI stdin and future adapters.
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

The runtime does not know whether input came from CLI stdin, CDP, a browser plugin, or tests. Those sources are adapters that normalize their input into `InputEvent`.

## Workspace Contract

The workspace is the shared data surface between the agent and the browser.

The built-in file tools support:

- `list_files`
- `read_file`
- `write_file`
- `delete_file`
- `complete_task`

The local workspace implementation restricts file access to the configured workspace root and rejects path traversal and symlink escape attempts. File paths exposed to tools are relative workspace paths so the model can safely reuse paths returned by one tool in another tool call.

## Running Locally

Create a workspace directory and start the CLI:

```bash
mkdir -p page
cargo run -p reshape-cli -- --workspace ./page
```

The CLI starts the local server and opens the default local URL in the browser
automatically. If the workspace does not have `index.html` yet, Reshape writes a
default static homepage so the browser has a clear starting surface. Then type a
natural-language request into stdin; generated pages replace the default by
writing `workspace/index.html`.

Agent turns are prompted to produce HTML artifacts in the workspace rather than
raw HTML in chat. `index.html` acts as the wiki-style hub: generated topic pages
should live under `pages/`, shared assets under `assets/`, and each generated
HTML page should be linked from the hub with relative links.

When the production browser extension build exists at
`extensions/reshape/build/chrome-mv3-prod`, startup loads it automatically into
the managed browser session as an unpacked Chrome extension.

The default provider is OpenAI Chat Completions. Put the API key directly in `~/.reshape/config.toml` as `llm.openai.api_key` before running an agent turn.

To render the completed `index.html` through the embedded browser adapter:

```bash
cargo run -p reshape-cli -- --workspace ./page --render-browser
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
- `docs/design/cdp-event-ingress.md`: future CDP event normalization.
- `docs/design/browser-plugin-websocket.md`: future browser-plugin WebSocket protocol.
- `docs/design/browser-render-pipeline.md`: future browser rendering and refresh pipeline.
- `docs/design/agent-browser-source-integration.md`: vendored `agent-browser` source boundaries and sync process.
- `docs/design/runtime-observability.md`: telemetry, audit, metrics, and health design.

## Near-Term Extensions

The next practical milestones are:

- Add a WebSocket ingress adapter for the browser extension.
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

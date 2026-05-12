# Reshape

Reshape is a local, single-session agent foundation for AI-rendered web pages.

The project goal is to let a user describe an interface in natural language, have an agent use an LLM to create or update HTML, CSS, JavaScript, and related files in a watched workspace, and let a browser render the result through future CDP and browser-extension adapters.

## Current Scope

This repository currently implements the agent foundation:

- A CLI process that owns the local runtime.
- A single fixed session: `local:main`.
- A normalized message protocol built around `Envelope<T>`.
- A runtime loop that calls an LLM provider and executes tool calls.
- A trait-based tool system with workspace file tools.
- A safe local workspace abstraction with path escape protection.
- A mock LLM provider for local development and tests.
- A system prompt contract for page-generation behavior.
- Design documents for future CDP, browser plugin, render, and observability layers.

Browser rendering, CDP control, and the floating browser-plugin chat UI are intentionally documented as adapter designs first. They are not coupled into the initial agent runtime.

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

Key modules:

- `protocol`: envelope, input events, output events, schema version, and error codes.
- `session`: the single-session state model, turn state, and `SessionStore` trait.
- `runtime`: agent turn orchestration and tool-loop execution.
- `llm`: provider trait, chat messages, tool-call types, and mock provider.
- `tools`: `Tool`, `ToolRegistry`, `ToolContext`, `ToolResult`, and built-in tools.
- `workspace`: safe local file access and workspace watcher traits.
- `ingress`: input-source abstraction for CLI stdin and future adapters.
- `observability`: telemetry trait boundary for logs, audit, metrics, and health.
- `cli`: argument parsing, dependency assembly, and process lifecycle.

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
cargo run -- --workspace ./page
```

Then type a natural-language request into stdin.

The current default provider is a mock provider intended for local development. It can exercise the file-tool loop and create `index.html` in the configured workspace.

## Testing

Run the full verification suite:

```bash
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
```

The test suite covers:

- CLI parsing and runtime assembly.
- Envelope and single-session protocol behavior.
- Runtime flow and session updates.
- LLM tool-loop execution and budget limits.
- Workspace file safety and watcher behavior.
- System prompt contract.

## Design Documents

Additional design notes live in `docs/`:

- `docs/agent-system-prompt.md`: system prompt contract for the page agent.
- `docs/design/cdp-event-ingress.md`: future CDP event normalization.
- `docs/design/browser-plugin-websocket.md`: future browser-plugin WebSocket protocol.
- `docs/design/browser-render-pipeline.md`: future browser rendering and refresh pipeline.
- `docs/design/runtime-observability.md`: telemetry, audit, metrics, and health design.

## Near-Term Extensions

The next practical milestones are:

- Add a real OpenAI-compatible or Anthropic-compatible `LlmProvider`.
- Add a WebSocket ingress adapter for the browser extension.
- Add a CDP adapter that turns user browser actions into `InputEvent::CdpUserEvent`.
- Add a render adapter that refreshes or updates the browser when workspace files change.
- Replace the no-op telemetry sink with structured logs and local audit persistence.

## Design Principles

- Keep one local session until the product needs more.
- Keep the agent runtime independent from input and rendering adapters.
- Prefer trait contracts over concrete cross-module dependencies.
- Keep tools atomic and composable.
- Treat files in the workspace as the source of truth for rendered output.
- Require explicit completion through `complete_task`, not heuristic loop detection.

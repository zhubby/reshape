# Reshape

Reshape is a local, single-session agent foundation for AI-rendered web pages.

The project goal is to let a user describe an interface in natural language, have an agent use an LLM to create or update HTML, CSS, JavaScript, and related files in a watched workspace, and let a browser render the result through CDP and browser-extension adapters.

## Architecture

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

- **reshape-core**: protocol, runtime, session, LLM, tools, workspace, ingress, and observability contracts.
- **reshape-cli**: CLI argument parsing, dependency assembly, stdin loop, and optional browser-render output adapter.
- **reshape-browser**: `BrowserRenderer` trait and `AgentBrowserRenderer` implementation backed by the browser façade.
- **agent-browser**: vendored `vercel-labs/agent-browser` 0.27.0 `cli/` source with additive `src/lib.rs` and `src/facade.rs`.

## Quick Start

```bash
mkdir -p page
cargo run -p reshape-cli -- --workspace ./page
```

Type a natural-language request into stdin. The default provider is a mock provider for local development.

To render through the embedded browser adapter:

```bash
cargo run -p reshape-cli -- --workspace ./page --render-browser
```
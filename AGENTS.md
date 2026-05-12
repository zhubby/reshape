# Repository Guidelines

## Project Structure & Module Organization

This repository is a single-crate Rust project (`reshape`). Modules are split by responsibility under `src/`:

- `src/cli`: CLI entrypoint, argument parsing, composition root — wires all concrete implementations into `RuntimeDeps`.
- `src/config`: `AppConfig` with builder-style mutators, workspace validation, runtime limits defaults.
- `src/runtime`: `AgentRuntime` — central orchestrator that drives the ReAct-style LLM → Tool → LLM loop.
- `src/llm`: `LlmProvider` trait, `ChatMessage`/`LlmResponse`/`ToolCall` types, `MockLlmProvider` for testing.
- `src/tools`: `Tool` trait, `ToolRegistry` trait, `InMemoryToolRegistry` with fluent builder, `FileTool` (list/read/write/delete), `CompleteTaskTool`.
- `src/workspace`: `Workspace` trait, `LocalWorkspace` (filesystem-backed, path-traversal defense), `WorkspaceWatcher` trait + `LocalWorkspaceWatcher` (notify crate).
- `src/session`: `Session` model, `TurnState` state machine, `SessionStore` trait, `InMemorySessionStore`.
- `src/protocol`: `Envelope<T>` wrapper, `EnvelopeHeader`, `InputEvent`, `OutputEvent`, `ErrorCode` — standardized message protocol.
- `src/bus`: `EventBus` trait, `InProcessBus` (mpsc channel-based) — defined but not currently wired into runtime flow.
- `src/ingress`: `IngressSource` trait, `CliStdinIngress` — async input abstraction over stdin readers.
- `src/observability`: `TelemetrySink` trait, `NoopTelemetry` — placeholder for future metrics/tracing.
- `src/prompt`: `system_prompt()` — loads agent system prompt via `include_str!` from `docs/agent-system-prompt.md`.
- `src/error`: `ReshapeError` enum (thiserror derive), `Result<T>` alias — all domain errors are explicit variants.
- `docs/`: Agent system prompt and design documentation.

Keep new code in the module that owns the domain concern; avoid leaking CLI-specific wiring logic into core runtime or protocol modules.

## Build, Test, and Development Commands

Use Cargo commands from repo root:

- `cargo check`: fast compile verification.
- `cargo build`: build the project.
- `cargo test`: run all integration tests.
- `cargo test --test agent_tool_loop`: run a specific integration test file.
- `cargo fmt`: apply Rust formatting.
- `cargo clippy -- -D warnings`: lint strictly.

To run the agent locally (requires a workspace directory):

- `cargo run -- --workspace /path/to/workspace`: interactive stdin loop with default mock LLM.
- `cargo run -- --workspace /path/to/workspace --mock-llm`: explicitly use mock LLM provider.
- `cargo run -- --workspace /path/to/workspace --model gpt-4o`: specify a real LLM model (requires API key).

## Rust Style and Idioms

- Target Rust 2024 edition for new code. Prefer edition-aware idioms.
- Use `async_trait` for all trait objects that need async methods (`LlmProvider`, `Tool`, `ToolRegistry`, `Workspace`, `WorkspaceWatcher`, `SessionStore`, `EventBus`, `TelemetrySink`, `IngressSource`).
- Use `Arc<dyn Trait>` for shared dependency references — `RuntimeDeps` holds 5 `Arc<dyn Trait>` fields.
- Derive `Default` when all fields have sensible defaults (e.g., `AppConfig`, `RuntimeLimits`, `InMemorySessionStore`, `MockLlmProvider`).
- Use concrete types (`struct`/`enum`) over `serde_json::Value` wherever shape is known. Only use `Value` at the tool argument boundary (where LLM output is inherently dynamic).
- **Match on types, never strings.** Only convert to strings at serialization/display boundaries. `InputSource`, `TurnState`, `ErrorCode`, `ChatRole`, and tool names should be matched as enum variants, not string comparisons.
- Prefer `From`/`Into`/`TryFrom`/`TryInto` over manual conversions. `#[from]` on `ReshapeError` variants handles automatic `From` impls for `serde_json::Error`, `std::io::Error`, and `notify::Error`.
- Use `anyhow::Result` for application-level errors if needed; `thiserror` for library/domain errors. The project uses a custom `Result<T>` alias over `ReshapeError` — use it everywhere, never bare `std::result::Result`.
- **Never `.unwrap()`/`.expect()` in production paths.** Use `?`, `ok_or_else`, `unwrap_or_default`, or `unwrap_or_else` for locks.
- Use `chrono` (workspace dep) for date/time — already imported in `EnvelopeHeader` timestamps.
- Prefer crates over subprocesses (`std::process::Command`). Use subprocesses only when no mature crate exists.
- Prefer guard clauses (early returns) over nested `if` blocks.
- Prefer `let-else` when destructuring must succeed and the failure path should return, `continue`, or `break`.
- Prefer `if let` chains, `matches!`, and pattern guards over nested single-arm `match` blocks when they make branching flatter and clearer.
- Prefer `Option`/`Result` combinators (`is_some_and`, `is_none_or`, `then_some`, `transpose`, `inspect`) when they keep ownership and control flow obvious; switch back to `match` once closure logic stops being trivial.
- Prefer iterators/combinators over manual loops. Use `Cow<'_, str>` when allocation is conditional.
- Prefer `tokio::fs` over `std::fs` for all workspace/file I/O — the runtime is fully async.
- Prefer `tokio::sync::Mutex` over `std::sync::Mutex` for shared state that must be held across `.await` points (e.g., `InMemorySessionStore`, `MockLlmProvider` responses queue).
- Run independent async work concurrently (`tokio::join!`, `futures::join_all`). Never use `block_on` inside async context.
- **Forbidden:** `Mutex<()>` / `Arc<Mutex<()>>` — mutex must guard actual state.
- Keep public API surfaces small. Use `#[must_use]` where return values matter.
- **Forbidden:** Never import `std::fs` or `std::io::BufReader` for synchronous file operations in async code paths. Always use `tokio::fs` and `tokio::io::BufReader`.

## Dependency Management

Dependencies are declared directly in the root `Cargo.toml` (single-crate project, no workspace). When adding new dependencies:

- Prefer minimal feature flags — only enable features actually used.
- Check if an existing dependency already provides the needed functionality before adding a new one.
- Keep dependency versions pinned to compatible ranges.

Current key dependencies:

| Dependency | Purpose |
|------------|---------|
| `tokio` | Async runtime (full features), async I/O, channels |
| `clap` | CLI argument parsing (derive) |
| `serde` / `serde_json` | Serialization for protocol and tool args |
| `thiserror` | Error enum derivation |
| `async-trait` | Async trait objects |
| `chrono` | Timestamps in envelope headers |
| `uuid` | Message/trace IDs in envelope headers |
| `notify` | Filesystem change watching |
| `tracing` / `tracing-subscriber` | Structured logging |
| `tempfile` | Test workspace isolation |

## Coding Style & Naming Conventions

Follow Rust 2024 defaults and `rustfmt` output (4-space indentation, trailing commas where formatter adds them). Prefer:

- `snake_case` for modules, functions, files, and tool names.
- `PascalCase` for types, traits, and enums.
- Small modules with explicit ownership boundaries.
- Each module has a `mod.rs` declaring public types, with implementation split into sibling files (e.g., `workspace/mod.rs` + `workspace/local.rs`).

Use `thiserror` for `ReshapeError` and avoid `unwrap()` in production paths.

When implementing tools, make tool metadata LLM-friendly:

- Write `description` so model planners can clearly infer **when** to call the tool.
- Design `parameters` schema with strong guidance (clear field semantics, constraints, defaults, and practical examples) to improve call accuracy and argument quality.
- Use `ToolResult::complete(summary)` only for `CompleteTaskTool` — it sets `should_continue = false` and terminates the agent loop.
- Use `ToolResult::success(content)` for all other tools — the loop continues.
- Use `ToolResult::error(content)` for tool execution failures — the loop continues (error is reported back to the LLM for recovery).

## Security & Workspace Boundaries

The workspace is the primary security boundary. All file operations must go through the `Workspace` trait — never bypass it with direct filesystem access.

### Path Traversal Defense (implemented in `LocalWorkspace`)

- `relative_path()` — rejects `..` and root path components; validates file extension is in whitelist (`html|htm|js|css|json|md|txt`).
- `resolve_existing()` — canonicalizes and verifies the resolved path stays under `self.root`.
- `resolve_for_write()` — checks for symlink escapes, verifies parent directory is under root, creates parent dirs if needed.
- `strip_root()` — removes root prefix for relative path output in tool results.

### Agent Loop Safety

- **Budget limits** (`max_tool_iterations`, `max_tool_calls`) prevent infinite loops. Default: 8 iterations, 32 calls.
- **Session key validation** — `AgentRuntime::process()` only accepts `DEFAULT_SESSION_KEY` (`"local:main"`).
- **No tool should ever access paths outside its workspace root.** All tool implementations must receive `ToolContext { workspace }` and delegate I/O through the `Workspace` trait.

### Credential Safety

Never commit API keys. LLM provider configuration should use environment variables (e.g., `OPENAI_API_KEY`). If sharing configs, redact credentials.

## Protocol & Envelope Guidelines

All messages flowing through the system use the `Envelope<T>` wrapper pattern:

- `EnvelopeHeader` provides `message_id`, `trace_id`, `session_key`, `timestamp`, `attempt`, and `schema_version` — standardized metadata for observability and future retry/replay.
- `Envelope::new(payload)` and `Envelope::for_session(session_key, payload)` auto-generate UUIDs and timestamps.
- `metadata: BTreeMap<String, Value>` allows arbitrary key-value extensions without schema changes.
- `DEFAULT_SCHEMA_VERSION = "1.0"` — increment when protocol shape changes in a non-backward-compatible way.
- All protocol types (`InputEvent`, `OutputEvent`, `ErrorCode`, etc.) are `Serialize + Deserialize` for future IPC/network use.

When adding new event variants or error codes:

- Add them to the existing enums — do not create parallel type hierarchies.
- Ensure they have clear, domain-specific names (not generic strings).
- Update `system_prompt` documentation if new events change agent behavior expectations.

## Agent Loop Architecture

The `AgentRuntime::process()` method drives a **ReAct-style loop**:

1. Validate session key, record telemetry event.
2. Load session, record input event in history.
3. Build message list: `[system_prompt, user_text_from_event]`.
4. Enter iteration loop (up to `max_tool_iterations`):
   - Call `LlmProvider::chat(messages, tool_defs, options)`.
   - If no tool calls → return `OutputEvent::FinalMessage`.
   - If tool calls present and at iteration limit → return `ToolBudgetExceeded` error.
   - For each tool call: increment counter, check budget, execute via `ToolRegistry`.
   - If `ToolResult.should_continue == false` → return `OutputEvent::Completed`.
   - Otherwise push tool result as `ChatMessage::tool(...)` and continue loop.
5. Save session before returning.

When modifying the agent loop:

- Preserve budget guard checks — every tool call must increment the counter and be checked against limits.
- Preserve the `should_continue` termination signal — `CompleteTaskTool` is the only tool that sets it to `false`.
- Preserve session persistence — `session.save()` must happen before returning, even on error paths.
- Do not add synchronous waits or blocking calls inside the loop — all operations must be async.
- Consider the `TurnState` machine (`Received → Validating → Executing → Publishing → Completed`) when adding lifecycle steps — invalid transitions must return `ReshapeError::InvalidStateTransition`.

## Testing Guidelines

### TDD Development Mode

This project follows **Test-Driven Development (TDD)** as its primary development workflow. Every feature, bug fix, or behavior change must be driven by tests written **before** the implementation code.

The TDD cycle is:

1. **Write a failing test** — define the desired behavior as a test that fails because the feature doesn't exist yet.
2. **Make the test pass** — write the minimal implementation code that satisfies the test.
3. **Refactor** — clean up the implementation while keeping all tests passing.

Rules:

- **No implementation without a test.** If a test doesn't exist for a behavior, write it first.
- **Tests define the contract.** The test is the specification — it describes what the code should do, not how.
- **Red → Green → Refactor, always.** Never skip the "red" step. A test must be seen failing before the implementation is written.
- **All tests must pass before completion.** Every modification should keep the full integration test suite passing.
- **Regression tests for bug fixes.** When fixing a bug, first write a test that reproduces the bug (it fails), then fix the bug (the test passes).

### Test Structure

Integration tests live in `reshape/tests/`. There are no `#[cfg(test)]` module tests within source files — all testing is integration-level.

Name tests by behavior, e.g., `natural_language_returns_final_message`, `unknown_tool_reports_error`.

### Test Infrastructure

- `MockLlmProvider` — queue-based mock for deterministic LLM responses. Use `MockLlmProvider::new([response1, response2])` for ordered responses, or `MockLlmProvider::default()` for auto-generated write_file → complete_task sequences.
- `tempfile::tempdir()` — create isolated workspace directories per test.
- `InMemorySessionStore::default()` — fresh session per test.
- `NoopTelemetry` — silent telemetry in all tests.
- `InMemoryToolRegistry::new().register(...)` — build tool registries with only the tools needed for the test.

### Test Categories

| Test file | Focus |
|-----------|-------|
| `agent_tool_loop.rs` | End-to-end tool loop: write file → complete, iteration budget exhaustion |
| `cli_smoke.rs` | CLI arg parsing, runtime building, stdin ingress, default mock behavior |
| `prompt_contract.rs` | System prompt content assertions (keyword checks) |
| `protocol_session.rs` | Envelope defaults, `TurnState` transitions, session persistence |
| `runtime_flow.rs` | Natural language → FinalMessage, consecutive turns, unknown tool error, session key rejection |
| `workspace_tools.rs` | Workspace read/write/list, path escape rejection, symlink escape rejection, file watcher |
| `scaffold_core.rs` | Core type and trait contract verification |

### Assertion Style

- Prefer enum equality assertions on output events: `assert_eq!(output.payload, OutputEvent::Completed { .. })`.
- Use string-content assertions for error messages only when enum matching is impractical.
- Verify workspace mutations via `tokio::fs::read_to_string` — always read back to confirm.
- Validate state machine transitions: assert valid paths succeed and invalid paths fail.

For tool and config changes, include enough test cases to cover core paths and edge cases (arg validation, provider routing, error handling when applicable). Every modification should keep all integration tests passing before completion.

### TDD Checklist for New Features

When adding a new feature, follow this checklist:

1. Identify the desired behavior and write a test name that describes it.
2. Write the test in the appropriate test file under `reshape/tests/`.
3. Run `cargo test` — confirm the new test **fails** (red).
4. Implement the minimal code to make the test pass (green).
5. Run `cargo test` — confirm **all** tests pass, not just the new one.
6. Refactor if needed, re-run `cargo test` to verify.
7. Run `cargo clippy -- -D warnings` to ensure no lint violations.
8. Run `cargo fmt` and verify formatting is clean.

## Configuration Guidelines

- `AppConfig` is the single source of truth for runtime configuration.
- **Never** persist config changes by mutating a stale in-memory snapshot and writing it back.
- When editing config, reload the latest state first, apply a targeted mutation, validate, then write.
- Builder methods (`with_model()`, `with_mock_llm()`) use move semantics — they consume and return `Self`.
- Default values are defined via `Default` impl and are intentional (e.g., `use_mock: true` by default for safe offline development).
- The `--config` CLI arg exists but is not yet wired to file parsing — when implementing config file loading, prefer TOML format and validate all fields.

## Documentation Guidelines

When adding or updating docs under `docs/`:

- The `docs/agent-system-prompt.md` is loaded at compile time via `include_str!` — any changes to it directly affect agent behavior.
- Keep the system prompt instructions precise and actionable: define what the agent is, what tools it has, workspace constraints, and when to call `complete_task`.
- Design documentation should live in `docs/design/`.
- Use clear heading hierarchy (`#`, `##`, `###`) and stable section names.

## Module Documentation & Changelog

Each module should maintain its own documentation:

**CHANGELOG.md** (at module or project root):

- Record main changes on every module modification.
- Format with date and type: `Added` / `Changed` / `Fixed` / `Removed`.

**README.md** (at project root):

- Describe module capabilities, implementation, and architecture.
- Keep in sync with code — update when descriptions become inaccurate.

## Git Commit Guidelines

Commit messages follow the [Conventional Commits](https://www.conventionalcommits.org/) specification. Each commit should be one logical change.

### Commit Message Format

```
<type>(<scope>): <subject>

<body>

<footer>
```

- **Subject line**: Required, imperative mood, lowercase, no trailing period, max 72 chars.
- **Body**: Optional, explains *what* and *why*, not *how*.
- **Footer**: Optional, use for `BREAKING CHANGE:`, `Closes #123`, etc.

### Commit Types

| Type       | Description                                 |
| ---------- | ------------------------------------------- |
| `feat`     | New feature                                 |
| `fix`      | Bug fix                                     |
| `docs`     | Documentation changes                       |
| `style`    | Code style (formatting, semicolons, etc.)   |
| `refactor` | Code refactoring without behavior change    |
| `perf`     | Performance improvements                    |
| `test`     | Test additions or corrections               |
| `chore`    | Maintenance tasks, dependencies, tooling    |
| `ci`       | CI/CD configuration changes                 |
| `build`    | Build system or external dependency changes |
| `revert`   | Reverting a previous commit                 |

### Examples

```
feat(runtime): add tool call budget tracking per session

fix(workspace): reject symlink escapes in resolve_for_write

docs(prompt): update system prompt with CDP event handling instructions

feat(tools): add delete_file tool to file tool enum

Closes #12

fix(config): validate workspace path before constructing AppConfig

BREAKING CHANGE: AppConfig::for_workspace now returns error instead of panicking
```

### Pull Request Guidelines

PRs should include:

- Purpose and impacted modules.
- Test evidence (commands run + results).
- Config/doc updates when behavior changes.
- Sample CLI output when user-facing behavior is modified.
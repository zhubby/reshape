# Agent Browser Source Integration

## Source

`crates/agent-browser` is vendored from `vercel-labs/agent-browser` version `0.27.0`, specifically the upstream Rust `cli/` crate.

The copied crate keeps the upstream CLI structure intact so it can still be compared against upstream behavior and tested independently. Local integration code is additive:

- `crates/agent-browser/src/lib.rs`
- `crates/agent-browser/src/facade.rs`
- `crates/agent-browser/tests/facade_protocol.rs`
- `crates/agent-browser/VENDORED.md`

Avoid editing copied upstream `.rs` files unless workspace compilation or a necessary API boundary makes it unavoidable.

Current necessary upstream-file adjustments are documented in `crates/agent-browser/VENDORED.md`: dashboard asset paths are made crate-local, and embedded daemon mode avoids redirecting the host process stderr.

## Boundaries

`reshape-core` does not depend on browser automation code. Browser rendering enters through:

```text
reshape-cli
  -> reshape-browser::BrowserRenderer
  -> agent-browser::BrowserSession
  -> agent-browser daemon/socket protocol
  -> Chrome/CDP
```

The workspace files remain the source of truth. The agent writes `index.html` and related assets; the browser adapter opens or reloads those files.

## Facade API

The local `agent-browser` façade intentionally exposes a small API:

- `BrowserOptions`
- `BrowserSession`
- `BrowserSession::open`
- `BrowserSession::reload`
- `BrowserSession::snapshot`
- `BrowserSession::screenshot`
- `BrowserSession::close`

It also exposes command builders such as `open_command` and `screenshot_command` for tests and future adapters that need to inspect protocol JSON without launching Chrome.

The first integration keeps the API focused on the render loop. It does not expose every upstream CLI command to `reshape` or to the LLM.

The façade starts the daemon in-process rather than through a user-installed external binary. This preserves source integration, but it means daemon configuration is currently process-wide through `AGENT_BROWSER_*` environment variables and should be treated as single-session behavior.

## Upstream Sync

To update the vendored source:

1. Replace the copied upstream portions under `crates/agent-browser` from the target upstream `cli/` version.
2. Preserve or reapply only the documented additive files listed above.
3. Record the new upstream version in `crates/agent-browser/VENDORED.md`.
4. Run:

```bash
cargo test -p agent-browser
cargo test -p reshape-browser
cargo test -p reshape-cli
```

5. Run the full workspace verification before merging:

```bash
cargo fmt --all --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Manual Chrome e2e tests remain opt-in because they depend on local browser availability.

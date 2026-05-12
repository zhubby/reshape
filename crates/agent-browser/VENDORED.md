# Vendored Source

This crate is copied from `vercel-labs/agent-browser` version `0.27.0`, upstream path `cli/`.

## Local Additions

The upstream CLI source is intended to remain intact. Local additions are:

- `src/lib.rs`: library target that declares the same internal modules as the binary target and exports the local façade.
- `src/facade.rs`: small Rust API for `BrowserSession`, `BrowserOptions`, command construction, and daemon/socket interaction.
- `tests/facade_protocol.rs`: local protocol tests for the façade.
- `VENDORED.md`: this maintenance note.

## Local Upstream File Adjustments

Two copied files have a workspace-path adjustment so the vendored crate stays self-contained:

- `build.rs`: dashboard placeholder path changed from `../packages/dashboard/out` to `packages/dashboard/out`.
- `src/native/stream/http.rs`: embedded dashboard asset path changed from `../packages/dashboard/out/` to `packages/dashboard/out/`.
- `src/native/daemon.rs`: embedded library mode skips Unix stderr file-descriptor redirection so an in-process daemon does not silence the host CLI.

Upstream `cli/` expects to live beside a repository-level `packages/` directory. Inside this workspace, keeping that relative path would generate `crates/packages/` during builds. The local path keeps generated dashboard placeholders under `crates/agent-browser/packages/`.

## Local Modification Policy

Do not edit copied upstream `.rs` files unless unavoidable. Prefer adding new files for integration behavior.

If an upstream file must change, document:

- the file path,
- the reason,
- the smallest possible diff,
- the expected conflict when updating upstream.

## Verification

Run the vendored crate tests after copying or updating upstream source:

```bash
cargo test -p agent-browser
```

Chrome-dependent e2e tests should stay opt-in unless the local environment is known to provide Chrome.

## Sync Process

When upgrading upstream:

1. Replace the upstream-copied portions of this crate from the new `agent-browser/cli` source.
2. Reapply the local additions listed above.
3. Update this file with the new upstream version.
4. Run `cargo test -p agent-browser`, then the dependent `reshape-browser` and `reshape-cli` tests.

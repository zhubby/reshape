# Changelog

## 2026-05-18

### Added

- Configuration guide for `reshape-core::config::AppConfig` and the matching TOML file fields.
- `map_workspace` tool for structured workspace file graphs, HTML/Markdown links, backlinks, missing links, entrypoints, and orphan pages.
- `reshape.reset_session` JSON-RPC method and browser extension reset button for clearing the local session history.
- `reshape workspace init` CLI command for creating the default workspace HTML and CSS without starting the server.
- `reshape workspace clean` CLI command for removing all workspace contents while preserving the workspace root.
- Optional `web_search` tool backed by Tavily, configured under `[tools.web_search]`.
- Optional `web_fetch` tool for downloading media and binary resources into the workspace.
- `Workspace::write_bytes` for binary writes through the existing workspace safety boundary.
- Browser extension popup theme preference with `system`, `light`, and `dark` modes.

### Changed

- Browser extension popup now uses Tailwind-powered shadcn-inspired local design tokens, button states, focus rings, message surfaces, and custom scrollbars.
- Browser extension build now includes Tailwind/PostCSS configuration and a direct `svgo` dev dependency.
- Browser extension popup now uses a minimalist flat layout with icon-only controls and an RPC status dot.
- Browser extension reset button now uses the `lucide-react` reset icon with refined button styling.
- `reshape` CLI help now documents global options, agent options, and workspace subcommands.
- File tools now return structured JSON to the model, including success status, paths, counts, truncation metadata, and recoverable error hints.
- `read_file` supports optional `offset` and `limit` arguments for line-numbered paginated reads.
- `write_file` reports HTML normalization metadata, including stylesheet href and validation status.

### Fixed

- Browser extension now keeps in-flight chat turns in the background snapshot, so reopening the popup during a long model response still shows the pending conversation.
- Browser extension send flow now returns immediately from the popup-to-background message and updates completion through background status broadcasts.
- Browser extension RPC history and reset calls now wait for matching JSON-RPC response IDs instead of consuming unrelated progress messages.

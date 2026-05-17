# Changelog

## 2026-05-18

### Added

- Optional `web_search` tool backed by Tavily, configured under `[tools.web_search]`.
- Optional `web_fetch` tool for downloading media and binary resources into the workspace.
- `Workspace::write_bytes` for binary writes through the existing workspace safety boundary.

### Changed

- File tools now return structured JSON to the model, including success status, paths, counts, truncation metadata, and recoverable error hints.
- `read_file` supports optional `offset` and `limit` arguments for line-numbered paginated reads.
- `write_file` reports HTML normalization metadata, including stylesheet href and validation status.

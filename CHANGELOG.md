# Changelog

## 2026-05-18

### Changed

- File tools now return structured JSON to the model, including success status, paths, counts, truncation metadata, and recoverable error hints.
- `read_file` supports optional `offset` and `limit` arguments for line-numbered paginated reads.
- `write_file` reports HTML normalization metadata, including stylesheet href and validation status.

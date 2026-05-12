# Runtime Observability Design

## Purpose

Observability should make a local single-session agent debuggable without forcing distributed infrastructure. The first implementation uses `TelemetrySink` as a trait so tracing, structured logs, files, SQLite, or OpenTelemetry can be swapped later.

## Minimum Events

- `inbound_received`
- `turn_validating`
- `turn_executing`
- `llm_request_started`
- `llm_request_finished`
- `tool_called`
- `tool_failed`
- `workspace_file_written`
- `final_response_published`
- `turn_completed`
- `turn_failed`

## Required Fields

Each structured event should include:

- `trace_id`
- `message_id`
- `session_key`
- `turn_index`
- `event_name`
- `timestamp`
- optional `tool_name`
- optional `provider`
- optional `model`
- optional `error_code`

## LLM Audit

LLM request and response bodies should be captured at the provider boundary. Persistence must be asynchronous so audit writes do not block the agent loop. Local file logging is acceptable for the first real provider; SQLite can be added when querying becomes necessary.

## Health Model

The CLI process can expose health through logs first, then a local HTTP endpoint later:

- `Live`: process is running.
- `Ready`: workspace, provider, and ingress are initialized.
- `Degraded`: provider or browser adapter is unavailable but CLI still accepts input.
- `Unavailable`: runtime cannot process turns.

## Metrics To Add Later

- agent turn duration
- LLM request duration
- tool call success/failure counts
- workspace write counts
- token usage
- plugin reconnect counts
- CDP disconnect counts

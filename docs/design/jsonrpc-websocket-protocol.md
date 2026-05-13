# JSON-RPC WebSocket Protocol

The reshape agent listens for inbound requests over a local WebSocket endpoint.
Frames are UTF-8 JSON text using JSON-RPC 2.0.

## Endpoint

Default JSON-RPC URL:

```text
ws://127.0.0.1:7331/v1/rpc
```

Browser plugins use the same URL. They may send a reshape-owned handshake frame
before regular JSON-RPC begins:

```text
ws://127.0.0.1:7331/v1/rpc
```

The host and port are configurable through CLI flags or the `[server]` section
in `~/.reshape/config.toml`.

## Scope

- The first protocol version supports a single runtime session: `local:main`.
- The server binds to `127.0.0.1` by default and does not require a token.
- One `reshape.input` request produces one JSON-RPC response.
- `/v1/rpc` requires a `reshape.rpc.handshake` frame as the first client text
  frame before later JSON-RPC frames.
- TypeScript clients should import protocol wire types from
  `extensions/reshape/src/generated/reshape.ts`. That file is generated from
  Rust types with `ts-rs` by running
  `cargo test -p reshape-cli --test ts_bindings`.
- Streaming output types are reserved in the wire format, but the current
  runtime returns a single `OutputEvent` after each turn.

## RPC Handshake

All RPC clients must send a small reshape-owned frame before JSON-RPC begins.
The handshake belongs to the RPC transport itself, not to any specific client
type.

Request:

```json
{
  "type": "reshape.rpc.handshake",
  "protocolVersion": "1.0",
  "client": {
    "name": "reshape-plasmo-extension",
    "version": "0.1.0"
  },
  "tab": {
    "id": 123,
    "url": "http://127.0.0.1:7331/",
    "title": "Reshape"
  }
}
```

Response:

```json
{
  "type": "reshape.rpc.handshake_ack",
  "protocolVersion": "1.0",
  "schemaVersion": "1.0",
  "sessionKey": "local:main"
}
```

If the first text frame is not a valid `reshape.rpc.handshake`, the server
returns a JSON-RPC invalid request or invalid params error and does not process
that frame as agent input.

## Methods

### `reshape.ping`

Health check for an open connection.

Request:

```json
{"jsonrpc":"2.0","id":"ping-1","method":"reshape.ping","params":{}}
```

Response:

```json
{"jsonrpc":"2.0","id":"ping-1","result":{"ok":true,"schemaVersion":"1.0"}}
```

### `reshape.input`

Runs one agent turn.

Request:

```json
{
  "jsonrpc": "2.0",
  "id": "turn-1",
  "method": "reshape.input",
  "params": {
    "sessionKey": "local:main",
    "schemaVersion": "1.0",
    "metadata": {
      "client": "example"
    },
    "input": {
      "type": "user_text",
      "text": "create a landing page"
    }
  }
}
```

Response:

```json
{
  "jsonrpc": "2.0",
  "id": "turn-1",
  "result": {
    "schemaVersion": "1.0",
    "messageId": "uuid",
    "traceId": "uuid",
    "sessionKey": "local:main",
    "output": {
      "type": "completed",
      "summary": "Mock page generated in index.html"
    },
    "metadata": {}
  }
}
```

## Input Types

### `user_text`

Maps to `InputEvent::UserText` with `InputSource::WebSocket`.

```json
{
  "type": "user_text",
  "text": "create a page"
}
```

## Output Types

Output payloads use stable snake_case type names instead of Rust enum serde
shapes.

- `final_message`: `{ "type": "final_message", "text": "..." }`
- `completed`: `{ "type": "completed", "summary": "..." }`
- `error`: `{ "type": "error", "code": "Failed", "message": "..." }`
- `tool_progress`: `{ "type": "tool_progress", "tool_name": "...", "message": "..." }`
- `workspace_file_changed`: `{ "type": "workspace_file_changed", "path": "index.html" }`
- `stream_chunk`: `{ "type": "stream_chunk", "text": "..." }`

## Errors

Errors follow JSON-RPC 2.0:

```json
{
  "jsonrpc": "2.0",
  "id": "turn-1",
  "error": {
    "code": -32602,
    "message": "invalid params: input.text is required",
    "data": {
      "errorCode": "ValidationFailed"
    }
  }
}
```

Error code mapping:

- `-32700`: invalid JSON.
- `-32600`: invalid JSON-RPC request.
- `-32601`: unknown method.
- `-32602`: invalid params.
- `-32000`: runtime, provider, tool, or I/O failure.

## Concurrency

The service serializes agent turns through a bounded worker queue. This preserves
the current single-session runtime invariant and avoids overlapping turn writes
to the in-memory session store.

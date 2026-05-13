# Browser Plugin WebSocket Design

## Purpose

The browser plugin provides a floating conversation window and forwards user messages to the local CLI process over WebSocket. It is an ingress adapter, not a separate session manager.

## Connection Contract

- The CLI process owns the WebSocket server.
- The plugin connects to `ws://127.0.0.1:<port>/v1/rpc` by default.
- The popup lets the user configure the RPC address. `127.0.0.1:7331` is
  normalized to `ws://127.0.0.1:7331/v1/rpc`.
- The first message must be the RPC handshake frame containing client version,
  tab ID, current URL, and optional page title.
- The CLI responds with accepted protocol version and the fixed session key `local:main`.

RPC handshake request:

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

RPC handshake acknowledgement:

```json
{
  "type": "reshape.rpc.handshake_ack",
  "protocolVersion": "1.0",
  "schemaVersion": "1.0",
  "sessionKey": "local:main"
}
```

## Message Envelope

After the handshake, plugin chat messages are sent as JSON-RPC
`reshape.input` frames on the same WebSocket connection. The current runtime
maps them to `InputEvent::UserText` with `InputSource::WebSocket` while the
plugin preserves tab context in request metadata:

- user text
- tab ID
- URL
- optional page title

Outbound runtime events can be streamed back to the plugin as:

- final response
- tool progress
- file changed
- error
- completed summary

## Heartbeats And Reconnects

- Plugin sends heartbeat every 15 seconds.
- CLI closes idle connections after 45 seconds without heartbeat.
- Reconnect does not create a new session; it resumes `local:main`.

## Security Boundary

- Bind to localhost by default.
- Require a random startup token for plugin connections.
- Never expose arbitrary filesystem operations over the plugin protocol; all writes still go through agent tools scoped to the configured workspace.

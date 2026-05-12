# Browser Plugin WebSocket Design

## Purpose

The browser plugin provides a floating conversation window and forwards user messages to the local CLI process over WebSocket. It is an ingress adapter, not a separate session manager.

## Connection Contract

- The CLI process owns the WebSocket server.
- The plugin connects to `ws://127.0.0.1:<port>/plugin`.
- The first message is a handshake containing plugin version, tab ID, current URL, and optional page title.
- The CLI responds with accepted protocol version and the fixed session key `local:main`.

## Message Envelope

Plugin messages map to `InputEvent::PluginMessage` and should preserve:

- user text
- tab ID
- URL
- viewport size
- optional selected text
- client timestamp

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

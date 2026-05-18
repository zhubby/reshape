# Reshape Browser Extension

Plasmo MV3 extension for chatting with the local reshape RPC server.

## Development

Install dependencies from this directory:

```bash
npm install
```

Run the extension in development mode:

```bash
npm run dev
```

Build a production extension:

```bash
npm run build
```

Load the generated build directory as an unpacked extension in Chrome.

## RPC Connection

The popup shows an RPC address input, Tailwind-powered shadcn-inspired icon
controls, custom scrollbars, an RPC status dot, and a persisted `system` /
`light` / `dark` theme toggle. The default RPC address value is:

```text
127.0.0.1:7331
```

The extension normalizes that value to:

```text
ws://127.0.0.1:7331/v1/rpc
```

On connect, the first WebSocket frame is the reshape RPC handshake:

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

After the server returns `reshape.rpc.handshake_ack`, chat messages are sent
as JSON-RPC `reshape.input` frames over the same connection.

## Checks

Regenerate Rust-owned protocol bindings before TypeScript checks:

```bash
cargo test -p reshape-cli --test ts_bindings
```

This writes `src/generated/reshape.ts`. Do not edit that file manually; update
the Rust protocol types and rerun the export test instead.

```bash
npm test
npm run typecheck
npm run build
```

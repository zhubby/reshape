# CDP Event Ingress Design

## Purpose

CDP event ingress converts browser-side user actions into `InputEvent::CdpUserEvent` without changing the agent runtime. The runtime continues to consume normalized events and remains unaware of Chrome DevTools Protocol details.

## Event Shape

Each CDP event should include:

- `event_type`: click, selection, input, navigation, or viewport-change.
- `selector_hint`: stable CSS selector or accessible-node hint when available.
- `text`: selected text, input value, or nearby visible text.
- `metadata`: current URL, viewport, pointer coordinates, DOM ancestry summary, and timestamp.

## Normalization Rules

- Send DOM summaries, not full page snapshots, by default.
- Preserve the user's visible intent: selected text and clicked element labels matter more than raw node IDs.
- Attach the current workspace-rendered file path when the browser is displaying a local file.
- Emit one `InputEvent` per meaningful user action.

## Runtime Boundary

CDP ingress implements the same ingress contract as CLI stdin. It must not call `AgentRuntime` internals directly. The only handoff is a normalized `Envelope<InputEvent>`.

## Failure Handling

- If CDP disconnects, surface a runtime event for observability and retry connection outside the agent loop.
- If an event payload is too large, truncate DOM context and include a `truncated: true` metadata flag.
- If the browser tab cannot be identified, emit no agent turn and log the dropped event.

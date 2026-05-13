import {
  PLUGIN_CLIENT_NAME,
  PLUGIN_CLIENT_VERSION,
  PLUGIN_PROTOCOL_VERSION,
  RESHAPE_SCHEMA_VERSION,
  RESHAPE_SESSION_KEY,
  type PluginHandshakeAck,
  type PluginHandshakeFrame,
  type ReshapeInputRequest,
  type RpcOutput,
  type RpcResponse,
  type TabContext
} from "./protocol"

export const DEFAULT_RPC_ADDRESS = "127.0.0.1:7331"

export type ChatResult = {
  id: string
  text: string
  output?: RpcOutput
}

export function normalizeRpcAddress(address: string): string {
  const trimmed = address.trim() || DEFAULT_RPC_ADDRESS
  const withScheme = /^[a-z]+:\/\//i.test(trimmed) ? trimmed : `ws://${trimmed}`
  const url = new URL(withScheme)
  url.protocol = url.protocol === "wss:" ? "wss:" : "ws:"
  url.pathname = "/v1/plugin"
  url.search = ""
  url.hash = ""
  return url.toString()
}

export function buildHandshakeFrame(tab: TabContext = {}): PluginHandshakeFrame {
  return {
    type: "reshape.plugin.handshake",
    protocolVersion: PLUGIN_PROTOCOL_VERSION,
    client: {
      name: PLUGIN_CLIENT_NAME,
      version: PLUGIN_CLIENT_VERSION
    },
    tab: {
      id: tab.id ?? null,
      url: tab.url ?? null,
      title: tab.title ?? null
    }
  }
}

export function isHandshakeAck(value: unknown): value is PluginHandshakeAck {
  if (!value || typeof value !== "object") {
    return false
  }
  const candidate = value as Partial<PluginHandshakeAck>
  return (
    candidate.type === "reshape.plugin.handshake_ack" &&
    candidate.protocolVersion === PLUGIN_PROTOCOL_VERSION &&
    candidate.schemaVersion === RESHAPE_SCHEMA_VERSION &&
    candidate.sessionKey === RESHAPE_SESSION_KEY
  )
}

export function buildInputRequest(
  id: string,
  text: string,
  tab: TabContext = {}
): ReshapeInputRequest {
  return {
    jsonrpc: "2.0",
    id,
    method: "reshape.input",
    params: {
      sessionKey: RESHAPE_SESSION_KEY,
      schemaVersion: RESHAPE_SCHEMA_VERSION,
      metadata: {
        client: PLUGIN_CLIENT_NAME,
        tabId: tab.id ?? null,
        url: tab.url ?? null,
        title: tab.title ?? null
      },
      input: {
        type: "user_text",
        text
      }
    }
  }
}

export function outputText(output: RpcOutput): string {
  switch (output.type) {
    case "final_message":
    case "stream_chunk":
      return output.text
    case "tool_progress":
      return `${output.tool_name}: ${output.message}`
    case "workspace_file_changed":
      return `Workspace file changed: ${output.path}`
    case "error":
      return output.message
    case "completed":
      return output.summary
  }
}

export async function connectAndHandshake(
  address: string,
  tab: TabContext
): Promise<WebSocket> {
  const socket = new WebSocket(normalizeRpcAddress(address))
  await waitForOpen(socket)
  socket.send(JSON.stringify(buildHandshakeFrame(tab)))
  const ack = await waitForJson(socket)
  if (!isHandshakeAck(ack)) {
    socket.close()
    throw new Error("reshape plugin handshake was not accepted")
  }
  return socket
}

export async function sendChatMessage(
  socket: WebSocket,
  text: string,
  tab: TabContext
): Promise<ChatResult> {
  const id = `turn-${Date.now()}`
  socket.send(JSON.stringify(buildInputRequest(id, text, tab)))
  const response = (await waitForJson(socket)) as RpcResponse

  if ("error" in response) {
    throw new Error(response.error.message)
  }

  return {
    id,
    text: outputText(response.result.output),
    output: response.result.output
  }
}

function waitForOpen(socket: WebSocket): Promise<void> {
  return new Promise((resolve, reject) => {
    socket.addEventListener("open", () => resolve(), { once: true })
    socket.addEventListener("error", () => reject(new Error("websocket error")), {
      once: true
    })
  })
}

function waitForJson(socket: WebSocket): Promise<unknown> {
  return new Promise((resolve, reject) => {
    socket.addEventListener(
      "message",
      (event) => {
        try {
          resolve(JSON.parse(event.data))
        } catch (error) {
          reject(error)
        }
      },
      { once: true }
    )
    socket.addEventListener("error", () => reject(new Error("websocket error")), {
      once: true
    })
  })
}

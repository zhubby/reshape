import {
  PLUGIN_CLIENT_NAME,
  PLUGIN_CLIENT_VERSION,
  RPC_PROTOCOL_VERSION,
  RESHAPE_SCHEMA_VERSION,
  RESHAPE_SESSION_KEY,
  type RpcHandshake,
  type RpcHandshakeAck,
  type RpcHistoryBody,
  type RpcHistoryMessage,
  type RpcProgressNotification,
  type ReshapeInputRequest,
  type RpcResultBody,
  type RpcOutput,
  type RpcResponse,
  type TabContext,
  type TurnProgressEvent
} from "./protocol"

export const DEFAULT_RPC_ADDRESS = "127.0.0.1:7331"
export type ConnectionStatus = "idle" | "connecting" | "connected" | "error"

export type ChatResult = {
  id: string
  text: string
  activity: TurnProgressEvent[]
  output?: RpcOutput
  metadata?: Record<string, unknown>
}

export type HistoryResult = {
  messages: RpcHistoryMessage[]
}

export type ProgressCallback = (event: TurnProgressEvent) => void

export function normalizeRpcAddress(address: string): string {
  const trimmed = address.trim() || DEFAULT_RPC_ADDRESS
  const withScheme = /^[a-z]+:\/\//i.test(trimmed) ? trimmed : `ws://${trimmed}`
  const url = new URL(withScheme)
  url.protocol = url.protocol === "wss:" ? "wss:" : "ws:"
  url.pathname = "/v1/rpc"
  url.search = ""
  url.hash = ""
  return url.toString()
}

export function buildHandshakeFrame(tab: TabContext = {}): RpcHandshake {
  return {
    type: "reshape.rpc.handshake",
    protocolVersion: RPC_PROTOCOL_VERSION,
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

export function isHandshakeAck(value: unknown): value is RpcHandshakeAck {
  if (!value || typeof value !== "object") {
    return false
  }
  const candidate = value as Partial<RpcHandshakeAck>
  return (
    candidate.type === "reshape.rpc.handshake_ack" &&
    candidate.protocolVersion === RPC_PROTOCOL_VERSION &&
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

export function buildHistoryRequest(id: string) {
  return {
    jsonrpc: "2.0",
    id,
    method: "reshape.history",
    params: {}
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

export function resultText(result: Pick<RpcResultBody, "output" | "metadata">): string {
  const text = outputText(result.output)
  const changedFiles = changedFilesFromMetadata(result.metadata)
  if (changedFiles.length === 0) {
    return text
  }
  return `${text}\nChanged files: ${changedFiles.join(", ")}`
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
    throw new Error("reshape rpc handshake was not accepted")
  }
  return socket
}

export async function sendChatMessage(
  socket: WebSocket,
  text: string,
  tab: TabContext,
  onProgress: ProgressCallback = () => {}
): Promise<ChatResult> {
  const id = `turn-${Date.now()}`
  socket.send(JSON.stringify(buildInputRequest(id, text, tab)))
  const response = (await waitForResponse(socket, id, onProgress)) as RpcResponse

  if ("error" in response) {
    throw new Error(response.error.message)
  }
  const result = response.result

  return {
    id,
    text: resultText(result),
    activity: progressEventsFromMetadata(result.metadata),
    output: result.output,
    metadata: result.metadata
  }
}

export async function fetchHistory(socket: WebSocket): Promise<HistoryResult> {
  const id = `history-${Date.now()}`
  socket.send(JSON.stringify(buildHistoryRequest(id)))
  const response = (await waitForJson(socket)) as
    | { jsonrpc: "2.0"; id: string; result: RpcHistoryBody }
    | { jsonrpc: "2.0"; id: string; error: { message: string } }

  if ("error" in response) {
    throw new Error(response.error.message)
  }

  return {
    messages: response.result.messages
  }
}

function changedFilesFromMetadata(metadata: Record<string, unknown>): string[] {
  const value = metadata.changedFiles
  if (!Array.isArray(value) || !value.every((item) => typeof item === "string")) {
    return []
  }
  return value
}

function progressEventsFromMetadata(metadata: Record<string, unknown>): TurnProgressEvent[] {
  const value = metadata.toolEvents
  if (!Array.isArray(value)) {
    return []
  }
  return value.filter(isTurnProgressEvent)
}

function isProgressNotification(value: unknown): value is RpcProgressNotification {
  if (!value || typeof value !== "object") {
    return false
  }
  const candidate = value as Partial<RpcProgressNotification>
  return candidate.jsonrpc === "2.0" && candidate.method === "reshape.progress"
}

function isTurnProgressEvent(value: unknown): value is TurnProgressEvent {
  if (!value || typeof value !== "object") {
    return false
  }
  const candidate = value as Partial<TurnProgressEvent>
  return (
    typeof candidate.turnId === "string" &&
    typeof candidate.sequence === "number" &&
    typeof candidate.kind === "string" &&
    typeof candidate.message === "string"
  )
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

function waitForResponse(
  socket: WebSocket,
  id: string,
  onProgress: ProgressCallback
): Promise<unknown> {
  return new Promise((resolve, reject) => {
    const cleanup = () => {
      socket.removeEventListener("message", handleMessage)
      socket.removeEventListener("error", handleError)
    }
    const handleError = () => {
      cleanup()
      reject(new Error("websocket error"))
    }
    const handleMessage = (event: MessageEvent) => {
      let value: unknown
      try {
        value = JSON.parse(event.data)
      } catch (error) {
        cleanup()
        reject(error)
        return
      }

      if (isProgressNotification(value)) {
        onProgress(value.params)
        return
      }

      if (isResponseForId(value, id)) {
        cleanup()
        resolve(value)
      }
    }

    socket.addEventListener("message", handleMessage)
    socket.addEventListener("error", handleError)
  })
}

function isResponseForId(value: unknown, id: string): value is RpcResponse {
  if (!value || typeof value !== "object") {
    return false
  }
  const candidate = value as Partial<RpcResponse>
  return candidate.jsonrpc === "2.0" && "id" in candidate && candidate.id === id
}

import {
  connectAndHandshake,
  fetchHistory,
  resetSession,
  sendChatMessage,
  type ChatResult,
  type ConnectionStatus,
  type HistoryResult,
  type ProgressCallback,
  RpcResponseError
} from "./rpc"
import type { TabContext } from "./protocol"

export type ConnectionSnapshot = {
  status: ConnectionStatus
  statusText: string
  address?: string
  history?: HistoryResult
  isWorking: boolean
}

type ConnectFn = (address: string, tab: TabContext) => Promise<WebSocket>
type HistoryFn = (socket: WebSocket) => Promise<HistoryResult>
type ResetSessionFn = (socket: WebSocket) => Promise<HistoryResult>
type SendFn = (
  socket: WebSocket,
  text: string,
  tab: TabContext,
  onProgress?: ProgressCallback
) => Promise<ChatResult>

type RpcConnectionDeps = {
  connect?: ConnectFn
  history?: HistoryFn
  resetSession?: ResetSessionFn
  send?: SendFn
}

export class RpcConnectionManager {
  private socket: WebSocket | null = null
  private status: ConnectionStatus = "idle"
  private statusText = "Handshake not started"
  private address?: string
  private history?: HistoryResult
  private isWorking = false
  private sendGeneration = 0
  private connectFn: ConnectFn
  private historyFn: HistoryFn
  private resetSessionFn: ResetSessionFn
  private sendFn: SendFn

  constructor(deps: RpcConnectionDeps = {}) {
    this.connectFn = deps.connect ?? connectAndHandshake
    this.historyFn = deps.history ?? fetchHistory
    this.resetSessionFn = deps.resetSession ?? resetSession
    this.sendFn = deps.send ?? sendChatMessage
  }

  snapshot(): ConnectionSnapshot {
    return {
      status: this.status,
      statusText: this.statusText,
      address: this.address,
      history: this.history,
      isWorking: this.isWorking
    }
  }

  async connect(address: string, tab: TabContext): Promise<ConnectionSnapshot> {
    if (this.status === "connected" && this.socket && this.address === address) {
      return this.snapshot()
    }

    this.status = "connecting"
    this.statusText = "Connecting to reshape RPC..."
    this.address = address
    this.isWorking = false
    this.sendGeneration += 1
    this.socket?.close()

    try {
      const socket = await this.connectFn(address, tab)
      this.socket = socket
      this.status = "connected"
      this.statusText = ""
      this.history = await this.historyFn(socket).catch(() => ({ messages: [] }))
      socket.addEventListener("close", () => {
        if (this.socket === socket) {
          this.socket = null
          this.status = "idle"
          this.statusText = "Connection closed"
          this.history = undefined
        }
      })
    } catch (error) {
      this.socket = null
      this.status = "error"
      this.statusText = error instanceof Error ? error.message : "Handshake failed"
      this.history = undefined
    }

    return this.snapshot()
  }

  async send(
    text: string,
    tab: TabContext,
    onProgress?: ProgressCallback,
    displayText = text
  ): Promise<ChatResult> {
    if (!this.socket || this.status !== "connected") {
      throw new Error("reshape RPC is not connected")
    }
    if (this.isWorking) {
      throw new RpcResponseError("Agent is already working")
    }

    const sendGeneration = ++this.sendGeneration
    this.isWorking = true
    this.statusText = "Agent is working..."
    this.history = {
      messages: [
        ...(this.history?.messages ?? []),
        { role: "user", text: displayText },
        { role: "reshape", text: "Working...", status: "working" }
      ]
    }

    try {
      const result = await this.sendFn(this.socket, text, tab, onProgress)
      if (this.sendGeneration === sendGeneration) {
        this.statusText = ""
        this.history = {
          messages: replaceLastWorkingMessage(this.history.messages, result.text)
        }
      }
      return result
    } catch (error) {
      if (error instanceof RpcResponseError) {
        if (this.sendGeneration === sendGeneration) {
          this.statusText = error.message
          this.history = {
            messages: replaceLastWorkingMessage(this.history.messages, error.message)
          }
        }
        throw error
      }
      if (this.sendGeneration === sendGeneration) {
        this.status = "error"
        this.statusText = error instanceof Error ? error.message : "Send failed"
        this.history = {
          messages: replaceLastWorkingMessage(this.history.messages, this.statusText)
        }
      }
      throw error
    } finally {
      if (this.sendGeneration === sendGeneration) {
        this.isWorking = false
      }
    }
  }

  async resetSession(): Promise<ConnectionSnapshot> {
    if (!this.socket || this.status !== "connected") {
      throw new Error("reshape RPC is not connected")
    }
    if (this.isWorking) {
      throw new Error("Agent is already working")
    }

    try {
      this.history = await this.resetSessionFn(this.socket)
      this.statusText = "Session reset"
      return this.snapshot()
    } catch (error) {
      this.status = "error"
      this.statusText = error instanceof Error ? error.message : "Reset failed"
      throw error
    }
  }

  disconnect(): ConnectionSnapshot {
    this.socket?.close()
    this.socket = null
    this.status = "idle"
    this.statusText = "Connection closed"
    this.history = undefined
    this.isWorking = false
    this.sendGeneration += 1
    return this.snapshot()
  }
}

function replaceLastWorkingMessage(
  messages: HistoryResult["messages"],
  text: string
): HistoryResult["messages"] {
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    const message = messages[index]
    if (message.role === "reshape" && message.status === "working") {
      return messages.map((candidate, candidateIndex) =>
        candidateIndex === index
          ? { role: "reshape", text, status: "complete" }
          : candidate
      )
    }
  }
  return [...messages, { role: "reshape", text, status: "complete" }]
}

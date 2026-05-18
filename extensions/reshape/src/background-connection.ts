import {
  connectAndHandshake,
  fetchHistory,
  resetSession,
  sendChatMessage,
  type ChatResult,
  type ConnectionStatus,
  type HistoryResult,
  type ProgressCallback
} from "./rpc"
import type { TabContext } from "./protocol"

export type ConnectionSnapshot = {
  status: ConnectionStatus
  statusText: string
  address?: string
  history?: HistoryResult
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
      history: this.history
    }
  }

  async connect(address: string, tab: TabContext): Promise<ConnectionSnapshot> {
    if (this.status === "connected" && this.socket && this.address === address) {
      return this.snapshot()
    }

    this.status = "connecting"
    this.statusText = "Connecting to reshape RPC..."
    this.address = address
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
    onProgress?: ProgressCallback
  ): Promise<ChatResult> {
    if (!this.socket || this.status !== "connected") {
      throw new Error("reshape RPC is not connected")
    }

    try {
      return await this.sendFn(this.socket, text, tab, onProgress)
    } catch (error) {
      this.status = "error"
      this.statusText = error instanceof Error ? error.message : "Send failed"
      throw error
    }
  }

  async resetSession(): Promise<ConnectionSnapshot> {
    if (!this.socket || this.status !== "connected") {
      throw new Error("reshape RPC is not connected")
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
    return this.snapshot()
  }
}

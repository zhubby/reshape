import {
  connectAndHandshake,
  sendChatMessage,
  type ChatResult,
  type ConnectionStatus
} from "./rpc"
import type { TabContext } from "./protocol"

export type ConnectionSnapshot = {
  status: ConnectionStatus
  statusText: string
  address?: string
}

type ConnectFn = (address: string, tab: TabContext) => Promise<WebSocket>
type SendFn = (
  socket: WebSocket,
  text: string,
  tab: TabContext
) => Promise<ChatResult>

type RpcConnectionDeps = {
  connect?: ConnectFn
  send?: SendFn
}

export class RpcConnectionManager {
  private socket: WebSocket | null = null
  private status: ConnectionStatus = "idle"
  private statusText = "尚未握手"
  private address?: string
  private connectFn: ConnectFn
  private sendFn: SendFn

  constructor(deps: RpcConnectionDeps = {}) {
    this.connectFn = deps.connect ?? connectAndHandshake
    this.sendFn = deps.send ?? sendChatMessage
  }

  snapshot(): ConnectionSnapshot {
    return {
      status: this.status,
      statusText: this.statusText,
      address: this.address
    }
  }

  async connect(address: string, tab: TabContext): Promise<ConnectionSnapshot> {
    if (this.status === "connected" && this.socket && this.address === address) {
      return this.snapshot()
    }

    this.status = "connecting"
    this.statusText = "正在连接 reshape RPC..."
    this.address = address
    this.socket?.close()

    try {
      const socket = await this.connectFn(address, tab)
      this.socket = socket
      this.status = "connected"
      this.statusText = "已连接到 reshape RPC"
      socket.addEventListener("close", () => {
        if (this.socket === socket) {
          this.socket = null
          this.status = "idle"
          this.statusText = "连接已关闭"
        }
      })
    } catch (error) {
      this.socket = null
      this.status = "error"
      this.statusText = error instanceof Error ? error.message : "握手失败"
    }

    return this.snapshot()
  }

  async send(text: string, tab: TabContext): Promise<ChatResult> {
    if (!this.socket || this.status !== "connected") {
      throw new Error("reshape RPC 尚未连接")
    }

    try {
      return await this.sendFn(this.socket, text, tab)
    } catch (error) {
      this.status = "error"
      this.statusText = error instanceof Error ? error.message : "发送失败"
      throw error
    }
  }

  disconnect(): ConnectionSnapshot {
    this.socket?.close()
    this.socket = null
    this.status = "idle"
    this.statusText = "连接已关闭"
    return this.snapshot()
  }
}

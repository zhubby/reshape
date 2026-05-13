import { useEffect, useMemo, useRef, useState } from "react"

import type { ChatResult, ConnectionStatus } from "./rpc"
import { loadRpcAddress, saveRpcAddress } from "./storage"
import type { TabContext } from "./protocol"

type ChatMessage = {
  role: "user" | "reshape" | "system"
  text: string
}
type BackgroundStatus = {
  status: ConnectionStatus
  statusText: string
  address?: string
}
type BackgroundResponse<T> =
  | ({ ok: true } & T)
  | {
      ok: false
      error: string
    }

function IndexPopup() {
  const [rpcAddress, setRpcAddress] = useState("127.0.0.1:7331")
  const [status, setStatus] = useState<ConnectionStatus>("idle")
  const [statusText, setStatusText] = useState("尚未握手")
  const [input, setInput] = useState("")
  const [messages, setMessages] = useState<ChatMessage[]>([
    {
      role: "system",
      text: "配置 reshape RPC 地址并完成握手后即可聊天。"
    }
  ])
  const didAutoConnect = useRef(false)

  const statusLabel = useMemo(() => {
    switch (status) {
      case "connected":
        return "握手成功"
      case "connecting":
        return "握手中"
      case "error":
        return "握手失败"
      case "idle":
        return "未连接"
    }
  }, [status])

  useEffect(() => {
    void initializeConnection()
  }, [])

  async function initializeConnection() {
    const address = await loadRpcAddress().catch(() => "127.0.0.1:7331")
    setRpcAddress(address)

    const status = await sendBackground<{ status: BackgroundStatus }>({
      type: "reshape.status"
    }).catch(() => null)
    if (status?.status) {
      applyStatus(status.status)
      if (status.status.status === "connected") {
        didAutoConnect.current = true
        return
      }
    }

    if (!didAutoConnect.current) {
      didAutoConnect.current = true
      setStatus("connecting")
      setStatusText("正在连接 reshape RPC...")
      try {
        const tab = await activeTabContext()
        const response = await sendBackground<{ status: BackgroundStatus }>({
          type: "reshape.connect",
          address,
          tab
        })
        applyStatus(response.status)
      } catch (error) {
        setStatus("error")
        setStatusText(error instanceof Error ? error.message : "握手失败")
      }
    }
  }

  function applyStatus(snapshot: BackgroundStatus) {
    setStatus(snapshot.status)
    setStatusText(snapshot.statusText)
    if (snapshot.address) {
      setRpcAddress(snapshot.address)
    }
  }

  async function connect() {
    setStatus("connecting")
    setStatusText("正在连接 reshape RPC...")

    try {
      await saveRpcAddress(rpcAddress)
      const tab = await activeTabContext()
      const response = await sendBackground<{ status: BackgroundStatus }>({
        type: "reshape.connect",
        address: rpcAddress,
        tab
      })
      applyStatus(response.status)
    } catch (error) {
      setStatus("error")
      setStatusText(error instanceof Error ? error.message : "握手失败")
    }
  }

  async function sendMessage() {
    const text = input.trim()
    if (!text || status !== "connected") {
      return
    }

    setInput("")
    setMessages((current) => [...current, { role: "user", text }])

    try {
      const tab = await activeTabContext()
      const response = await sendBackground<{
        result: ChatResult
        status: BackgroundStatus
      }>({
        type: "reshape.send",
        text,
        tab
      })
      const result = response.result
      applyStatus(response.status)
      setMessages((current) => [...current, { role: "reshape", text: result.text }])
    } catch (error) {
      setMessages((current) => [
        ...current,
        {
          role: "system",
          text: error instanceof Error ? error.message : "发送失败"
        }
      ])
      setStatus("error")
    }
  }

  return (
    <main style={styles.shell}>
      <header style={styles.header}>
        <div>
          <h1 style={styles.title}>Reshape</h1>
          <p style={styles.subtitle}>本地 RPC 聊天插件</p>
        </div>
        <span style={{ ...styles.badge, ...statusColor(status) }}>{statusLabel}</span>
      </header>

      <section style={styles.fieldGroup}>
        <label style={styles.label} htmlFor="rpc-address">
          RPC 地址
        </label>
        <div style={styles.addressRow}>
          <input
            id="rpc-address"
            value={rpcAddress}
            onChange={(event) => setRpcAddress(event.currentTarget.value)}
            placeholder="127.0.0.1:7331"
            style={styles.input}
          />
          <button type="button" onClick={connect} style={styles.secondaryButton}>
            握手
          </button>
        </div>
        <p style={styles.statusText}>{statusText}</p>
      </section>

      <section style={styles.messages} aria-label="聊天消息">
        {messages.map((message, index) => (
          <article key={`${message.role}-${index}`} style={messageStyle(message.role)}>
            {message.text}
          </article>
        ))}
      </section>

      <form
        style={styles.chatForm}
        onSubmit={(event) => {
          event.preventDefault()
          void sendMessage()
        }}>
        <input
          value={input}
          onChange={(event) => setInput(event.currentTarget.value)}
          disabled={status !== "connected"}
          placeholder={status === "connected" ? "告诉 reshape 要做什么..." : "请先握手"}
          style={styles.input}
        />
        <button
          type="submit"
          disabled={status !== "connected" || input.trim().length === 0}
          style={styles.primaryButton}>
          发送
        </button>
      </form>
    </main>
  )
}

function sendBackground<T>(message: Record<string, unknown>): Promise<T> {
  return new Promise((resolve, reject) => {
    chrome.runtime.sendMessage(message, (response: BackgroundResponse<T>) => {
      const error = chrome.runtime.lastError
      if (error) {
        reject(new Error(error.message))
        return
      }
      if (!isOkResponse(response)) {
        reject(new Error(response?.error || "background request failed"))
        return
      }
      resolve(response)
    })
  })
}

function isOkResponse<T>(response: BackgroundResponse<T> | undefined): response is { ok: true } & T {
  return response?.ok === true
}

async function activeTabContext(): Promise<TabContext> {
  if (!chrome?.tabs?.query) {
    return {}
  }
  const [tab] = await chrome.tabs.query({ active: true, currentWindow: true })
  return {
    id: tab?.id,
    url: tab?.url,
    title: tab?.title
  }
}

function statusColor(status: ConnectionStatus) {
  if (status === "connected") {
    return { background: "#e9f8ef", color: "#17633a" }
  }
  if (status === "error") {
    return { background: "#fff0ef", color: "#9d2a1f" }
  }
  if (status === "connecting") {
    return { background: "#eef4ff", color: "#204f9c" }
  }
  return { background: "#f1f2f4", color: "#4b5563" }
}

function messageStyle(role: ChatMessage["role"]) {
  return {
    ...styles.message,
    ...(role === "user" ? styles.userMessage : {}),
    ...(role === "reshape" ? styles.reshapeMessage : {}),
    ...(role === "system" ? styles.systemMessage : {})
  }
}

const styles = {
  shell: {
    width: 380,
    minHeight: 520,
    boxSizing: "border-box",
    padding: 18,
    display: "flex",
    flexDirection: "column",
    gap: 16,
    color: "#172033",
    background: "#fbfaf7",
    fontFamily:
      "-apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif"
  } as React.CSSProperties,
  header: {
    display: "flex",
    justifyContent: "space-between",
    alignItems: "flex-start",
    gap: 12
  } as React.CSSProperties,
  title: {
    margin: 0,
    fontSize: 24,
    lineHeight: 1.1
  } as React.CSSProperties,
  subtitle: {
    margin: "6px 0 0",
    color: "#667085",
    fontSize: 13
  } as React.CSSProperties,
  badge: {
    borderRadius: 999,
    padding: "6px 10px",
    fontSize: 12,
    fontWeight: 700,
    whiteSpace: "nowrap"
  } as React.CSSProperties,
  fieldGroup: {
    display: "flex",
    flexDirection: "column",
    gap: 8
  } as React.CSSProperties,
  label: {
    fontSize: 12,
    fontWeight: 700,
    color: "#475467"
  } as React.CSSProperties,
  addressRow: {
    display: "flex",
    gap: 8
  } as React.CSSProperties,
  input: {
    flex: 1,
    minWidth: 0,
    border: "1px solid #d0d5dd",
    borderRadius: 10,
    padding: "10px 12px",
    fontSize: 14,
    outlineColor: "#265ee8",
    background: "#ffffff"
  } as React.CSSProperties,
  secondaryButton: {
    border: "1px solid #1f2937",
    borderRadius: 10,
    padding: "0 14px",
    background: "#ffffff",
    color: "#111827",
    fontWeight: 700,
    cursor: "pointer"
  } as React.CSSProperties,
  primaryButton: {
    border: 0,
    borderRadius: 10,
    padding: "0 16px",
    background: "#172033",
    color: "#ffffff",
    fontWeight: 700,
    cursor: "pointer"
  } as React.CSSProperties,
  statusText: {
    margin: 0,
    minHeight: 18,
    color: "#667085",
    fontSize: 12
  } as React.CSSProperties,
  messages: {
    flex: 1,
    display: "flex",
    flexDirection: "column",
    gap: 10,
    overflowY: "auto",
    border: "1px solid #e4e7ec",
    borderRadius: 14,
    padding: 12,
    background: "#ffffff"
  } as React.CSSProperties,
  message: {
    maxWidth: "86%",
    borderRadius: 14,
    padding: "9px 11px",
    fontSize: 13,
    lineHeight: 1.45,
    whiteSpace: "pre-wrap"
  } as React.CSSProperties,
  userMessage: {
    alignSelf: "flex-end",
    background: "#172033",
    color: "#ffffff"
  } as React.CSSProperties,
  reshapeMessage: {
    alignSelf: "flex-start",
    background: "#f2f4f7",
    color: "#172033"
  } as React.CSSProperties,
  systemMessage: {
    alignSelf: "center",
    background: "#fff7df",
    color: "#6b4e16"
  } as React.CSSProperties,
  chatForm: {
    display: "flex",
    gap: 8
  } as React.CSSProperties
}

export default IndexPopup

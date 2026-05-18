import { useEffect, useMemo, useRef, useState } from "react"

import type { ChatResult, ConnectionStatus, HistoryResult } from "./rpc"
import { loadRpcAddress, saveRpcAddress } from "./storage"
import type { TabContext, TurnProgressEvent } from "./protocol"
import {
  appendActivityToWorkingMessage,
  completeWorkingMessage,
  connectionActionLabel,
  effectiveConnectionStatus,
  isConnectedToEditedAddress,
  messagesFromHistory,
  type PopupMessage
} from "./popup-state"

type ChatMessage = PopupMessage
type BackgroundMessage = { type: "reshape.progress"; event: TurnProgressEvent }
type BackgroundStatus = {
  status: ConnectionStatus
  statusText: string
  address?: string
  history?: HistoryResult
}
type BackgroundResponse<T> =
  | ({ ok: true } & T)
  | {
      ok: false
      error: string
    }

const INITIAL_MESSAGES: ChatMessage[] = [
  {
    role: "system",
    text: "Set the reshape RPC address and complete the handshake to start chatting."
  }
]

function IndexPopup() {
  const [rpcAddress, setRpcAddress] = useState("127.0.0.1:7331")
  const [status, setStatus] = useState<ConnectionStatus>("idle")
  const [statusText, setStatusText] = useState("Handshake not started")
  const [connectedAddress, setConnectedAddress] = useState<string | undefined>()
  const [input, setInput] = useState("")
  const [messages, setMessages] = useState<ChatMessage[]>(INITIAL_MESSAGES)
  const didAutoConnect = useRef(false)
  const effectiveStatus = effectiveConnectionStatus(status, rpcAddress, connectedAddress)
  const canChat = isConnectedToEditedAddress(status, rpcAddress, connectedAddress)
  const actionLabel = connectionActionLabel({
    status,
    rpcAddress,
    connectedAddress
  })

  const statusLabel = useMemo(() => {
    switch (effectiveStatus) {
      case "connected":
        return "Connected"
      case "connecting":
        return "Connecting"
      case "error":
        return "Handshake failed"
      case "idle":
        return "Not connected"
    }
  }, [effectiveStatus])

  useEffect(() => {
    void initializeConnection()
  }, [])

  useEffect(() => {
    const listener = (message: BackgroundMessage) => {
      if (message?.type !== "reshape.progress") {
        return
      }
      setMessages((current) => appendActivityToWorkingMessage(current, message.event))
      setStatusText(message.event.message)
    }
    chrome.runtime.onMessage.addListener(listener)
    return () => chrome.runtime.onMessage.removeListener(listener)
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
      setStatusText("Connecting to reshape RPC...")
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
        setStatusText(error instanceof Error ? error.message : "Handshake failed")
      }
    }
  }

  function applyStatus(snapshot: BackgroundStatus) {
    setStatus(snapshot.status)
    setStatusText(snapshot.statusText)
    setMessages((current) => messagesFromHistory(current, snapshot.history))
    if (snapshot.address) {
      setRpcAddress(snapshot.address)
      setConnectedAddress(snapshot.status === "connected" ? snapshot.address : undefined)
      return
    }
    if (snapshot.status !== "connected") {
      setConnectedAddress(undefined)
    }
  }

  async function connect() {
    setStatus("connecting")
    setStatusText("Connecting to reshape RPC...")

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
      setStatusText(error instanceof Error ? error.message : "Handshake failed")
    }
  }

  async function disconnect() {
    try {
      const response = await sendBackground<{ status: BackgroundStatus }>({
        type: "reshape.disconnect"
      })
      applyStatus(response.status)
    } catch (error) {
      setStatus("error")
      setStatusText(error instanceof Error ? error.message : "Disconnect failed")
    }
  }

  async function resetSession() {
    if (!canChat) {
      return
    }

    try {
      const response = await sendBackground<{ status: BackgroundStatus }>({
        type: "reshape.resetSession"
      })
      applyStatus(response.status)
      setMessages(INITIAL_MESSAGES)
    } catch (error) {
      setStatus("error")
      setStatusText(error instanceof Error ? error.message : "Reset failed")
    }
  }

  function handleConnectionAction() {
    if (canChat) {
      void disconnect()
      return
    }
    void connect()
  }

  function handleAddressChange(value: string) {
    setRpcAddress(value)
    if (status === "connected" && value.trim() !== connectedAddress?.trim()) {
      setStatusText("RPC address changed. Run handshake to connect.")
    }
  }

  async function sendMessage() {
    const text = input.trim()
    if (!text || !canChat) {
      return
    }

    setInput("")
    setStatusText("Agent is working...")
    setMessages((current) => [
      ...current,
      { role: "user", text },
      { role: "reshape", text: "Working...", status: "working", activity: [] }
    ])

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
      setStatus(response.status.status)
      setStatusText(response.status.statusText)
      if (response.status.address) {
        setConnectedAddress(
          response.status.status === "connected" ? response.status.address : undefined
        )
      }
      setMessages((current) => completeWorkingMessage(current, result))
    } catch (error) {
      setMessages((current) => [
        ...current,
        {
          role: "system",
          text: error instanceof Error ? error.message : "Send failed"
        }
      ])
      setStatus("error")
    }
  }

  return (
    <main style={styles.shell}>
      <header style={styles.header}>
        <div>
          <div style={styles.titleRow}>
            <h1 style={styles.title}>Reshape</h1>
            <button
              type="button"
              aria-label="Reset session"
              title="Reset session"
              disabled={!canChat}
              onClick={() => void resetSession()}
              style={{
                ...styles.iconButton,
                ...(!canChat ? styles.iconButtonDisabled : {})
              }}>
              <ResetIcon />
            </button>
          </div>
          <p style={styles.subtitle}>Local RPC chat extension</p>
        </div>
        <span style={{ ...styles.badge, ...statusColor(effectiveStatus) }}>{statusLabel}</span>
      </header>

      <section style={styles.fieldGroup}>
        <label style={styles.label} htmlFor="rpc-address">
          RPC address
        </label>
        <div style={styles.addressRow}>
          <input
            id="rpc-address"
            value={rpcAddress}
            onChange={(event) => handleAddressChange(event.currentTarget.value)}
            placeholder="127.0.0.1:7331"
            style={styles.input}
          />
          <button type="button" onClick={handleConnectionAction} style={styles.secondaryButton}>
            {actionLabel}
          </button>
        </div>
        {statusText ? <p style={styles.statusText}>{statusText}</p> : null}
      </section>

      <section style={styles.messages} aria-label="Chat messages">
        {messages.map((message, index) => (
          <article key={`${message.role}-${index}`} style={messageStyle(message.role)}>
            <div>{message.text}</div>
            {message.activity?.length ? <Activity events={message.activity} /> : null}
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
          disabled={!canChat}
          placeholder={canChat ? "Tell reshape what to do..." : "Handshake first"}
          style={styles.input}
        />
        <button
          type="submit"
          disabled={!canChat || input.trim().length === 0}
          style={styles.primaryButton}>
          Send
        </button>
      </form>
    </main>
  )
}

function ResetIcon() {
  return (
    <svg
      aria-hidden="true"
      viewBox="0 0 24 24"
      width="16"
      height="16"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round">
      <path d="M3 12a9 9 0 1 0 3-6.7" />
      <path d="M3 4v6h6" />
    </svg>
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

function Activity({ events }: { events: TurnProgressEvent[] }) {
  return (
    <details style={styles.activity}>
      <summary style={styles.activitySummary}>Activity</summary>
      <ol style={styles.activityList}>
        {events.map((event) => (
          <li key={`${event.turnId}-${event.sequence}`} style={styles.activityItem}>
            <span style={styles.activityKind}>{activityLabel(event)}</span>
            {event.toolName ? <span style={styles.activityTool}>{event.toolName}</span> : null}
            {event.argumentsPreview ? (
              <code style={styles.activityCode}>{event.argumentsPreview}</code>
            ) : null}
            {event.resultPreview ? (
              <code style={styles.activityCode}>{event.resultPreview}</code>
            ) : null}
          </li>
        ))}
      </ol>
    </details>
  )
}

function activityLabel(event: TurnProgressEvent): string {
  switch (event.kind) {
    case "turn_started":
      return "Started"
    case "assistant_message":
      return "Message"
    case "tool_started":
      return "Running"
    case "tool_finished":
      return "Finished"
    case "tool_failed":
      return "Failed"
    case "turn_completed":
      return "Completed"
    case "turn_failed":
      return "Failed"
  }
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
  titleRow: {
    display: "flex",
    alignItems: "center",
    gap: 8
  } as React.CSSProperties,
  title: {
    margin: 0,
    fontSize: 24,
    lineHeight: 1.1
  } as React.CSSProperties,
  iconButton: {
    width: 28,
    height: 28,
    border: "1px solid #d0d5dd",
    borderRadius: 8,
    display: "inline-flex",
    alignItems: "center",
    justifyContent: "center",
    background: "#ffffff",
    color: "#344054",
    cursor: "pointer",
    padding: 0
  } as React.CSSProperties,
  iconButtonDisabled: {
    color: "#98a2b3",
    cursor: "not-allowed",
    opacity: 0.65
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
    minWidth: 86,
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
  activity: {
    marginTop: 8,
    whiteSpace: "normal"
  } as React.CSSProperties,
  activitySummary: {
    cursor: "pointer",
    color: "#475467",
    fontSize: 12,
    fontWeight: 700
  } as React.CSSProperties,
  activityList: {
    margin: "8px 0 0",
    paddingLeft: 18,
    display: "flex",
    flexDirection: "column",
    gap: 6
  } as React.CSSProperties,
  activityItem: {
    display: "flex",
    flexDirection: "column",
    gap: 4
  } as React.CSSProperties,
  activityKind: {
    color: "#344054",
    fontSize: 12,
    fontWeight: 700
  } as React.CSSProperties,
  activityTool: {
    color: "#475467",
    fontSize: 12
  } as React.CSSProperties,
  activityCode: {
    maxWidth: 260,
    overflow: "hidden",
    textOverflow: "ellipsis",
    borderRadius: 6,
    padding: "5px 6px",
    background: "#ffffff",
    color: "#344054",
    fontSize: 11,
    fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace"
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

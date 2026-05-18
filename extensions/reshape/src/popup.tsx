import { useEffect, useMemo, useRef, useState } from "react"
import { Monitor, Moon, Plug, RotateCcw, Send, Sun, Unplug, X } from "lucide-react"

import type { ChatResult, ConnectionStatus, HistoryResult } from "./rpc"
import { loadRpcAddress, loadThemeMode, saveRpcAddress, saveThemeMode } from "./storage"
import type { TabContext, TurnProgressEvent } from "./protocol"
import {
  completeWorkingMessage,
  composePrompt,
  connectionActionLabel,
  contextLabel,
  effectiveConnectionStatus,
  failWorkingMessage,
  isConnectedToEditedAddress,
  messagesFromHistory,
  nextThemeMode,
  resolveThemeMode,
  userBubbleText,
  type PopupMessage,
  type ResolvedTheme,
  type ThemeMode
} from "./popup-state"
import type { SelectionContext } from "./selection-context"

type ChatMessage = PopupMessage
type BackgroundMessage =
  | { type: "reshape.progress"; event: TurnProgressEvent }
  | { type: "reshape.pendingContext"; context: SelectionContext | null }
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
type StatusResponse = {
  status: BackgroundStatus
  pendingContext?: SelectionContext | null
}

const INITIAL_MESSAGES: ChatMessage[] = []

function IndexPopup() {
  const [rpcAddress, setRpcAddress] = useState("127.0.0.1:7331")
  const [status, setStatus] = useState<ConnectionStatus>("idle")
  const [statusText, setStatusText] = useState("Handshake not started")
  const [connectedAddress, setConnectedAddress] = useState<string | undefined>()
  const [input, setInput] = useState("")
  const [pendingContext, setPendingContext] = useState<SelectionContext | null>(null)
  const [messages, setMessages] = useState<ChatMessage[]>(INITIAL_MESSAGES)
  const [themeMode, setThemeMode] = useState<ThemeMode>("system")
  const [resolvedTheme, setResolvedTheme] = useState<ResolvedTheme>(() =>
    resolveThemeMode("system", prefersDarkScheme())
  )
  const didAutoConnect = useRef(false)
  const effectiveStatus = effectiveConnectionStatus(status, rpcAddress, connectedAddress)
  const canChat = isConnectedToEditedAddress(status, rpcAddress, connectedAddress)
  const actionLabel = connectionActionLabel({
    status,
    rpcAddress,
    connectedAddress
  })
  const themeVars = useMemo(() => themeStyleVariables(resolvedTheme), [resolvedTheme])

  useEffect(() => {
    void initializeConnection()
  }, [])

  useEffect(() => {
    let isMounted = true
    void loadThemeMode()
      .then((mode) => {
        if (!isMounted) {
          return
        }
        setThemeMode(mode)
        setResolvedTheme(resolveThemeMode(mode, prefersDarkScheme()))
      })
      .catch(() => undefined)

    return () => {
      isMounted = false
    }
  }, [])

  useEffect(() => {
    const media = window.matchMedia?.("(prefers-color-scheme: dark)")
    if (!media) {
      return
    }

    const updateSystemTheme = () => {
      setResolvedTheme((current) =>
        themeMode === "system" ? resolveThemeMode("system", media.matches) : current
      )
    }

    updateSystemTheme()
    if (media.addEventListener) {
      media.addEventListener("change", updateSystemTheme)
    } else {
      media.addListener?.(updateSystemTheme)
    }
    return () => {
      if (media.removeEventListener) {
        media.removeEventListener("change", updateSystemTheme)
      } else {
        media.removeListener?.(updateSystemTheme)
      }
    }
  }, [themeMode])

  useEffect(() => {
    const listener = (message: BackgroundMessage) => {
      if (message?.type === "reshape.pendingContext") {
        setPendingContext(message.context)
        return
      }
      if (message?.type !== "reshape.progress") {
        return
      }
      setStatusText(message.event.message)
    }
    chrome.runtime.onMessage.addListener(listener)
    return () => chrome.runtime.onMessage.removeListener(listener)
  }, [])

  async function initializeConnection() {
    const address = await loadRpcAddress().catch(() => "127.0.0.1:7331")
    setRpcAddress(address)

    const status = await sendBackground<StatusResponse>({
      type: "reshape.status"
    }).catch(() => null)
    if (status?.status) {
      applyStatus(status.status)
      setPendingContext(status.pendingContext ?? null)
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

  function toggleThemeMode() {
    const next = nextThemeMode(themeMode)
    setThemeMode(next)
    setResolvedTheme(resolveThemeMode(next, prefersDarkScheme()))
    void saveThemeMode(next)
  }

  async function sendMessage() {
    const contextToSend = pendingContext
    const text = composePrompt(input, pendingContext)
    const bubbleText = userBubbleText(input, pendingContext)
    if (!bubbleText || !canChat) {
      return
    }

    setInput("")
    if (contextToSend) {
      setPendingContext(null)
      await sendBackground<{ pendingContext: null }>({
        type: "reshape.clearPendingContext"
      }).catch(() => undefined)
    }
    setStatusText("Agent is working...")
    setMessages((current) => [
      ...current,
      { role: "user", text: bubbleText },
      { role: "reshape", text: "Working...", status: "working" }
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
      const errorText = error instanceof Error ? error.message : "Send failed"
      if (contextToSend && shouldRestorePendingContext(errorText)) {
        setPendingContext(contextToSend)
        await sendBackground<{ pendingContext: SelectionContext }>({
          type: "reshape.setPendingContext",
          context: contextToSend
        }).catch(() => undefined)
      }
      setMessages((current) => failWorkingMessage(current, errorText))
      if (shouldRestorePendingContext(errorText)) {
        setStatus("error")
      }
      setStatusText(errorText)
    }
  }

  return (
    <main className="reshape-shell" data-theme={resolvedTheme} style={themeVars}>
      <style>{popupStyles}</style>
      <header className="reshape-header">
        <div className="reshape-title-row">
          <h1 className="reshape-title">Reshape</h1>
          <span
            aria-label={`RPC status: ${effectiveStatus}`}
            title={`RPC status: ${effectiveStatus}`}
            className="reshape-status-dot"
            data-status={effectiveStatus}
          />
        </div>
        <div className="reshape-toolbar">
          <button
            type="button"
            aria-label="Reset session"
            title="Reset session"
            disabled={!canChat}
            onClick={() => void resetSession()}
            className="reshape-button reshape-button-icon">
            <RotateCcw aria-hidden="true" size={16} strokeWidth={2} />
          </button>
          <button
            type="button"
            aria-label={`Theme: ${themeMode}`}
            title={`Theme: ${themeMode}`}
            onClick={toggleThemeMode}
            className="reshape-button reshape-button-icon">
            {themeIcon(themeMode)}
          </button>
        </div>
      </header>

      <section className="reshape-field-group">
        <label className="reshape-label" htmlFor="rpc-address">
          RPC address
        </label>
        <div className="reshape-address-row">
          <input
            id="rpc-address"
            value={rpcAddress}
            onChange={(event) => handleAddressChange(event.currentTarget.value)}
            placeholder="127.0.0.1:7331"
            className="reshape-input"
          />
          <button
            type="button"
            aria-label={actionLabel}
            title={actionLabel}
            onClick={handleConnectionAction}
            className="reshape-button reshape-button-square reshape-button-accent">
            {canChat ? (
              <Unplug aria-hidden="true" size={16} strokeWidth={2} />
            ) : (
              <Plug aria-hidden="true" size={16} strokeWidth={2} />
            )}
          </button>
        </div>
        {statusText ? <p className="reshape-status-text">{statusText}</p> : null}
      </section>

      <section className="reshape-messages" aria-label="Chat messages">
        {messages.map((message, index) => (
          <article
            key={`${message.role}-${index}`}
            className={messageClassName(message.role)}>
            <div>{message.text}</div>
          </article>
        ))}
      </section>

      <form
        className="reshape-chat-form"
        onSubmit={(event) => {
          event.preventDefault()
          void sendMessage()
        }}>
        <div className="reshape-composer">
          {pendingContext ? (
            <div className="reshape-context-chip">
              <span className="reshape-context-chip-text">{contextLabel(pendingContext)}</span>
              <button
                type="button"
                aria-label="Remove captured selection"
                title="Remove captured selection"
                onClick={() => void clearPendingContext()}
                className="reshape-button reshape-button-chip-remove">
                <X aria-hidden="true" size={13} strokeWidth={2} />
              </button>
            </div>
          ) : null}
          <input
            value={input}
            onChange={(event) => setInput(event.currentTarget.value)}
            disabled={!canChat}
            placeholder={
              canChat
                ? pendingContext
                  ? "Add an instruction..."
                  : "Tell reshape what to do..."
                : "Handshake first"
            }
            className="reshape-input"
          />
        </div>
        <button
          type="submit"
          aria-label="Send"
          title="Send"
          disabled={!canChat || (input.trim().length === 0 && !pendingContext)}
          className="reshape-button reshape-button-square reshape-button-primary">
          <Send aria-hidden="true" size={16} strokeWidth={2} />
        </button>
      </form>
    </main>
  )

  async function clearPendingContext() {
    setPendingContext(null)
    await sendBackground<{ pendingContext: null }>({
      type: "reshape.clearPendingContext"
    }).catch(() => undefined)
  }
}

function shouldRestorePendingContext(errorText: string): boolean {
  const normalized = errorText.toLowerCase()
  return normalized.includes("websocket") || normalized.includes("not connected")
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

function prefersDarkScheme(): boolean {
  return window.matchMedia?.("(prefers-color-scheme: dark)").matches ?? false
}

function themeIcon(mode: ThemeMode) {
  if (mode === "light") {
    return <Sun aria-hidden="true" size={16} strokeWidth={2} />
  }
  if (mode === "dark") {
    return <Moon aria-hidden="true" size={16} strokeWidth={2} />
  }
  return <Monitor aria-hidden="true" size={16} strokeWidth={2} />
}

function statusDotStyle(status: ConnectionStatus) {
  if (status === "connected") {
    return { background: "#22c55e" }
  }
  if (status === "connecting") {
    return { background: "#eab308" }
  }
  return { background: "#ef4444" }
}

function messageStyle(styles: ReturnType<typeof createStyles>, role: ChatMessage["role"]) {
  return {
    ...styles.message,
    ...(role === "user" ? styles.userMessage : {}),
    ...(role === "reshape" ? styles.reshapeMessage : {}),
    ...(role === "system" ? styles.systemMessage : {})
  }
}

function createStyles(theme: ResolvedTheme) {
  const palette =
    theme === "dark"
      ? {
          background: "#151515",
          surface: "#1d1d1d",
          surfaceMuted: "#242424",
          border: "#343434",
          text: "#f4f4f0",
          muted: "#a1a1aa",
          strong: "#ffffff",
          inverse: "#111111",
          focus: "#84cc16",
          warningSurface: "#302a16",
          warningText: "#fde68a"
        }
      : {
          background: "#faf9f6",
          surface: "#ffffff",
          surfaceMuted: "#f2f1ed",
          border: "#dedbd2",
          text: "#191918",
          muted: "#6f6b63",
          strong: "#111111",
          inverse: "#ffffff",
          focus: "#2563eb",
          warningSurface: "#fff7df",
          warningText: "#6b4e16"
        }

  return {
  shell: {
    width: 380,
    height: 520,
    boxSizing: "border-box",
    padding: 18,
    display: "flex",
    flexDirection: "column",
    gap: 14,
    overflow: "hidden",
    color: palette.text,
    background: palette.background,
    fontFamily:
      "-apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif"
  } as React.CSSProperties,
  header: {
    display: "flex",
    justifyContent: "space-between",
    alignItems: "center",
    gap: 12
  } as React.CSSProperties,
  toolbar: {
    display: "flex",
    alignItems: "center",
    gap: 6
  } as React.CSSProperties,
  titleRow: {
    display: "flex",
    alignItems: "center",
    gap: 9
  } as React.CSSProperties,
  title: {
    margin: 0,
    fontSize: 22,
    lineHeight: 1.1,
    fontWeight: 650,
    letterSpacing: 0,
    color: palette.strong
  } as React.CSSProperties,
  statusDot: {
    width: 8,
    height: 8,
    borderRadius: 999,
    flex: "0 0 auto"
  } as React.CSSProperties,
  iconButton: {
    width: 32,
    height: 32,
    border: `1px solid ${palette.border}`,
    borderRadius: 7,
    display: "inline-flex",
    alignItems: "center",
    justifyContent: "center",
    background: palette.surface,
    color: palette.text,
    cursor: "pointer",
    padding: 0,
    flex: "0 0 auto",
    transition: "background 120ms ease, border-color 120ms ease, color 120ms ease"
  } as React.CSSProperties,
  actionIconButton: {
    width: 44,
    height: 44,
    border: `1px solid ${palette.border}`,
    borderRadius: 8,
    display: "inline-flex",
    alignItems: "center",
    justifyContent: "center",
    background: palette.surface,
    color: palette.text,
    cursor: "pointer",
    padding: 0,
    flex: "0 0 auto",
    transition: "background 120ms ease, border-color 120ms ease, color 120ms ease"
  } as React.CSSProperties,
  iconButtonDisabled: {
    color: palette.muted,
    cursor: "not-allowed",
    opacity: 0.52
  } as React.CSSProperties,
  fieldGroup: {
    display: "flex",
    flexDirection: "column",
    gap: 7
  } as React.CSSProperties,
  label: {
    fontSize: 12,
    fontWeight: 600,
    color: palette.muted
  } as React.CSSProperties,
  addressRow: {
    display: "flex",
    alignItems: "center",
    gap: 8
  } as React.CSSProperties,
  input: {
    flex: 1,
    minWidth: 0,
    height: 44,
    boxSizing: "border-box",
    border: `1px solid ${palette.border}`,
    borderRadius: 8,
    padding: "10px 12px",
    fontSize: 14,
    lineHeight: "20px",
    outlineColor: palette.focus,
    background: palette.surface,
    color: palette.text
  } as React.CSSProperties,
  composer: {
    flex: 1,
    minWidth: 0,
    display: "flex",
    flexDirection: "column",
    gap: 6
  } as React.CSSProperties,
  contextChip: {
    display: "flex",
    alignItems: "center",
    gap: 6,
    maxWidth: "100%",
    border: `1px solid ${palette.border}`,
    borderRadius: 8,
    padding: "6px 8px",
    background: palette.surfaceMuted,
    color: palette.text,
    fontSize: 12,
    lineHeight: 1.2
  } as React.CSSProperties,
  contextChipText: {
    minWidth: 0,
    overflow: "hidden",
    textOverflow: "ellipsis",
    whiteSpace: "nowrap"
  } as React.CSSProperties,
  contextChipRemove: {
    width: 22,
    height: 22,
    border: `1px solid ${palette.border}`,
    borderRadius: 6,
    padding: 0,
    background: palette.surface,
    color: palette.text,
    cursor: "pointer",
    display: "inline-flex",
    alignItems: "center",
    justifyContent: "center",
    flex: "0 0 auto"
  } as React.CSSProperties,
  statusText: {
    margin: 0,
    minHeight: 18,
    maxHeight: 36,
    overflow: "hidden",
    color: palette.muted,
    fontSize: 12
  } as React.CSSProperties,
  messages: {
    flex: 1,
    minHeight: 0,
    display: "flex",
    flexDirection: "column",
    gap: 10,
    overflowY: "auto",
    border: `1px solid ${palette.border}`,
    borderRadius: 8,
    padding: 12,
    background: palette.surface
  } as React.CSSProperties,
  message: {
    maxWidth: "86%",
    borderRadius: 8,
    padding: "9px 11px",
    fontSize: 13,
    lineHeight: 1.45,
    whiteSpace: "pre-wrap"
  } as React.CSSProperties,
  userMessage: {
    alignSelf: "flex-end",
    background: palette.strong,
    color: palette.inverse
  } as React.CSSProperties,
  reshapeMessage: {
    alignSelf: "flex-start",
    background: palette.surfaceMuted,
    color: palette.text
  } as React.CSSProperties,
  systemMessage: {
    alignSelf: "center",
    background: palette.warningSurface,
    color: palette.warningText
  } as React.CSSProperties,
  chatForm: {
    display: "flex",
    alignItems: "flex-end",
    gap: 8
  } as React.CSSProperties
}
}

export default IndexPopup

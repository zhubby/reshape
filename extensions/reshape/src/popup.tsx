import { useEffect, useRef, useState } from "react"
import { CircleStop, Monitor, Moon, Plug, RotateCcw, Send, Sun, X } from "lucide-react"

import type { ConnectionStatus, HistoryResult } from "./rpc"
import "./style.css"
import { loadRpcAddress, loadThemeMode, saveRpcAddress, saveThemeMode } from "./storage"
import type { TabContext, TurnProgressEvent } from "./protocol"
import {
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
  | { type: "reshape.statusChanged"; status: BackgroundStatus }
type BackgroundStatus = {
  status: ConnectionStatus
  statusText: string
  address?: string
  history?: HistoryResult
  isWorking?: boolean
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
  const [isWorking, setIsWorking] = useState(false)
  const [themeMode, setThemeMode] = useState<ThemeMode>("system")
  const [resolvedTheme, setResolvedTheme] = useState<ResolvedTheme>(() =>
    resolveThemeMode("system", prefersDarkScheme())
  )
  const didAutoConnect = useRef(false)
  const effectiveStatus = effectiveConnectionStatus(status, rpcAddress, connectedAddress)
  const canChat = isConnectedToEditedAddress(status, rpcAddress, connectedAddress)
  const canSend = canChat && !isWorking
  const actionLabel = connectionActionLabel({
    status,
    rpcAddress,
    connectedAddress
  })
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
      if (message?.type === "reshape.statusChanged") {
        applyStatus(message.status)
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
    setIsWorking(snapshot.isWorking ?? false)
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
    if (!canSend) {
      return
    }

    try {
      const response = await sendBackground<{ status: BackgroundStatus }>({
        type: "reshape.resetSession"
      })
      applyStatus(response.status)
      setMessages(INITIAL_MESSAGES)
      setIsWorking(false)
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
    if (!bubbleText || !canSend) {
      return
    }

    setInput("")
    setIsWorking(true)
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
        status: BackgroundStatus
      }>({
        type: "reshape.send",
        text,
        displayText: bubbleText,
        tab
      })
      applyStatus(response.status)
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
      setIsWorking(false)
      setStatusText(errorText)
    }
  }

  return (
    <main className="reshape-shell" data-theme={resolvedTheme}>
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
            disabled={!canSend}
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
              <CircleStop aria-hidden="true" size={16} strokeWidth={2} />
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
            disabled={!canSend}
            placeholder={
              isWorking
                ? "Waiting for response..."
                : canChat
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
          disabled={!canSend || (input.trim().length === 0 && !pendingContext)}
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

function messageClassName(role: ChatMessage["role"]) {
  if (role === "user") {
    return "reshape-message reshape-message-user"
  }
  if (role === "reshape") {
    return "reshape-message reshape-message-reshape"
  }
  return "reshape-message reshape-message-system"
}

export default IndexPopup

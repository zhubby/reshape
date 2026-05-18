import type { ChatResult, ConnectionStatus } from "./rpc"
export { composePrompt, contextLabel, userBubbleText } from "./selection-context"

export type PopupMessage = {
  role: "user" | "reshape" | "system"
  text: string
  status?: "working" | "complete"
}

export type ThemeMode = "system" | "light" | "dark"
export type ResolvedTheme = "light" | "dark"

export function nextThemeMode(mode: ThemeMode): ThemeMode {
  if (mode === "system") {
    return "light"
  }
  if (mode === "light") {
    return "dark"
  }
  return "system"
}

export function normalizeThemeMode(value: unknown): ThemeMode {
  if (value === "light" || value === "dark" || value === "system") {
    return value
  }
  return "system"
}

export function resolveThemeMode(mode: ThemeMode, prefersDark: boolean): ResolvedTheme {
  if (mode === "system") {
    return prefersDark ? "dark" : "light"
  }
  return mode
}

export function isConnectedToEditedAddress(
  status: ConnectionStatus,
  rpcAddress: string,
  connectedAddress?: string
): boolean {
  return status === "connected" && rpcAddress.trim() === connectedAddress?.trim()
}

export function effectiveConnectionStatus(
  status: ConnectionStatus,
  rpcAddress: string,
  connectedAddress?: string
): ConnectionStatus {
  if (
    status === "connected" &&
    !isConnectedToEditedAddress(status, rpcAddress, connectedAddress)
  ) {
    return "idle"
  }
  return status
}

export function connectionActionLabel({
  status,
  rpcAddress,
  connectedAddress
}: {
  status: ConnectionStatus
  rpcAddress: string
  connectedAddress?: string
}): "Disconnect" | "Handshake" {
  return isConnectedToEditedAddress(status, rpcAddress, connectedAddress)
    ? "Disconnect"
    : "Handshake"
}

export function messagesFromHistory(
  fallback: PopupMessage[],
  history?: { messages: PopupMessage[] }
): PopupMessage[] {
  return history?.messages.length ? history.messages : fallback
}

export function completeWorkingMessage(
  messages: PopupMessage[],
  result: ChatResult
): PopupMessage[] {
  const index = lastWorkingReshapeIndex(messages)
  if (index === -1) {
    const last = messages.at(-1)
    if (last?.role === "reshape" && last.text === result.text) {
      return messages
    }
    return [...messages, { role: "reshape", text: result.text }]
  }

  return messages.map((message, messageIndex) => {
    if (messageIndex !== index) {
      return message
    }
    return {
      role: message.role,
      text: result.text,
      status: "complete"
    }
  })
}

export function failWorkingMessage(messages: PopupMessage[], text: string): PopupMessage[] {
  const index = lastWorkingReshapeIndex(messages)
  if (index === -1) {
    return [...messages, { role: "reshape", text }]
  }

  return messages.map((message, messageIndex) => {
    if (messageIndex !== index) {
      return message
    }
    return {
      role: message.role,
      text,
      status: "complete"
    }
  })
}

function lastWorkingReshapeIndex(messages: PopupMessage[]): number {
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    const message = messages[index]
    if (message.role === "reshape" && message.status === "working") {
      return index
    }
  }
  return -1
}

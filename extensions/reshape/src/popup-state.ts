import type { ChatResult, ConnectionStatus } from "./rpc"
export { composePrompt, contextLabel, userBubbleText } from "./selection-context"

export type PopupMessage = {
  role: "user" | "reshape" | "system"
  text: string
  status?: "working" | "complete"
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
}): "Handshake" | "Stop" {
  return isConnectedToEditedAddress(status, rpcAddress, connectedAddress)
    ? "Stop"
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

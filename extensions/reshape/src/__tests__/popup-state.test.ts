import { describe, expect, it } from "vitest"

import {
  completeWorkingMessage,
  connectionActionLabel,
  effectiveConnectionStatus,
  failWorkingMessage,
  isConnectedToEditedAddress,
  messagesFromHistory
} from "../popup-state"

describe("popup connection state", () => {
  it("shows stop only when the edited address is already connected", () => {
    expect(
      connectionActionLabel({
        status: "connected",
        rpcAddress: "127.0.0.1:7331",
        connectedAddress: "127.0.0.1:7331"
      })
    ).toBe("Stop")
  })

  it("shows handshake after a failure or address edit", () => {
    expect(
      connectionActionLabel({
        status: "error",
        rpcAddress: "127.0.0.1:7331",
        connectedAddress: "127.0.0.1:7331"
      })
    ).toBe("Handshake")

    expect(
      connectionActionLabel({
        status: "connected",
        rpcAddress: "localhost:7331",
        connectedAddress: "127.0.0.1:7331"
      })
    ).toBe("Handshake")
  })

  it("treats a connected socket as idle after the address is edited", () => {
    expect(
      isConnectedToEditedAddress("connected", "localhost:7331", "127.0.0.1:7331")
    ).toBe(false)
    expect(
      effectiveConnectionStatus("connected", "localhost:7331", "127.0.0.1:7331")
    ).toBe("idle")
  })

  it("uses hydrated backend history when it is available", () => {
    const fallback = []
    const history = {
      messages: [{ role: "user" as const, text: "previous prompt" }]
    }

    expect(messagesFromHistory(fallback, history)).toEqual(history.messages)
  })

  it("keeps the empty default when backend history is empty", () => {
    const fallback = []

    expect(messagesFromHistory(fallback, { messages: [] })).toEqual(fallback)
  })

  it("completes the latest working reshape message with final text only", () => {
    const activity = [
      {
        turnId: "turn-1",
        sequence: 1,
        kind: "turn_started" as const,
        toolName: null,
        argumentsPreview: null,
        resultPreview: null,
        message: "Agent turn started"
      }
    ]

    expect(
      completeWorkingMessage(
        [
          { role: "reshape", text: "Earlier", status: "complete" },
          { role: "reshape", text: "Working...", status: "working" }
        ],
        {
          id: "turn-1",
          text: "Done",
          activity
        }
      )
    ).toEqual([
      { role: "reshape", text: "Earlier", status: "complete" },
      { role: "reshape", text: "Done", status: "complete" }
    ])
  })

  it("replaces the latest working reshape message on failure", () => {
    expect(
      failWorkingMessage(
        [
          { role: "user", text: "change title" },
          { role: "reshape", text: "Working...", status: "working" }
        ],
        "Send failed"
      )
    ).toEqual([
      { role: "user", text: "change title" },
      { role: "reshape", text: "Send failed", status: "complete" }
    ])
  })
})

import { describe, expect, it, vi } from "vitest"

import {
  DEFAULT_RPC_ADDRESS,
  buildHistoryRequest,
  buildHandshakeFrame,
  buildInputRequest,
  buildResetSessionRequest,
  fetchHistory,
  isHandshakeAck,
  normalizeRpcAddress,
  outputText,
  resultText,
  resetSession,
  sendChatMessage
} from "../rpc"

describe("normalizeRpcAddress", () => {
  it("defaults to the local reshape rpc endpoint", () => {
    expect(normalizeRpcAddress("")).toBe("ws://127.0.0.1:7331/v1/rpc")
    expect(DEFAULT_RPC_ADDRESS).toBe("127.0.0.1:7331")
  })

  it("normalizes host and port input", () => {
    expect(normalizeRpcAddress("localhost:7331")).toBe(
      "ws://localhost:7331/v1/rpc"
    )
  })

  it("preserves a full websocket URL while forcing the rpc path", () => {
    expect(normalizeRpcAddress("ws://127.0.0.1:7331/v1/plugin")).toBe(
      "ws://127.0.0.1:7331/v1/rpc"
    )
  })
})

describe("rpc protocol frames", () => {
  it("builds the required handshake frame", () => {
    const frame = buildHandshakeFrame({
      id: 123,
      url: "http://127.0.0.1:7331/",
      title: "Reshape"
    })

    expect(frame).toEqual({
      type: "reshape.rpc.handshake",
      protocolVersion: "1.0",
      client: {
        name: "reshape-plasmo-extension",
        version: "0.1.0"
      },
      tab: {
        id: 123,
        url: "http://127.0.0.1:7331/",
        title: "Reshape"
      }
    })
  })

  it("detects handshake acknowledgements", () => {
    expect(
      isHandshakeAck({
        type: "reshape.rpc.handshake_ack",
        protocolVersion: "1.0",
        schemaVersion: "1.0",
        sessionKey: "local:main"
      })
    ).toBe(true)
  })

  it("builds reshape.input requests with tab metadata", () => {
    const request = buildInputRequest("turn-1", "create a page", {
      id: 123,
      url: "http://127.0.0.1:7331/",
      title: "Reshape"
    })

    expect(request.method).toBe("reshape.input")
    expect(request.params.input).toEqual({
      type: "user_text",
      text: "create a page"
    })
    expect(request.params.metadata).toEqual({
      client: "reshape-plasmo-extension",
      tabId: 123,
      url: "http://127.0.0.1:7331/",
      title: "Reshape"
    })
  })

  it("builds reshape.history requests", () => {
    expect(buildHistoryRequest("history-1")).toEqual({
      jsonrpc: "2.0",
      id: "history-1",
      method: "reshape.history",
      params: {}
    })
  })

  it("builds reshape.reset_session requests", () => {
    expect(buildResetSessionRequest("reset-1")).toEqual({
      jsonrpc: "2.0",
      id: "reset-1",
      method: "reshape.reset_session",
      params: {}
    })
  })
})

describe("outputText", () => {
  it("returns user-readable content from stable RPC outputs", () => {
    expect(
      outputText({
        type: "completed",
        summary: "Mock page generated in index.html"
      })
    ).toBe("Mock page generated in index.html")

    expect(
      outputText({
        type: "final_message",
        text: "Done"
      })
    ).toBe("Done")
  })
})

describe("resultText", () => {
  it("returns only the rpc output text when changed file metadata is present", () => {
    expect(
      resultText({
        output: {
          type: "completed",
          summary: "Page updated"
        },
        metadata: {
          changedFiles: ["index.html", "assets/site.css"]
        }
      })
    ).toBe("Page updated")
  })

  it("ignores malformed changed file metadata", () => {
    expect(
      resultText({
        output: {
          type: "completed",
          summary: "Page updated"
        },
        metadata: {
          changedFiles: [1, "index.html"]
        }
      })
    ).toBe("Page updated")
  })
})

describe("sendChatMessage", () => {
  it("reports progress notifications before resolving the final response", async () => {
    vi.spyOn(Date, "now").mockReturnValue(1)
    const socket = new FakeSocket([
      {
        jsonrpc: "2.0",
        method: "reshape.progress",
        params: {
          turnId: "turn-1",
          sequence: 1,
          kind: "tool_started",
          toolName: "write_file",
          argumentsPreview: "{\"path\":\"index.html\"}",
          resultPreview: null,
          message: "Running write_file"
        }
      },
      {
        jsonrpc: "2.0",
        id: "turn-1",
        result: {
          schemaVersion: "1.0",
          messageId: "message-1",
          traceId: "trace-1",
          sessionKey: "local:main",
          output: {
            type: "completed",
            summary: "Done"
          },
          metadata: {
            toolEvents: [
              {
                turnId: "turn-1",
                sequence: 1,
                kind: "tool_started",
                toolName: "write_file",
                argumentsPreview: "{\"path\":\"index.html\"}",
                resultPreview: null,
                message: "Running write_file"
              }
            ]
          }
        }
      }
    ])
    const progress: unknown[] = []

    const result = await sendChatMessage(
      socket as unknown as WebSocket,
      "create a page",
      {},
      (event) => progress.push(event)
    )

    expect(progress).toEqual([
      {
        turnId: "turn-1",
        sequence: 1,
        kind: "tool_started",
        toolName: "write_file",
        argumentsPreview: "{\"path\":\"index.html\"}",
        resultPreview: null,
        message: "Running write_file"
      }
    ])
    expect(result.text).toBe("Done")
    expect(result.activity).toEqual(progress)
    vi.restoreAllMocks()
  })
})

describe("resetSession", () => {
  it("returns the reset session history", async () => {
    vi.spyOn(Date, "now").mockReturnValue(2)
    const socket = new FakeSocket([
      {
        jsonrpc: "2.0",
        id: "reset-2",
        result: {
          schemaVersion: "1.0",
          sessionKey: "local:main",
          messages: []
        }
      }
    ])

    const result = await resetSession(socket as unknown as WebSocket)

    expect(result.messages).toEqual([])
    vi.restoreAllMocks()
  })
})

describe("fetchHistory", () => {
  it("ignores unrelated progress notifications while waiting for history", async () => {
    vi.spyOn(Date, "now").mockReturnValue(3)
    const socket = new FakeSocket([
      {
        jsonrpc: "2.0",
        method: "reshape.progress",
        params: {
          turnId: "turn-1",
          sequence: 1,
          kind: "tool_started",
          toolName: "write_file",
          argumentsPreview: "{}",
          resultPreview: null,
          message: "Running write_file"
        }
      },
      {
        jsonrpc: "2.0",
        id: "history-3",
        result: {
          schemaVersion: "1.0",
          sessionKey: "local:main",
          messages: [{ role: "user", text: "previous prompt" }]
        }
      }
    ])

    const result = await fetchHistory(socket as unknown as WebSocket)

    expect(result.messages).toEqual([{ role: "user", text: "previous prompt" }])
    vi.restoreAllMocks()
  })
})

class FakeSocket extends EventTarget {
  constructor(private readonly frames: unknown[]) {
    super()
  }

  send() {
    this.frames.forEach((frame) => {
      queueMicrotask(() => {
        this.dispatchEvent(
          new MessageEvent("message", {
            data: JSON.stringify(frame)
          })
        )
      })
    })
  }
}

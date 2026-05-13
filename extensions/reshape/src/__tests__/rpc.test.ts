import { describe, expect, it } from "vitest"

import {
  DEFAULT_RPC_ADDRESS,
  buildHandshakeFrame,
  buildInputRequest,
  isHandshakeAck,
  normalizeRpcAddress,
  outputText
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

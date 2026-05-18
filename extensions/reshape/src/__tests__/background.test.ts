import { afterEach, describe, expect, it, vi } from "vitest"

describe("background progress forwarding", () => {
  afterEach(() => {
    vi.resetModules()
    vi.unstubAllGlobals()
  })

  it("consumes missing receiver errors when forwarding progress", async () => {
    const sendMessage = vi.fn((_message, callback: () => void) => {
      callback()
    })

    vi.stubGlobal("chrome", {
      runtime: {
        lastError: { message: "Receiving end does not exist." },
        onMessage: {
          addListener: vi.fn()
        },
        sendMessage
      }
    })

    const { forwardProgress } = await import("../background")

    expect(() =>
      forwardProgress({
        turnId: "turn-1",
        sequence: 1,
        kind: "tool_started",
        toolName: "write_file",
        argumentsPreview: "{}",
        resultPreview: null,
        message: "Running write_file"
      })
    ).not.toThrow()
    expect(sendMessage).toHaveBeenCalledOnce()
  })
})

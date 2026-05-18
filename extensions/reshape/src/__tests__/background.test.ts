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

  it("registers a selection-only context menu", async () => {
    const create = vi.fn()
    vi.stubGlobal("chrome", {
      contextMenus: {
        create,
        onClicked: {
          addListener: vi.fn()
        }
      },
      runtime: {
        onInstalled: {
          addListener: vi.fn()
        },
        onMessage: {
          addListener: vi.fn()
        },
        sendMessage: vi.fn()
      }
    })

    const { registerSelectionContextMenu } = await import("../background")
    registerSelectionContextMenu()

    expect(create).toHaveBeenCalledWith({
      id: "reshape-capture-selection",
      title: "Capture selection in Reshape",
      contexts: ["selection"]
    })
  })

  it("stores pending selection context without sending an rpc request", async () => {
    const sendMessage = vi.fn((_message, callback: () => void) => callback())
    const managerSend = vi.fn()
    vi.stubGlobal("chrome", {
      action: {
        openPopup: vi.fn().mockResolvedValue(undefined)
      },
      scripting: {
        executeScript: vi.fn().mockResolvedValue([
          {
            result: {
              selectedText: "Launch the beta in June.",
              selectorHint: "main > section#roadmap",
              nearestHeading: "Roadmap",
              nearbyText: "Roadmap Launch the beta in June. Follow up with GA.",
              rect: { x: 1, y: 2, width: 3, height: 4 }
            }
          }
        ])
      },
      contextMenus: {
        create: vi.fn(),
        onClicked: {
          addListener: vi.fn()
        }
      },
      runtime: {
        onInstalled: {
          addListener: vi.fn()
        },
        onMessage: {
          addListener: vi.fn()
        },
        sendMessage,
        lastError: undefined
      }
    })

    const { handleSelectionContextMenuClick, setManagerForTests, getPendingContext } =
      await import("../background")
    setManagerForTests({ send: managerSend })

    await handleSelectionContextMenuClick(
      {
        menuItemId: "reshape-capture-selection",
        selectionText: "Launch the beta in June.",
        pageUrl: "https://example.test/notes"
      } as chrome.contextMenus.OnClickData,
      {
        id: 123,
        title: "Product notes",
        url: "https://example.test/notes"
      } as chrome.tabs.Tab
    )

    expect(managerSend).not.toHaveBeenCalled()
    expect(await getPendingContext()).toMatchObject({
      title: "Product notes",
      pageUrl: "https://example.test/notes",
      selectedText: "Launch the beta in June.",
      selectorHint: "main > section#roadmap"
    })
    expect(sendMessage).toHaveBeenCalledWith(
      {
        type: "reshape.pendingContext",
        context: expect.objectContaining({
          selectedText: "Launch the beta in June."
        })
      },
      expect.any(Function)
    )
  })
})

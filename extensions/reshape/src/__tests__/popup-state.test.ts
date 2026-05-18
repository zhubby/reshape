import { describe, expect, it } from "vitest"

import {
  composePrompt,
  contextLabel,
  completeWorkingMessage,
  connectionActionLabel,
  effectiveConnectionStatus,
  failWorkingMessage,
  isConnectedToEditedAddress,
  messagesFromHistory,
  nextThemeMode,
  normalizeThemeMode,
  resolveThemeMode,
  userBubbleText
} from "../popup-state"
import {
  contextFromMenuClick,
  lineNumberForSelection,
  workspacePathFromUrl
} from "../selection-context"

describe("popup connection state", () => {
  it("shows disconnect only when the edited address is already connected", () => {
    expect(
      connectionActionLabel({
        status: "connected",
        rpcAddress: "127.0.0.1:7331",
        connectedAddress: "127.0.0.1:7331"
      })
    ).toBe("Disconnect")
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

  it("does not duplicate a completed reshape message after status hydration", () => {
    expect(
      completeWorkingMessage(
        [
          { role: "user", text: "change title" },
          { role: "reshape", text: "Done", status: "complete" }
        ],
        {
          id: "turn-1",
          text: "Done",
          activity: []
        }
      )
    ).toEqual([
      { role: "user", text: "change title" },
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

  it("builds a concise label for pending browser selection context", () => {
    expect(
      contextLabel({
        title: "Product notes",
        pageUrl: "https://example.test/notes",
        filePath: "pages/product-notes.html",
        lineNumber: 12,
        selectedText: "Selected text from the current document"
      })
    ).toBe('Selection: pages/product-notes.html:12 · "Selected text from the current document"')
  })

  it("combines user intent with pending browser selection context for the llm", () => {
    expect(
      composePrompt("Fold this into the docs", {
        title: "Product notes",
        pageUrl: "http://127.0.0.1:7331/pages/product-notes.html",
        filePath: "pages/product-notes.html",
        lineNumber: 4,
        nearestHeading: "Roadmap",
        selectedText: "Launch the beta in June.",
        nearbyText: "Roadmap Launch the beta in June. Follow up with GA."
      })
    ).toBe(`User intent:
Fold this into the docs

Browser selection context:
- Title: Product notes
- URL: http://127.0.0.1:7331/pages/product-notes.html
- File: pages/product-notes.html
- Line: 4
- Element/Heading: Roadmap
- Selected text:
Launch the beta in June.
- Nearby context:
Roadmap Launch the beta in June. Follow up with GA.`)
  })

  it("uses a default wiki-tree intent when selection context is sent without typed text", () => {
    expect(
      userBubbleText("", {
        title: "Product notes",
        filePath: "pages/product-notes.html",
        selectedText: "Launch the beta in June."
      })
    ).toBe(
      'Use this browser selection to improve the wiki tree and related pages.\nSelection: pages/product-notes.html · "Launch the beta in June."'
    )
  })

  it("maps local render urls to workspace-relative files", () => {
    expect(workspacePathFromUrl("http://127.0.0.1:7331/")).toBe("index.html")
    expect(workspacePathFromUrl("http://127.0.0.1:7331/pages/topic.html")).toBe(
      "pages/topic.html"
    )
    expect(workspacePathFromUrl("http://127.0.0.1:7331/docs")).toBe(
      "docs/index.html"
    )
  })

  it("finds a best-effort line number for selected text", () => {
    expect(
      lineNumberForSelection(
        "Intro\nRoadmap\nLaunch the beta in June.\nFollow up with GA.",
        "Launch the beta in June."
      )
    ).toBe(3)
  })

  it("derives file and line context from a context menu capture", () => {
    expect(
      contextFromMenuClick(
        {
          menuItemId: "reshape-capture-selection",
          selectionText: "Launch the beta in June.",
          pageUrl: "http://127.0.0.1:7331/pages/product-notes.html"
        } as chrome.contextMenus.OnClickData,
        { title: "Product notes" } as chrome.tabs.Tab,
        {
          documentText: "Intro\nLaunch the beta in June.",
          nearestHeading: "Roadmap",
          selectedText: "Launch the beta in June."
        }
      )
    ).toMatchObject({
      filePath: "pages/product-notes.html",
      lineNumber: 2,
      nearestHeading: "Roadmap"
    })
  })
})

describe("popup theme state", () => {
  it("cycles through system, light, and dark themes", () => {
    expect(nextThemeMode("system")).toBe("light")
    expect(nextThemeMode("light")).toBe("dark")
    expect(nextThemeMode("dark")).toBe("system")
  })

  it("falls back to system for missing or invalid stored themes", () => {
    expect(normalizeThemeMode(undefined)).toBe("system")
    expect(normalizeThemeMode("unknown")).toBe("system")
    expect(normalizeThemeMode("dark")).toBe("dark")
  })

  it("resolves system theme from the current color scheme preference", () => {
    expect(resolveThemeMode("system", true)).toBe("dark")
    expect(resolveThemeMode("system", false)).toBe("light")
    expect(resolveThemeMode("light", true)).toBe("light")
    expect(resolveThemeMode("dark", false)).toBe("dark")
  })
})

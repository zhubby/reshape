import { RpcConnectionManager } from "./background-connection"
import type { TabContext, TurnProgressEvent } from "./protocol"
import {
  captureSelectionDetails,
  contextFromMenuClick,
  type CapturedSelectionDetails,
  type SelectionContext
} from "./selection-context"
import {
  clearPendingContext as clearStoredPendingContext,
  loadPendingContext,
  savePendingContext
} from "./storage"

type BackgroundMessage =
  | { type: "reshape.status" }
  | { type: "reshape.connect"; address: string; tab: TabContext }
  | { type: "reshape.send"; text: string; tab: TabContext }
  | { type: "reshape.resetSession" }
  | { type: "reshape.clearPendingContext" }
  | { type: "reshape.setPendingContext"; context: SelectionContext }
  | { type: "reshape.disconnect" }

export const SELECTION_CONTEXT_MENU_ID = "reshape-capture-selection"

let manager: Pick<RpcConnectionManager, "snapshot" | "connect" | "send" | "resetSession" | "disconnect"> =
  new RpcConnectionManager()
let pendingContext: SelectionContext | null | undefined

chrome.runtime.onMessage.addListener((message: BackgroundMessage, _sender, sendResponse) => {
  void handleMessage(message)
    .then((response) => sendResponse({ ok: true, ...response }))
    .catch((error) =>
      sendResponse({
        ok: false,
        error: error instanceof Error ? error.message : "background request failed"
      })
    )

  return true
})

async function handleMessage(message: BackgroundMessage) {
  switch (message.type) {
    case "reshape.status":
      return { status: manager.snapshot(), pendingContext: await getPendingContext() }
    case "reshape.connect":
      return { status: await manager.connect(message.address, message.tab) }
    case "reshape.send":
      return {
        result: await manager.send(message.text, message.tab, forwardProgress),
        status: manager.snapshot()
      }
    case "reshape.resetSession":
      return { status: await manager.resetSession() }
    case "reshape.clearPendingContext":
      await clearPendingContext()
      return { pendingContext: null }
    case "reshape.setPendingContext":
      await setPendingContext(message.context)
      return { pendingContext: message.context }
    case "reshape.disconnect":
      return { status: manager.disconnect() }
  }
}

export function registerSelectionContextMenu() {
  if (!chrome?.contextMenus?.create) {
    return
  }
  chrome.contextMenus.create({
    id: SELECTION_CONTEXT_MENU_ID,
    title: "Capture selection in Reshape",
    contexts: ["selection"]
  })
}

chrome.contextMenus?.onClicked?.addListener((info, tab) => {
  void handleSelectionContextMenuClick(info, tab)
})

chrome.runtime.onInstalled?.addListener(() => {
  registerSelectionContextMenu()
})

export async function handleSelectionContextMenuClick(
  info: chrome.contextMenus.OnClickData,
  tab?: chrome.tabs.Tab
) {
  if (info.menuItemId !== SELECTION_CONTEXT_MENU_ID) {
    return
  }
  const details = await captureSelectionFromTab(tab).catch(() => null)
  const context = contextFromMenuClick(info, tab, details)
  await setPendingContext(context)
  await openPopupBestEffort()
}

export async function getPendingContext(): Promise<SelectionContext | null> {
  if (pendingContext !== undefined) {
    return pendingContext
  }
  pendingContext = await loadPendingContext().catch(() => null)
  return pendingContext
}

export async function setPendingContext(context: SelectionContext): Promise<void> {
  pendingContext = context
  await savePendingContext(context)
  notifyPendingContext(context)
}

async function clearPendingContext(): Promise<void> {
  pendingContext = null
  await clearStoredPendingContext()
  notifyPendingContext(null)
}

async function captureSelectionFromTab(
  tab?: chrome.tabs.Tab
): Promise<CapturedSelectionDetails | null> {
  if (!tab?.id || !chrome?.scripting?.executeScript) {
    return null
  }
  const [result] = await chrome.scripting.executeScript({
    target: { tabId: tab.id },
    func: captureSelectionDetails
  })
  return (result?.result as CapturedSelectionDetails | undefined) ?? null
}

async function openPopupBestEffort(): Promise<void> {
  await chrome.action?.openPopup?.().catch(() => undefined)
}

function notifyPendingContext(context: SelectionContext | null) {
  chrome.runtime.sendMessage(
    {
      type: "reshape.pendingContext",
      context
    },
    () => {
      void chrome.runtime.lastError
    }
  )
}

export function forwardProgress(event: TurnProgressEvent) {
  chrome.runtime.sendMessage(
    {
      type: "reshape.progress",
      event
    },
    () => {
      void chrome.runtime.lastError
    }
  )
}

export function setManagerForTests(testManager: Partial<typeof manager>) {
  const base = new RpcConnectionManager()
  manager = Object.assign(base, testManager)
}

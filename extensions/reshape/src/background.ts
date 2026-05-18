import { RpcConnectionManager } from "./background-connection"
import type { TabContext, TurnProgressEvent } from "./protocol"

type BackgroundMessage =
  | { type: "reshape.status" }
  | { type: "reshape.connect"; address: string; tab: TabContext }
  | { type: "reshape.send"; text: string; tab: TabContext }
  | { type: "reshape.resetSession" }
  | { type: "reshape.disconnect" }

const manager = new RpcConnectionManager()

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
      return { status: manager.snapshot() }
    case "reshape.connect":
      return { status: await manager.connect(message.address, message.tab) }
    case "reshape.send":
      return {
        result: await manager.send(message.text, message.tab, forwardProgress),
        status: manager.snapshot()
      }
    case "reshape.resetSession":
      return { status: await manager.resetSession() }
    case "reshape.disconnect":
      return { status: manager.disconnect() }
  }
}

function forwardProgress(event: TurnProgressEvent) {
  chrome.runtime.sendMessage(
    {
      type: "reshape.progress",
      event
    },
    () => undefined
  )
}

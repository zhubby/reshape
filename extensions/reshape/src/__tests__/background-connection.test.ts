import { describe, expect, it, vi } from "vitest"

import { RpcConnectionManager } from "../background-connection"

describe("RpcConnectionManager", () => {
  it("keeps the connected socket across status checks", async () => {
    const socket = new EventTarget() as WebSocket
    socket.close = vi.fn()
    const connect = vi.fn().mockResolvedValue(socket)
    const manager = new RpcConnectionManager({ connect })

    const connected = await manager.connect("127.0.0.1:7331", { id: 1 })
    const status = manager.snapshot()

    expect(connected.status).toBe("connected")
    expect(status.status).toBe("connected")
    expect(connect).toHaveBeenCalledTimes(1)
  })

  it("reuses the existing socket when connecting to the same address", async () => {
    const socket = new EventTarget() as WebSocket
    socket.close = vi.fn()
    const connect = vi.fn().mockResolvedValue(socket)
    const manager = new RpcConnectionManager({ connect })

    await manager.connect("127.0.0.1:7331", { id: 1 })
    await manager.connect("127.0.0.1:7331", { id: 1 })

    expect(connect).toHaveBeenCalledTimes(1)
    expect(socket.close).not.toHaveBeenCalled()
  })

  it("sends messages over the connected socket", async () => {
    const socket = new EventTarget() as WebSocket
    socket.close = vi.fn()
    const send = vi.fn().mockResolvedValue({ id: "turn-1", text: "ok" })
    const manager = new RpcConnectionManager({
      connect: vi.fn().mockResolvedValue(socket),
      send
    })

    await manager.connect("127.0.0.1:7331", { id: 1 })
    const result = await manager.send("create a page", { id: 1 })

    expect(result.text).toBe("ok")
    expect(send).toHaveBeenCalledWith(socket, "create a page", { id: 1 })
  })
})

import { describe, expect, it, vi } from "vitest"

import { RpcConnectionManager } from "../background-connection"
import { RpcResponseError } from "../rpc"

describe("RpcConnectionManager", () => {
  it("keeps the connected socket across status checks", async () => {
    const socket = new EventTarget() as WebSocket
    socket.close = vi.fn()
    const connect = vi.fn().mockResolvedValue(socket)
    const history = vi.fn().mockResolvedValue({ messages: [] })
    const manager = new RpcConnectionManager({ connect, history })

    const connected = await manager.connect("127.0.0.1:7331", { id: 1 })
    const status = manager.snapshot()

    expect(connected.status).toBe("connected")
    expect(status.status).toBe("connected")
    expect(connect).toHaveBeenCalledTimes(1)
    expect(history).toHaveBeenCalledWith(socket)
  })

  it("reuses the existing socket when connecting to the same address", async () => {
    const socket = new EventTarget() as WebSocket
    socket.close = vi.fn()
    const connect = vi.fn().mockResolvedValue(socket)
    const manager = new RpcConnectionManager({
      connect,
      history: vi.fn().mockResolvedValue({ messages: [] })
    })

    await manager.connect("127.0.0.1:7331", { id: 1 })
    await manager.connect("127.0.0.1:7331", { id: 1 })

    expect(connect).toHaveBeenCalledTimes(1)
    expect(socket.close).not.toHaveBeenCalled()
  })

  it("sends messages over the connected socket", async () => {
    const socket = new EventTarget() as WebSocket
    socket.close = vi.fn()
    const send = vi.fn().mockResolvedValue({
      id: "turn-1",
      text: "ok",
      metadata: { changedFiles: ["index.html"] }
    })
    const manager = new RpcConnectionManager({
      connect: vi.fn().mockResolvedValue(socket),
      history: vi.fn().mockResolvedValue({ messages: [] }),
      resetSession: vi.fn().mockResolvedValue({ messages: [] }),
      send
    })

    await manager.connect("127.0.0.1:7331", { id: 1 })
    const result = await manager.send("create a page", { id: 1 })

    expect(result.text).toBe("ok")
    expect(result.metadata).toEqual({ changedFiles: ["index.html"] })
    expect(send).toHaveBeenCalledWith(socket, "create a page", { id: 1 }, undefined)
  })

  it("passes progress callbacks through to the rpc client", async () => {
    const socket = new EventTarget() as WebSocket
    socket.close = vi.fn()
    const progress = vi.fn()
    const send = vi.fn().mockResolvedValue({
      id: "turn-1",
      text: "ok",
      activity: []
    })
    const manager = new RpcConnectionManager({
      connect: vi.fn().mockResolvedValue(socket),
      history: vi.fn().mockResolvedValue({ messages: [] }),
      send
    })

    await manager.connect("127.0.0.1:7331", { id: 1 })
    await manager.send("create a page", { id: 1 }, progress)

    expect(send).toHaveBeenCalledWith(socket, "create a page", { id: 1 }, progress)
  })

  it("keeps the socket connected after a json-rpc business error", async () => {
    const socket = new EventTarget() as WebSocket
    socket.close = vi.fn()
    const manager = new RpcConnectionManager({
      connect: vi.fn().mockResolvedValue(socket),
      history: vi.fn().mockResolvedValue({ messages: [] }),
      send: vi.fn().mockRejectedValue(new RpcResponseError("provider error"))
    })

    await manager.connect("127.0.0.1:7331", { id: 1 })
    await expect(manager.send("create a page", { id: 1 })).rejects.toThrow(
      "provider error"
    )

    expect(manager.snapshot()).toMatchObject({
      status: "connected",
      statusText: "provider error",
      address: "127.0.0.1:7331"
    })
    expect(socket.close).not.toHaveBeenCalled()
  })

  it("marks the connection errored after a transport send failure", async () => {
    const socket = new EventTarget() as WebSocket
    socket.close = vi.fn()
    const manager = new RpcConnectionManager({
      connect: vi.fn().mockResolvedValue(socket),
      history: vi.fn().mockResolvedValue({ messages: [] }),
      send: vi.fn().mockRejectedValue(new Error("websocket error"))
    })

    await manager.connect("127.0.0.1:7331", { id: 1 })
    await expect(manager.send("create a page", { id: 1 })).rejects.toThrow(
      "websocket error"
    )

    expect(manager.snapshot()).toMatchObject({
      status: "error",
      statusText: "websocket error"
    })
  })

  it("reports connection state in English", async () => {
    const socket = new EventTarget() as WebSocket
    socket.close = vi.fn()
    const manager = new RpcConnectionManager({
      connect: vi.fn().mockResolvedValue(socket),
      history: vi.fn().mockResolvedValue({ messages: [] })
    })

    expect(manager.snapshot().statusText).toBe("Handshake not started")

    const connected = await manager.connect("127.0.0.1:7331", { id: 1 })
    expect(connected.statusText).toBe("")

    const disconnected = manager.disconnect()
    expect(disconnected.statusText).toBe("Connection closed")
  })

  it("exposes hydrated history after connecting", async () => {
    const socket = new EventTarget() as WebSocket
    socket.close = vi.fn()
    const manager = new RpcConnectionManager({
      connect: vi.fn().mockResolvedValue(socket),
      history: vi.fn().mockResolvedValue({
        messages: [{ role: "user", text: "previous prompt" }]
      })
    })

    const connected = await manager.connect("127.0.0.1:7331", { id: 1 })

    expect(connected.history?.messages).toEqual([
      { role: "user", text: "previous prompt" }
    ])
  })

  it("resets the connected session and clears hydrated history", async () => {
    const socket = new EventTarget() as WebSocket
    socket.close = vi.fn()
    const resetSession = vi.fn().mockResolvedValue({ messages: [] })
    const manager = new RpcConnectionManager({
      connect: vi.fn().mockResolvedValue(socket),
      history: vi.fn().mockResolvedValue({
        messages: [{ role: "user", text: "previous prompt" }]
      }),
      resetSession
    })

    await manager.connect("127.0.0.1:7331", { id: 1 })
    const snapshot = await manager.resetSession()

    expect(resetSession).toHaveBeenCalledWith(socket)
    expect(snapshot.status).toBe("connected")
    expect(snapshot.history?.messages).toEqual([])
  })
})

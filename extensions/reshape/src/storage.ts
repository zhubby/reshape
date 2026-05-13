import { Storage } from "@plasmohq/storage"

import { DEFAULT_RPC_ADDRESS } from "./rpc"

const storage = new Storage()
const RPC_ADDRESS_KEY = "reshape.rpcAddress"

export async function loadRpcAddress(): Promise<string> {
  return (await storage.get<string>(RPC_ADDRESS_KEY)) || DEFAULT_RPC_ADDRESS
}

export async function saveRpcAddress(address: string): Promise<void> {
  const trimmed = address.trim() || DEFAULT_RPC_ADDRESS
  await storage.set(RPC_ADDRESS_KEY, trimmed)
}

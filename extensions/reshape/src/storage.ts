import { Storage } from "@plasmohq/storage"

import { DEFAULT_RPC_ADDRESS } from "./rpc"
import { normalizeThemeMode, type ThemeMode } from "./popup-state"
import type { SelectionContext } from "./selection-context"

const storage = new Storage()
const RPC_ADDRESS_KEY = "reshape.rpcAddress"
const PENDING_CONTEXT_KEY = "reshape.pendingContext"
const THEME_MODE_KEY = "reshape.themeMode"

export async function loadRpcAddress(): Promise<string> {
  return (await storage.get<string>(RPC_ADDRESS_KEY)) || DEFAULT_RPC_ADDRESS
}

export async function saveRpcAddress(address: string): Promise<void> {
  const trimmed = address.trim() || DEFAULT_RPC_ADDRESS
  await storage.set(RPC_ADDRESS_KEY, trimmed)
}

export async function loadPendingContext(): Promise<SelectionContext | null> {
  return (await storage.get<SelectionContext>(PENDING_CONTEXT_KEY)) || null
}

export async function savePendingContext(context: SelectionContext): Promise<void> {
  await storage.set(PENDING_CONTEXT_KEY, context)
}

export async function clearPendingContext(): Promise<void> {
  await storage.remove(PENDING_CONTEXT_KEY)
}

export async function loadThemeMode(): Promise<ThemeMode> {
  return normalizeThemeMode(await storage.get<unknown>(THEME_MODE_KEY))
}

export async function saveThemeMode(mode: ThemeMode): Promise<void> {
  await storage.set(THEME_MODE_KEY, mode)
}

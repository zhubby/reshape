import type {
  PluginHandshake as GeneratedPluginHandshake,
  PluginHandshakeAck as GeneratedPluginHandshakeAck,
  ReshapeInputRequest as GeneratedReshapeInputRequest,
  RpcOutput as GeneratedRpcOutput,
  RpcWireResponse
} from "./generated/reshape"

export const PLUGIN_PROTOCOL_VERSION = "1.0"
export const RESHAPE_SCHEMA_VERSION = "1.0"
export const RESHAPE_SESSION_KEY = "local:main"
export const PLUGIN_CLIENT_NAME = "reshape-plasmo-extension"
export const PLUGIN_CLIENT_VERSION = "0.1.0"

export type TabContext = {
  id?: number
  url?: string
  title?: string
}

export type PluginHandshakeFrame = GeneratedPluginHandshake
export type PluginHandshakeAck = GeneratedPluginHandshakeAck
export type ReshapeInputRequest = GeneratedReshapeInputRequest
export type RpcOutput = GeneratedRpcOutput
export type RpcResponse = RpcWireResponse

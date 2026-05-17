import type {
  RpcHandshake as GeneratedRpcHandshake,
  RpcHandshakeAck as GeneratedRpcHandshakeAck,
  ReshapeInputRequest as GeneratedReshapeInputRequest,
  RpcHistoryBody as GeneratedRpcHistoryBody,
  RpcHistoryMessage as GeneratedRpcHistoryMessage,
  RpcProgressNotification as GeneratedRpcProgressNotification,
  RpcResultBody as GeneratedRpcResultBody,
  RpcOutput as GeneratedRpcOutput,
  RpcWireResponse,
  TurnProgressEvent as GeneratedTurnProgressEvent
} from "./generated/reshape"

export const RPC_PROTOCOL_VERSION = "1.0"
export const RESHAPE_SCHEMA_VERSION = "1.0"
export const RESHAPE_SESSION_KEY = "local:main"
export const PLUGIN_CLIENT_NAME = "reshape-plasmo-extension"
export const PLUGIN_CLIENT_VERSION = "0.1.0"

export type TabContext = {
  id?: number
  url?: string
  title?: string
}

export type RpcHandshake = GeneratedRpcHandshake
export type RpcHandshakeAck = GeneratedRpcHandshakeAck
export type ReshapeInputRequest = GeneratedReshapeInputRequest
export type RpcHistoryBody = GeneratedRpcHistoryBody
export type RpcHistoryMessage = GeneratedRpcHistoryMessage
export type RpcProgressNotification = GeneratedRpcProgressNotification
export type RpcResultBody = GeneratedRpcResultBody
export type RpcOutput = GeneratedRpcOutput
export type RpcResponse = RpcWireResponse
export type TurnProgressEvent = GeneratedTurnProgressEvent

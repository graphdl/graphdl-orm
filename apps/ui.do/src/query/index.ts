/**
 * Public re-exports for the query layer.
 *
 *   createArestQueryClient() — a QueryClient preset with AREST-friendly
 *     defaults (staleTime=0 so invalidation always refetches).
 *   createArestQueryBridge() — EventSource subscriber that invalidates
 *     on every CellEvent. Pair with the client.
 *   createArestQueryKeys() — helpers for building the canonical keys.
 */
export {
  createArestQueryBridge,
  createArestQueryKeys,
  nounToSlug,
  type ArestQueryBridge,
  type ArestQueryBridgeOptions,
  type ArestQueryKeys,
  type CellEventPayload,
} from './arestQueryBridge'
export { createArestQueryClient } from './arestQueryClient'

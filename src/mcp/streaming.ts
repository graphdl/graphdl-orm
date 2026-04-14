/**
 * MCP Streaming Transport — cell events over streamable HTTP.
 *
 * Browser clients connect to /mcp/ and receive cell events as MCP
 * notifications. When a cell changes via ↓ (store), the server pushes
 * a notification through the stream. The client's WASM engine folds
 * the event locally.
 *
 * This is the paper's Section 7.5: "A browser runtime is a node in the
 * same model. Cells replicated locally fold events with zero latency."
 * The MCP transport IS the event stream.
 */

import { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js'
import { z } from 'zod'
import { systemCall, safeJson } from './engine-bridge.js'

/**
 * Minimal shape of the BroadcastDO stub the streaming tools need.
 * Matches the RPC subset used here so a vi.fn()-based mock can stand
 * in for tests without importing the DO runtime.
 */
export interface BroadcastStreamingStub {
  registerFilter(filter: { domain: string; noun?: string; entityId?: string }): Promise<string>
  unsubscribe(id: string): Promise<boolean>
  listSubscribers(): Promise<readonly string[]>
}

/**
 * Create an MCP server instance configured for streaming cell events.
 *
 * `env` exposes `BROADCAST` — the BroadcastDO namespace (see
 * src/broadcast-do.ts + wrangler.jsonc). When supplied, subscribe/
 * unsubscribe/list calls go through the real DO; when absent (e.g.
 * local stdio mode with no DO runtime), the tools return a
 * degraded-but-honest response.
 *
 * Event delivery: subscriptions registered via MCP go into the DO's
 * registry with a no-op callback. Clients receive actual CellEvent
 * frames by opening the companion `/api/events` SSE stream with the
 * same filter shape. MCP-transport notifications are deferred — the
 * streamable-HTTP wiring for server→client pushes is a separate
 * concern tracked in the ui.do chain.
 */
export interface StreamingEnv {
  BROADCAST?: {
    idFromName(name: string): unknown
    get(id: unknown): BroadcastStreamingStub
  }
}

export function createStreamingServer(env?: StreamingEnv) {
  const server = new McpServer({
    name: 'arest-streaming',
    version: '0.1.0',
  })

  function broadcastStub(): BroadcastStreamingStub | null {
    if (!env?.BROADCAST) return null
    return env.BROADCAST.get(env.BROADCAST.idFromName('global'))
  }

  server.tool(
    'arest_list',
    'List entities of a noun type. Returns the full population via the shared ' +
    'WASM engine handle; page/limit are client-side pagination hints carried ' +
    'through to the response for the caller to apply.',
    {
      noun: z.string().describe('Entity noun to enumerate (e.g. "Order").'),
      domain: z.string().describe('Domain slug for namespacing — today informational; the engine serves one D per isolate.'),
      page: z.number().optional(),
      limit: z.number().optional(),
    },
    async ({ noun, domain, page, limit }) => {
      const raw = await systemCall(`list:${noun}`, '')
      const parsed = safeJson(raw, null as unknown)
      const entities = Array.isArray(parsed) ? parsed : []
      // Pagination is applied here on the materialised array. The
      // engine's list:{noun} returns the full set today; future work
      // can push pagination into the generator for large populations.
      const start = typeof page === 'number' && typeof limit === 'number'
        ? page * limit : 0
      const end = typeof limit === 'number' ? start + limit : entities.length
      const pageItems = entities.slice(start, end)
      return {
        content: [{
          type: 'text' as const,
          text: JSON.stringify({
            noun,
            domain,
            total: entities.length,
            page: typeof page === 'number' ? page : 0,
            limit: typeof limit === 'number' ? limit : entities.length,
            entities: pageItems,
          }),
        }],
      }
    },
  )

  server.tool(
    'arest_subscriptions',
    'List active event subscription ids on the BroadcastDO registry.',
    {},
    async () => {
      const stub = broadcastStub()
      if (!stub) {
        return {
          content: [{
            type: 'text' as const,
            text: JSON.stringify({
              subscribers: [],
              _note: 'BroadcastDO binding unavailable; returning empty list',
            }),
          }],
        }
      }
      const subscribers = await stub.listSubscribers()
      return {
        content: [{
          type: 'text' as const,
          text: JSON.stringify({ subscribers }),
        }],
      }
    },
  )

  server.tool(
    'arest_subscribe',
    'Register a cell-event subscription filter. Returns a subscription id; ' +
    'open GET /api/events with the same filter to receive CellEvent frames.',
    {
      domain: z.string().describe('Required. The domain slug to scope the subscription.'),
      noun: z.string().optional().describe('Narrow to a single noun type.'),
      entityId: z.string().optional().describe('Narrow to a single entity id.'),
    },
    async ({ domain, noun, entityId }) => {
      const stub = broadcastStub()
      if (!stub) {
        return {
          content: [{
            type: 'text' as const,
            text: JSON.stringify({
              subscribed: false,
              error: 'BroadcastDO binding unavailable',
            }),
          }],
        }
      }
      const id = await stub.registerFilter({ domain, noun, entityId })
      return {
        content: [{
          type: 'text' as const,
          text: JSON.stringify({
            subscribed: true,
            subscriptionId: id,
            filter: { domain, noun, entityId },
            eventsUrl: `/api/events?domain=${encodeURIComponent(domain)}` +
              (noun ? `&noun=${encodeURIComponent(noun)}` : '') +
              (entityId ? `&entityId=${encodeURIComponent(entityId)}` : ''),
          }),
        }],
      }
    },
  )

  server.tool(
    'arest_unsubscribe',
    'Remove a subscription by id.',
    {
      id: z.string().describe('The subscription id returned by arest_subscribe.'),
    },
    async ({ id }) => {
      const stub = broadcastStub()
      if (!stub) {
        return {
          content: [{
            type: 'text' as const,
            text: JSON.stringify({
              unsubscribed: false,
              error: 'BroadcastDO binding unavailable',
            }),
          }],
        }
      }
      const removed = await stub.unsubscribe(id)
      return {
        content: [{
          type: 'text' as const,
          text: JSON.stringify({ unsubscribed: removed, id }),
        }],
      }
    },
  )

  return server
}

/**
 * Cell event payload — what gets streamed to subscribed clients.
 * The client's WASM engine folds this event using foldl transition.
 */
export interface CellEvent {
  /** The domain this event belongs to */
  domain: string
  /** The noun type of the changed entity */
  noun: string
  /** The entity ID */
  entityId: string
  /** The operation that caused the change */
  operation: 'create' | 'update' | 'delete' | 'transition'
  /** The fact changes (new bindings) */
  facts: Record<string, unknown>
  /** Server timestamp */
  timestamp: number
  /** Event sequence number for replay */
  sequence: number
}

/**
 * Format a cell event as an MCP notification payload.
 * Clients receive this through the streamable HTTP connection.
 */
export function formatCellEvent(event: CellEvent) {
  return {
    method: 'notifications/cell_event',
    params: event,
  }
}

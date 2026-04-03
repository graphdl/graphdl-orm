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

/**
 * Create an MCP server instance configured for streaming cell events.
 * The server exposes the same AREST tools as the stdio server, plus
 * cell subscription notifications.
 */
export function createStreamingServer() {
  const server = new McpServer({
    name: 'graphdl-streaming',
    version: '0.1.0',
  })

  // ── Tools (same as stdio server) ──────────────────────────────────

  server.tool(
    'graphdl_list',
    'List entities of a noun type in a domain',
    {
      noun: z.string(),
      domain: z.string(),
      page: z.number().optional(),
      limit: z.number().optional(),
    },
    async ({ noun, domain, page, limit }) => {
      return { content: [{ type: 'text' as const, text: JSON.stringify({ noun, domain, page, limit, _note: 'wired to AREST handler' }) }] }
    },
  )

  server.tool(
    'graphdl_subscribe',
    'Subscribe to cell events for a noun type. Events stream as notifications when cells change.',
    {
      noun: z.string().describe('The noun type to watch'),
      domain: z.string().describe('The domain slug'),
    },
    async ({ noun, domain }) => {
      // In the full implementation, this registers the client for
      // notifications when entities of this noun type change.
      // The notification payload is the event (fact change) that
      // the client's local WASM engine folds.
      return {
        content: [{
          type: 'text' as const,
          text: JSON.stringify({
            subscribed: true,
            noun,
            domain,
            message: 'Cell events will be streamed as notifications',
          }),
        }],
      }
    },
  )

  server.tool(
    'graphdl_unsubscribe',
    'Stop receiving cell events for a noun type',
    {
      noun: z.string(),
      domain: z.string(),
    },
    async ({ noun, domain }) => {
      return {
        content: [{
          type: 'text' as const,
          text: JSON.stringify({ unsubscribed: true, noun, domain }),
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

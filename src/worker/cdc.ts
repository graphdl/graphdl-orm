/**
 * CDC (Change Data Capture) — Worker-layer aggregation of per-entity change
 * events. After the metamodel-eats-itself refactor, each EntityDB DO owns its
 * own CDC table. Domain-wide CDC feeds are assembled here from batch commits
 * rather than inside a monolithic DomainDB.
 */

import type { BatchEntity } from '../batch-wal'

// =========================================================================
// Types
// =========================================================================

export interface CdcEvent {
  entityId: string
  type: string
  domain: string
  operation: 'create' | 'update' | 'delete'
  timestamp: string
  data?: Record<string, unknown>
}

// =========================================================================
// Build CDC events from a batch commit
// =========================================================================

/**
 * Converts a batch of entities into CDC events. All events in a single
 * call share one timestamp so consumers see them as an atomic batch.
 *
 * Delete events omit `data` — the entity is gone, only its identity matters.
 */
export function buildCdcEvents(
  entities: BatchEntity[],
  operation: 'create' | 'update' | 'delete',
): CdcEvent[] {
  const timestamp = new Date().toISOString()
  return entities.map(entity => ({
    entityId: entity.id,
    type: entity.type,
    domain: entity.domain,
    operation,
    timestamp,
    data: operation === 'delete' ? undefined : entity.data,
  }))
}

// =========================================================================
// Format CDC message for downstream consumers
// =========================================================================

/**
 * Serialises CDC events into a JSON message suitable for WebSocket
 * broadcast, queue publish, or Durable Object alarm relay.
 */
export function formatCdcMessage(events: CdcEvent[]): string {
  return JSON.stringify({ type: 'cdc', events })
}

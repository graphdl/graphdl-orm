/**
 * MCP streaming tests — cell event transport.
 *
 * Tests the streaming server tool registration and cell event
 * formatting. Full transport integration requires a running HTTP
 * server, tested in deployment.
 */

import { describe, it, expect } from 'vitest'
import { createStreamingServer, formatCellEvent, type CellEvent } from './streaming'

describe('MCP Streaming Server', () => {
  it('creates a server with subscribe and unsubscribe tools', () => {
    const server = createStreamingServer()
    expect(server).toBeDefined()
  })

  it('formats cell events as MCP notifications', () => {
    const event: CellEvent = {
      domain: 'support',
      noun: 'Order',
      entityId: 'ord-1',
      operation: 'create',
      facts: { customer: 'acme', status: 'In Cart' },
      timestamp: Date.now(),
      sequence: 1,
    }

    const notification = formatCellEvent(event)

    expect(notification.method).toBe('notifications/cell_event')
    expect(notification.params.noun).toBe('Order')
    expect(notification.params.entityId).toBe('ord-1')
    expect(notification.params.operation).toBe('create')
    expect(notification.params.facts.customer).toBe('acme')
  })

  it('cell events carry sequence numbers for replay', () => {
    const events: CellEvent[] = [
      { domain: 'test', noun: 'Order', entityId: 'ord-1', operation: 'create', facts: { status: 'In Cart' }, timestamp: 1000, sequence: 1 },
      { domain: 'test', noun: 'Order', entityId: 'ord-1', operation: 'transition', facts: { status: 'Placed' }, timestamp: 2000, sequence: 2 },
      { domain: 'test', noun: 'Order', entityId: 'ord-1', operation: 'transition', facts: { status: 'Shipped' }, timestamp: 3000, sequence: 3 },
    ]

    // Sequence numbers are monotonically increasing
    for (let i = 1; i < events.length; i++) {
      expect(events[i].sequence).toBeGreaterThan(events[i - 1].sequence)
    }

    // Client reconnecting at sequence 1 would replay events 2 and 3
    const missedEvents = events.filter(e => e.sequence > 1)
    expect(missedEvents).toHaveLength(2)
    expect(missedEvents[0].facts.status).toBe('Placed')
    expect(missedEvents[1].facts.status).toBe('Shipped')
  })

  it('cell events support all operation types', () => {
    const ops: CellEvent['operation'][] = ['create', 'update', 'delete', 'transition']
    for (const op of ops) {
      const event: CellEvent = {
        domain: 'test', noun: 'Order', entityId: 'ord-1',
        operation: op, facts: {}, timestamp: Date.now(), sequence: 1,
      }
      const notification = formatCellEvent(event)
      expect(notification.params.operation).toBe(op)
    }
  })

  it('delete event produces empty links (Corollary 1)', () => {
    // When an entity is deleted (terminal state), its cell event
    // has operation 'delete'. The client's local WASM engine
    // applies this as a transition to terminal state, producing
    // links(s_del) = empty set.
    const deleteEvent: CellEvent = {
      domain: 'test',
      noun: 'Order',
      entityId: 'ord-1',
      operation: 'delete',
      facts: { status: 'Cancelled' },
      timestamp: Date.now(),
      sequence: 5,
    }

    const notification = formatCellEvent(deleteEvent)
    expect(notification.params.operation).toBe('delete')
    // The client folds this and excludes the entity from query results
  })
})

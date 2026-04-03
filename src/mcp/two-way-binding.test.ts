/**
 * Two-way data binding tests — client and server cell sync.
 *
 * Simulates the paper's Section 5.2: "when a cell's contents change
 * via ↓, the ρ-application re-evaluates and the bound function fires."
 *
 * Tests both directions:
 * - Server → Client: server writes to a cell, client receives event, folds it
 * - Client → Server: client writes locally, sends event to server, server validates and folds
 * - Conflict: both write to the same cell, server is authoritative
 */

import { describe, it, expect } from 'vitest'
import { formatCellEvent, type CellEvent } from './streaming'

/** Simulates a cell store with bound functions that fire on ↓ (store). */
function createCellWithBindings() {
  let state: Record<string, unknown> = {}
  const bindings: Array<(newState: Record<string, unknown>) => void> = []
  const eventLog: CellEvent[] = []
  let sequence = 0

  return {
    /** Read the cell's current state */
    get: () => ({ ...state }),

    /** ↓ store: write to the cell, fire all bound functions */
    put: (newState: Record<string, unknown>) => {
      state = { ...state, ...newState }
      bindings.forEach(fn => fn(state))
    },

    /** Bind a function that fires on every ↓ */
    bind: (fn: (s: Record<string, unknown>) => void) => {
      bindings.push(fn)
    },

    /** Record an event for the event stream */
    emit: (domain: string, noun: string, entityId: string, operation: CellEvent['operation'], facts: Record<string, unknown>) => {
      sequence++
      const event: CellEvent = { domain, noun, entityId, operation, facts, timestamp: Date.now(), sequence }
      eventLog.push(event)
      return event
    },

    /** Get the event log */
    events: () => [...eventLog],

    /** Fold an incoming event into the cell */
    fold: (event: CellEvent) => {
      state = { ...state, ...event.facts }
      bindings.forEach(fn => fn(state))
    },
  }
}

describe('Server → Client (push)', () => {
  it('server write triggers client binding through event stream', () => {
    const serverCell = createCellWithBindings()
    const clientCell = createCellWithBindings()
    const clientRenders: string[] = []

    // Client binds a render function
    clientCell.bind((s) => {
      clientRenders.push(`render:${s.status}`)
    })

    // Server writes: Order created with status "In Cart"
    serverCell.put({ id: 'ord-1', status: 'In Cart' })
    const event = serverCell.emit('test', 'Order', 'ord-1', 'create', { id: 'ord-1', status: 'In Cart' })

    // Event streams to client, client folds it
    clientCell.fold(event)

    expect(clientCell.get().status).toBe('In Cart')
    expect(clientRenders).toEqual(['render:In Cart'])
  })

  it('server transition streams to client', () => {
    const serverCell = createCellWithBindings()
    const clientCell = createCellWithBindings()
    const clientStatuses: string[] = []

    clientCell.bind((s) => clientStatuses.push(String(s.status)))

    // Initial state
    serverCell.put({ status: 'In Cart' })
    clientCell.fold(serverCell.emit('test', 'Order', 'ord-1', 'create', { status: 'In Cart' }))

    // Server transitions: In Cart → Placed → Shipped
    serverCell.put({ status: 'Placed' })
    clientCell.fold(serverCell.emit('test', 'Order', 'ord-1', 'transition', { status: 'Placed' }))

    serverCell.put({ status: 'Shipped' })
    clientCell.fold(serverCell.emit('test', 'Order', 'ord-1', 'transition', { status: 'Shipped' }))

    expect(clientStatuses).toEqual(['In Cart', 'Placed', 'Shipped'])
    expect(serverCell.get().status).toBe('Shipped')
    expect(clientCell.get().status).toBe('Shipped')
  })
})

describe('Client → Server (optimistic + validate)', () => {
  it('client writes locally, sends event, server validates and accepts', () => {
    const clientCell = createCellWithBindings()
    const serverCell = createCellWithBindings()
    const clientRenders: string[] = []

    clientCell.bind((s) => clientRenders.push(`render:${s.customer}`))

    // Client writes optimistically (instant UI update)
    clientCell.put({ customer: 'acme' })
    const event = clientCell.emit('test', 'Order', 'ord-1', 'update', { customer: 'acme' })

    // Client sees the update immediately
    expect(clientRenders).toEqual(['render:acme'])

    // Server receives event, validates (no constraint violation), accepts
    serverCell.fold(event)
    expect(serverCell.get().customer).toBe('acme')
  })

  it('client writes optimistically, server rejects, client rolls back', () => {
    const clientCell = createCellWithBindings()
    const clientRenders: string[] = []

    clientCell.bind((s) => clientRenders.push(`render:${s.customer}`))

    // Client writes optimistically
    clientCell.put({ customer: 'acme' })
    expect(clientRenders).toEqual(['render:acme'])

    // Server rejects (e.g., UC violation from concurrent write)
    // Server sends authoritative state back
    const rollbackEvent: CellEvent = {
      domain: 'test', noun: 'Order', entityId: 'ord-1',
      operation: 'update',
      facts: { customer: 'beta' }, // server's authoritative state
      timestamp: Date.now(), sequence: 2,
    }
    clientCell.fold(rollbackEvent)

    // Client's optimistic state is overwritten by server's authoritative state
    expect(clientCell.get().customer).toBe('beta')
    expect(clientRenders).toEqual(['render:acme', 'render:beta'])
  })
})

describe('Two-way binding convergence', () => {
  it('slider widget on client A updates widget on client B through server', () => {
    const serverCell = createCellWithBindings()
    const clientA = createCellWithBindings()
    const clientB = createCellWithBindings()
    const displayValues: number[] = []

    // Client B has a display widget bound to the cell
    clientB.bind((s) => {
      if (s.value !== undefined) displayValues.push(Number(s.value))
    })

    // Client A moves a slider to 42
    clientA.put({ value: 42 })
    const event = clientA.emit('test', 'Slider', 'slider-1', 'update', { value: 42 })

    // Event goes to server
    serverCell.fold(event)

    // Server pushes to client B
    clientB.fold(event)

    expect(displayValues).toEqual([42])
    expect(serverCell.get().value).toBe(42)
    expect(clientB.get().value).toBe(42)
  })

  it('rapid updates from client A stream to client B in order', () => {
    const serverCell = createCellWithBindings()
    const clientA = createCellWithBindings()
    const clientB = createCellWithBindings()
    const bValues: number[] = []

    clientB.bind((s) => {
      if (s.value !== undefined) bValues.push(Number(s.value))
    })

    // Client A sends rapid slider updates
    for (let v = 0; v <= 100; v += 10) {
      clientA.put({ value: v })
      const event = clientA.emit('test', 'Slider', 'slider-1', 'update', { value: v })
      serverCell.fold(event)
      clientB.fold(event)
    }

    expect(bValues).toEqual([0, 10, 20, 30, 40, 50, 60, 70, 80, 90, 100])
    expect(serverCell.get().value).toBe(100)
    expect(clientA.get().value).toBe(100)
    expect(clientB.get().value).toBe(100)
  })

  it('event replay after client reconnect', () => {
    const serverCell = createCellWithBindings()
    const clientCell = createCellWithBindings()
    const renders: string[] = []

    clientCell.bind((s) => renders.push(String(s.status || '')))

    // Server processes 3 events while client is disconnected
    serverCell.put({ status: 'In Cart' })
    const e1 = serverCell.emit('test', 'Order', 'ord-1', 'create', { status: 'In Cart' })
    serverCell.put({ status: 'Placed' })
    const e2 = serverCell.emit('test', 'Order', 'ord-1', 'transition', { status: 'Placed' })
    serverCell.put({ status: 'Shipped' })
    const e3 = serverCell.emit('test', 'Order', 'ord-1', 'transition', { status: 'Shipped' })

    // Client reconnects and replays from last known sequence (0 = replay all)
    const allEvents = serverCell.events()
    const missedEvents = allEvents.filter(e => e.sequence > 0) // client missed everything

    for (const event of missedEvents) {
      clientCell.fold(event)
    }

    // Client converges to server state
    expect(clientCell.get().status).toBe('Shipped')
    expect(renders).toEqual(['In Cart', 'Placed', 'Shipped'])
  })
})

/**
 * Topology tests — client-server, sharded cluster, peer-to-peer.
 *
 * Three distinct models for cell replication:
 * 1. Client-Server: client untrusted, server authoritative
 * 2. Sharded Cluster: master-slave, RMAP partitions cells across shards
 * 3. Peer-to-Peer: no master, shared event ordering required
 */

import { describe, it, expect } from 'vitest'
import { type CellEvent } from './streaming'

/** Minimal cell that folds events in order. */
function createCell(initialState: Record<string, unknown> = {}) {
  let state = { ...initialState }
  let lastSequence = 0

  return {
    get: () => ({ ...state }),
    fold: (event: CellEvent) => {
      state = { ...state, ...event.facts }
      lastSequence = event.sequence
    },
    sequence: () => lastSequence,
  }
}

function event(seq: number, facts: Record<string, unknown>, op: CellEvent['operation'] = 'update'): CellEvent {
  return { domain: 'test', noun: 'Entity', entityId: 'e1', operation: op, facts, timestamp: Date.now(), sequence: seq }
}

// ── Sharded Cluster (master-slave) ──────────────────────────────────

describe('Sharded Cluster (master-slave)', () => {
  it('RMAP routes events to the owning shard', () => {
    // Two shards, each owns different cells
    const shardA = { owns: new Set(['Order', 'Customer']), cell: createCell() }
    const shardB = { owns: new Set(['Product', 'Invoice']), cell: createCell() }

    function route(noun: string) {
      if (shardA.owns.has(noun)) return shardA
      if (shardB.owns.has(noun)) return shardB
      throw new Error(`No shard for ${noun}`)
    }

    // Order event routes to shard A
    const orderEvent = event(1, { status: 'Placed' })
    orderEvent.noun = 'Order'
    route('Order').cell.fold(orderEvent)

    // Product event routes to shard B
    const productEvent = event(2, { name: 'Widget' })
    productEvent.noun = 'Product'
    route('Product').cell.fold(productEvent)

    expect(shardA.cell.get().status).toBe('Placed')
    expect(shardB.cell.get().name).toBe('Widget')

    // Shard A doesn't see product data
    expect(shardA.cell.get().name).toBeUndefined()
  })

  it('cross-shard reads use committed state (equation 15)', () => {
    const shardA = createCell()
    const shardB = createCell()

    // Shard A commits a customer
    shardA.fold(event(1, { customer: 'acme', revenue: 1000 }))

    // Shard B needs to read customer data for an invoice
    // It reads committed state from shard A (equation 15: P = union of FILE:D_n)
    const committedCustomer = shardA.get()
    const invoiceData = { customer: committedCustomer.customer, amount: 500 }
    shardB.fold(event(2, invoiceData))

    expect(shardB.get().customer).toBe('acme')
    expect(shardB.get().amount).toBe(500)
  })

  it('adding a shard scales horizontally without changing definitions', () => {
    // Start with one shard owning everything
    const shards = [
      { owns: new Set(['Order', 'Customer', 'Product']), cell: createCell() },
    ]

    // Route everything to shard 0
    shards[0].cell.fold({ ...event(1, { type: 'Order' }), noun: 'Order' })
    shards[0].cell.fold({ ...event(2, { type: 'Product' }), noun: 'Product' })

    // Add a second shard and redistribute
    shards.push({ owns: new Set(['Product']), cell: createCell() })
    shards[0].owns.delete('Product')

    // New product events route to shard 1
    function route(noun: string) {
      return shards.find(s => s.owns.has(noun))!
    }

    const newProductEvent = { ...event(3, { name: 'New Widget' }), noun: 'Product' }
    route('Product').cell.fold(newProductEvent)

    // Shard 1 has the product, shard 0 doesn't
    expect(shards[1].cell.get().name).toBe('New Widget')
    expect(shards[0].cell.get().name).toBeUndefined()
  })
})

// ── Peer-to-Peer ────────────────────────────────────────────────────

describe('Peer-to-Peer', () => {
  it('peers folding same events in same order converge', () => {
    const peerA = createCell()
    const peerB = createCell()

    const events = [
      event(1, { value: 10 }),
      event(2, { value: 20 }),
      event(3, { value: 30 }),
    ]

    // Both peers fold in the same order
    for (const e of events) {
      peerA.fold(e)
      peerB.fold(e)
    }

    expect(peerA.get()).toEqual(peerB.get())
    expect(peerA.get().value).toBe(30)
  })

  it('peers folding same events in different order may diverge', () => {
    const peerA = createCell()
    const peerB = createCell()

    // Two conflicting writes to the same field
    const e1 = event(1, { value: 'from-A' })
    const e2 = event(2, { value: 'from-B' })

    // Peer A sees e1 then e2
    peerA.fold(e1)
    peerA.fold(e2)

    // Peer B sees e2 then e1 (different order)
    peerB.fold(e2)
    peerB.fold(e1)

    // They diverge because foldl is order-dependent
    expect(peerA.get().value).toBe('from-B') // last write wins
    expect(peerB.get().value).toBe('from-A') // last write wins, but different last
    expect(peerA.get().value).not.toBe(peerB.get().value)
  })

  it('shared event log guarantees convergence (total order)', () => {
    const peerA = createCell()
    const peerB = createCell()

    // Both peers read from the same ordered log
    const log: CellEvent[] = [
      event(1, { x: 1 }),
      event(2, { y: 2 }),
      event(3, { x: 10 }), // overwrites x
    ]

    // Peer A reads the full log
    for (const e of log) peerA.fold(e)

    // Peer B reads the same log (maybe delayed, but same order)
    for (const e of log) peerB.fold(e)

    expect(peerA.get()).toEqual(peerB.get())
    expect(peerA.get().x).toBe(10)
    expect(peerA.get().y).toBe(2)
  })

  it('cell partitioning avoids conflicts without consensus', () => {
    // Each peer owns different cells. No shared writes.
    const peerA = createCell() // owns 'slider'
    const peerB = createCell() // owns 'display'

    // Peer A writes to its cell
    const sliderEvent = event(1, { value: 42 })
    peerA.fold(sliderEvent)

    // Peer B reads from peer A's committed state (cross-cell read)
    // and writes to its own cell
    const displayEvent = event(2, { displayValue: peerA.get().value })
    peerB.fold(displayEvent)

    expect(peerA.get().value).toBe(42)
    expect(peerB.get().displayValue).toBe(42)

    // No conflict because they own different cells
    // This is Definition 2 applied per-peer
  })

  it('Definition 2 violation produces inconsistent state', () => {
    // Two peers write to the SAME cell simultaneously
    // without coordination. This violates Definition 2.
    const cell = createCell({ counter: 0 })

    // Peer A reads counter=0, increments to 1
    const counterA = (cell.get().counter as number) + 1
    // Peer B reads counter=0 (stale!), increments to 1
    const counterB = (cell.get().counter as number) + 1

    // Both write "1" — one increment is lost
    cell.fold(event(1, { counter: counterA }))
    cell.fold(event(2, { counter: counterB }))

    // Counter should be 2 but it's 1. Definition 2 violation.
    expect(cell.get().counter).toBe(1)
    expect(cell.get().counter).not.toBe(2) // the lost update
  })
})

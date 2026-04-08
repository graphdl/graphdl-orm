/**
 * Consensus — Constraint-Based Peer Validation (Corollary 3)
 *
 * The constraint set C_S is the consensus predicate. Peers do not need an
 * external consensus protocol; they only need to share the same constraint
 * set and the same ordered event log. If both conditions hold, all peers
 * converge to the same state and agree on the validity of every proposed
 * event.
 */

import { describe, it, expect } from 'vitest'
import { CellChain, mergeChains } from '../../mcp/hash-chain'
import type { CellEvent } from '../../mcp/streaming'

// ── Inline constraint machinery ──────────────────────────────────────────────

type Constraint = (population: Map<string, unknown>, event: CellEvent) => string | null

function validate(
  population: Map<string, unknown>,
  event: CellEvent,
  constraints: Constraint[],
): { valid: boolean; violations: string[] } {
  const violations: string[] = []
  for (const constraint of constraints) {
    const result = constraint(population, event)
    if (result !== null) violations.push(result)
  }
  return { valid: violations.length === 0, violations }
}

function applyEvent(population: Map<string, unknown>, event: CellEvent): Map<string, unknown> {
  const next = new Map(population)
  if (event.operation === 'delete') {
    next.delete(event.entityId)
  } else {
    const existing = (next.get(event.entityId) as Record<string, unknown>) ?? {}
    next.set(event.entityId, { ...existing, ...event.facts, id: event.entityId })
  }
  return next
}

// UC constraint: "Each Order was placed by at most one Customer"
// If an order already has a customer and a different one is being assigned, reject.
const ucOrderHasOneCustomer: Constraint = (population, event) => {
  if (event.noun !== 'Order') return null
  const newCustomerId = event.facts['customerId']
  if (newCustomerId === undefined) return null

  const existing = population.get(event.entityId) as Record<string, unknown> | undefined
  if (!existing) return null
  if (existing['customerId'] === undefined) return null
  if (existing['customerId'] === newCustomerId) return null

  return (
    `Uniqueness constraint violated: Order ${event.entityId} was placed by Customer ` +
    `${String(existing['customerId'])} and cannot be reassigned to Customer ${String(newCustomerId)}.`
  )
}

const CONSTRAINTS: Constraint[] = [ucOrderHasOneCustomer]

// ── Helper: build a minimal CellEvent ────────────────────────────────────────

let seq = 0
function makeEvent(
  noun: string,
  entityId: string,
  operation: CellEvent['operation'],
  facts: Record<string, unknown>,
): CellEvent {
  return {
    domain: 'test',
    noun,
    entityId,
    operation,
    facts,
    timestamp: Date.now(),
    sequence: ++seq,
  }
}

// ── Two-peer consensus ────────────────────────────────────────────────────────

describe('Two-peer consensus', () => {
  it('Peer A proposes valid event, peer B validates with constraints → accepted', () => {
    // Population is empty; order O1 does not yet have a customer.
    const population = new Map<string, unknown>()
    const event = makeEvent('Order', 'O1', 'create', { customerId: 'C1' })

    const peerB = validate(population, event, CONSTRAINTS)

    expect(peerB.valid).toBe(true)
    expect(peerB.violations).toHaveLength(0)
  })

  it('Peer A proposes UC-violating event (second customer for same order), peer B rejects', () => {
    // Order O1 already has Customer C1 in the population.
    const population = new Map<string, unknown>([
      ['O1', { id: 'O1', customerId: 'C1' }],
    ])
    const event = makeEvent('Order', 'O1', 'update', { customerId: 'C2' })

    const peerB = validate(population, event, CONSTRAINTS)

    expect(peerB.valid).toBe(false)
    expect(peerB.violations).toHaveLength(1)
    expect(peerB.violations[0]).toMatch(/Uniqueness constraint violated/)
    expect(peerB.violations[0]).toMatch(/O1/)
    expect(peerB.violations[0]).toMatch(/C1/)
    expect(peerB.violations[0]).toMatch(/C2/)
  })

  it('Peers with identical populations and same constraint set always agree on validity', () => {
    const population = new Map<string, unknown>([
      ['O2', { id: 'O2', customerId: 'C10' }],
    ])

    // Valid event (adding a new order)
    const validEvent = makeEvent('Order', 'O3', 'create', { customerId: 'C11' })
    const peerA_valid = validate(population, validEvent, CONSTRAINTS)
    const peerB_valid = validate(population, validEvent, CONSTRAINTS)
    expect(peerA_valid.valid).toBe(peerB_valid.valid)
    expect(peerA_valid.violations).toEqual(peerB_valid.violations)

    // Invalid event (UC violation)
    const invalidEvent = makeEvent('Order', 'O2', 'update', { customerId: 'C99' })
    const peerA_invalid = validate(population, invalidEvent, CONSTRAINTS)
    const peerB_invalid = validate(population, invalidEvent, CONSTRAINTS)
    expect(peerA_invalid.valid).toBe(peerB_invalid.valid)
    expect(peerA_invalid.violations).toEqual(peerB_invalid.violations)
    expect(peerA_invalid.valid).toBe(false)
  })

  it('Sequence of events validated by constraint set alone — first two accepted, third (UC violation) rejected', () => {
    let population = new Map<string, unknown>()

    // Event 1: create order O4 with Customer C5
    const e1 = makeEvent('Order', 'O4', 'create', { customerId: 'C5' })
    const r1 = validate(population, e1, CONSTRAINTS)
    expect(r1.valid).toBe(true)
    population = applyEvent(population, e1)

    // Event 2: create order O5 with Customer C6
    const e2 = makeEvent('Order', 'O5', 'create', { customerId: 'C6' })
    const r2 = validate(population, e2, CONSTRAINTS)
    expect(r2.valid).toBe(true)
    population = applyEvent(population, e2)

    // Event 3: try to reassign O4 to Customer C7 — violates UC
    const e3 = makeEvent('Order', 'O4', 'update', { customerId: 'C7' })
    const r3 = validate(population, e3, CONSTRAINTS)
    expect(r3.valid).toBe(false)
    expect(r3.violations[0]).toMatch(/Uniqueness constraint violated/)
    // Population is unchanged — rejected events are not applied
    expect((population.get('O4') as Record<string, unknown>)['customerId']).toBe('C5')
  })
})

// ── n > 2 peers with hash chain ───────────────────────────────────────────────

describe('n > 2 peers with hash chain', () => {
  it('All peers folding same ordered log reach the same state', () => {
    const events: CellEvent[] = [
      makeEvent('Order', 'O10', 'create', { customerId: 'CA' }),
      makeEvent('Order', 'O11', 'create', { customerId: 'CB' }),
      makeEvent('Order', 'O12', 'create', { customerId: 'CC' }),
    ]

    // Three peers independently fold the same ordered log
    const peers = [new CellChain(), new CellChain(), new CellChain()]
    for (const event of events) {
      for (const peer of peers) {
        peer.append(event)
      }
    }

    // All peers must have identical tips (same state)
    const tips = peers.map(p => p.getTip())
    expect(tips[0]).toBe(tips[1])
    expect(tips[1]).toBe(tips[2])

    // All peers must have identical chain lengths
    const lengths = peers.map(p => p.length())
    expect(lengths[0]).toBe(3)
    expect(lengths[1]).toBe(3)
    expect(lengths[2]).toBe(3)

    // All chains verify as intact
    for (const peer of peers) {
      expect(peer.verify()).toBe(true)
    }
  })

  it('Fork detection identifies the divergence point', () => {
    const sharedEvent1 = makeEvent('Order', 'O20', 'create', { customerId: 'CX' })
    const sharedEvent2 = makeEvent('Order', 'O21', 'create', { customerId: 'CY' })

    const peerA = new CellChain()
    const peerB = new CellChain()

    // Both peers agree on the first two events
    peerA.append(sharedEvent1)
    peerB.append(sharedEvent1)
    peerA.append(sharedEvent2)
    peerB.append(sharedEvent2)

    // No fork yet — one is not ahead of the other
    const noFork = peerA.detectFork(peerB)
    expect(noFork).toBeNull()

    // Peers now diverge: each appends a different event at position 3
    const eventA = makeEvent('Order', 'O22A', 'create', { customerId: 'CZ_A' })
    const eventB = makeEvent('Order', 'O22B', 'create', { customerId: 'CZ_B' })
    peerA.append(eventA)
    peerB.append(eventB)

    // Fork is now detected
    const fork = peerA.detectFork(peerB)
    expect(fork).not.toBeNull()
    expect(fork!.forkAt).toBe(2) // diverge at index 2 (third event)
    expect(fork!.thisLength).toBe(3)
    expect(fork!.otherLength).toBe(3)
  })

  it('Longer chain wins on fork', () => {
    const sharedEvent1 = makeEvent('Order', 'O30', 'create', { customerId: 'CM' })

    const peerA = new CellChain()
    const peerB = new CellChain()

    peerA.append(sharedEvent1)
    peerB.append(sharedEvent1)

    // Divergence: peer A gets one extra event, peer B gets a conflicting one
    const eventA1 = makeEvent('Order', 'O31A', 'create', { customerId: 'CN_A' })
    const eventA2 = makeEvent('Order', 'O32A', 'create', { customerId: 'CO_A' })
    const eventB1 = makeEvent('Order', 'O31B', 'create', { customerId: 'CN_B' })

    peerA.append(eventA1)
    peerA.append(eventA2) // peerA is now length 3
    peerB.append(eventB1)  // peerB is length 2

    // peerA is longer → mergeChains returns peerA
    const winner = mergeChains(peerA, peerB)
    expect(winner.length()).toBe(3)
    expect(winner.getTip()).toBe(peerA.getTip())
    expect(winner.verify()).toBe(true)
  })
})

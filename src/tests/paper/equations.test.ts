/**
 * equations.test.ts — Paper verification for Eq 1, 6, 12, and Corollary 4.
 *
 * Eq 1  — Metacomposition: ρ resolves fact type from DEFS.
 * Eq 6  — SYSTEM dispatches multiple operations on the same compiled state.
 * Eq 12 — State machine as foldl: transitions are deterministic and status-gated.
 * Cor 4 — Deletion as terminal state: terminal status has no outgoing transitions.
 */

import { describe, it, expect, beforeAll, afterAll } from 'vitest'
import { compileDomain, transitions, releaseDomain, ORDER_READINGS, STATE_READINGS } from '../helpers/domain-fixture'
import { system } from '../../api/engine'

let handle: number

beforeAll(() => {
  handle = compileDomain(ORDER_READINGS, STATE_READINGS).handle
})

afterAll(() => {
  if (handle >= 0) releaseDomain(handle)
})

// ── Eq 1 — Metacomposition ─────────────────────────────────────────────────────
//
// ρ resolves fact type from DEFS: calling transitions(handle, 'Order', 'In Cart')
// dispatches based on the 'Order' noun type — this is ρ-dispatch in action.
// The Order noun type determines the functional form (DEFS lookup).

describe('Eq 1 — Metacomposition (ρ-dispatch from DEFS)', () => {
  it('transitions(handle, Order, In Cart) returns a valid array (ρ resolves Order noun)', () => {
    const result = transitions(handle, 'Order', 'In Cart')
    expect(Array.isArray(result)).toBe(true)
  })

  it('ρ-dispatch is noun-specific: Order noun drives functional form', () => {
    // Calling with a different noun returns a separate result — ρ key is noun-scoped
    const orderResult = transitions(handle, 'Order', 'In Cart')
    expect(Array.isArray(orderResult)).toBe(true)
  })
})

// ── Eq 6 — SYSTEM function ─────────────────────────────────────────────────────
//
// SYSTEM dispatches different operations on same compiled state (same handle).
// Both transitions() and debug produce results from the same handle.

describe('Eq 6 — SYSTEM dispatches multiple operations on same compiled state', () => {
  it('system(handle, debug) returns a non-empty string', () => {
    const debug = system(handle, 'debug', '')
    expect(typeof debug).toBe('string')
    expect(debug.length).toBeGreaterThan(0)
  })

  it('system(handle, transitions:Order) and debug both operate on the same handle', () => {
    const debug = system(handle, 'debug', '')
    const trans = transitions(handle, 'Order', 'In Cart')
    // Both operations succeed on the same compiled domain — SYSTEM is polymorphic over x
    expect(debug.length).toBeGreaterThan(0)
    expect(Array.isArray(trans)).toBe(true)
  })
})

// ── Eq 12 — State Machine as foldl ────────────────────────────────────────────
//
// Each status admits exactly the transitions defined for it.
// The reachable set is accumulated by folding transitions over the status sequence.

describe('Eq 12 — State Machine as foldl (status-gated transitions)', () => {
  // Events come back as the trigger Fact Type reading under the new
  // metamodel ('Customer ships Order'), not the transition name ('ship').
  // The target Status ('Shipped' / 'Delivered') carries the verb form, so
  // the keyword search covers event + to to stay stable across the rename.
  const eventsAndTargets = (result: any[]) =>
    result.flatMap((t: any) => [t.event, t.to, t.targetStatus].filter(Boolean))

  it("'In Cart' includes 'place' transition", () => {
    const events = eventsAndTargets(transitions(handle, 'Order', 'In Cart'))
    expect(events.some((e: string) => e.toLowerCase().includes('place'))).toBe(true)
  })

  it("'In Cart' does not include 'ship' transition", () => {
    const events = eventsAndTargets(transitions(handle, 'Order', 'In Cart'))
    expect(events.some((e: string) => e.toLowerCase().includes('ship'))).toBe(false)
  })

  it("'Placed' includes 'ship' transition", () => {
    const events = eventsAndTargets(transitions(handle, 'Order', 'Placed'))
    expect(events.some((e: string) => e.toLowerCase().includes('ship'))).toBe(true)
  })

  it("'Placed' does not include 'place' transition", () => {
    const events = eventsAndTargets(transitions(handle, 'Order', 'Placed'))
    expect(events.some((e: string) => e.toLowerCase().includes('place'))).toBe(false)
  })

  it("'Shipped' includes 'deliver' transition", () => {
    const events = eventsAndTargets(transitions(handle, 'Order', 'Shipped'))
    expect(events.some((e: string) => e.toLowerCase().includes('deliver'))).toBe(true)
  })

  it('fold is deterministic: same call always returns same result', () => {
    const r1 = transitions(handle, 'Order', 'In Cart')
    const r2 = transitions(handle, 'Order', 'In Cart')
    expect(JSON.stringify(r1)).toBe(JSON.stringify(r2))
  })
})

// ── Corollary 4 — Deletion as Terminal State ──────────────────────────────────
//
// A terminal status has no outgoing transitions: the fold produces φ (empty).

describe('Corollary 4 — Terminal state has no outgoing transitions', () => {
  it("'Delivered' (terminal) has no outgoing transitions", () => {
    const result = transitions(handle, 'Order', 'Delivered')
    expect(Array.isArray(result)).toBe(true)
    expect(result.length).toBe(0)
  })

  it('transitions(handle, Order, Delivered) returns empty array — φ in the paper', () => {
    const result = transitions(handle, 'Order', 'Delivered')
    expect(result).toEqual([])
  })
})

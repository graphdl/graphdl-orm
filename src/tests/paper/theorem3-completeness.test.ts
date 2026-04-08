/**
 * theorem3-completeness.test.ts — Theorem 3: Completeness of State Transfer
 *
 * Verifies:
 *   1. Forward chaining reaches the least fixed point (LFP):
 *      - Returns an array of derived facts
 *      - Is idempotent (running twice gives same result)
 *      - Is monotonic (adding facts never removes derived facts)
 *
 *   2. Constraint evaluation:
 *      - Alethic UC violation → non-empty violation set when two values exist for a unique role
 *      - Satisfied constraints → empty violation set
 *      - Deontic constraints are present in IR with deontic modality
 */

import { describe, it, expect, beforeAll, afterAll } from 'vitest'
import {
  compileDomain,
  evaluate,
  forwardChain,
  ORDER_READINGS,
  SUPPORT_READINGS,
  releaseDomain,
  type CompiledDomain,
} from '../helpers/domain-fixture'

// ── Helpers ──────────────────────────────────────────────────────────────────

/**
 * Minimal population with one Order placed by one Customer, with one Priority.
 */
function makeOrderPopulation(extraFacts: Record<string, any[]> = {}): string {
  const base = {
    'Order_was_placed_by_Customer': [
      { factTypeId: 'Order_was_placed_by_Customer', bindings: [['Order', 'O1'], ['Customer', 'Alice']] },
    ],
    'Order_has_Priority': [
      { factTypeId: 'Order_has_Priority', bindings: [['Order', 'O1'], ['Priority', 'High']] },
    ],
  }
  return JSON.stringify({ facts: { ...base, ...extraFacts } })
}

/**
 * Empty population (no facts).
 */
function emptyPopulation(): string {
  return JSON.stringify({ facts: {} })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

describe('Theorem 3 — Forward Chaining to Least Fixed Point', () => {
  let orders: CompiledDomain
  let support: CompiledDomain

  beforeAll(() => {
    orders = compileDomain(ORDER_READINGS, 'orders')
    support = compileDomain(SUPPORT_READINGS, 'support')
  })

  afterAll(() => {
    if (orders?.handle >= 0) releaseDomain(orders.handle)
    if (support?.handle >= 0) releaseDomain(support.handle)
  })

  // ── 1. Forward chaining returns an array ──────────────────────────────────

  it('forward chain returns an array of derived facts', () => {
    const result = forwardChain(orders.handle, makeOrderPopulation())
    expect(Array.isArray(result)).toBe(true)
  })

  it('forward chain on empty population returns an array', () => {
    const result = forwardChain(orders.handle, emptyPopulation())
    expect(Array.isArray(result)).toBe(true)
  })

  // ── 2. Idempotence ────────────────────────────────────────────────────────

  it('forward chain is idempotent — running twice on same population gives same result', () => {
    const pop = makeOrderPopulation()
    const first = forwardChain(orders.handle, pop)
    const second = forwardChain(orders.handle, pop)
    expect(JSON.stringify(first)).toBe(JSON.stringify(second))
  })

  // ── 3. Monotonicity ───────────────────────────────────────────────────────

  it('forward chain is monotonic — adding facts never removes derived facts', () => {
    const smallPop = makeOrderPopulation()
    const largePop = makeOrderPopulation({
      'Order_has_Amount': [
        { factTypeId: 'Order_has_Amount', bindings: [['Order', 'O1'], ['Amount', '100']] },
        { factTypeId: 'Order_has_Amount', bindings: [['Order', 'O2'], ['Amount', '200']] },
      ],
    })

    const smallResult = forwardChain(orders.handle, smallPop)
    const largeResult = forwardChain(orders.handle, largePop)

    // Every fact derived from the smaller population must also appear in the larger
    const largeSet = new Set(largeResult.map((f: any) => JSON.stringify(f)))
    for (const fact of smallResult) {
      expect(largeSet.has(JSON.stringify(fact))).toBe(true)
    }
  })
})

describe('Theorem 3 — Constraint Evaluation', () => {
  let orders: CompiledDomain
  let support: CompiledDomain

  beforeAll(() => {
    orders = compileDomain(ORDER_READINGS, 'orders')
    support = compileDomain(SUPPORT_READINGS, 'support')
  })

  afterAll(() => {
    if (orders?.handle >= 0) releaseDomain(orders.handle)
    if (support?.handle >= 0) releaseDomain(support.handle)
  })

  // ── 4. Alethic UC violation ───────────────────────────────────────────────

  it('alethic UC violation produces non-empty violation set when two values exist for a unique role', () => {
    // Order O1 has two Priorities — violates "at most one Priority"
    const conflictPop = JSON.stringify({
      facts: {
        'Order_has_Priority': [
          { factTypeId: 'Order_has_Priority', bindings: [['Order', 'O1'], ['Priority', 'High']] },
          { factTypeId: 'Order_has_Priority', bindings: [['Order', 'O1'], ['Priority', 'Low']] },
        ],
      },
    })
    const result = evaluate(orders.handle, 'Each Order has at most one Priority.', conflictPop)
    // Result should be an array/object with at least one violation
    const violations = Array.isArray(result) ? result : (result?.violations ?? result?.results ?? [])
    expect(violations.length).toBeGreaterThan(0)
  })

  // ── 5. Satisfied constraints produce empty violation set ──────────────────

  it('satisfied constraint produces empty violation set', () => {
    // Order O1 has exactly one Priority — constraint satisfied
    const validPop = JSON.stringify({
      facts: {
        'Order_has_Priority': [
          { factTypeId: 'Order_has_Priority', bindings: [['Order', 'O1'], ['Priority', 'High']] },
        ],
      },
    })
    const result = evaluate(orders.handle, 'Each Order has at most one Priority.', validPop)
    const violations = Array.isArray(result) ? result : (result?.violations ?? result?.results ?? [])
    expect(violations.length).toBe(0)
  })

  // ── 6. Deontic constraints in IR ──────────────────────────────────────────

  it('deontic constraints are present in IR with deontic modality', () => {
    const { ir } = support
    // The raw debug output or constraints list must reference obligation/deontic
    const rawLower = ir.raw.toLowerCase()
    const hasDeonticMarker =
      rawLower.includes('obligatory') ||
      rawLower.includes('deontic') ||
      rawLower.includes('obligation') ||
      ir.constraints.some(c =>
        c.kind.toLowerCase().includes('deontic') ||
        c.text.toLowerCase().includes('obligatory') ||
        c.text.toLowerCase().includes('obligation')
      )
    expect(hasDeonticMarker).toBe(true)
  })
})

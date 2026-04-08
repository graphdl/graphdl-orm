/**
 * sharding.test.ts — Equations 14-16: Sharded Evaluation with RMAP Demux and Per-Cell Folds
 *
 * Pure TypeScript — no WASM dependency, no engine imports.
 * Simulates cells, events, RMAP routing, and foldl transitions to verify
 * algebraic claims from the AREST paper about distributed evaluation.
 *
 * Equation 14 — RMAP routes events to owning shard (cell ID = entity ID)
 * Equation 15 — Per-cell folds are independent and deterministic
 * Equation 16 — Cross-cell queries read committed (union) state
 */

import { describe, it, expect } from 'vitest'

// ── Types ─────────────────────────────────────────────────────────────────────

interface Event {
  type: string
  entityId: string
  data: Record<string, unknown>
  timestamp: number
}

interface Cell {
  id: string
  noun: string
  data: Record<string, unknown>
  status: string
  events: Event[]
}

type TransitionMap = Record<string, Record<string, string>>

// ── Helpers ───────────────────────────────────────────────────────────────────

function createCell(id: string, noun: string): Cell {
  return {
    id,
    noun,
    data: {},
    status: 'In Cart',
    events: [],
  }
}

/**
 * applyTransition: (s, e) → s' with guard checking valid transitions.
 * Returns the same cell (unchanged) if the transition is invalid.
 */
function applyTransition(
  cell: Cell,
  event: Event,
  validTransitions: TransitionMap,
): Cell {
  const allowed = validTransitions[cell.status]
  if (!allowed) return cell
  const nextStatus = allowed[event.type]
  if (!nextStatus) return cell  // guard: invalid transition, no state change
  return {
    ...cell,
    status: nextStatus,
    data: { ...cell.data, ...event.data },
    events: [...cell.events, event],
  }
}

/**
 * rmapRoute: route event to owning cell (Equation 14).
 * Cell ID is the entity ID — each entity owns exactly one cell.
 */
function rmapRoute(event: Event): string {
  return event.entityId
}

/**
 * foldEvents: apply events to cell in order (foldl), Equation 15.
 * Each cell folds only its own event stream.
 */
function foldEvents(
  cell: Cell,
  events: Event[],
  getValidTransitions: () => TransitionMap,
): Cell {
  return events.reduce(
    (currentCell, event) => applyTransition(currentCell, event, getValidTransitions()),
    cell,
  )
}

// ── Transition configuration ──────────────────────────────────────────────────

const ORDER_TRANSITIONS: TransitionMap = {
  'In Cart':  { place:   'Placed'    },
  'Placed':   { ship:    'Shipped'   },
  'Shipped':  { deliver: 'Delivered' },
  'Delivered': {},
}

function getOrderTransitions(): TransitionMap {
  return ORDER_TRANSITIONS
}

// ── Equation 14: RMAP demux ───────────────────────────────────────────────────

describe('Equation 14 — RMAP routes events to owning shard', () => {
  it('events for different entities route to different cells', () => {
    const events: Event[] = [
      { type: 'place', entityId: 'order-1', data: {}, timestamp: 1 },
      { type: 'place', entityId: 'order-2', data: {}, timestamp: 2 },
      { type: 'ship',  entityId: 'order-1', data: {}, timestamp: 3 },
    ]

    const routed = new Map<string, Event[]>()
    for (const event of events) {
      const cellId = rmapRoute(event)
      if (!routed.has(cellId)) routed.set(cellId, [])
      routed.get(cellId)!.push(event)
    }

    expect(routed.has('order-1')).toBe(true)
    expect(routed.has('order-2')).toBe(true)
    expect(routed.get('order-1')).toHaveLength(2)
    expect(routed.get('order-2')).toHaveLength(1)
  })

  it('multiple events for same entity route to the same cell', () => {
    const events: Event[] = [
      { type: 'place',   entityId: 'order-42', data: {}, timestamp: 1 },
      { type: 'ship',    entityId: 'order-42', data: {}, timestamp: 2 },
      { type: 'deliver', entityId: 'order-42', data: {}, timestamp: 3 },
    ]

    const cellIds = events.map(rmapRoute)
    const uniqueCells = new Set(cellIds)
    expect(uniqueCells.size).toBe(1)
    expect([...uniqueCells][0]).toBe('order-42')
  })

  it('rmapRoute is deterministic — same event always routes to same cell', () => {
    const event: Event = { type: 'place', entityId: 'order-99', data: {}, timestamp: 1 }
    expect(rmapRoute(event)).toBe(rmapRoute(event))
    expect(rmapRoute(event)).toBe('order-99')
  })

  it('events with different entityIds never share a cell', () => {
    const events: Event[] = [
      { type: 'place', entityId: 'A', data: {}, timestamp: 1 },
      { type: 'place', entityId: 'B', data: {}, timestamp: 2 },
      { type: 'place', entityId: 'C', data: {}, timestamp: 3 },
    ]
    const cellIds = events.map(rmapRoute)
    const unique = new Set(cellIds)
    expect(unique.size).toBe(3)
  })
})

// ── Equation 15: Per-cell folds are independent and deterministic ─────────────

describe('Equation 15 — Per-cell folds are independent and deterministic', () => {
  it('each cell folds its own event stream without reading other cells', () => {
    const cell1 = createCell('order-1', 'Order')
    const cell2 = createCell('order-2', 'Order')

    const events1: Event[] = [
      { type: 'place', entityId: 'order-1', data: { placed: true }, timestamp: 1 },
      { type: 'ship',  entityId: 'order-1', data: { shipped: true }, timestamp: 2 },
    ]
    const events2: Event[] = [
      { type: 'place', entityId: 'order-2', data: { placed: true }, timestamp: 3 },
    ]

    const result1 = foldEvents(cell1, events1, getOrderTransitions)
    const result2 = foldEvents(cell2, events2, getOrderTransitions)

    expect(result1.status).toBe('Shipped')
    expect(result2.status).toBe('Placed')
    // Each cell is independent — result1's state is unaffected by result2
    expect(result1.id).toBe('order-1')
    expect(result2.id).toBe('order-2')
  })

  it('cell fold is pure — same events always produce same state (deterministic)', () => {
    const events: Event[] = [
      { type: 'place', entityId: 'order-5', data: { qty: 3 }, timestamp: 10 },
      { type: 'ship',  entityId: 'order-5', data: { carrier: 'UPS' }, timestamp: 20 },
    ]

    const resultA = foldEvents(createCell('order-5', 'Order'), events, getOrderTransitions)
    const resultB = foldEvents(createCell('order-5', 'Order'), events, getOrderTransitions)

    expect(resultA.status).toBe(resultB.status)
    expect(resultA.data).toEqual(resultB.data)
    expect(resultA.events.length).toBe(resultB.events.length)
  })

  it('folding an empty event stream leaves cell in initial state', () => {
    const cell = createCell('order-7', 'Order')
    const result = foldEvents(cell, [], getOrderTransitions)
    expect(result.status).toBe('In Cart')
    expect(result.events).toHaveLength(0)
  })

  it('foldl applies events in order — out-of-order application produces different result', () => {
    const events: Event[] = [
      { type: 'place', entityId: 'order-8', data: {}, timestamp: 1 },
      { type: 'ship',  entityId: 'order-8', data: {}, timestamp: 2 },
    ]
    const reversed: Event[] = [...events].reverse()

    const forwardResult = foldEvents(createCell('order-8', 'Order'), events, getOrderTransitions)
    const reverseResult = foldEvents(createCell('order-8', 'Order'), reversed, getOrderTransitions)

    // Forward: In Cart → place → Placed → ship → Shipped
    expect(forwardResult.status).toBe('Shipped')
    // Reversed: In Cart → ship (invalid from In Cart) → place → Placed
    expect(reverseResult.status).toBe('Placed')
    expect(forwardResult.status).not.toBe(reverseResult.status)
  })

  it('complete lifecycle fold produces terminal state', () => {
    const cell = createCell('order-full', 'Order')
    const events: Event[] = [
      { type: 'place',   entityId: 'order-full', data: {}, timestamp: 1 },
      { type: 'ship',    entityId: 'order-full', data: {}, timestamp: 2 },
      { type: 'deliver', entityId: 'order-full', data: {}, timestamp: 3 },
    ]
    const result = foldEvents(cell, events, getOrderTransitions)
    expect(result.status).toBe('Delivered')
    expect(result.events).toHaveLength(3)
  })
})

// ── Equation 16: Cross-cell queries read committed state ──────────────────────

describe('Equation 16 — Cross-cell queries read committed state', () => {
  /**
   * Simulate a committed population P = union of all committed cell states.
   */
  function buildPopulation(cells: Cell[]): Map<string, Cell> {
    const population = new Map<string, Cell>()
    for (const cell of cells) {
      population.set(cell.id, cell)
    }
    return population
  }

  it('population P is the union of all committed cell states', () => {
    const cell1 = foldEvents(
      createCell('order-1', 'Order'),
      [{ type: 'place', entityId: 'order-1', data: {}, timestamp: 1 }],
      getOrderTransitions,
    )
    const cell2 = foldEvents(
      createCell('order-2', 'Order'),
      [{ type: 'place', entityId: 'order-2', data: {}, timestamp: 2 },
       { type: 'ship',  entityId: 'order-2', data: {}, timestamp: 3 }],
      getOrderTransitions,
    )

    const population = buildPopulation([cell1, cell2])

    expect(population.size).toBe(2)
    expect(population.get('order-1')?.status).toBe('Placed')
    expect(population.get('order-2')?.status).toBe('Shipped')
  })

  it('cross-cell read sees committed state from all cells', () => {
    const cells = ['order-A', 'order-B', 'order-C'].map(id =>
      foldEvents(
        createCell(id, 'Order'),
        [{ type: 'place', entityId: id, data: {}, timestamp: 1 }],
        getOrderTransitions,
      )
    )

    const population = buildPopulation(cells)

    // Cross-cell query: count all Placed orders across all cells
    let placedCount = 0
    for (const cell of population.values()) {
      if (cell.status === 'Placed') placedCount++
    }

    expect(placedCount).toBe(3)
  })

  it('cross-cell read sees committed state, not in-progress mutations', () => {
    // Committed population is a snapshot at a point in time
    const committed = buildPopulation([
      foldEvents(
        createCell('order-X', 'Order'),
        [{ type: 'place', entityId: 'order-X', data: {}, timestamp: 1 }],
        getOrderTransitions,
      ),
    ])

    // In-progress mutation (not yet committed)
    const inProgress = applyTransition(
      committed.get('order-X')!,
      { type: 'ship', entityId: 'order-X', data: {}, timestamp: 2 },
      getOrderTransitions(),
    )

    // Cross-cell query reads from committed population, not in-progress
    expect(committed.get('order-X')?.status).toBe('Placed')
    // The in-progress mutation is separate and not yet visible
    expect(inProgress.status).toBe('Shipped')
    expect(committed.get('order-X')?.status).toBe('Placed')
  })

  it('each cell contributes exactly once to the population union', () => {
    const cell = foldEvents(
      createCell('order-only', 'Order'),
      [{ type: 'place', entityId: 'order-only', data: {}, timestamp: 1 }],
      getOrderTransitions,
    )

    // Adding the same cell twice does not duplicate it in the population
    const population = buildPopulation([cell, cell])
    expect(population.size).toBe(1)
  })
})

// ── Horizontal scaling ────────────────────────────────────────────────────────

describe('Horizontal scaling — adding shards preserves existing cell state', () => {
  it('adding a new cell does not change any existing cell state', () => {
    const existingCell = foldEvents(
      createCell('order-1', 'Order'),
      [{ type: 'place', entityId: 'order-1', data: {}, timestamp: 1 }],
      getOrderTransitions,
    )

    const populationBefore = buildPopulationSnapshot([existingCell])

    // A new shard (cell) is added for a different entity
    const newCell = foldEvents(
      createCell('order-new', 'Order'),
      [{ type: 'place', entityId: 'order-new', data: {}, timestamp: 2 }],
      getOrderTransitions,
    )

    const populationAfter = buildPopulationSnapshot([existingCell, newCell])

    // Existing cell state is unchanged
    expect(populationAfter.get('order-1')?.status).toBe(populationBefore.get('order-1')?.status)
    expect(populationAfter.get('order-1')?.events.length).toBe(
      populationBefore.get('order-1')?.events.length,
    )
    // New cell is present
    expect(populationAfter.size).toBe(2)
  })

  it('invalid transitions are rejected without changing state (guard)', () => {
    const cell = createCell('order-guarded', 'Order')

    // Try to ship without placing first — invalid from 'In Cart'
    const afterInvalid = applyTransition(
      cell,
      { type: 'ship', entityId: 'order-guarded', data: {}, timestamp: 1 },
      getOrderTransitions(),
    )

    expect(afterInvalid.status).toBe('In Cart')
    expect(afterInvalid.events).toHaveLength(0)
  })

  it('invalid transition from terminal state is rejected', () => {
    const cell = foldEvents(
      createCell('order-done', 'Order'),
      [
        { type: 'place',   entityId: 'order-done', data: {}, timestamp: 1 },
        { type: 'ship',    entityId: 'order-done', data: {}, timestamp: 2 },
        { type: 'deliver', entityId: 'order-done', data: {}, timestamp: 3 },
      ],
      getOrderTransitions,
    )

    expect(cell.status).toBe('Delivered')

    // Attempt to re-ship a delivered order — not in transition map
    const afterInvalid = applyTransition(
      cell,
      { type: 'ship', entityId: 'order-done', data: {}, timestamp: 4 },
      getOrderTransitions(),
    )

    expect(afterInvalid.status).toBe('Delivered')
    expect(afterInvalid.events).toHaveLength(3)
  })

  it('N independent shards each fold independently without interference', () => {
    const shardCount = 5
    const shards = Array.from({ length: shardCount }, (_, i) => {
      const id = `shard-order-${i}`
      const eventCount = i + 1  // each shard gets a different number of events
      const events: Event[] = []
      const sequence = ['place', 'ship', 'deliver']
      for (let j = 0; j < Math.min(eventCount, sequence.length); j++) {
        events.push({ type: sequence[j], entityId: id, data: {}, timestamp: j + 1 })
      }
      return foldEvents(createCell(id, 'Order'), events, getOrderTransitions)
    })

    const expectedStatuses = ['Placed', 'Shipped', 'Delivered', 'Delivered', 'Delivered']
    shards.forEach((shard, i) => {
      expect(shard.status).toBe(expectedStatuses[i])
    })

    // No shard shares state with another
    const ids = shards.map(s => s.id)
    expect(new Set(ids).size).toBe(shardCount)
  })
})

// ── Local helper (used in horizontal scaling tests) ───────────────────────────

function buildPopulationSnapshot(cells: Cell[]): Map<string, Cell> {
  const m = new Map<string, Cell>()
  for (const c of cells) m.set(c.id, c)
  return m
}

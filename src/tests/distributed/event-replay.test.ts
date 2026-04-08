// Event Replay — Deterministic State Reconstruction via pure foldl
// Pure logic tests — no WASM, no engine imports.

interface Event {
  type: string
  data: Record<string, unknown>
  timestamp: number
}

interface CellState {
  status: string
  data: Record<string, unknown>
}

const transitionMap: Record<string, Record<string, string>> = {
  'In Cart': { place: 'Placed' },
  'Placed': { ship: 'Shipped' },
  'Shipped': { deliver: 'Delivered' },
  'Delivered': {},
}

function transition(state: CellState, event: Event): CellState {
  const targets = transitionMap[state.status] || {}
  const nextStatus = targets[event.type]
  if (!nextStatus) return state // guard fails
  return { status: nextStatus, data: { ...state.data, ...event.data } }
}

function replay(events: Event[]): CellState {
  return events.reduce(transition, { status: 'In Cart', data: {} })
}

// ---------------------------------------------------------------------------

describe('Event Replay — deterministic state reconstruction', () => {
  const placeEvent: Event = { type: 'place', data: { orderId: 'ord-1' }, timestamp: 1 }
  const shipEvent: Event = { type: 'ship', data: { trackingId: 'trk-99' }, timestamp: 2 }
  const deliverEvent: Event = { type: 'deliver', data: { deliveredAt: '2026-04-08' }, timestamp: 3 }

  it('replaying the same events always produces the same final state (In Cart → place → ship → deliver → Delivered)', () => {
    const events = [placeEvent, shipEvent, deliverEvent]
    const stateA = replay(events)
    const stateB = replay(events)

    expect(stateA).toEqual(stateB)
    expect(stateA.status).toBe('Delivered')
    expect(stateA.data).toMatchObject({
      orderId: 'ord-1',
      trackingId: 'trk-99',
      deliveredAt: '2026-04-08',
    })
  })

  it('partial replay produces intermediate state (just place → Placed)', () => {
    const state = replay([placeEvent])

    expect(state.status).toBe('Placed')
    expect(state.data).toMatchObject({ orderId: 'ord-1' })
    expect(state.data).not.toHaveProperty('trackingId')
  })

  it('replay after disconnect: catching up from scratch produces same state as continuous', () => {
    // Continuous processing
    const continuous = replay([placeEvent, shipEvent, deliverEvent])

    // Simulate disconnect after placeEvent, then catch up from scratch
    const fromScratch = replay([placeEvent, shipEvent, deliverEvent])

    expect(fromScratch).toEqual(continuous)
    expect(fromScratch.status).toBe('Delivered')
  })

  it('replay after disconnect: catching up from checkpoint (resume from Shipped, apply deliver)', () => {
    // Checkpoint: already at Shipped after processing place + ship
    const checkpoint: CellState = replay([placeEvent, shipEvent])
    expect(checkpoint.status).toBe('Shipped')

    // Resume: apply only the remaining event
    const resumed = transition(checkpoint, deliverEvent)

    // Full replay from scratch
    const full = replay([placeEvent, shipEvent, deliverEvent])

    expect(resumed).toEqual(full)
    expect(resumed.status).toBe('Delivered')
  })

  it('invalid events in stream are no-ops (ship before place is rejected by guard)', () => {
    // ship is only valid from Placed; applying it from In Cart must be a no-op
    const state = replay([shipEvent])

    expect(state.status).toBe('In Cart')
    expect(state.data).toEqual({})
  })

  it('event ordering matters: ship then place ≠ place then ship (different final states)', () => {
    const placeThenShip = replay([placeEvent, shipEvent])
    const shipThenPlace = replay([shipEvent, placeEvent])

    // place then ship: In Cart → Placed → Shipped
    expect(placeThenShip.status).toBe('Shipped')

    // ship then place: ship is rejected (guard fails from In Cart), then place succeeds → Placed
    expect(shipThenPlace.status).toBe('Placed')

    expect(placeThenShip.status).not.toBe(shipThenPlace.status)
  })

  it('empty event stream produces initial state (In Cart, empty data)', () => {
    const state = replay([])

    expect(state.status).toBe('In Cart')
    expect(state.data).toEqual({})
  })
})

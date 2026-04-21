import { describe, expect, it } from 'vitest'
import { createMachine } from 'xstate'
import {
  arestToXStateConfig,
  describeStatuses,
  listStatuses,
  type ArestStateMachineDefinition,
  type ArestTransition,
} from '../xstateConfig'

const orderSMD: ArestStateMachineDefinition = {
  id: 'Order',
  noun: 'Order',
  initial: 'In Cart',
}

const orderTransitions: ArestTransition[] = [
  { id: 'place',   from: 'In Cart', to: 'Placed'    },
  { id: 'ship',    from: 'Placed',  to: 'Shipped'   },
  { id: 'deliver', from: 'Shipped', to: 'Delivered' },
  { id: 'cancel',  from: 'Placed',  to: 'Cancelled', event: 'cancel-order' },
]

describe('arestToXStateConfig', () => {
  it('builds an xstate config with every mentioned status as a state', () => {
    const cfg = arestToXStateConfig(orderSMD, orderTransitions)
    expect(cfg.id).toBe('Order')
    expect(cfg.initial).toBe('In Cart')
    expect(Object.keys(cfg.states).sort()).toEqual([
      'Cancelled', 'Delivered', 'In Cart', 'Placed', 'Shipped',
    ])
  })

  it('maps each transition onto the source state\'s `on` block by event', () => {
    const cfg = arestToXStateConfig(orderSMD, orderTransitions)
    expect(cfg.states['In Cart'].on).toEqual({ place: 'Placed' })
    // Explicit event overrides the transition id.
    expect(cfg.states['Placed'].on).toEqual({ ship: 'Shipped', 'cancel-order': 'Cancelled' })
    expect(cfg.states['Shipped'].on).toEqual({ deliver: 'Delivered' })
  })

  it('marks states with no outgoing transitions as final', () => {
    const cfg = arestToXStateConfig(orderSMD, orderTransitions)
    expect(cfg.states['Delivered'].type).toBe('final')
    expect(cfg.states['Cancelled'].type).toBe('final')
    expect(cfg.states['Placed'].type).toBeUndefined()
  })

  it('includes the initial status even when it has no outgoing transitions', () => {
    const cfg = arestToXStateConfig({ id: 'X', noun: 'X', initial: 'Start' }, [])
    expect(cfg.states['Start']).toEqual({ type: 'final' })
    expect(cfg.initial).toBe('Start')
  })

  it('produces a config that xstate 5 accepts via createMachine', () => {
    const cfg = arestToXStateConfig(orderSMD, orderTransitions)
    // createMachine throws on malformed configs (e.g. unknown target
    // state in an `on` block). This asserts that arestToXStateConfig
    // produces a valid machine whenever the facts are consistent.
    expect(() => createMachine(cfg)).not.toThrow()
  })
})

describe('listStatuses / describeStatuses', () => {
  it('listStatuses returns every status name sorted, deduped', () => {
    expect(listStatuses(orderSMD, orderTransitions)).toEqual([
      'Cancelled', 'Delivered', 'In Cart', 'Placed', 'Shipped',
    ])
  })

  it('describeStatuses classifies initial / terminal per AREST derivation rules', () => {
    const infos = describeStatuses(orderSMD, orderTransitions)
    const byName = Object.fromEntries(infos.map((i) => [i.name, i]))
    expect(byName['In Cart'].isInitial).toBe(true)
    expect(byName['In Cart'].isTerminal).toBe(false)
    expect(byName['Delivered'].isTerminal).toBe(true)
    expect(byName['Cancelled'].isTerminal).toBe(true)
    expect(byName['Placed'].isTerminal).toBe(false)
    expect(byName['Placed'].outgoing.map((t) => t.id).sort()).toEqual(['cancel', 'ship'])
  })
})

import { describe, it, expect } from 'vitest'
import { AppModel } from './app-model'
import type { NounDef, FactTypeDef, ConstraintDef, SpanDef, StateMachineDef } from './types'

/** Minimal DomainModel stub for testing. */
function stubDomain(
  domainId: string,
  data: {
    nouns?: Map<string, NounDef>
    factTypes?: Map<string, FactTypeDef>
    constraints?: ConstraintDef[]
    constraintSpans?: Map<string, SpanDef[]>
    stateMachines?: StateMachineDef[]
  },
) {
  return {
    domainId,
    nouns: async () => data.nouns || new Map(),
    factTypes: async () => data.factTypes || new Map(),
    constraints: async () => data.constraints || [],
    constraintSpans: async () => data.constraintSpans || new Map(),
    stateMachines: async () => data.stateMachines || [],
  } as any
}

function noun(name: string, domainId: string, objectType: 'entity' | 'value' = 'entity'): NounDef {
  return { id: name, name, domainId, objectType }
}

function ft(id: string, reading: string, roles: Array<{ nounName: string; roleIndex: number }>): FactTypeDef {
  return {
    id,
    reading,
    arity: roles.length,
    roles: roles.map(r => ({ id: `${id}-r${r.roleIndex}`, nounDef: {} as any, ...r })),
  }
}

describe('AppModel', () => {
  it('merges nouns from multiple domains', async () => {
    const d1 = stubDomain('vehicle-data', {
      nouns: new Map([
        ['Vehicle', noun('Vehicle', 'vehicle-data')],
        ['Make', noun('Make', 'vehicle-data')],
      ]),
    })
    const d2 = stubDomain('listings', {
      nouns: new Map([
        ['Listing', noun('Listing', 'listings')],
        ['Dealer', noun('Dealer', 'listings')],
      ]),
    })

    const app = new AppModel('auto-dev', [d1, d2])
    const nouns = await app.nouns()

    expect(nouns.size).toBe(4)
    expect(nouns.has('Vehicle')).toBe(true)
    expect(nouns.has('Listing')).toBe(true)
  })

  it('merges fact types from multiple domains', async () => {
    const d1 = stubDomain('d1', {
      factTypes: new Map([
        ['ft1', ft('ft1', 'Vehicle has Make', [{ nounName: 'Vehicle', roleIndex: 0 }, { nounName: 'Make', roleIndex: 1 }])],
      ]),
    })
    const d2 = stubDomain('d2', {
      factTypes: new Map([
        ['ft2', ft('ft2', 'Listing has Price', [{ nounName: 'Listing', roleIndex: 0 }, { nounName: 'Price', roleIndex: 1 }])],
      ]),
    })

    const app = new AppModel('app', [d1, d2])
    const fts = await app.factTypes()

    expect(fts.size).toBe(2)
    expect(fts.has('ft1')).toBe(true)
    expect(fts.has('ft2')).toBe(true)
  })

  it('concatenates constraints from all domains', async () => {
    const c1: ConstraintDef = { id: 'c1', kind: 'UC', modality: 'Alethic', text: 'Each Vehicle has at most one VIN', spans: [] }
    const c2: ConstraintDef = { id: 'c2', kind: 'MC', modality: 'Alethic', text: 'Each Listing has exactly one Price', spans: [] }

    const d1 = stubDomain('d1', { constraints: [c1] })
    const d2 = stubDomain('d2', { constraints: [c2] })

    const app = new AppModel('app', [d1, d2])
    const constraints = await app.constraints()

    expect(constraints).toHaveLength(2)
    expect(constraints[0].id).toBe('c1')
    expect(constraints[1].id).toBe('c2')
  })

  it('merges constraint spans across domains', async () => {
    const d1 = stubDomain('d1', {
      constraintSpans: new Map([
        ['c1', [{ factTypeId: 'ft1', roleIndex: 0 }]],
      ]),
    })
    const d2 = stubDomain('d2', {
      constraintSpans: new Map([
        ['c1', [{ factTypeId: 'ft2', roleIndex: 1 }]], // same constraint ID, different span
        ['c2', [{ factTypeId: 'ft3', roleIndex: 0 }]],
      ]),
    })

    const app = new AppModel('app', [d1, d2])
    const spans = await app.constraintSpans()

    expect(spans.get('c1')).toHaveLength(2)
    expect(spans.get('c2')).toHaveLength(1)
  })

  it('later domain nouns override earlier on collision', async () => {
    const d1 = stubDomain('d1', {
      nouns: new Map([['Status', noun('Status', 'd1', 'value')]]),
    })
    const d2 = stubDomain('d2', {
      nouns: new Map([['Status', noun('Status', 'd2', 'entity')]]),
    })

    const app = new AppModel('app', [d1, d2])
    const nouns = await app.nouns()

    expect(nouns.get('Status')?.objectType).toBe('entity') // d2 wins
  })

  it('exposes appId as domainId for generator compatibility', () => {
    const app = new AppModel('my-app', [])
    expect(app.domainId).toBe('my-app')
  })

  it('merges state machines from all domains', async () => {
    const sm1: StateMachineDef = {
      id: 'sm1', nounName: 'Order', nounDef: {} as any,
      statuses: [{ id: 's1', name: 'Draft' }],
      transitions: [{ from: 'Draft', to: 'Placed', event: 'place', eventTypeId: 'e1' }],
    }
    const sm2: StateMachineDef = {
      id: 'sm2', nounName: 'Ticket', nounDef: {} as any,
      statuses: [{ id: 's2', name: 'Open' }],
      transitions: [{ from: 'Open', to: 'Closed', event: 'close', eventTypeId: 'e2' }],
    }

    const d1 = stubDomain('d1', { stateMachines: [sm1] })
    const d2 = stubDomain('d2', { stateMachines: [sm2] })

    const app = new AppModel('app', [d1, d2])
    const sms = await app.stateMachines()

    expect(sms).toHaveLength(2)
    expect(sms[0].nounName).toBe('Order')
    expect(sms[1].nounName).toBe('Ticket')
  })
})

import { describe, it, expect } from 'vitest'
import { render } from './renderer'
import type { NounDef, FactTypeDef, ConstraintDef, StateMachineDef, Generator } from './types'

// Minimal mock model implementing the accessor interface render() needs
function mockAccessors(data: {
  nouns?: NounDef[]
  factTypes?: FactTypeDef[]
  constraints?: ConstraintDef[]
  stateMachines?: StateMachineDef[]
}) {
  const nounMap = new Map((data.nouns ?? []).map((n) => [n.name, n]))
  const ftMap = new Map((data.factTypes ?? []).map((ft) => [ft.id, ft]))
  return {
    nouns: async () => nounMap,
    factTypes: async () => ftMap,
    factTypesFor: async (noun: NounDef) =>
      (data.factTypes ?? []).filter((ft) => ft.roles.some((r) => r.nounDef.name === noun.name && r.roleIndex === 0)),
    constraintsFor: async (fts: FactTypeDef[]) => {
      const ids = new Set(fts.map((f) => f.id))
      return (data.constraints ?? []).filter((c) => c.spans.some((s) => ids.has(s.factTypeId)))
    },
    constraints: async () => data.constraints ?? [],
    stateMachines: async () => new Map((data.stateMachines ?? []).map((sm) => [sm.id, sm])),
  }
}

describe('render()', () => {
  const customer: NounDef = { id: 'n1', name: 'Customer', objectType: 'entity', domainId: 'd1' }
  const email: NounDef = { id: 'n2', name: 'Email', objectType: 'value', domainId: 'd1', valueType: 'string' }

  it('visits entity nouns with their fact types and constraints', async () => {
    const ft: FactTypeDef = {
      id: 'gs1',
      reading: 'Customer has Email',
      arity: 2,
      roles: [
        { id: 'r1', nounName: 'Customer', nounDef: customer, roleIndex: 0 },
        { id: 'r2', nounName: 'Email', nounDef: email, roleIndex: 1 },
      ],
    }
    const uc: ConstraintDef = {
      id: 'c1',
      kind: 'UC',
      modality: 'Alethic',
      text: 'Each Customer has at most one Email',
      spans: [{ factTypeId: 'gs1', roleIndex: 1 }],
    }

    const gen: Generator<string, string[]> = {
      noun: {
        entity: (noun, fts, cs) => `entity:${noun.name}(fts=${fts.length},cs=${cs.length})`,
        value: (noun) => `value:${noun.name}`,
      },
      combine: (parts) => parts,
    }

    const result = await render(mockAccessors({ nouns: [customer, email], factTypes: [ft], constraints: [uc] }), gen)
    expect(result).toContain('entity:Customer(fts=1,cs=1)')
    expect(result).toContain('value:Email')
  })

  it('visits fact types by arity when factType renderer provided', async () => {
    const ft: FactTypeDef = {
      id: 'gs1',
      reading: 'Customer has Email',
      arity: 2,
      roles: [
        { id: 'r1', nounName: 'Customer', nounDef: customer, roleIndex: 0 },
        { id: 'r2', nounName: 'Email', nounDef: email, roleIndex: 1 },
      ],
    }

    const gen: Generator<string, string[]> = {
      noun: { entity: () => 'e', value: () => 'v' },
      factType: {
        binary: (entityRole, valueRole, cs) => `binary:${entityRole.nounName}->${valueRole.nounName}`,
      },
      combine: (parts) => parts,
    }

    const result = await render(mockAccessors({ nouns: [customer, email], factTypes: [ft] }), gen)
    expect(result).toContain('binary:Customer->Email')
  })

  it('visits state machines when stateMachine renderer provided', async () => {
    const sm: StateMachineDef = {
      id: 'sm1',
      nounName: 'Order',
      nounDef: customer,
      statuses: [{ id: 's1', name: 'Draft' }],
      transitions: [{ from: 'Draft', to: 'Pending', event: 'Submit', eventTypeId: 'et1' }],
    }

    const gen: Generator<string, string[]> = {
      noun: { entity: () => 'e', value: () => 'v' },
      stateMachine: (sm) => `sm:${sm.nounName}(${sm.statuses.length} states)`,
      combine: (parts) => parts,
    }

    const result = await render(mockAccessors({ nouns: [customer], stateMachines: [sm] }), gen)
    expect(result).toContain('sm:Order(1 states)')
  })
})

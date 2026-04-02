/**
 * Tests for reconstructIR and DEFS-driven resolution.
 *
 * reconstructIR assembles a ConstraintIR from entity queries. This replaced
 * the serialized ir:${domain} cell. The tests verify that entities seeded
 * by emit_entities (Rust) reconstruct correctly into the IR the WASM
 * evaluator expects.
 */

import { describe, it, expect, vi } from 'vitest'
import { reconstructIR } from './engine'

function mockRegistry(entities: Record<string, any[]>) {
  return {
    getEntityIds: vi.fn(async (type: string, _domain: string) =>
      (entities[type] || []).map((e: any) => e.id),
    ),
  }
}

function mockGetStub(entities: Record<string, any[]>) {
  const allEntities = Object.values(entities).flat()
  return (id: string) => ({
    get: vi.fn(async () => {
      const e = allEntities.find((ent: any) => ent.id === id)
      return e ? { id: e.id, type: e.type, data: e.data, ...e.data } : null
    }),
  })
}

describe('reconstructIR', () => {
  it('returns null when no nouns or schemas exist', async () => {
    const registry = mockRegistry({})
    const getStub = mockGetStub({})
    const result = await reconstructIR(registry, getStub, 'test')
    expect(result).toBeNull()
  })

  it('reconstructs nouns from Noun entities', async () => {
    const entities = {
      'Noun': [
        { id: 'Customer', type: 'Noun', data: { name: 'Customer', objectType: 'entity', domain: 'test' } },
        { id: 'Priority', type: 'Noun', data: { name: 'Priority', objectType: 'value', domain: 'test' } },
      ],
      'Graph Schema': [],
      'Role': [],
      'Constraint': [],
      'Derivation Rule': [],
      'State Machine Definition': [],
      'Status': [],
      'Transition': [],
      'Instance Fact': [],
    }
    const ir = JSON.parse((await reconstructIR(mockRegistry(entities), mockGetStub(entities), 'test'))!)
    expect(ir.nouns.Customer.objectType).toBe('entity')
    expect(ir.nouns.Priority.objectType).toBe('value')
  })

  it('reconstructs fact types from Graph Schema and Role entities', async () => {
    const entities = {
      'Noun': [
        { id: 'Customer', type: 'Noun', data: { name: 'Customer', objectType: 'entity', domain: 'test' } },
        { id: 'Order', type: 'Noun', data: { name: 'Order', objectType: 'entity', domain: 'test' } },
      ],
      'Graph Schema': [
        { id: 'Order_placedBy_Customer', type: 'Graph Schema', data: { name: 'Order_placedBy_Customer', reading: 'Order was placed by Customer', arity: 2, domain: 'test' } },
      ],
      'Role': [
        { id: 'Order_placedBy_Customer:role:0', type: 'Role', data: { nounName: 'Order', position: 0, graphSchema: 'Order_placedBy_Customer', domain: 'test' } },
        { id: 'Order_placedBy_Customer:role:1', type: 'Role', data: { nounName: 'Customer', position: 1, graphSchema: 'Order_placedBy_Customer', domain: 'test' } },
      ],
      'Constraint': [],
      'Derivation Rule': [],
      'State Machine Definition': [],
      'Status': [],
      'Transition': [],
      'Instance Fact': [],
    }
    const ir = JSON.parse((await reconstructIR(mockRegistry(entities), mockGetStub(entities), 'test'))!)
    const ft = ir.factTypes.Order_placedBy_Customer
    expect(ft).toBeDefined()
    expect(ft.reading).toBe('Order was placed by Customer')
    expect(ft.roles).toHaveLength(2)
    expect(ft.roles[0].nounName).toBe('Order')
    expect(ft.roles[1].nounName).toBe('Customer')
  })

  it('reconstructs constraints with kind and spans', async () => {
    const entities = {
      'Noun': [
        { id: 'Order', type: 'Noun', data: { name: 'Order', objectType: 'entity', domain: 'test' } },
      ],
      'Graph Schema': [],
      'Role': [],
      'Constraint': [
        { id: 'Each Order has at most one Customer', type: 'Constraint', data: {
          text: 'Each Order has at most one Customer', kind: 'UC', modality: 'Alethic',
          reading: 'Order_placedBy_Customer', domain: 'test',
          spans: [{ factTypeId: 'Order_placedBy_Customer', roleIndex: 0 }],
        }},
      ],
      'Derivation Rule': [],
      'State Machine Definition': [],
      'Status': [],
      'Transition': [],
      'Instance Fact': [],
    }
    const ir = JSON.parse((await reconstructIR(mockRegistry(entities), mockGetStub(entities), 'test'))!)
    expect(ir.constraints).toHaveLength(1)
    expect(ir.constraints[0].kind).toBe('UC')
    expect(ir.constraints[0].spans[0].factTypeId).toBe('Order_placedBy_Customer')
  })

  it('reconstructs state machines from SM, Status, and Transition entities', async () => {
    const smId = 'sm:Order'
    const entities = {
      'Noun': [
        { id: 'Order', type: 'Noun', data: { name: 'Order', objectType: 'entity', domain: 'test' } },
      ],
      'Graph Schema': [],
      'Role': [],
      'Constraint': [],
      'Derivation Rule': [],
      'State Machine Definition': [
        { id: smId, type: 'State Machine Definition', data: { name: 'Order', forNoun: 'Order', domain: 'test' } },
      ],
      'Status': [
        { id: `${smId}:In Cart`, type: 'Status', data: { name: 'In Cart', stateMachineDefinition: smId, domain: 'test' } },
        { id: `${smId}:Placed`, type: 'Status', data: { name: 'Placed', stateMachineDefinition: smId, domain: 'test' } },
      ],
      'Transition': [
        { id: `${smId}:place`, type: 'Transition', data: { event: 'place', from: 'In Cart', to: 'Placed', stateMachineDefinition: smId, domain: 'test' } },
      ],
      'Instance Fact': [],
    }
    const ir = JSON.parse((await reconstructIR(mockRegistry(entities), mockGetStub(entities), 'test'))!)
    expect(ir.stateMachines.Order).toBeDefined()
    expect(ir.stateMachines.Order.nounName).toBe('Order')
    expect(ir.stateMachines.Order.statuses).toContain('In Cart')
    expect(ir.stateMachines.Order.statuses).toContain('Placed')
    expect(ir.stateMachines.Order.transitions).toHaveLength(1)
    expect(ir.stateMachines.Order.transitions[0].event).toBe('place')
  })

  it('reconstructs instance facts', async () => {
    const entities = {
      'Noun': [
        { id: 'API Product', type: 'Noun', data: { name: 'API Product', objectType: 'entity', domain: 'test' } },
      ],
      'Graph Schema': [],
      'Role': [],
      'Constraint': [],
      'Derivation Rule': [],
      'State Machine Definition': [],
      'Status': [],
      'Transition': [],
      'Instance Fact': [
        { id: 'if:1', type: 'Instance Fact', data: { subjectNoun: 'Noun', subjectValue: 'API Product', fieldName: 'URI', objectNoun: '', objectValue: '/api' } },
      ],
    }
    const ir = JSON.parse((await reconstructIR(mockRegistry(entities), mockGetStub(entities), 'test'))!)
    expect(ir.generalInstanceFacts).toHaveLength(1)
    expect(ir.generalInstanceFacts[0].subjectValue).toBe('API Product')
    expect(ir.generalInstanceFacts[0].objectValue).toBe('/api')
  })

  it('reconstructs backed-by from Instance Fact entities', async () => {
    const entities = {
      'Noun': [
        { id: 'Customer', type: 'Noun', data: { name: 'Customer', objectType: 'entity', domain: 'test' } },
      ],
      'Graph Schema': [],
      'Role': [],
      'Constraint': [],
      'Derivation Rule': [],
      'State Machine Definition': [],
      'Status': [],
      'Transition': [],
      'Instance Fact': [
        { id: 'if:2', type: 'Instance Fact', data: { subjectNoun: 'Noun', subjectValue: 'Customer', fieldName: 'is backed by', objectNoun: 'External System', objectValue: 'auth.vin' } },
      ],
    }
    const ir = JSON.parse((await reconstructIR(mockRegistry(entities), mockGetStub(entities), 'test'))!)
    const backedFact = ir.generalInstanceFacts.find(
      (f: any) => f.subjectNoun === 'Noun' && f.objectNoun === 'External System'
    )
    expect(backedFact).toBeDefined()
    expect(backedFact.subjectValue).toBe('Customer')
    expect(backedFact.objectValue).toBe('auth.vin')
  })
})

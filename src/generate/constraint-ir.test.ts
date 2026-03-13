// src/generate/constraint-ir.test.ts
import { describe, it, expect, vi } from 'vitest'
import { generateConstraintIR } from './constraint-ir'

/**
 * Create a mock DB that returns canned data for findInCollection calls.
 * This mirrors the pattern used in openapi.test.ts and xstate.test.ts.
 */
function createMockDB(data: Record<string, any[]>) {
  return {
    findInCollection: vi.fn(async (slug: string, _where?: any, _opts?: any) => ({
      docs: data[slug] || [],
      totalDocs: (data[slug] || []).length,
      limit: 10000,
      page: 1,
      hasNextPage: false,
    })),
  }
}

describe('generateConstraintIR', () => {
  it('generates IR with nouns, factTypes, constraints, and stateMachines', async () => {
    const db = createMockDB({
      'nouns': [
        { id: 'n1', name: 'Customer', objectType: 'entity', domain: 'd1' },
        { id: 'n2', name: 'Name', objectType: 'value', valueType: 'string', domain: 'd1' },
      ],
      'graph-schemas': [
        { id: 'gs1', name: 'CustomerName', domain: 'd1' },
      ],
      'readings': [
        { id: 'r1', text: 'Customer has Name', graphSchema: 'gs1', domain: 'd1' },
      ],
      'roles': [
        { id: 'ro1', reading: 'r1', noun: 'n1', graphSchema: 'gs1', roleIndex: 0 },
        { id: 'ro2', reading: 'r1', noun: 'n2', graphSchema: 'gs1', roleIndex: 1 },
      ],
      'constraints': [
        { id: 'c1', kind: 'UC', modality: 'Alethic', text: 'Each Customer has at most one Name', domain: 'd1' },
      ],
      'constraint-spans': [
        { id: 'cs1', constraint: 'c1', role: 'ro1' },
      ],
      'state-machine-definitions': [],
      'statuses': [],
      'transitions': [],
      'event-types': [],
      'guards': [],
    })

    const ir = await generateConstraintIR(db, 'd1')

    // Domain
    expect(ir.domain).toBe('d1')

    // Nouns
    expect(ir.nouns['Customer']).toEqual({ objectType: 'entity' })
    expect(ir.nouns['Name']).toEqual({ objectType: 'value', valueType: 'string' })

    // FactTypes
    expect(Object.keys(ir.factTypes)).toHaveLength(1)
    const ft = ir.factTypes['gs1']
    expect(ft.reading).toBe('Customer has Name')
    expect(ft.roles).toEqual([
      { nounName: 'Customer', roleIndex: 0 },
      { nounName: 'Name', roleIndex: 1 },
    ])

    // Constraints
    expect(ir.constraints).toHaveLength(1)
    expect(ir.constraints[0]).toMatchObject({
      id: 'c1',
      kind: 'UC',
      modality: 'Alethic',
      text: 'Each Customer has at most one Name',
      spans: [{ factTypeId: 'gs1', roleIndex: 0 }],
    })
    // UC alethic → no deonticOperator
    expect(ir.constraints[0].deonticOperator).toBeUndefined()

    // No state machines
    expect(Object.keys(ir.stateMachines)).toHaveLength(0)
  })

  it('re-derives deonticOperator from constraint text', async () => {
    const db = createMockDB({
      'nouns': [
        { id: 'n1', name: 'SupportResponse', objectType: 'entity', domain: 'd1' },
        { id: 'n2', name: 'ProhibitedText', objectType: 'value', domain: 'd1' },
      ],
      'graph-schemas': [
        { id: 'gs1', name: 'ResponseContainsProhibited', domain: 'd1' },
      ],
      'readings': [
        { id: 'r1', text: 'SupportResponse contains ProhibitedText', graphSchema: 'gs1', domain: 'd1' },
      ],
      'roles': [
        { id: 'ro1', reading: 'r1', noun: 'n1', graphSchema: 'gs1', roleIndex: 0 },
        { id: 'ro2', reading: 'r1', noun: 'n2', graphSchema: 'gs1', roleIndex: 1 },
      ],
      'constraints': [
        {
          id: 'c1', kind: 'UC', modality: 'Deontic',
          text: 'It is forbidden that SupportResponse contains ProhibitedText',
          domain: 'd1',
        },
      ],
      'constraint-spans': [
        { id: 'cs1', constraint: 'c1', role: 'ro1' },
      ],
      'state-machine-definitions': [],
      'statuses': [],
      'transitions': [],
      'event-types': [],
      'guards': [],
    })

    const ir = await generateConstraintIR(db, 'd1')

    expect(ir.constraints[0].deonticOperator).toBe('forbidden')
    expect(ir.constraints[0].modality).toBe('Deontic')
  })

  it('generates state machine IR with transitions', async () => {
    const db = createMockDB({
      'nouns': [
        { id: 'n1', name: 'SupportRequest', objectType: 'entity', domain: 'd1' },
      ],
      'graph-schemas': [],
      'readings': [],
      'roles': [],
      'constraints': [],
      'constraint-spans': [],
      'state-machine-definitions': [
        { id: 'sm1', title: 'SupportRequest', noun: 'n1', domain: 'd1' },
      ],
      'statuses': [
        { id: 's1', name: 'Received', stateMachineDefinition: 'sm1', domain: 'd1', createdAt: '2026-01-01' },
        { id: 's2', name: 'Investigating', stateMachineDefinition: 'sm1', domain: 'd1', createdAt: '2026-01-02' },
        { id: 's3', name: 'Resolved', stateMachineDefinition: 'sm1', domain: 'd1', createdAt: '2026-01-03' },
      ],
      'transitions': [
        { id: 't1', from: 's1', to: 's2', eventType: 'et1', domain: 'd1' },
        { id: 't2', from: 's2', to: 's3', eventType: 'et2', domain: 'd1' },
      ],
      'event-types': [
        { id: 'et1', name: 'investigate', domain: 'd1' },
        { id: 'et2', name: 'resolve', domain: 'd1' },
      ],
      'guards': [],
    })

    const ir = await generateConstraintIR(db, 'd1')

    const sm = ir.stateMachines['sm1']
    expect(sm).toBeDefined()
    expect(sm.nounName).toBe('SupportRequest')
    expect(sm.statuses).toEqual(['Received', 'Investigating', 'Resolved'])
    expect(sm.transitions).toHaveLength(2)
    expect(sm.transitions[0]).toEqual({ from: 'Received', to: 'Investigating', event: 'investigate' })
    expect(sm.transitions[1]).toEqual({ from: 'Investigating', to: 'Resolved', event: 'resolve' })
  })

  it('resolves guard → graph_schema → constraint_spans → constraints', async () => {
    const db = createMockDB({
      'nouns': [
        { id: 'n1', name: 'Order', objectType: 'entity', domain: 'd1' },
        { id: 'n2', name: 'Payment', objectType: 'entity', domain: 'd1' },
      ],
      'graph-schemas': [
        { id: 'gs1', name: 'OrderPayment', domain: 'd1' },
      ],
      'readings': [
        { id: 'r1', text: 'Order has Payment', graphSchema: 'gs1', domain: 'd1' },
      ],
      'roles': [
        { id: 'ro1', reading: 'r1', noun: 'n1', graphSchema: 'gs1', roleIndex: 0 },
        { id: 'ro2', reading: 'r1', noun: 'n2', graphSchema: 'gs1', roleIndex: 1 },
      ],
      'constraints': [
        { id: 'c1', kind: 'MC', modality: 'Alethic', text: 'Each Order has at least one Payment', domain: 'd1' },
      ],
      'constraint-spans': [
        { id: 'cs1', constraint: 'c1', role: 'ro1' },
      ],
      'state-machine-definitions': [
        { id: 'sm1', title: 'Order', noun: 'n1', domain: 'd1' },
      ],
      'statuses': [
        { id: 's1', name: 'Pending', stateMachineDefinition: 'sm1', domain: 'd1', createdAt: '2026-01-01' },
        { id: 's2', name: 'Paid', stateMachineDefinition: 'sm1', domain: 'd1', createdAt: '2026-01-02' },
      ],
      'transitions': [
        { id: 't1', from: 's1', to: 's2', eventType: 'et1', domain: 'd1' },
      ],
      'event-types': [
        { id: 'et1', name: 'pay', domain: 'd1' },
      ],
      'guards': [
        { id: 'g1', transition: 't1', graphSchema: 'gs1', domain: 'd1' },
      ],
    })

    const ir = await generateConstraintIR(db, 'd1')

    const sm = ir.stateMachines['sm1']
    expect(sm.transitions[0].guard).toEqual({
      graphSchemaId: 'gs1',
      constraintIds: ['c1'],
    })
  })

  it('handles empty domain gracefully', async () => {
    const db = createMockDB({
      'nouns': [],
      'graph-schemas': [],
      'readings': [],
      'roles': [],
      'constraints': [],
      'constraint-spans': [],
      'state-machine-definitions': [],
      'statuses': [],
      'transitions': [],
      'event-types': [],
      'guards': [],
    })

    const ir = await generateConstraintIR(db, 'd1')

    expect(ir.domain).toBe('d1')
    expect(ir.nouns).toEqual({})
    expect(ir.factTypes).toEqual({})
    expect(ir.constraints).toEqual([])
    expect(ir.stateMachines).toEqual({})
  })

  it('includes noun enum values and superType', async () => {
    const db = createMockDB({
      'nouns': [
        { id: 'n1', name: 'Priority', objectType: 'value', valueType: 'string', enumValues: '["low","medium","high"]', domain: 'd1' },
        { id: 'n2', name: 'Customer', objectType: 'entity', domain: 'd1' },
        { id: 'n3', name: 'PremiumCustomer', objectType: 'entity', superType: 'n2', domain: 'd1' },
      ],
      'graph-schemas': [],
      'readings': [],
      'roles': [],
      'constraints': [],
      'constraint-spans': [],
      'state-machine-definitions': [],
      'statuses': [],
      'transitions': [],
      'event-types': [],
      'guards': [],
    })

    const ir = await generateConstraintIR(db, 'd1')

    expect(ir.nouns['Priority']).toEqual({
      objectType: 'value',
      valueType: 'string',
      enumValues: ['low', 'medium', 'high'],
    })
    expect(ir.nouns['PremiumCustomer']).toEqual({
      objectType: 'entity',
      superType: 'Customer',
    })
  })
})

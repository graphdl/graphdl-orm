import { describe, it, expect } from 'vitest'
import { buildSchemaFromEntities } from './schema-from-entities'

describe('buildSchemaFromEntities', () => {
  it('assembles state machines from entity data', async () => {
    // Mock entities matching what the claims pipeline produces
    const entities: Record<string, Array<{ id: string; type: string; data: Record<string, unknown> }>> = {
      'Noun': [
        { id: 'n1', type: 'Noun', data: { name: 'Order', objectType: 'entity', domain: 'orders' } },
      ],
      'Reading': [],
      'Role': [],
      'Constraint': [],
      'Constraint Span': [],
      'Graph Schema': [],
      'State Machine Definition': [
        { id: 'Order', type: 'State Machine Definition', data: { name: 'Order', forNoun: 'Order', domain: 'orders' } },
      ],
      'Status': [
        { id: 'Draft', type: 'Status', data: { name: 'Draft', definedInStateMachineDefinition: 'Order', isInitial: true, domain: 'orders' } },
        { id: 'Placed', type: 'Status', data: { name: 'Placed', definedInStateMachineDefinition: 'Order', domain: 'orders' } },
        { id: 'Cancelled', type: 'Status', data: { name: 'Cancelled', definedInStateMachineDefinition: 'Order', domain: 'orders' } },
      ],
      'Transition': [
        { id: 'place', type: 'Transition', data: { name: 'place', fromStatus: 'Draft', toStatus: 'Placed', triggeredByEventType: 'place', domain: 'orders' } },
        { id: 'cancel', type: 'Transition', data: { name: 'cancel', fromStatus: 'Draft', toStatus: 'Cancelled', triggeredByEventType: 'cancel', domain: 'orders' } },
      ],
      'Event Type': [
        { id: 'place', type: 'Event Type', data: { name: 'place', domain: 'orders' } },
        { id: 'cancel', type: 'Event Type', data: { name: 'cancel', domain: 'orders' } },
      ],
      'Guard': [],
      'Verb': [],
      'Function': [],
    }

    const fetchEntities = async (type: string, _domain: string) => entities[type] || []
    const schema = await buildSchemaFromEntities('orders', fetchEntities)

    // Should have the Order state machine
    expect(Object.keys(schema.stateMachines).length).toBe(1)
    const sm = schema.stateMachines['Order']
    expect(sm).toBeDefined()
    expect(sm.nounName).toBe('Order')
    expect(sm.statuses).toContain('Draft')
    expect(sm.statuses).toContain('Placed')
    expect(sm.statuses).toContain('Cancelled')
    // Initial status should be first
    expect(sm.statuses[0]).toBe('Draft')
    // Transitions
    expect(sm.transitions.length).toBe(2)
    expect(sm.transitions[0].from).toBe('Draft')
    expect(sm.transitions[0].to).toBe('Placed')
    expect(sm.transitions[0].event).toBe('place')
  })

  it('handles entity without state machine', async () => {
    const entities: Record<string, Array<{ id: string; type: string; data: Record<string, unknown> }>> = {
      'Noun': [{ id: 'n1', type: 'Noun', data: { name: 'Category', objectType: 'entity' } }],
      'Reading': [], 'Role': [], 'Constraint': [], 'Constraint Span': [],
      'Graph Schema': [], 'State Machine Definition': [], 'Status': [],
      'Transition': [], 'Event Type': [], 'Guard': [], 'Verb': [], 'Function': [],
    }
    const fetchEntities = async (type: string) => entities[type] || []
    const schema = await buildSchemaFromEntities('catalog', fetchEntities)

    expect(Object.keys(schema.stateMachines).length).toBe(0)
  })
})

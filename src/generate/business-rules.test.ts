// src/generate/business-rules.test.ts
import { describe, it, expect, beforeEach } from 'vitest'
import { generateBusinessRules } from './business-rules'
import { createMockModel, mkNounDef, mkValueNounDef, mkFactType, mkConstraint, mkStateMachine, resetIds } from '../model/test-utils'

describe('generateBusinessRules', () => {
  beforeEach(() => {
    resetIds()
  })

  it('generates IR with nouns, factTypes, constraints, and stateMachines', async () => {
    const customer = mkNounDef({ name: 'Customer' })
    const name = mkValueNounDef({ name: 'Name', valueType: 'string' })
    const ft = mkFactType({
      reading: 'Customer has Name',
      roles: [
        { nounDef: customer, roleIndex: 0 },
        { nounDef: name, roleIndex: 1 },
      ],
    })

    const model = createMockModel({
      nouns: [customer, name],
      factTypes: [ft],
      constraints: [
        mkConstraint({
          kind: 'UC',
          text: 'Each Customer has at most one Name',
          spans: [{ factTypeId: ft.id, roleIndex: 0 }],
        }),
      ],
    })

    const ir = await generateBusinessRules(model)

    // Domain
    expect(ir.domain).toBe('d1')

    // Nouns
    expect(ir.nouns['Customer']).toEqual({ objectType: 'entity' })
    expect(ir.nouns['Name']).toEqual({ objectType: 'value', valueType: 'string' })

    // FactTypes
    expect(Object.keys(ir.factTypes)).toHaveLength(1)
    const irFt = ir.factTypes[ft.id]
    expect(irFt.reading).toBe('Customer has Name')
    expect(irFt.roles).toEqual([
      { nounName: 'Customer', roleIndex: 0 },
      { nounName: 'Name', roleIndex: 1 },
    ])

    // Constraints
    expect(ir.constraints).toHaveLength(1)
    expect(ir.constraints[0]).toMatchObject({
      kind: 'UC',
      modality: 'Alethic',
      text: 'Each Customer has at most one Name',
      spans: [{ factTypeId: ft.id, roleIndex: 0 }],
    })
    // UC alethic → no deonticOperator
    expect(ir.constraints[0].deonticOperator).toBeUndefined()

    // No state machines
    expect(Object.keys(ir.stateMachines)).toHaveLength(0)
  })

  it('passes through deonticOperator from DomainModel', async () => {
    const response = mkNounDef({ name: 'SupportResponse' })
    const prohibited = mkValueNounDef({ name: 'ProhibitedText' })
    const ft = mkFactType({
      reading: 'SupportResponse contains ProhibitedText',
      roles: [
        { nounDef: response, roleIndex: 0 },
        { nounDef: prohibited, roleIndex: 1 },
      ],
    })

    const model = createMockModel({
      nouns: [response, prohibited],
      factTypes: [ft],
      constraints: [
        mkConstraint({
          kind: 'UC',
          modality: 'Deontic',
          deonticOperator: 'forbidden',
          text: 'It is forbidden that SupportResponse contains ProhibitedText',
          spans: [{ factTypeId: ft.id, roleIndex: 0 }],
        }),
      ],
    })

    const ir = await generateBusinessRules(model)

    expect(ir.constraints[0].deonticOperator).toBe('forbidden')
    expect(ir.constraints[0].modality).toBe('Deontic')
  })

  it('generates state machine IR with transitions', async () => {
    const supportRequest = mkNounDef({ name: 'SupportRequest' })
    const sm = mkStateMachine({
      nounDef: supportRequest,
      statuses: [
        { id: 's1', name: 'Received' },
        { id: 's2', name: 'Investigating' },
        { id: 's3', name: 'Resolved' },
      ],
      transitions: [
        { from: 'Received', to: 'Investigating', event: 'investigate', eventTypeId: 'et1' },
        { from: 'Investigating', to: 'Resolved', event: 'resolve', eventTypeId: 'et2' },
      ],
    })

    const model = createMockModel({
      nouns: [supportRequest],
      stateMachines: [sm],
    })

    const ir = await generateBusinessRules(model)

    const irSm = ir.stateMachines[sm.id]
    expect(irSm).toBeDefined()
    expect(irSm.nounName).toBe('SupportRequest')
    expect(irSm.statuses).toEqual(['Received', 'Investigating', 'Resolved'])
    expect(irSm.transitions).toHaveLength(2)
    expect(irSm.transitions[0]).toEqual({ from: 'Received', to: 'Investigating', event: 'investigate' })
    expect(irSm.transitions[1]).toEqual({ from: 'Investigating', to: 'Resolved', event: 'resolve' })
  })

  it('includes guard on transition when present', async () => {
    const order = mkNounDef({ name: 'Order' })
    const payment = mkNounDef({ name: 'Payment' })
    const ft = mkFactType({
      reading: 'Order has Payment',
      roles: [
        { nounDef: order, roleIndex: 0 },
        { nounDef: payment, roleIndex: 1 },
      ],
    })

    const constraint = mkConstraint({
      kind: 'MC',
      text: 'Each Order has at least one Payment',
      spans: [{ factTypeId: ft.id, roleIndex: 0 }],
    })

    const sm = mkStateMachine({
      nounDef: order,
      statuses: [
        { id: 's1', name: 'Pending' },
        { id: 's2', name: 'Paid' },
      ],
      transitions: [
        {
          from: 'Pending',
          to: 'Paid',
          event: 'pay',
          eventTypeId: 'et1',
          guard: { graphSchemaId: ft.id, constraintIds: [constraint.id] },
        },
      ],
    })

    const model = createMockModel({
      nouns: [order, payment],
      factTypes: [ft],
      constraints: [constraint],
      stateMachines: [sm],
    })

    const ir = await generateBusinessRules(model)

    const irSm = ir.stateMachines[sm.id]
    expect(irSm.transitions[0].guard).toEqual({
      graphSchemaId: ft.id,
      constraintIds: [constraint.id],
    })
  })

  it('handles empty domain gracefully', async () => {
    const model = createMockModel({})

    const ir = await generateBusinessRules(model)

    expect(ir.domain).toBe('d1')
    expect(ir.nouns).toEqual({})
    expect(ir.factTypes).toEqual({})
    expect(ir.constraints).toEqual([])
    expect(ir.stateMachines).toEqual({})
  })

  it('includes noun enum values and superType', async () => {
    const priority = mkValueNounDef({ name: 'Priority', valueType: 'string', enumValues: ['low', 'medium', 'high'] })
    const customer = mkNounDef({ name: 'Customer' })
    const premiumCustomer = mkNounDef({ name: 'PremiumCustomer', superType: 'Customer' })

    const model = createMockModel({
      nouns: [priority, customer, premiumCustomer],
    })

    const ir = await generateBusinessRules(model)

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

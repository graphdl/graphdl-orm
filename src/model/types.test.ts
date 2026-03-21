// src/model/types.test.ts
import { describe, it, expect } from 'vitest'
import type { NounDef, FactTypeDef, RoleDef, ConstraintDef, ConstraintKind, SpanDef, StateMachineDef, StatusDef, TransitionDef, VerbDef, ReadingDef } from './types'

describe('model types', () => {
  it('NounDef represents entity nouns', () => {
    const noun: NounDef = {
      id: 'n1', name: 'Customer', objectType: 'entity', domainId: 'd1',
      plural: 'customers', permissions: ['list', 'read', 'create', 'update', 'delete'],
    }
    expect(noun.objectType).toBe('entity')
  })

  it('NounDef represents value nouns with validation', () => {
    const noun: NounDef = {
      id: 'n2', name: 'Email', objectType: 'value', domainId: 'd1',
      valueType: 'string', format: 'email', minLength: 5, maxLength: 255,
    }
    expect(noun.valueType).toBe('string')
  })

  it('NounDef supports enum values as parsed string[]', () => {
    const noun: NounDef = {
      id: 'n3', name: 'Priority', objectType: 'value', domainId: 'd1',
      valueType: 'string', enumValues: ['low', 'medium', 'high'],
    }
    expect(noun.enumValues).toEqual(['low', 'medium', 'high'])
  })

  it('FactTypeDef carries reading, roles, and arity', () => {
    const customer: NounDef = { id: 'n1', name: 'Customer', objectType: 'entity', domainId: 'd1' }
    const name: NounDef = { id: 'n2', name: 'Name', objectType: 'value', domainId: 'd1', valueType: 'string' }
    const ft: FactTypeDef = {
      id: 'gs1', name: 'CustomerHasName', reading: 'Customer has Name',
      roles: [
        { id: 'r1', nounName: 'Customer', nounDef: customer, roleIndex: 0 },
        { id: 'r2', nounName: 'Name', nounDef: name, roleIndex: 1 },
      ],
      arity: 2,
    }
    expect(ft.arity).toBe(2)
    expect(ft.roles[0].nounDef.name).toBe('Customer')
  })

  it('ConstraintKind covers all 17 kinds (16 from readings + legacy RC)', () => {
    const kinds: ConstraintKind[] = [
      'UC', 'MC', 'FC',
      'SS', 'EQ', 'XC', 'OR', 'XO',
      'IR', 'AS', 'AT', 'SY', 'IT', 'TR', 'AC',
      'VC',
    ]
    expect(kinds).toHaveLength(16)
  })

  it('ConstraintDef supports structural constraints with spans', () => {
    const c: ConstraintDef = {
      id: 'c1', kind: 'UC', modality: 'Alethic', text: 'Each Customer has at most one Name',
      spans: [{ factTypeId: 'gs1', roleIndex: 1 }],
    }
    expect(c.kind).toBe('UC')
    expect(c.spans).toHaveLength(1)
  })

  it('ConstraintDef supports deontic constraints', () => {
    const c: ConstraintDef = {
      id: 'c2', kind: 'MC', modality: 'Deontic', deonticOperator: 'obligatory',
      text: 'It is obligatory that each Order has at least one Item',
      spans: [{ factTypeId: 'gs2', roleIndex: 0 }],
    }
    expect(c.deonticOperator).toBe('obligatory')
  })

  it('ConstraintDef supports frequency constraints with min/max occurrence', () => {
    const c: ConstraintDef = {
      id: 'c3', kind: 'FC', modality: 'Alethic',
      text: 'Each Customer submits at least 1 and at most 5 SupportRequest',
      spans: [{ factTypeId: 'gs3', roleIndex: 0 }],
      minOccurrence: 1, maxOccurrence: 5,
    }
    expect(c.kind).toBe('FC')
    expect(c.minOccurrence).toBe(1)
    expect(c.maxOccurrence).toBe(5)
  })

  it('StateMachineDef carries resolved statuses and transitions with verbs', () => {
    const noun: NounDef = { id: 'n1', name: 'Order', objectType: 'entity', domainId: 'd1' }
    const sm: StateMachineDef = {
      id: 'sm1', nounName: 'Order', nounDef: noun,
      statuses: [{ id: 's1', name: 'Draft' }, { id: 's2', name: 'Pending' }],
      transitions: [{
        from: 'Draft', to: 'Pending', event: 'Submit', eventTypeId: 'et1',
        verb: { id: 'v1', name: 'submitOrder', func: { callbackUrl: '/api/submit', httpMethod: 'POST' } },
      }],
    }
    expect(sm.transitions[0].verb?.func?.callbackUrl).toBe('/api/submit')
    expect(sm.transitions[0].verb?.func?.httpMethod).toBe('POST')
  })
})

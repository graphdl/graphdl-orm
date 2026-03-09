import { describe, it, expect } from 'vitest'
import { domainParseToClaims, stateMachineParseToClaims, readingDefsToClaims } from './converters'
import type { DomainParseResult, StateMachineParseResult, ReadingDef } from '../seed/parser'

describe('domainParseToClaims', () => {
  it('converts entity types to entity nouns with plural', () => {
    const parsed: DomainParseResult = {
      entityTypes: [
        { name: 'Customer', referenceScheme: ['Name'] },
        { name: 'Order', referenceScheme: ['OrderId'] },
      ],
      valueTypes: [],
      readings: [],
      instanceFacts: [],
      deonticConstraints: [],
      deonticConstraintInstances: [],
    }

    const claims = domainParseToClaims(parsed)

    expect(claims.nouns).toHaveLength(2)
    expect(claims.nouns[0]).toEqual({
      name: 'Customer',
      objectType: 'entity',
      plural: 'customers',
    })
    expect(claims.nouns[1]).toEqual({
      name: 'Order',
      objectType: 'entity',
      plural: 'orders',
    })
  })

  it('converts value types to value nouns with metadata', () => {
    const parsed: DomainParseResult = {
      entityTypes: [],
      valueTypes: [
        { name: 'Email', valueType: 'string', format: 'email' },
        { name: 'Priority', valueType: 'string', enum: 'low,medium,high' },
        { name: 'Age', valueType: 'integer', minimum: 0, maximum: 150 },
      ],
      readings: [],
      instanceFacts: [],
      deonticConstraints: [],
      deonticConstraintInstances: [],
    }

    const claims = domainParseToClaims(parsed)

    expect(claims.nouns).toHaveLength(3)
    expect(claims.nouns[0]).toEqual({
      name: 'Email',
      objectType: 'value',
      valueType: 'string',
      format: 'email',
    })
    expect(claims.nouns[1]).toEqual({
      name: 'Priority',
      objectType: 'value',
      valueType: 'string',
      enum: ['low', 'medium', 'high'],
    })
    expect(claims.nouns[2]).toEqual({
      name: 'Age',
      objectType: 'value',
      valueType: 'integer',
      minimum: 0,
      maximum: 150,
    })
  })

  it('converts readings to claims readings with extracted nouns and predicate', () => {
    const parsed: DomainParseResult = {
      entityTypes: [{ name: 'Customer', referenceScheme: [] }],
      valueTypes: [{ name: 'Email', valueType: 'string' }],
      readings: [
        { text: 'Customer has Email', multiplicity: '*:1' },
      ],
      instanceFacts: [],
      deonticConstraints: [],
      deonticConstraintInstances: [],
    }

    const claims = domainParseToClaims(parsed)

    expect(claims.readings).toHaveLength(1)
    expect(claims.readings[0]).toEqual({
      text: 'Customer has Email',
      nouns: ['Customer', 'Email'],
      predicate: 'has',
      multiplicity: '*:1',
    })
  })

  it('converts subtype readings to claims.subtypes', () => {
    const parsed: DomainParseResult = {
      entityTypes: [
        { name: 'Animal', referenceScheme: [] },
        { name: 'Dog', referenceScheme: [] },
      ],
      valueTypes: [],
      readings: [
        { text: 'Dog is a subtype of Animal', multiplicity: 'subtype' },
      ],
      instanceFacts: [],
      deonticConstraints: [],
      deonticConstraintInstances: [],
    }

    const claims = domainParseToClaims(parsed)

    expect(claims.readings).toHaveLength(0)
    expect(claims.subtypes).toEqual([{ child: 'Dog', parent: 'Animal' }])
  })

  it('converts instance facts with quoted values to claims.facts', () => {
    const parsed: DomainParseResult = {
      entityTypes: [{ name: 'Customer', referenceScheme: [] }],
      valueTypes: [{ name: 'Email', valueType: 'string' }],
      readings: [],
      instanceFacts: ["Customer 'Alice' has Email 'alice@example.com'"],
      deonticConstraints: [],
      deonticConstraintInstances: [],
    }

    const claims = domainParseToClaims(parsed)

    expect(claims.facts).toHaveLength(1)
    expect(claims.facts![0]).toEqual({
      reading: 'Customer has Email',
      values: [
        { noun: 'Customer', value: 'Alice' },
        { noun: 'Email', value: 'alice@example.com' },
      ],
    })
  })

  it('converts instance facts without quoted values to readings', () => {
    const parsed: DomainParseResult = {
      entityTypes: [],
      valueTypes: [],
      readings: [],
      instanceFacts: ['subscribe runs SubscribeCustomer'],
      deonticConstraints: [],
      deonticConstraintInstances: [],
    }

    const claims = domainParseToClaims(parsed)

    // "subscribe" is lowercase so not detected as a noun; "SubscribeCustomer" is PascalCase
    expect(claims.facts).toHaveLength(0)
    expect(claims.readings.length).toBeGreaterThanOrEqual(1)
    const r = claims.readings.find((r) => r.text === 'subscribe runs SubscribeCustomer')
    expect(r).toBeTruthy()
  })

  it('converts deontic constraints to readings', () => {
    const parsed: DomainParseResult = {
      entityTypes: [{ name: 'Customer', referenceScheme: [] }],
      valueTypes: [{ name: 'Email', valueType: 'string' }],
      readings: [],
      instanceFacts: [],
      deonticConstraints: ['Customer must have Email'],
      deonticConstraintInstances: [],
    }

    const claims = domainParseToClaims(parsed)

    expect(claims.readings).toHaveLength(1)
    expect(claims.readings[0].text).toBe('Customer must have Email')
    expect(claims.readings[0].multiplicity).toBe('*:1')
  })

  it('converts deontic constraint instances to readings with quoted values', () => {
    const parsed: DomainParseResult = {
      entityTypes: [{ name: 'Customer', referenceScheme: [] }],
      valueTypes: [{ name: 'Email', valueType: 'string' }],
      readings: [],
      instanceFacts: [],
      deonticConstraints: [],
      deonticConstraintInstances: [
        { constraint: 'Customer must have Email', instance: '"All customers need an email"' },
      ],
    }

    const claims = domainParseToClaims(parsed)

    expect(claims.readings).toHaveLength(1)
    expect(claims.readings[0].text).toBe("Customer must have Email 'All customers need an email'")
  })

  it('handles explicit UC notation by adding to claims.constraints', () => {
    const parsed: DomainParseResult = {
      entityTypes: [
        { name: 'Student', referenceScheme: [] },
        { name: 'Course', referenceScheme: [] },
        { name: 'Grade', referenceScheme: [] },
      ],
      valueTypes: [],
      readings: [
        {
          text: 'Student takes Course with Grade',
          multiplicity: 'ternary',
          ucs: [['Student', 'Course']],
        },
      ],
      instanceFacts: [],
      deonticConstraints: [],
      deonticConstraintInstances: [],
    }

    const claims = domainParseToClaims(parsed)

    expect(claims.constraints).toHaveLength(1)
    expect(claims.constraints[0]).toEqual({
      kind: 'UC',
      modality: 'Alethic',
      reading: 'Student takes Course with Grade',
      roles: [0, 1], // Student=0, Course=1
    })
    // Multiplicity should be cleared to avoid duplicate constraint creation
    expect(claims.readings[0].multiplicity).toBeUndefined()
  })

  it('skips SS (subset) constraint readings', () => {
    const parsed: DomainParseResult = {
      entityTypes: [
        { name: 'StateMachine', referenceScheme: [] },
        { name: 'Status', referenceScheme: [] },
      ],
      valueTypes: [],
      readings: [
        {
          text: 'If some StateMachine is in some Status then that Status is defined somewhere',
          multiplicity: 'SS',
        },
      ],
      instanceFacts: [],
      deonticConstraints: [],
      deonticConstraintInstances: [],
    }

    const claims = domainParseToClaims(parsed)

    expect(claims.readings).toHaveLength(0)
  })
})

describe('stateMachineParseToClaims', () => {
  it('converts state machine transitions to claims format', () => {
    const parsed: StateMachineParseResult = {
      states: ['Open', 'InProgress', 'Closed'],
      transitions: [
        { from: 'Open', to: 'InProgress', event: 'start' },
        { from: 'InProgress', to: 'Closed', event: 'close' },
        { from: 'Open', to: 'Closed', event: 'cancel', guard: 'isAdmin' },
      ],
    }

    const claims = stateMachineParseToClaims(parsed, 'Ticket')

    expect(claims.nouns).toEqual([{ name: 'Ticket', objectType: 'entity' }])
    expect(claims.readings).toHaveLength(0)
    expect(claims.constraints).toHaveLength(0)
    expect(claims.transitions).toHaveLength(3)
    expect(claims.transitions![0]).toEqual({
      entity: 'Ticket',
      from: 'Open',
      to: 'InProgress',
      event: 'start',
    })
    expect(claims.transitions![1]).toEqual({
      entity: 'Ticket',
      from: 'InProgress',
      to: 'Closed',
      event: 'close',
    })
    expect(claims.transitions![2]).toEqual({
      entity: 'Ticket',
      from: 'Open',
      to: 'Closed',
      event: 'cancel',
    })
  })

  it('handles empty transitions', () => {
    const parsed: StateMachineParseResult = {
      states: ['Draft'],
      transitions: [],
    }

    const claims = stateMachineParseToClaims(parsed, 'Document')

    expect(claims.nouns).toEqual([{ name: 'Document', objectType: 'entity' }])
    expect(claims.transitions).toHaveLength(0)
  })
})

describe('readingDefsToClaims', () => {
  it('discovers nouns from PascalCase words and creates readings', () => {
    const readings: ReadingDef[] = [
      { text: 'Customer has Email', multiplicity: '*:1' },
      { text: 'Customer submits Order', multiplicity: '*:*' },
    ]

    const claims = readingDefsToClaims(readings)

    // Should discover Customer, Email, Order as nouns
    const nounNames = claims.nouns.map((n) => n.name).sort()
    expect(nounNames).toEqual(['Customer', 'Email', 'Order'])

    // All nouns should be entity type (FORML2 can't distinguish)
    for (const noun of claims.nouns) {
      expect(noun.objectType).toBe('entity')
    }

    expect(claims.readings).toHaveLength(2)
    expect(claims.readings[0]).toEqual({
      text: 'Customer has Email',
      nouns: ['Customer', 'Email'],
      predicate: 'has',
      multiplicity: '*:1',
    })
    expect(claims.readings[1]).toEqual({
      text: 'Customer submits Order',
      nouns: ['Customer', 'Order'],
      predicate: 'submits',
      multiplicity: '*:*',
    })
  })

  it('handles subtype readings', () => {
    const readings: ReadingDef[] = [
      { text: 'Dog is a subtype of Animal', multiplicity: 'subtype' },
      { text: 'Animal has Name', multiplicity: '*:1' },
    ]

    const claims = readingDefsToClaims(readings)

    expect(claims.subtypes).toEqual([{ child: 'Dog', parent: 'Animal' }])
    // The subtype reading should NOT be in claims.readings
    expect(claims.readings).toHaveLength(1)
    expect(claims.readings[0].text).toBe('Animal has Name')
  })

  it('handles explicit UC notation in FORML2', () => {
    const readings: ReadingDef[] = [
      {
        text: 'Student takes Course with Grade',
        multiplicity: 'ternary',
        ucs: [['Student', 'Course']],
      },
    ]

    const claims = readingDefsToClaims(readings)

    expect(claims.constraints).toHaveLength(1)
    expect(claims.constraints[0]).toEqual({
      kind: 'UC',
      modality: 'Alethic',
      reading: 'Student takes Course with Grade',
      roles: [0, 1],
    })
    expect(claims.readings[0].multiplicity).toBeUndefined()
  })

  it('skips SS readings', () => {
    const readings: ReadingDef[] = [
      {
        text: 'If some StateMachine is in some Status then that Status is valid',
        multiplicity: 'SS',
      },
    ]

    const claims = readingDefsToClaims(readings)

    expect(claims.readings).toHaveLength(0)
  })

  it('handles empty input', () => {
    const claims = readingDefsToClaims([])

    expect(claims.nouns).toHaveLength(0)
    expect(claims.readings).toHaveLength(0)
    expect(claims.constraints).toHaveLength(0)
  })

  it('ignores common English words when discovering nouns', () => {
    const readings: ReadingDef[] = [
      { text: 'Each Customer has some Email', multiplicity: '*:1' },
    ]

    const claims = readingDefsToClaims(readings)

    // "Each" should be ignored as a common English word
    const nounNames = claims.nouns.map((n) => n.name)
    expect(nounNames).not.toContain('Each')
    expect(nounNames).toContain('Customer')
    expect(nounNames).toContain('Email')
  })
})

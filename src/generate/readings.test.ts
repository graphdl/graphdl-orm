import { describe, it, expect } from 'vitest'
import { generateReadings } from './readings'

// ---------------------------------------------------------------------------
// Mock DB helper
// ---------------------------------------------------------------------------

function mockDB(data: Record<string, any[]>) {
  return {
    findInCollection: async (slug: string, where?: any, _opts?: any) => {
      let docs = data[slug] || []
      if (where) {
        docs = docs.filter((doc) => {
          for (const [key, condition] of Object.entries(where)) {
            const cond = condition as any
            if (cond?.equals !== undefined) {
              if (doc[key] !== cond.equals) return false
            }
          }
          return true
        })
      }
      return { docs, totalDocs: docs.length, hasNextPage: false, page: 1, limit: 100 }
    },
  }
}

// ---------------------------------------------------------------------------
// Test data factories
// ---------------------------------------------------------------------------

function mkNoun(id: string, name: string, extra?: Record<string, any>) {
  return { id, name, objectType: 'entity', domain: 'dom1', ...extra }
}

function mkValueNoun(id: string, name: string, valueType: string, extra?: Record<string, any>) {
  return { id, name, objectType: 'value', domain: 'dom1', valueType, ...extra }
}

function mkReading(id: string, text: string, graphSchemaId: string) {
  return { id, text, graphSchema: graphSchemaId, domain: 'dom1' }
}

function mkRole(id: string, nounId: string, graphSchemaId: string) {
  return { id, noun: nounId, graphSchema: graphSchemaId }
}

function mkConstraint(id: string, kind: string, modality?: string) {
  return { id, kind, modality }
}

function mkConstraintSpan(id: string, constraintId: string, roleId: string) {
  return { id, constraint: constraintId, role: roleId }
}

function mkSMDef(id: string, title: string) {
  return { id, title, domain: 'dom1' }
}

function mkStatus(id: string, name: string, smdId: string) {
  return { id, name, stateMachineDefinition: smdId }
}

function mkTransition(id: string, fromId: string, toId: string, eventTypeId: string) {
  return { id, from: fromId, to: toId, eventType: eventTypeId }
}

function mkEventType(id: string, name: string) {
  return { id, name }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

describe('generateReadings', () => {
  it('returns format forml2', async () => {
    const db = mockDB({
      nouns: [],
      readings: [],
      'constraint-spans': [],
      constraints: [],
      roles: [],
      'state-machine-definitions': [],
      'event-types': [],
      statuses: [],
      transitions: [],
    })

    const result = await generateReadings(db, 'dom1')
    expect(result.format).toBe('forml2')
  })

  it('returns empty text for empty domain', async () => {
    const db = mockDB({
      nouns: [],
      readings: [],
      'constraint-spans': [],
      constraints: [],
      roles: [],
      'state-machine-definitions': [],
      'event-types': [],
      statuses: [],
      transitions: [],
    })

    const result = await generateReadings(db, 'dom1')
    expect(result.text).toBe('')
  })

  // --- Entity types ---

  it('outputs entity types with names', async () => {
    const db = mockDB({
      nouns: [mkNoun('n1', 'Customer'), mkNoun('n2', 'Order')],
      readings: [],
      'constraint-spans': [],
      constraints: [],
      roles: [],
      'state-machine-definitions': [],
      'event-types': [],
      statuses: [],
      transitions: [],
    })

    const result = await generateReadings(db, 'dom1')
    expect(result.text).toContain('# Entity Types')
    expect(result.text).toContain('Customer')
    expect(result.text).toContain('Order')
  })

  it('includes reference scheme in parentheses', async () => {
    const db = mockDB({
      nouns: [mkNoun('n1', 'Customer', { referenceScheme: 'CustomerId' })],
      readings: [],
      'constraint-spans': [],
      constraints: [],
      roles: [],
      'state-machine-definitions': [],
      'event-types': [],
      statuses: [],
      transitions: [],
    })

    const result = await generateReadings(db, 'dom1')
    expect(result.text).toContain('Customer (CustomerId)')
  })

  it('includes supertype notation', async () => {
    const db = mockDB({
      nouns: [
        mkNoun('n1', 'Person'),
        mkNoun('n2', 'Employee', { superType: 'n1' }),
      ],
      readings: [],
      'constraint-spans': [],
      constraints: [],
      roles: [],
      'state-machine-definitions': [],
      'event-types': [],
      statuses: [],
      transitions: [],
    })

    const result = await generateReadings(db, 'dom1')
    expect(result.text).toContain('Employee : Person')
  })

  it('includes both reference scheme and supertype', async () => {
    const db = mockDB({
      nouns: [
        mkNoun('n1', 'Person'),
        mkNoun('n2', 'Employee', { referenceScheme: 'EmployeeId', superType: 'n1' }),
      ],
      readings: [],
      'constraint-spans': [],
      constraints: [],
      roles: [],
      'state-machine-definitions': [],
      'event-types': [],
      statuses: [],
      transitions: [],
    })

    const result = await generateReadings(db, 'dom1')
    expect(result.text).toContain('Employee (EmployeeId) : Person')
  })

  // --- Value types ---

  it('outputs value types with metadata', async () => {
    const db = mockDB({
      nouns: [mkValueNoun('v1', 'Email', 'string', { format: 'email' })],
      readings: [],
      'constraint-spans': [],
      constraints: [],
      roles: [],
      'state-machine-definitions': [],
      'event-types': [],
      statuses: [],
      transitions: [],
    })

    const result = await generateReadings(db, 'dom1')
    expect(result.text).toContain('# Value Types')
    expect(result.text).toContain('Email (string, format: email)')
  })

  it('outputs value type with pattern', async () => {
    const db = mockDB({
      nouns: [mkValueNoun('v1', 'ZipCode', 'string', { pattern: '\\d{5}' })],
      readings: [],
      'constraint-spans': [],
      constraints: [],
      roles: [],
      'state-machine-definitions': [],
      'event-types': [],
      statuses: [],
      transitions: [],
    })

    const result = await generateReadings(db, 'dom1')
    expect(result.text).toContain('ZipCode (string, pattern: \\d{5})')
  })

  it('outputs value type with enum', async () => {
    const db = mockDB({
      nouns: [mkValueNoun('v1', 'Color', 'string', { enumValues: 'red,green,blue' })],
      readings: [],
      'constraint-spans': [],
      constraints: [],
      roles: [],
      'state-machine-definitions': [],
      'event-types': [],
      statuses: [],
      transitions: [],
    })

    const result = await generateReadings(db, 'dom1')
    expect(result.text).toContain('Color (string, enum: red,green,blue)')
  })

  it('outputs value type with only name when no metadata', async () => {
    const db = mockDB({
      nouns: [{ id: 'v1', name: 'Weight', objectType: 'value', domain: 'dom1' }],
      readings: [],
      'constraint-spans': [],
      constraints: [],
      roles: [],
      'state-machine-definitions': [],
      'event-types': [],
      statuses: [],
      transitions: [],
    })

    const result = await generateReadings(db, 'dom1')
    expect(result.text).toContain('# Value Types')
    // Should just be "Weight" with no parentheses
    const valueSection = result.text.split('# Value Types')[1]
    expect(valueSection).toContain('\nWeight\n')
  })

  // --- Readings ---

  it('outputs reading texts', async () => {
    const db = mockDB({
      nouns: [],
      readings: [
        mkReading('r1', 'Customer places Order', 'gs1'),
        mkReading('r2', 'Order has OrderDate', 'gs2'),
      ],
      'constraint-spans': [],
      constraints: [],
      roles: [],
      'state-machine-definitions': [],
      'event-types': [],
      statuses: [],
      transitions: [],
    })

    const result = await generateReadings(db, 'dom1')
    expect(result.text).toContain('# Readings')
    expect(result.text).toContain('Customer places Order')
    expect(result.text).toContain('Order has OrderDate')
  })

  it('skips readings with no text', async () => {
    const db = mockDB({
      nouns: [],
      readings: [
        { id: 'r1', text: '', graphSchema: 'gs1', domain: 'dom1' },
        { id: 'r2', text: null, graphSchema: 'gs2', domain: 'dom1' },
        mkReading('r3', 'Customer has Name', 'gs3'),
      ],
      'constraint-spans': [],
      constraints: [],
      roles: [],
      'state-machine-definitions': [],
      'event-types': [],
      statuses: [],
      transitions: [],
    })

    const result = await generateReadings(db, 'dom1')
    expect(result.text).toContain('Customer has Name')
    // Should not contain empty lines where blank readings would be
    const readingsSection = result.text.split('# Readings')[1]
    const lines = readingsSection!.split('\n').filter((l) => l.trim().length > 0)
    expect(lines).toHaveLength(1) // only "Customer has Name"
  })

  // --- Constraint annotations ---

  it('annotates readings with [UC] for uniqueness constraint', async () => {
    const db = mockDB({
      nouns: [],
      readings: [mkReading('r1', 'Customer has CustomerId', 'gs1')],
      'constraint-spans': [mkConstraintSpan('cs1', 'c1', 'role1')],
      constraints: [mkConstraint('c1', 'UC')],
      roles: [mkRole('role1', 'n1', 'gs1')],
      'state-machine-definitions': [],
      'event-types': [],
      statuses: [],
      transitions: [],
    })

    const result = await generateReadings(db, 'dom1')
    expect(result.text).toContain('Customer has CustomerId [UC]')
  })

  it('annotates readings with [DUC] for deontic uniqueness constraint', async () => {
    const db = mockDB({
      nouns: [],
      readings: [mkReading('r1', 'Employee has Badge', 'gs1')],
      'constraint-spans': [mkConstraintSpan('cs1', 'c1', 'role1')],
      constraints: [mkConstraint('c1', 'UC', 'Deontic')],
      roles: [mkRole('role1', 'n1', 'gs1')],
      'state-machine-definitions': [],
      'event-types': [],
      statuses: [],
      transitions: [],
    })

    const result = await generateReadings(db, 'dom1')
    expect(result.text).toContain('Employee has Badge [DUC]')
  })

  it('annotates with multiple constraints on same reading', async () => {
    const db = mockDB({
      nouns: [],
      readings: [mkReading('r1', 'Order has Quantity', 'gs1')],
      'constraint-spans': [
        mkConstraintSpan('cs1', 'c1', 'role1'),
        mkConstraintSpan('cs2', 'c2', 'role2'),
      ],
      constraints: [
        mkConstraint('c1', 'UC'),
        mkConstraint('c2', 'MC'),
      ],
      roles: [
        mkRole('role1', 'n1', 'gs1'),
        mkRole('role2', 'n2', 'gs1'),
      ],
      'state-machine-definitions': [],
      'event-types': [],
      statuses: [],
      transitions: [],
    })

    const result = await generateReadings(db, 'dom1')
    expect(result.text).toContain('Order has Quantity [UC] [MC]')
  })

  it('does not annotate reading when constraint is on a different graphSchema', async () => {
    const db = mockDB({
      nouns: [],
      readings: [
        mkReading('r1', 'Customer has Name', 'gs1'),
        mkReading('r2', 'Order has Amount', 'gs2'),
      ],
      'constraint-spans': [mkConstraintSpan('cs1', 'c1', 'role1')],
      constraints: [mkConstraint('c1', 'UC')],
      // role1 belongs to gs2, not gs1
      roles: [mkRole('role1', 'n1', 'gs2')],
      'state-machine-definitions': [],
      'event-types': [],
      statuses: [],
      transitions: [],
    })

    const result = await generateReadings(db, 'dom1')
    expect(result.text).toContain('Customer has Name\n') // no annotation
    expect(result.text).toContain('Order has Amount [UC]')
  })

  // --- State machines ---

  it('outputs state machine transitions', async () => {
    const db = mockDB({
      nouns: [],
      readings: [],
      'constraint-spans': [],
      constraints: [],
      roles: [],
      'state-machine-definitions': [mkSMDef('smd1', 'OrderLifecycle')],
      'event-types': [
        mkEventType('evt1', 'Submit'),
        mkEventType('evt2', 'Approve'),
      ],
      statuses: [
        mkStatus('s1', 'Draft', 'smd1'),
        mkStatus('s2', 'Pending', 'smd1'),
        mkStatus('s3', 'Approved', 'smd1'),
      ],
      transitions: [
        mkTransition('t1', 's1', 's2', 'evt1'),
        mkTransition('t2', 's2', 's3', 'evt2'),
      ],
    })

    const result = await generateReadings(db, 'dom1')
    expect(result.text).toContain('# State Machine: OrderLifecycle')
    expect(result.text).toContain('OrderLifecycle transitions from Draft to Pending on Submit')
    expect(result.text).toContain('OrderLifecycle transitions from Pending to Approved on Approve')
  })

  it('uses id as fallback when state machine has no title', async () => {
    const db = mockDB({
      nouns: [],
      readings: [],
      'constraint-spans': [],
      constraints: [],
      roles: [],
      'state-machine-definitions': [{ id: 'smd1', domain: 'dom1' }],
      'event-types': [mkEventType('evt1', 'Go')],
      statuses: [
        mkStatus('s1', 'Start', 'smd1'),
        mkStatus('s2', 'End', 'smd1'),
      ],
      transitions: [mkTransition('t1', 's1', 's2', 'evt1')],
    })

    const result = await generateReadings(db, 'dom1')
    expect(result.text).toContain('# State Machine: smd1')
    expect(result.text).toContain('smd1 transitions from Start to End on Go')
  })

  it('skips transition when from/to/event cannot be resolved', async () => {
    const db = mockDB({
      nouns: [],
      readings: [],
      'constraint-spans': [],
      constraints: [],
      roles: [],
      'state-machine-definitions': [mkSMDef('smd1', 'Broken')],
      'event-types': [],
      statuses: [mkStatus('s1', 'Active', 'smd1')],
      // Transition references unknown to-status and event type
      transitions: [mkTransition('t1', 's1', 'unknown-status', 'unknown-event')],
    })

    const result = await generateReadings(db, 'dom1')
    expect(result.text).toContain('# State Machine: Broken')
    // Should not contain any transition line since to/event can't be resolved
    expect(result.text).not.toContain('transitions from')
  })

  it('handles state machine with no statuses', async () => {
    const db = mockDB({
      nouns: [],
      readings: [],
      'constraint-spans': [],
      constraints: [],
      roles: [],
      'state-machine-definitions': [mkSMDef('smd1', 'Empty')],
      'event-types': [],
      statuses: [],
      transitions: [],
    })

    const result = await generateReadings(db, 'dom1')
    expect(result.text).toContain('# State Machine: Empty')
    expect(result.text).not.toContain('transitions from')
  })

  // --- Full round-trip ---

  it('outputs all sections in correct order', async () => {
    const db = mockDB({
      nouns: [
        mkNoun('n1', 'Customer', { referenceScheme: 'CustomerId' }),
        mkValueNoun('v1', 'Email', 'string', { format: 'email' }),
      ],
      readings: [mkReading('r1', 'Customer has Email', 'gs1')],
      'constraint-spans': [mkConstraintSpan('cs1', 'c1', 'role1')],
      constraints: [mkConstraint('c1', 'UC')],
      roles: [mkRole('role1', 'n1', 'gs1')],
      'state-machine-definitions': [mkSMDef('smd1', 'CustomerLifecycle')],
      'event-types': [mkEventType('evt1', 'Activate')],
      statuses: [
        mkStatus('s1', 'Inactive', 'smd1'),
        mkStatus('s2', 'Active', 'smd1'),
      ],
      transitions: [mkTransition('t1', 's1', 's2', 'evt1')],
    })

    const result = await generateReadings(db, 'dom1')

    // Check section ordering
    const entityIdx = result.text.indexOf('# Entity Types')
    const valueIdx = result.text.indexOf('# Value Types')
    const readingIdx = result.text.indexOf('# Readings')
    const smIdx = result.text.indexOf('# State Machine:')

    expect(entityIdx).toBeGreaterThanOrEqual(0)
    expect(valueIdx).toBeGreaterThan(entityIdx)
    expect(readingIdx).toBeGreaterThan(valueIdx)
    expect(smIdx).toBeGreaterThan(readingIdx)
  })

  // --- Domain scoping ---

  it('only includes nouns and readings for the requested domain', async () => {
    const db = mockDB({
      nouns: [
        mkNoun('n1', 'Customer'),
        { id: 'n2', name: 'OtherEntity', objectType: 'entity', domain: 'dom2' },
      ],
      readings: [
        mkReading('r1', 'Customer has Name', 'gs1'),
        { id: 'r2', text: 'OtherEntity has Stuff', graphSchema: 'gs2', domain: 'dom2' },
      ],
      'constraint-spans': [],
      constraints: [],
      roles: [],
      'state-machine-definitions': [],
      'event-types': [],
      statuses: [],
      transitions: [],
    })

    const result = await generateReadings(db, 'dom1')
    expect(result.text).toContain('Customer')
    expect(result.text).not.toContain('OtherEntity')
    expect(result.text).toContain('Customer has Name')
    expect(result.text).not.toContain('OtherEntity has Stuff')
  })
})

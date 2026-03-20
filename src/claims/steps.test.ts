import { describe, it, expect, vi } from 'vitest'
import { createScope } from './scope'
import type { Scope } from './scope'
import {
  ensureNoun,
  OPEN_WORLD_NOUNS,
  ingestNouns,
  ingestSubtypes,
  ingestReadings,
  ingestConstraints,
  ingestTransitions,
  ingestFacts,
} from './steps'

// ---------------------------------------------------------------------------
// Mock DB (same pattern as ingest.test.ts)
// ---------------------------------------------------------------------------

function mockDb() {
  const store: Record<string, any[]> = {}
  let idCounter = 0

  return {
    store,
    findInCollection: vi.fn(async (collection: string, where: any, opts?: any) => {
      const all = store[collection] || []
      const filtered = all.filter((doc: any) => {
        for (const [key, cond] of Object.entries(where)) {
          if (typeof cond === 'object' && cond !== null && 'equals' in (cond as any)) {
            const fieldVal = key === 'domain' ? doc.domain : doc[key]
            if (fieldVal !== (cond as any).equals) return false
          }
        }
        return true
      })
      return { docs: filtered, totalDocs: filtered.length }
    }),
    createInCollection: vi.fn(async (collection: string, body: any) => {
      const doc = { id: `id-${++idCounter}`, ...body }
      if (!store[collection]) store[collection] = []
      store[collection].push(doc)
      return doc
    }),
    updateInCollection: vi.fn(async (collection: string, id: string, updates: any) => {
      const coll = store[collection] || []
      const doc = coll.find((d: any) => d.id === id)
      if (doc) Object.assign(doc, updates)
      return doc
    }),
    createEntity: vi.fn(async (domainId: string, nounName: string, fields: any, reference?: string) => {
      const doc = { id: `entity-${++idCounter}`, domain: domainId, noun: nounName, reference, ...fields }
      const key = `entities_${nounName}`
      if (!store[key]) store[key] = []
      store[key].push(doc)
      return doc
    }),
    applySchema: vi.fn(async () => ({ tableMap: {}, fieldMap: {} })),
  }
}

// ---------------------------------------------------------------------------
// Step 1: ingestNouns
// ---------------------------------------------------------------------------

describe('ingestNouns', () => {
  it('creates nouns and adds them to scope', async () => {
    const db = mockDb()
    const scope = createScope()
    const nouns = [
      { name: 'Customer', objectType: 'entity' as const },
      { name: 'Name', objectType: 'value' as const, valueType: 'string' },
    ]

    const count = await ingestNouns(db as any, nouns, 'd1', scope)

    expect(count).toBe(2)
    expect(db.store.nouns).toHaveLength(2)
    expect(scope.nouns.size).toBe(2)
    expect(scope.nouns.get('d1:Customer')).toBeDefined()
    expect(scope.nouns.get('d1:Name')).toBeDefined()
  })

  it('handles enum values', async () => {
    const db = mockDb()
    const scope = createScope()
    const nouns = [
      { name: 'Priority', objectType: 'value' as const, valueType: 'string', enumValues: ['Low', 'Medium', 'High'] },
    ]

    await ingestNouns(db as any, nouns, 'd1', scope)

    expect(db.store.nouns[0].enumValues).toBe('Low, Medium, High')
  })

  it('auto-detects open-world assumption for matching noun names', async () => {
    const db = mockDb()
    const scope = createScope()
    const nouns = [
      { name: 'Right', objectType: 'entity' as const },
      { name: 'Legal Right', objectType: 'entity' as const },
    ]

    await ingestNouns(db as any, nouns, 'd1', scope)

    expect(db.store.nouns[0].worldAssumption).toBe('open')
    expect(db.store.nouns[1].worldAssumption).toBe('open')
  })

  it('prefixes errors with domainId', async () => {
    const db = mockDb()
    // Force an error by making createInCollection throw
    db.createInCollection.mockRejectedValueOnce(new Error('DB write failed'))
    db.findInCollection.mockResolvedValueOnce({ docs: [], totalDocs: 0 })
    const scope = createScope()

    await ingestNouns(db as any, [{ name: 'Broken', objectType: 'entity' }], 'myDomain', scope)

    expect(scope.errors).toHaveLength(1)
    expect(scope.errors[0]).toMatch(/^\[myDomain\]/)
  })
})

// ---------------------------------------------------------------------------
// Step 2: ingestSubtypes
// ---------------------------------------------------------------------------

describe('ingestSubtypes', () => {
  it('links child to parent noun', async () => {
    const db = mockDb()
    const scope = createScope()

    // Pre-populate scope with nouns
    await ingestNouns(db as any, [
      { name: 'Resource', objectType: 'entity' },
      { name: 'Request', objectType: 'entity' },
    ], 'd1', scope)

    await ingestSubtypes(db as any, [{ child: 'Request', parent: 'Resource' }], 'd1', scope)

    expect(db.updateInCollection).toHaveBeenCalled()
    const updateCall = db.updateInCollection.mock.calls.find(
      ([coll, _id, data]: [string, string, any]) => coll === 'nouns' && data.superType,
    )
    expect(updateCall).toBeDefined()
    // The child (Request) should get the parent's ID as superType
    const requestNoun = scope.nouns.get('d1:Request')
    const resourceNoun = scope.nouns.get('d1:Resource')
    expect(updateCall![2].superType).toBe(resourceNoun!.id)
  })

  it('creates parent noun if not in scope', async () => {
    const db = mockDb()
    const scope = createScope()

    // Only add child to scope
    await ingestNouns(db as any, [
      { name: 'Request', objectType: 'entity' },
    ], 'd1', scope)

    await ingestSubtypes(db as any, [{ child: 'Request', parent: 'Resource' }], 'd1', scope)

    // Parent should have been created and added to scope
    expect(scope.nouns.get('d1:Resource')).toBeDefined()
    expect(db.updateInCollection).toHaveBeenCalled()
  })
})

// ---------------------------------------------------------------------------
// Step 3: ingestReadings
// ---------------------------------------------------------------------------

describe('ingestReadings', () => {
  it('creates graph schema, reading, and roles', async () => {
    const db = mockDb()
    const scope = createScope()

    await ingestNouns(db as any, [
      { name: 'Customer', objectType: 'entity' },
      { name: 'Name', objectType: 'value', valueType: 'string' },
    ], 'd1', scope)

    const count = await ingestReadings(db as any, [
      { text: 'Customer has Name', nouns: ['Customer', 'Name'], predicate: 'has' },
    ], 'd1', scope)

    expect(count).toBe(1)
    expect(db.store['graph-schemas']).toHaveLength(1)
    expect(db.store.readings).toHaveLength(1)
    expect(db.store.readings[0].text).toBe('Customer has Name')
    expect(db.store.roles).toBeDefined()
    expect(db.store.roles.length).toBeGreaterThanOrEqual(2)
    // Schema should be in scope
    expect(scope.schemas.size).toBe(1)
  })

  it('handles derivation readings', async () => {
    const db = mockDb()
    const scope = createScope()

    const count = await ingestReadings(db as any, [
      {
        text: "Person has FullName := Person has FirstName + ' ' + Person has LastName.",
        nouns: ['Person', 'FullName'],
        predicate: ':=',
      },
    ], 'd1', scope)

    expect(count).toBe(1)
    expect(db.store.readings[0].text).toContain(':=')
    expect(scope.schemas.size).toBe(1)
  })

  it('is idempotent — increments scope.skipped for existing readings', async () => {
    const db = mockDb()
    const scope = createScope()

    await ingestNouns(db as any, [
      { name: 'Customer', objectType: 'entity' },
      { name: 'Name', objectType: 'value', valueType: 'string' },
    ], 'd1', scope)

    const readings = [
      { text: 'Customer has Name', nouns: ['Customer', 'Name'], predicate: 'has' },
    ]

    // First ingestion
    const first = await ingestReadings(db as any, readings, 'd1', scope)
    expect(first).toBe(1)
    expect(scope.skipped).toBe(0)

    // Second ingestion — same reading already exists in DB
    const second = await ingestReadings(db as any, readings, 'd1', scope)
    expect(second).toBe(0)
    expect(scope.skipped).toBe(1)
    // Only 1 reading in the store
    expect(db.store.readings).toHaveLength(1)
  })

  it('auto-creates nouns referenced in reading but not in scope', async () => {
    const db = mockDb()
    const scope = createScope()
    // Do NOT pre-create nouns — they should be auto-created

    const count = await ingestReadings(db as any, [
      { text: 'Customer has Name', nouns: ['Customer', 'Name'], predicate: 'has' },
    ], 'd1', scope)

    expect(count).toBe(1)
    expect(scope.nouns.get('d1:Customer')).toBeDefined()
    expect(scope.nouns.get('d1:Name')).toBeDefined()
  })
})

// ---------------------------------------------------------------------------
// Step 4: ingestConstraints
// ---------------------------------------------------------------------------

describe('ingestConstraints', () => {
  it('creates constraints with spans for a known reading', async () => {
    const db = mockDb()
    const scope = createScope()

    // Set up nouns and reading
    await ingestNouns(db as any, [
      { name: 'Customer', objectType: 'entity' },
      { name: 'Name', objectType: 'value', valueType: 'string' },
    ], 'd1', scope)
    await ingestReadings(db as any, [
      { text: 'Customer has Name', nouns: ['Customer', 'Name'], predicate: 'has' },
    ], 'd1', scope)

    await ingestConstraints(db as any, [
      { kind: 'UC', modality: 'Alethic', reading: 'Customer has Name', roles: [0] },
    ], 'd1', scope)

    expect(db.store.constraints).toBeDefined()
    expect(db.store.constraints.length).toBeGreaterThanOrEqual(1)
    expect(db.store['constraint-spans']).toBeDefined()
  })

  it('reports error when reading is not found', async () => {
    const db = mockDb()
    const scope = createScope()

    await ingestConstraints(db as any, [
      { kind: 'UC', modality: 'Alethic', reading: 'Unknown has Thing', roles: [0] },
    ], 'd1', scope)

    expect(scope.errors.length).toBeGreaterThan(0)
    expect(scope.errors[0]).toContain('not found')
    expect(scope.errors[0]).toMatch(/^\[d1\]/)
  })

  it('increments scope.skipped for duplicate constraints', async () => {
    const db = mockDb()
    const scope = createScope()

    await ingestNouns(db as any, [
      { name: 'Customer', objectType: 'entity' },
      { name: 'Name', objectType: 'value', valueType: 'string' },
    ], 'd1', scope)
    await ingestReadings(db as any, [
      { text: 'Customer has Name', nouns: ['Customer', 'Name'], predicate: 'has' },
    ], 'd1', scope)

    const constraint = {
      kind: 'UC' as const,
      modality: 'Alethic' as const,
      reading: 'Customer has Name',
      roles: [0],
      text: 'Each Customer has at most one Name',
    }

    await ingestConstraints(db as any, [constraint], 'd1', scope)
    expect(scope.skipped).toBe(0)

    await ingestConstraints(db as any, [constraint], 'd1', scope)
    expect(scope.skipped).toBe(1)
  })

  it('creates set-comparison constraints without host reading', async () => {
    const db = mockDb()
    const scope = createScope()

    await ingestConstraints(db as any, [
      { kind: 'XC', modality: 'Alethic', reading: '', roles: [] },
    ], 'd1', scope)

    expect(db.store.constraints).toHaveLength(1)
    expect(db.store.constraints[0].kind).toBe('XC')
  })
})

// ---------------------------------------------------------------------------
// Step 5: ingestTransitions
// ---------------------------------------------------------------------------

describe('ingestTransitions', () => {
  it('creates state machine definition, statuses, events, and transitions', async () => {
    const db = mockDb()
    const scope = createScope()

    await ingestNouns(db as any, [
      { name: 'Order', objectType: 'entity' },
    ], 'd1', scope)

    const count = await ingestTransitions(db as any, [
      { entity: 'Order', from: 'New', to: 'Shipped', event: 'Ship' },
      { entity: 'Order', from: 'Shipped', to: 'Delivered', event: 'Deliver' },
    ], 'd1', scope)

    expect(count).toBe(1) // 1 entity = 1 state machine
    expect(db.store['state-machine-definitions']).toHaveLength(1)
    expect(db.store.statuses).toHaveLength(3) // New, Shipped, Delivered
    expect(db.store['event-types']).toHaveLength(2) // Ship, Deliver
    expect(db.store.transitions).toHaveLength(2)
  })

  it('creates noun if entity is not in scope', async () => {
    const db = mockDb()
    const scope = createScope()
    // Do NOT pre-create the noun

    const count = await ingestTransitions(db as any, [
      { entity: 'Ticket', from: 'Open', to: 'Closed', event: 'Close' },
    ], 'd1', scope)

    expect(count).toBe(1)
    expect(scope.nouns.get('d1:Ticket')).toBeDefined()
  })
})

// ---------------------------------------------------------------------------
// Step 6: ingestFacts
// ---------------------------------------------------------------------------

describe('ingestFacts', () => {
  it('creates entity instances via createEntity', async () => {
    const db = mockDb()
    const scope = createScope()

    await ingestFacts(db as any, [
      {
        reading: 'Customer has Name',
        values: [
          { noun: 'Customer', value: 'Acme' },
          { noun: 'Name', value: 'Acme Corp' },
        ],
      },
    ], 'd1', scope)

    expect(db.createEntity).toHaveBeenCalledWith('d1', 'Customer', { name: 'Acme Corp' }, 'Acme')
  })

  it('normalizes entity-centric fact format', async () => {
    const db = mockDb()
    const scope = createScope()

    await ingestFacts(db as any, [
      {
        entity: 'Status',
        entityValue: 'Received',
        predicate: 'has',
        valueType: 'Display Color',
        value: 'blue',
      },
    ], 'd1', scope)

    expect(db.createEntity).toHaveBeenCalledWith('d1', 'Status', { displayColor: 'blue' }, 'Received')
  })

  it('reports error for facts with no reading or entity', async () => {
    const db = mockDb()
    const scope = createScope()

    await ingestFacts(db as any, [
      { values: [{ noun: 'X', value: 'Y' }] },
    ], 'd1', scope)

    expect(scope.errors.length).toBeGreaterThan(0)
    expect(scope.errors[0]).toMatch(/^\[d1\]/)
    expect(scope.errors[0]).toContain('no reading or entity')
  })

  it('reports error for facts with no entity name', async () => {
    const db = mockDb()
    const scope = createScope()

    await ingestFacts(db as any, [
      { reading: 'something', values: [] },
    ], 'd1', scope)

    expect(scope.errors.length).toBeGreaterThan(0)
    expect(scope.errors[0]).toContain('no entity name')
  })

  it('does not crash when values[1].noun is undefined', async () => {
    const db = mockDb()
    const scope = createScope()

    // values[1] exists but has no noun property — should not throw
    await ingestFacts(db as any, [
      {
        reading: 'Customer has Name',
        values: [
          { noun: 'Customer', value: 'Acme' },
          { value: 'Acme Corp' }, // missing noun
        ],
      },
    ], 'd1', scope)

    // Should not crash. createEntity should either not be called or be called
    // without the field (since noun was missing and couldn't derive field name).
    // Either way, no uncaught exception.
    expect(scope.errors.length).toBe(0)
  })

  it('does not crash when values[1] is completely empty', async () => {
    const db = mockDb()
    const scope = createScope()

    await ingestFacts(db as any, [
      {
        reading: 'Customer has Name',
        values: [
          { noun: 'Customer', value: 'Acme' },
          {}, // no noun, no value
        ],
      },
    ], 'd1', scope)

    // Should not crash with "Cannot read properties of undefined (reading 'split')"
    expect(scope.errors.length).toBe(0)
  })
})

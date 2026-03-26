import { describe, it, expect, vi } from 'vitest'
import { ingestProject, type ExtractedClaims } from './ingest'

// ---------------------------------------------------------------------------
// Mock DB — only needed for instance facts (createEntity) and applySchema
// ---------------------------------------------------------------------------

function mockDb() {
  const store: Record<string, any[]> = {}
  let idCounter = 0

  return {
    store,
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
// Helper: build empty claims with overrides
// ---------------------------------------------------------------------------

function emptyClaims(overrides: Partial<ExtractedClaims> = {}): ExtractedClaims {
  return { nouns: [], readings: [], constraints: [], ...overrides }
}

// ---------------------------------------------------------------------------
// Helper: extract entities of a given type from a batch
// ---------------------------------------------------------------------------

function batchEntities(result: { batch: { entities: any[] } }, type: string) {
  return result.batch.entities.filter((e: any) => e.type === type)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

describe('ingestProject', () => {
  it('cross-domain noun resolution — Domain B facts reference Domain A nouns', async () => {
    const db = mockDb()

    // Domain A defines 'Status' with state machine transitions
    const domainA: ExtractedClaims = emptyClaims({
      nouns: [
        { name: 'Status', objectType: 'value', valueType: 'string', enumValues: ['Open', 'Closed'] },
      ],
      transitions: [
        { entity: 'Status', from: 'Open', to: 'Closed', event: 'Close' },
      ],
    })

    // Domain B references 'Status' in instance facts (noun NOT defined in domain B)
    const domainB: ExtractedClaims = emptyClaims({
      nouns: [
        { name: 'Order', objectType: 'entity' },
      ],
      facts: [
        {
          entity: 'Order',
          entityValue: 'ORD-1',
          predicate: 'has',
          valueType: 'Status',
          value: 'Open',
        },
      ],
    })

    const result = await ingestProject(db as any, [
      { domainId: 'domainA', claims: domainA },
      { domainId: 'domainB', claims: domainB },
    ])

    // Order is a declared entity type — its instance facts go to BatchBuilder
    const domainBResult = result.domains.get('domainB')!
    const orderEntities = (domainBResult.batch?.entities || []).filter((e: any) => e.type === 'Order')
    expect(orderEntities.length).toBe(1)
    expect(orderEntities[0].data.status).toBe('Open')

    // Totals should reflect both domains
    expect(result.totals.nouns).toBe(2) // Status + Order
    expect(result.domains.get('domainA')!.nouns).toBe(1)
    expect(result.domains.get('domainB')!.nouns).toBe(1)
  })

  it('reports errors when a fact references a completely unknown noun', async () => {
    const db = mockDb()

    // Make createEntity fail to simulate unresolvable reference
    db.createEntity.mockRejectedValue(new Error('entity table "Ghost" does not exist'))

    const domainA: ExtractedClaims = emptyClaims({
      nouns: [
        { name: 'Customer', objectType: 'entity' },
      ],
      facts: [
        {
          entity: 'Ghost',
          entityValue: 'g1',
          predicate: 'has',
          valueType: 'Phantom',
          value: 'spooky',
        },
      ],
    })

    const result = await ingestProject(db as any, [
      { domainId: 'd1', claims: domainA },
    ])

    // totals.errors should contain the error about the failed fact
    expect(result.totals.errors.length).toBeGreaterThan(0)
    expect(result.totals.errors.some(e => e.includes('Ghost'))).toBe(true)

    // per-domain errors should also have it
    const d1result = result.domains.get('d1')!
    expect(d1result.errors.length).toBeGreaterThan(0)
    expect(d1result.errors.some(e => e.includes('Ghost'))).toBe(true)
  })

  it('multiple domains share a noun defined in domain 1', async () => {
    const db = mockDb()

    // Domain 1 defines 'Currency' noun
    const domain1: ExtractedClaims = emptyClaims({
      nouns: [
        { name: 'Currency', objectType: 'value', valueType: 'string' },
      ],
    })

    // Domain 2 references 'Currency' in a reading
    const domain2: ExtractedClaims = emptyClaims({
      nouns: [
        { name: 'Product', objectType: 'entity' },
      ],
      readings: [
        { text: 'Product has Currency', nouns: ['Product', 'Currency'], predicate: 'has' },
      ],
    })

    // Domain 3 also references 'Currency' in a reading
    const domain3: ExtractedClaims = emptyClaims({
      nouns: [
        { name: 'Invoice', objectType: 'entity' },
      ],
      readings: [
        { text: 'Invoice has Currency', nouns: ['Invoice', 'Currency'], predicate: 'has' },
      ],
    })

    const result = await ingestProject(db as any, [
      { domainId: 'd1', claims: domain1 },
      { domainId: 'd2', claims: domain2 },
      { domainId: 'd3', claims: domain3 },
    ])

    // All domains should succeed without errors
    expect(result.totals.errors).toHaveLength(0)

    // Domain 2 and 3 should each have created 1 reading
    expect(result.domains.get('d2')!.readings).toBe(1)
    expect(result.domains.get('d3')!.readings).toBe(1)

    // Totals: 3 nouns (Currency, Product, Invoice) + 2 readings
    expect(result.totals.nouns).toBe(3)
    expect(result.totals.readings).toBe(2)

    // The 'Currency' noun should exist in the batch (may have domain-local copies
    // when auto-creating for readings — that's expected behavior)
    const currencyNouns = batchEntities(result, 'Noun').filter(
      (e: any) => e.data.name === 'Currency',
    )
    expect(currencyNouns.length).toBeGreaterThanOrEqual(1)
  })

  it('returns correct per-domain result shape', async () => {
    const db = mockDb()

    const result = await ingestProject(db as any, [
      {
        domainId: 'd1',
        claims: emptyClaims({
          nouns: [{ name: 'A', objectType: 'entity' }],
          readings: [],
        }),
      },
      {
        domainId: 'd2',
        claims: emptyClaims({
          nouns: [{ name: 'B', objectType: 'entity' }],
          readings: [],
        }),
      },
    ])

    // Both domains should be in the result map
    expect(result.domains.size).toBe(2)
    expect(result.domains.has('d1')).toBe(true)
    expect(result.domains.has('d2')).toBe(true)

    // Each domain result should have the expected shape
    const d1 = result.domains.get('d1')!
    expect(d1).toHaveProperty('nouns')
    expect(d1).toHaveProperty('readings')
    expect(d1).toHaveProperty('stateMachines')
    expect(d1).toHaveProperty('skipped')
    expect(d1).toHaveProperty('errors')

    // Totals shape
    expect(result.totals).toHaveProperty('nouns')
    expect(result.totals).toHaveProperty('readings')
    expect(result.totals).toHaveProperty('stateMachines')
    expect(result.totals).toHaveProperty('errors')

    // Combined batch should have all entities
    expect(result.batch).toBeDefined()
    expect(result.batch.entities.length).toBeGreaterThanOrEqual(2)
  })

  it('handles empty domains array gracefully', async () => {
    const db = mockDb()

    const result = await ingestProject(db as any, [])

    expect(result.domains.size).toBe(0)
    expect(result.totals.nouns).toBe(0)
    expect(result.totals.readings).toBe(0)
    expect(result.totals.stateMachines).toBe(0)
    expect(result.totals.errors).toHaveLength(0)
    expect(result.batch.entities).toHaveLength(0)
  })
})

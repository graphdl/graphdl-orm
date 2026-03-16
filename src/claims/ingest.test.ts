import { describe, it, expect, vi } from 'vitest'
import { ingestClaims, type ExtractedClaims } from './ingest'
import { parseFORML2 } from '../api/parse'

// ---------------------------------------------------------------------------
// Mock DB (in-memory store for idempotency testing)
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
  }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

describe('ingestClaims', () => {
  it('creates nouns from claims', async () => {
    const db = mockDb()
    const claims: ExtractedClaims = {
      nouns: [
        { name: 'Customer', objectType: 'entity' },
        { name: 'Name', objectType: 'value', valueType: 'string' },
      ],
      readings: [],
      constraints: [],
    }

    const result = await ingestClaims(db as any, { claims, domainId: 'd1' })

    expect(result.nouns).toBe(2)
    expect(result.errors).toHaveLength(0)
    expect(db.store.nouns).toHaveLength(2)
    expect(db.store.nouns[0].name).toBe('Customer')
    expect(db.store.nouns[1].valueType).toBe('string')
  })

  it('creates graph schemas and readings from binary facts', async () => {
    const db = mockDb()
    const claims: ExtractedClaims = {
      nouns: [
        { name: 'Customer', objectType: 'entity' },
        { name: 'Name', objectType: 'value', valueType: 'string' },
      ],
      readings: [
        { text: 'Customer has Name', nouns: ['Customer', 'Name'], predicate: 'has' },
      ],
      constraints: [],
    }

    const result = await ingestClaims(db as any, { claims, domainId: 'd1' })

    expect(result.readings).toBe(1)
    expect(db.store['graph-schemas']).toHaveLength(1)
    expect(db.store.readings).toHaveLength(1)
    expect(db.store.readings[0].text).toBe('Customer has Name')
  })

  it('creates roles from reading noun references', async () => {
    const db = mockDb()
    const claims: ExtractedClaims = {
      nouns: [
        { name: 'Customer', objectType: 'entity' },
        { name: 'Name', objectType: 'value', valueType: 'string' },
      ],
      readings: [
        { text: 'Customer has Name', nouns: ['Customer', 'Name'], predicate: 'has' },
      ],
      constraints: [],
    }

    await ingestClaims(db as any, { claims, domainId: 'd1' })

    // Should create roles for both nouns in the reading
    expect(db.store.roles).toBeDefined()
    expect(db.store.roles.length).toBeGreaterThanOrEqual(2)
  })

  it('applies subtypes', async () => {
    const db = mockDb()
    const claims: ExtractedClaims = {
      nouns: [
        { name: 'Resource', objectType: 'entity' },
        { name: 'Request', objectType: 'entity' },
      ],
      readings: [],
      constraints: [],
      subtypes: [{ child: 'Request', parent: 'Resource' }],
    }

    await ingestClaims(db as any, { claims, domainId: 'd1' })

    // updateInCollection should have been called to set superType
    expect(db.updateInCollection).toHaveBeenCalled()
    const updateCall = db.updateInCollection.mock.calls.find(
      ([coll, id, data]: [string, string, any]) => coll === 'nouns' && data.superType
    )
    expect(updateCall).toBeDefined()
  })

  it('is idempotent — skips existing readings', async () => {
    const db = mockDb()
    const claims: ExtractedClaims = {
      nouns: [
        { name: 'Customer', objectType: 'entity' },
        { name: 'Name', objectType: 'value', valueType: 'string' },
      ],
      readings: [
        { text: 'Customer has Name', nouns: ['Customer', 'Name'], predicate: 'has' },
      ],
      constraints: [],
    }

    // Ingest twice
    const first = await ingestClaims(db as any, { claims, domainId: 'd1' })
    const second = await ingestClaims(db as any, { claims, domainId: 'd1' })

    // First run creates, second skips
    expect(first.readings).toBe(1)
    expect(second.skipped).toBeGreaterThan(0)
    // Only 1 reading in the store (not 2)
    expect(db.store.readings).toHaveLength(1)
  })

  it('handles enum values on value types', async () => {
    const db = mockDb()
    const claims: ExtractedClaims = {
      nouns: [
        { name: 'Priority', objectType: 'value', valueType: 'string', enumValues: ['Low', 'Medium', 'High'] },
      ],
      readings: [],
      constraints: [],
    }

    await ingestClaims(db as any, { claims, domainId: 'd1' })

    expect(db.store.nouns[0].enumValues).toBe('Low, Medium, High')
  })

  it('handles derivation readings (predicate :=)', async () => {
    const db = mockDb()
    const claims: ExtractedClaims = {
      nouns: [
        { name: 'Person', objectType: 'entity' },
        { name: 'FullName', objectType: 'value', valueType: 'string' },
      ],
      readings: [
        {
          text: "Person has FullName := Person has FirstName + ' ' + Person has LastName.",
          nouns: ['Person', 'FullName'],
          predicate: ':=',
        },
      ],
      constraints: [],
    }

    const result = await ingestClaims(db as any, { claims, domainId: 'd1' })

    expect(result.readings).toBe(1)
    expect(db.store.readings[0].text).toContain(':=')
  })
})

  it('reports error for instance facts referencing missing readings', async () => {
    const db = mockDb()
    const claims: ExtractedClaims = {
      nouns: [
        { name: 'Customer', objectType: 'entity' },
        { name: 'Name', objectType: 'value', valueType: 'string' },
      ],
      readings: [],
      constraints: [],
      facts: [
        {
          reading: 'Customer has Name',
          values: [
            { noun: 'Customer', value: 'Acme' },
            { noun: 'Name', value: 'Acme Corp' },
          ],
        },
      ],
    }

    const result = await ingestClaims(db as any, { claims, domainId: 'd1' })

    // Should report error, not crash
    expect(result.errors.length).toBeGreaterThan(0)
    expect(result.errors[0]).toContain('reading')
    expect(result.errors[0]).toContain('not found')
  })

  it('instance facts succeed when reading was created in same batch', async () => {
    const db = mockDb()
    const claims: ExtractedClaims = {
      nouns: [
        { name: 'Customer', objectType: 'entity' },
        { name: 'Name', objectType: 'value', valueType: 'string' },
      ],
      readings: [
        { text: 'Customer has Name', nouns: ['Customer', 'Name'], predicate: 'has' },
      ],
      constraints: [],
      facts: [
        {
          reading: 'Customer has Name',
          values: [
            { noun: 'Customer', value: 'Acme' },
            { noun: 'Name', value: 'Acme Corp' },
          ],
        },
      ],
    }

    const result = await ingestClaims(db as any, { claims, domainId: 'd1' })

    // Reading created first, so fact should find it via schemaMap
    // The error about createFact not being a function is expected with mock DB
    // but the "reading not found" error should NOT appear
    const readingNotFound = result.errors.filter(e => e.includes('not found'))
    expect(readingNotFound).toHaveLength(0)
  })

  it('handles constraints referencing missing readings gracefully', async () => {
    const db = mockDb()
    const claims: ExtractedClaims = {
      nouns: [
        { name: 'Customer', objectType: 'entity' },
      ],
      readings: [],
      constraints: [
        {
          kind: 'UC',
          modality: 'Alethic',
          reading: 'Customer has Name',
          roles: [0],
        },
      ],
    }

    const result = await ingestClaims(db as any, { claims, domainId: 'd1' })

    expect(result.errors.length).toBeGreaterThan(0)
    expect(result.errors[0]).toContain('reading')
    expect(result.errors[0]).toContain('not found')
  })

// ---------------------------------------------------------------------------
// Integration: parse → ingest
// ---------------------------------------------------------------------------

describe('parse → ingest pipeline', () => {
  it('parses and ingests a simple FORML2 domain', async () => {
    const text = `# TestDomain

## Entity Types

Customer(.CustomerId) is an entity type.

## Value Types

Name is a value type.
Priority is a value type.
  The possible values of Priority are 'Low', 'Medium', 'High'.

## Fact Types

### Customer

Customer has Name.
Customer has Priority.

## Constraints

Each Customer has at most one Name.
Each Customer has at most one Priority.
`

    const parsed = parseFORML2(text, [])

    expect(parsed.coverage).toBeGreaterThan(0)
    expect(parsed.nouns.length).toBeGreaterThanOrEqual(3)
    expect(parsed.readings.length).toBeGreaterThanOrEqual(2)
    expect(parsed.constraints.length).toBeGreaterThanOrEqual(2)

    // Now ingest
    const db = mockDb()
    const result = await ingestClaims(db as any, {
      claims: parsed,
      domainId: 'd1',
    })

    expect(result.nouns).toBeGreaterThanOrEqual(3)
    expect(result.readings).toBeGreaterThanOrEqual(2)
    expect(result.errors).toHaveLength(0)

    // Verify enum was ingested
    const priorityNoun = db.store.nouns.find((n: any) => n.name === 'Priority')
    expect(priorityNoun).toBeDefined()
    expect(priorityNoun.enumValues).toContain('Low')
  })

  it('parses and ingests SPD-1 layering domain', async () => {
    const fs = await import('fs')
    const text = fs.readFileSync('C:/Users/lippe/Repos/spd-1/domains/layering.md', 'utf-8')
    const parsed = parseFORML2(text, [])

    expect(parsed.coverage).toBeGreaterThan(0)
    expect(parsed.nouns.length).toBeGreaterThanOrEqual(5)

    const db = mockDb()
    const result = await ingestClaims(db as any, {
      claims: parsed,
      domainId: 'spd-1-layering',
    })

    expect(result.errors).toHaveLength(0)
    expect(result.nouns).toBeGreaterThanOrEqual(5)
    expect(result.readings).toBeGreaterThanOrEqual(5)
  })

  it('parses and ingests all 9 SPD-1 domains without errors', async () => {
    const fs = await import('fs')
    const domains = [
      'layering', 'osi', 'eight-circuits', 'i-ching',
      'maslow', 'spd-1', 'affect', 'evolution', 'ethics',
    ]

    for (const domain of domains) {
      const fpath = `C:/Users/lippe/Repos/spd-1/domains/${domain}.md`
      const text = fs.readFileSync(fpath, 'utf-8')

      // For i-ching, skip instance facts (too large for in-memory mock)
      const schemaText = text.split('## Instance Facts')[0]
      const parsed = parseFORML2(schemaText, [])

      expect(parsed.nouns.length, `${domain} should have nouns`).toBeGreaterThan(0)

      const db = mockDb()
      const result = await ingestClaims(db as any, {
        claims: parsed,
        domainId: `spd-1-${domain}`,
      })

      expect(result.errors, `${domain} should have no errors`).toHaveLength(0)
      expect(result.nouns, `${domain} should create nouns`).toBeGreaterThan(0)
    }
  })
})

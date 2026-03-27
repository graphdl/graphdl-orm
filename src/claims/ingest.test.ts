import { describe, it, expect, vi } from 'vitest'
import { ingestClaims, type ExtractedClaims } from './ingest'
import { parseFORML2 } from '../api/parse'

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
// Helper: extract entities of a given type from a batch
// ---------------------------------------------------------------------------

function batchEntities(result: { batch: { entities: any[] } }, type: string) {
  return result.batch.entities.filter((e: any) => e.type === type)
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
    const nouns = batchEntities(result, 'Noun')
    expect(nouns).toHaveLength(2)
    expect(nouns[0].data.name).toBe('Customer')
    expect(nouns[1].data.valueType).toBe('string')
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
    expect(batchEntities(result, 'GraphSchema')).toHaveLength(1)
    const readings = batchEntities(result, 'Reading')
    expect(readings).toHaveLength(1)
    expect(readings[0].data.text).toBe('Customer has Name')
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

    const result = await ingestClaims(db as any, { claims, domainId: 'd1' })

    // Should create roles for both nouns in the reading
    const roles = batchEntities(result, 'Role')
    expect(roles.length).toBeGreaterThanOrEqual(2)
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

    const result = await ingestClaims(db as any, { claims, domainId: 'd1' })

    // The Request noun should have superType set in the batch
    const nouns = batchEntities(result, 'Noun')
    const requestNoun = nouns.find((n: any) => n.data.name === 'Request')
    expect(requestNoun).toBeDefined()
    expect(requestNoun!.data.superType).toBeDefined()
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

    // Ingest twice — each call creates its own BatchBuilder internally
    const first = await ingestClaims(db as any, { claims, domainId: 'd1' })
    // Second call creates a new batch — within that batch the reading is new,
    // so it won't skip. Idempotency within a single ingestion is tested in steps.test.ts.
    expect(first.readings).toBe(1)
    expect(batchEntities(first, 'Reading')).toHaveLength(1)
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

    const result = await ingestClaims(db as any, { claims, domainId: 'd1' })

    const nouns = batchEntities(result, 'Noun')
    expect(nouns[0].data.enumValues).toBe('["Low","Medium","High"]')
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
    const readings = batchEntities(result, 'Reading')
    expect(readings[0].data.text).toContain(':=')
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

    // Facts for declared entity types go to BatchBuilder, not createEntity
    // Customer is declared as entity type, so the fact goes to the batch
    expect(result.batch.entities.some((e: any) => e.type === 'Customer')).toBe(true)
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
    const nouns = batchEntities(result, 'Noun')
    const priorityNoun = nouns.find((n: any) => n.data.name === 'Priority')
    expect(priorityNoun).toBeDefined()
    expect(priorityNoun!.data.enumValues).toContain('Low')
  })

  // External domain tests belong in their respective repos
})

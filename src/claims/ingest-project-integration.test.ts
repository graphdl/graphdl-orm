import { describe, it, expect, vi } from 'vitest'
import { ingestProject } from './ingest'
import { parseFORML2 } from '../api/parse'
import * as fs from 'fs'
import * as path from 'path'

// ---------------------------------------------------------------------------
// Mock DB (same pattern as steps.test.ts)
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
// Integration test: support.auto.dev domains
// ---------------------------------------------------------------------------

describe('ingestProject integration: support.auto.dev', () => {
  it('ingests all domains and resolves cross-domain status display colors', async () => {
    const domainsDir = path.resolve(__dirname, '../../../support.auto.dev/domains')
    if (!fs.existsSync(domainsDir)) {
      console.log('Skipping: support.auto.dev not found at', domainsDir)
      return
    }

    const domainFiles = fs.readdirSync(domainsDir).filter(f => f.endsWith('.md'))
    const domains = domainFiles.map(file => {
      const text = fs.readFileSync(path.join(domainsDir, file), 'utf-8')
      const slug = `support-auto-dev-${file.replace('.md', '')}`
      const claims = parseFORML2(text, [])
      return { domainId: slug, claims }
    })

    const db = mockDb()
    const result = await ingestProject(db as any, domains)

    // Constraint-reading resolution errors are expected: the parser emits
    // constraints whose reading could not be matched to a fact-type (e.g.
    // deontic constraints referencing unresolved readings).  Only non-
    // constraint errors would indicate a real problem.
    const constraintReadingErrors = result.totals.errors.filter(
      e => /constraint: reading ".*" not found/.test(e),
    )
    const unexpectedErrors = result.totals.errors.filter(
      e => !/constraint: reading ".*" not found/.test(e),
    )

    if (unexpectedErrors.length > 0) {
      console.log('Unexpected errors:', unexpectedErrors)
    }
    expect(unexpectedErrors).toHaveLength(0)

    // Log constraint resolution errors for visibility
    if (constraintReadingErrors.length > 0) {
      console.log(`${constraintReadingErrors.length} constraint-reading resolution errors (expected)`)
    }

    // Should have created nouns across multiple domains
    expect(result.totals.nouns).toBeGreaterThan(10)

    // Status display color facts should have been created
    expect(db.createEntity).toHaveBeenCalled()
    const statusCalls = db.createEntity.mock.calls.filter(
      ([_d, noun]: [string, string]) => noun === 'Status'
    )
    expect(statusCalls.length).toBeGreaterThanOrEqual(4)
  })
})

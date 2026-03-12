import { describe, it, expect, vi } from 'vitest'
import { createWithHook, type HookContext } from './index'

// Import to ensure hooks are registered via side-effects
import './nouns'
import './readings'
import './constraints'
import './state-machines'

/**
 * Integration test using a mock DB that accumulates state.
 * Verifies that creating a Reading with indented constraints
 * triggers the full hook chain.
 */
function statefulMockDb() {
  const store: Record<string, Record<string, any>[]> = {
    nouns: [],
    'graph-schemas': [],
    readings: [],
    roles: [],
    constraints: [],
    'constraint-spans': [],
  }
  let counter = 0

  return {
    store,
    findInCollection: vi.fn(async (collection: string, where: any, opts?: any) => {
      const docs = store[collection] || []
      // Simple filtering by where clauses
      const filtered = docs.filter(doc => {
        for (const [field, condition] of Object.entries(where)) {
          const cond = condition as any
          if (cond.equals !== undefined) {
            // Map Payload field names to possible stored field names
            const val = doc[field] ?? doc[field.replace('_id', '')] ?? doc[field.replace(/_/g, '')]
            if (val !== cond.equals) return false
          }
        }
        return true
      })
      return { docs: filtered, totalDocs: filtered.length }
    }),
    createInCollection: vi.fn(async (collection: string, body: any) => {
      const id = `${collection}-${++counter}`
      const doc = { id, ...body }
      if (!store[collection]) store[collection] = []
      store[collection].push(doc)
      return doc
    }),
    updateInCollection: vi.fn(async (collection: string, id: string, body: any) => {
      const coll = store[collection] || []
      const idx = coll.findIndex(d => d.id === id)
      if (idx >= 0) Object.assign(coll[idx], body)
      return coll[idx] || { id, ...body }
    }),
  }
}

describe('Hook composition integration', () => {
  it('Reading with indented constraint creates full object graph', async () => {
    const db = statefulMockDb()
    const context: HookContext = { domainId: 'd1', allNouns: [] }

    // Simulate what the POST handler does
    const readingData = {
      text: 'Customer has Name.\n  Each Customer has at most one Name.',
      domain: 'd1',
    }

    const { doc, hookResult } = await createWithHook(db, 'readings', readingData, context)

    // Reading was created
    expect(doc.text).toContain('Customer has Name')

    // Nouns were created
    expect(db.store['nouns'].length).toBe(2)
    const nounNames = db.store['nouns'].map(n => n.name).sort()
    expect(nounNames).toEqual(['Customer', 'Name'])

    // Graph schema was created
    expect(db.store['graph-schemas'].length).toBe(1)
    expect(db.store['graph-schemas'][0].name).toBe('CustomerName')

    // Roles were created
    expect(db.store['roles'].length).toBe(2)

    // Constraint was created (delegated from reading hook)
    expect(db.store['constraints'].length).toBeGreaterThanOrEqual(1)
  })
})

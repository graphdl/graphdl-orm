import { describe, it, expect, vi } from 'vitest'
import type { GraphDLDBLike } from './do-adapter'
import { createDomainAdapter } from './do-adapter'
import { ingestNouns } from './claims/steps'
import { createScope } from './claims/scope'

// ---------------------------------------------------------------------------
// Mock target (simulates a DomainDB RPC stub or any GraphDLDBLike)
// ---------------------------------------------------------------------------

function mockTarget() {
  const store: Record<string, any[]> = {}
  let idCounter = 0

  return {
    store,
    findInCollection: vi.fn(async (collection: string, where: any, _opts?: any) => {
      const all = store[collection] || []
      const filtered = all.filter((doc: any) => {
        for (const [key, cond] of Object.entries(where)) {
          if (typeof cond === 'object' && cond !== null && 'equals' in (cond as any)) {
            const fieldVal = doc[key]
            if (fieldVal !== (cond as any).equals) return false
          }
        }
        return true
      })
      return { docs: filtered, totalDocs: filtered.length }
    }),
    createInCollection: vi.fn(async (collection: string, data: any) => {
      const doc = { id: `id-${++idCounter}`, ...data }
      if (!store[collection]) store[collection] = []
      store[collection].push(doc)
      return doc
    }),
    updateInCollection: vi.fn(async (collection: string, id: string, updates: any) => {
      const coll = store[collection] || []
      const doc = coll.find((d: any) => d.id === id)
      if (doc) Object.assign(doc, updates)
      return doc ?? null
    }),
  } satisfies GraphDLDBLike & { store: Record<string, any[]> }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

describe('createDomainAdapter', () => {
  it('routes findInCollection to the target', async () => {
    const target = mockTarget()
    const adapter = createDomainAdapter(target)

    target.store['nouns'] = [{ id: 'n1', name: 'Customer', domain: 'd1' }]
    const result = await adapter.findInCollection('nouns', { name: { equals: 'Customer' } }, { limit: 1 })

    expect(result.docs).toHaveLength(1)
    expect(result.docs[0].name).toBe('Customer')
    expect(target.findInCollection).toHaveBeenCalledWith('nouns', { name: { equals: 'Customer' } }, { limit: 1 })
  })

  it('routes createInCollection to the target', async () => {
    const target = mockTarget()
    const adapter = createDomainAdapter(target)

    const doc = await adapter.createInCollection('nouns', { name: 'Order', domain: 'd1' })

    expect(doc).toHaveProperty('id')
    expect(doc.name).toBe('Order')
    expect(target.createInCollection).toHaveBeenCalledWith('nouns', { name: 'Order', domain: 'd1' })
  })

  it('routes updateInCollection to the target', async () => {
    const target = mockTarget()
    const adapter = createDomainAdapter(target)

    // seed a record
    target.store['nouns'] = [{ id: 'n1', name: 'Customer', domain: 'd1' }]
    const updated = await adapter.updateInCollection('nouns', 'n1', { objectType: 'entity' })

    expect(updated).toMatchObject({ id: 'n1', objectType: 'entity' })
    expect(target.updateInCollection).toHaveBeenCalledWith('nouns', 'n1', { objectType: 'entity' })
  })
})

// ---------------------------------------------------------------------------
// Integration test: step functions work through the adapter
// ---------------------------------------------------------------------------

describe('DO adapter integration', () => {
  it('ingestNouns works through the adapter', async () => {
    const target = mockTarget()
    const adapter = createDomainAdapter(target)
    const scope = createScope()

    const nouns = [
      { name: 'Customer', objectType: 'entity' as const },
      { name: 'Name', objectType: 'value' as const, valueType: 'string' },
    ]

    const count = await ingestNouns(adapter as any, nouns, 'd1', scope)

    expect(count).toBe(2)
    expect(target.store['nouns']).toHaveLength(2)
    expect(scope.nouns.size).toBe(2)
    expect(scope.nouns.get('d1:Customer')).toBeDefined()
    expect(scope.nouns.get('d1:Name')).toBeDefined()
  })
})

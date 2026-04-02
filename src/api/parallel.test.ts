/**
 * Parallelization tests — Definition 2 (Cell Isolation).
 *
 * Concurrent μ applications over disjoint cells are permitted.
 * Each cell folds its stream independently: D'_n = foldl μ_n D_n E_n.
 * These tests verify that parallel operations on disjoint entities
 * produce correct, independent results.
 */

import { describe, it, expect, vi } from 'vitest'
import { reconstructIR } from './engine'

/** In-memory cell store simulating Durable Object storage. */
function createCellStore() {
  const cells = new Map<string, any>()
  // Index by (type, domain) → set of entity IDs
  const domainTypeIndex = new Map<string, Set<string>>()

  function indexKey(type: string, domain: string) { return `${type}::${domain}` }

  return {
    cells,
    getStub: (id: string) => ({
      get: vi.fn(async () => cells.get(id) || null),
      put: vi.fn(async (data: any) => {
        cells.set(id, { id, ...data })
        const type = data.type || 'unknown'
        const domain = data.domain || ''
        const key = indexKey(type, domain)
        if (!domainTypeIndex.has(key)) domainTypeIndex.set(key, new Set())
        domainTypeIndex.get(key)!.add(id)
      }),
    }),
    registry: {
      getEntityIds: vi.fn(async (type: string, domain: string) =>
        Array.from(domainTypeIndex.get(indexKey(type, domain)) || []),
      ),
      materializeBatch: vi.fn(async (entities: any[]) => {
        for (const e of entities) {
          cells.set(e.id, e)
          const type = e.type || 'unknown'
          const domain = e.domain || e.data?.domain || ''
          const key = indexKey(type, domain)
          if (!domainTypeIndex.has(key)) domainTypeIndex.set(key, new Set())
          domainTypeIndex.get(key)!.add(e.id)
        }
      }),
    },
  }
}

/** Seed a domain's entities into the cell store. */
async function seedEntities(store: ReturnType<typeof createCellStore>, entities: any[]) {
  await store.registry.materializeBatch(entities)
}

describe('Cell Isolation (Definition 2)', () => {
  it('parallel seeds of disjoint domains produce independent results', async () => {
    const store = createCellStore()

    const domainA = [
      { id: 'Customer', type: 'Noun', domain: 'sales', data: { name: 'Customer', objectType: 'entity', domain: 'sales' } },
      { id: 'Order', type: 'Noun', domain: 'sales', data: { name: 'Order', objectType: 'entity', domain: 'sales' } },
      { id: 'Order_placedBy_Customer', type: 'Graph Schema', domain: 'sales', data: { name: 'Order_placedBy_Customer', reading: 'Order was placed by Customer', arity: 2, domain: 'sales' } },
      { id: 'Order_placedBy_Customer:role:0', type: 'Role', domain: 'sales', data: { nounName: 'Order', position: 0, graphSchema: 'Order_placedBy_Customer', domain: 'sales' } },
      { id: 'Order_placedBy_Customer:role:1', type: 'Role', domain: 'sales', data: { nounName: 'Customer', position: 1, graphSchema: 'Order_placedBy_Customer', domain: 'sales' } },
    ]

    const domainB = [
      { id: 'Vehicle', type: 'Noun', domain: 'fleet', data: { name: 'Vehicle', objectType: 'entity', domain: 'fleet' } },
      { id: 'Driver', type: 'Noun', domain: 'fleet', data: { name: 'Driver', objectType: 'entity', domain: 'fleet' } },
      { id: 'Vehicle_assignedTo_Driver', type: 'Graph Schema', domain: 'fleet', data: { name: 'Vehicle_assignedTo_Driver', reading: 'Vehicle is assigned to Driver', arity: 2, domain: 'fleet' } },
      { id: 'Vehicle_assignedTo_Driver:role:0', type: 'Role', domain: 'fleet', data: { nounName: 'Vehicle', position: 0, graphSchema: 'Vehicle_assignedTo_Driver', domain: 'fleet' } },
      { id: 'Vehicle_assignedTo_Driver:role:1', type: 'Role', domain: 'fleet', data: { nounName: 'Driver', position: 1, graphSchema: 'Vehicle_assignedTo_Driver', domain: 'fleet' } },
    ]

    // Seed both domains in parallel
    await Promise.all([
      seedEntities(store, domainA),
      seedEntities(store, domainB),
    ])

    // Reconstruct both domains in parallel
    const [irA, irB] = await Promise.all([
      reconstructIR(store.registry, store.getStub, 'sales'),
      reconstructIR(store.registry, store.getStub, 'fleet'),
    ])

    const parsedA = JSON.parse(irA!)
    const parsedB = JSON.parse(irB!)

    // Each domain sees only its own nouns and fact types
    expect(Object.keys(parsedA.nouns)).toContain('Customer')
    expect(Object.keys(parsedA.nouns)).toContain('Order')
    expect(Object.keys(parsedA.nouns)).not.toContain('Vehicle')

    expect(Object.keys(parsedB.nouns)).toContain('Vehicle')
    expect(Object.keys(parsedB.nouns)).toContain('Driver')
    expect(Object.keys(parsedB.nouns)).not.toContain('Customer')

    expect(Object.keys(parsedA.factTypes)).toContain('Order_placedBy_Customer')
    expect(Object.keys(parsedB.factTypes)).toContain('Vehicle_assignedTo_Driver')
  })

  it('parallel reconstructIR calls do not interfere', async () => {
    const store = createCellStore()

    const entities = [
      { id: 'Person', type: 'Noun', domain: 'hr', data: { name: 'Person', objectType: 'entity', domain: 'hr' } },
      { id: 'Role', type: 'Noun', domain: 'hr', data: { name: 'Role', objectType: 'entity', domain: 'hr' } },
    ]
    await seedEntities(store, entities)

    // Call reconstructIR 10 times in parallel on the same domain
    const results = await Promise.all(
      Array.from({ length: 10 }, () => reconstructIR(store.registry, store.getStub, 'hr')),
    )

    // All 10 results should be identical
    const first = results[0]
    for (const r of results) {
      expect(r).toBe(first)
    }

    const parsed = JSON.parse(first!)
    expect(Object.keys(parsed.nouns)).toHaveLength(2)
  })

  it('concurrent entity writes to disjoint cells produce consistent state', async () => {
    const store = createCellStore()

    // Simulate 50 parallel entity writes to different cells
    const writes = Array.from({ length: 50 }, (_, i) => {
      const id = `entity-${i}`
      return store.getStub(id).put({
        type: 'Noun',
        domain: 'load',
        data: { name: `Noun${i}`, objectType: 'entity', domain: 'load' },
      })
    })

    await Promise.all(writes)

    // All 50 cells should exist with correct data
    for (let i = 0; i < 50; i++) {
      const cell = await store.getStub(`entity-${i}`).get()
      expect(cell).not.toBeNull()
      expect(cell.data.name).toBe(`Noun${i}`)
    }
  })

  it('DEFS registration from parallel Instance Fact queries is deterministic', async () => {
    const store = createCellStore()

    // Seed instance facts for multiple backed nouns
    const instanceFacts = [
      { id: 'if:1', type: 'Instance Fact', domain: 'api', data: { subjectNoun: 'Noun', subjectValue: 'Customer', fieldName: 'is backed by', objectNoun: 'External System', objectValue: 'auth.vin' } },
      { id: 'if:2', type: 'Instance Fact', domain: 'api', data: { subjectNoun: 'Noun', subjectValue: 'Vehicle', fieldName: 'is backed by', objectNoun: 'External System', objectValue: 'auto.dev' } },
      { id: 'if:3', type: 'Instance Fact', domain: 'api', data: { subjectNoun: 'Noun', subjectValue: 'API Product', fieldName: 'URI', objectNoun: '', objectValue: '/api' } },
    ]
    const nouns = [
      { id: 'Customer', type: 'Noun', domain: 'api', data: { name: 'Customer', objectType: 'entity', domain: 'api' } },
      { id: 'Vehicle', type: 'Noun', domain: 'api', data: { name: 'Vehicle', objectType: 'entity', domain: 'api' } },
      { id: 'API Product', type: 'Noun', domain: 'api', data: { name: 'API Product', objectType: 'entity', domain: 'api' } },
    ]
    await seedEntities(store, [...nouns, ...instanceFacts])

    // Reconstruct IR 5 times in parallel — all should produce same instance facts
    const results = await Promise.all(
      Array.from({ length: 5 }, () => reconstructIR(store.registry, store.getStub, 'api')),
    )

    for (const r of results) {
      const ir = JSON.parse(r!)
      const backedFacts = ir.generalInstanceFacts.filter(
        (f: any) => f.objectNoun === 'External System',
      )
      expect(backedFacts).toHaveLength(2)
      expect(backedFacts.map((f: any) => f.subjectValue).sort()).toEqual(['Customer', 'Vehicle'])
    }
  })
})

describe('Two-way data binding (Section 5.2)', () => {
  it('ρ-application re-evaluates when cell contents change via ↓', async () => {
    const store = createCellStore()
    const notifications: string[] = []

    // Bound function: (ρ fact) : render → widget
    // Re-evaluates on each ↓ (store) to the cell.
    const boundRender = (cellId: string, data: any) => {
      notifications.push(`render:${cellId}:${data.data?.status || 'unknown'}`)
    }

    // ↓ store: Order in "In Cart"
    const stub = store.getStub('ord-1')
    await stub.put({ type: 'Order', domain: 'test', data: { customer: 'acme', status: 'In Cart' } })
    const initial = await stub.get()
    boundRender('ord-1', initial)

    expect(notifications).toEqual(['render:ord-1:In Cart'])

    // ↓ store: transition to "Placed"
    await stub.put({ type: 'Order', domain: 'test', data: { customer: 'acme', status: 'Placed' } })
    const updated = await stub.get()
    boundRender('ord-1', updated)

    expect(notifications).toEqual(['render:ord-1:In Cart', 'render:ord-1:Placed'])
  })

  it('bound functions on disjoint cells fire independently', async () => {
    const store = createCellStore()
    const log: string[] = []

    const bind = (id: string) => async () => {
      const cell = await store.getStub(id).get()
      if (cell) log.push(`${id}:${cell.data?.status || 'init'}`)
    }

    const bindA = bind('order-1')
    const bindB = bind('order-2')

    await store.getStub('order-1').put({ type: 'Order', domain: 'test', data: { status: 'In Cart' } })
    await store.getStub('order-2').put({ type: 'Order', domain: 'test', data: { status: 'In Cart' } })

    // Both bindings fire in parallel
    await Promise.all([bindA(), bindB()])
    expect(log).toContain('order-1:In Cart')
    expect(log).toContain('order-2:In Cart')

    // Update only order-1
    await store.getStub('order-1').put({ type: 'Order', domain: 'test', data: { status: 'Placed' } })
    log.length = 0
    await bindA()
    expect(log).toEqual(['order-1:Placed'])
  })

  it('representation contains derived facts from reconstructed schema', async () => {
    const store = createCellStore()

    const entities = [
      { id: 'Customer', type: 'Noun', domain: 'crm', data: { name: 'Customer', objectType: 'entity', domain: 'crm' } },
      { id: 'FirstName', type: 'Noun', domain: 'crm', data: { name: 'FirstName', objectType: 'value', domain: 'crm' } },
      { id: 'Customer_has_FirstName', type: 'Graph Schema', domain: 'crm', data: { name: 'Customer_has_FirstName', reading: 'Customer has FirstName', arity: 2, domain: 'crm' } },
      { id: 'Customer_has_FirstName:role:0', type: 'Role', domain: 'crm', data: { nounName: 'Customer', position: 0, graphSchema: 'Customer_has_FirstName', domain: 'crm' } },
      { id: 'Customer_has_FirstName:role:1', type: 'Role', domain: 'crm', data: { nounName: 'FirstName', position: 1, graphSchema: 'Customer_has_FirstName', domain: 'crm' } },
    ]
    await seedEntities(store, entities)

    const ir = JSON.parse((await reconstructIR(store.registry, store.getStub, 'crm'))!)

    expect(ir.nouns.Customer).toBeDefined()
    expect(ir.factTypes.Customer_has_FirstName).toBeDefined()
    expect(ir.factTypes.Customer_has_FirstName.roles[0].nounName).toBe('Customer')
    expect(ir.factTypes.Customer_has_FirstName.roles[1].nounName).toBe('FirstName')
  })
})

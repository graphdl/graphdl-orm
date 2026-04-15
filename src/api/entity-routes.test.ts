import { describe, it, expect, vi } from 'vitest'
import {
  handleListEntities,
  handleGetEntity,
  handleCreateEntity,
  handleDeleteEntity,
  populateDepthForEntity,
} from './entity-routes'

describe('entity-routes', () => {
  // ── handleListEntities ──────────────────────────────────────────────

  it('handleListEntities returns entities by type from Registry fan-out', async () => {
    const registry = { getEntityIds: vi.fn().mockResolvedValue(['e1', 'e2']) }
    const entities = new Map([
      ['e1', { get: vi.fn().mockResolvedValue({ id: 'e1', type: 'Noun', data: { name: 'Customer' } }) }],
      ['e2', { get: vi.fn().mockResolvedValue({ id: 'e2', type: 'Noun', data: { name: 'Order' } }) }],
    ])
    const result = await handleListEntities('Noun', 'tickets', registry, (id) => entities.get(id)!)
    expect(result.docs).toHaveLength(2)
    expect(result.totalDocs).toBe(2)
  })

  it('handleGetEntity returns single entity by ID', async () => {
    const stub = { get: vi.fn().mockResolvedValue({ id: 'e1', type: 'Noun', data: { name: 'Customer' } }) }
    const result = await handleGetEntity(stub)
    expect(result).toBeDefined()
    expect(result!.data.name).toBe('Customer')
  })

  it('handleListEntities filters by domain', async () => {
    const registry = { getEntityIds: vi.fn().mockResolvedValue(['e1']) }
    const entities = new Map([
      ['e1', { get: vi.fn().mockResolvedValue({ id: 'e1', type: 'Noun', data: { name: 'Customer' } }) }],
    ])
    await handleListEntities('Noun', 'tickets', registry, (id) => entities.get(id)!)
    expect(registry.getEntityIds).toHaveBeenCalledWith('Noun', 'tickets')
  })

  it('handleListEntities returns warnings for unreachable DOs', async () => {
    const registry = { getEntityIds: vi.fn().mockResolvedValue(['e1', 'e2']) }
    const entities = new Map([
      ['e1', { get: vi.fn().mockResolvedValue({ id: 'e1', type: 'Noun', data: { name: 'A' } }) }],
      ['e2', { get: vi.fn().mockRejectedValue(new Error('DO unreachable')) }],
    ])
    const result = await handleListEntities('Noun', 'tickets', registry, (id) => entities.get(id)!)
    expect(result.docs).toHaveLength(1)
    expect(result.warnings).toContain('e2')
  })

  it('handleListEntities paginates with limit and page', async () => {
    const ids = Array.from({ length: 5 }, (_, i) => `e${i}`)
    const registry = { getEntityIds: vi.fn().mockResolvedValue(ids) }
    const entities = new Map(
      ids.map((id) => [
        id,
        { get: vi.fn().mockResolvedValue({ id, type: 'Noun', data: { name: `N${id}` } }) },
      ]),
    )
    const result = await handleListEntities('Noun', 'tickets', registry, (id) => entities.get(id)!, { limit: 2, page: 2 })
    expect(result.docs).toHaveLength(2)
    expect(result.totalDocs).toBe(5)
    expect(result.page).toBe(2)
    expect(result.totalPages).toBe(3)
    expect(result.hasNextPage).toBe(true)
    expect(result.hasPrevPage).toBe(true)
  })

  it('handleListEntities defaults to limit=100, page=1', async () => {
    const registry = { getEntityIds: vi.fn().mockResolvedValue(['e1']) }
    const entities = new Map([
      ['e1', { get: vi.fn().mockResolvedValue({ id: 'e1', type: 'Noun', data: { name: 'A' } }) }],
    ])
    const result = await handleListEntities('Noun', 'tickets', registry, (id) => entities.get(id)!)
    expect(result.limit).toBe(100)
    expect(result.page).toBe(1)
  })

  it('handleListEntities includes all cells from registry (no soft-delete filtering)', async () => {
    const registry = { getEntityIds: vi.fn().mockResolvedValue(['e1', 'e2']) }
    const entities = new Map([
      ['e1', { get: vi.fn().mockResolvedValue({ id: 'e1', type: 'Noun', data: { name: 'A' } }) }],
      ['e2', { get: vi.fn().mockResolvedValue({ id: 'e2', type: 'Noun', data: { name: 'B' } }) }],
    ])
    const result = await handleListEntities('Noun', 'tickets', registry, (id) => entities.get(id)!)
    expect(result.docs).toHaveLength(2)
  })

  // ── handleGetEntity ─────────────────────────────────────────────────

  it('handleGetEntity returns null when cell not found', async () => {
    const stub = { get: vi.fn().mockResolvedValue(null) }
    const result = await handleGetEntity(stub)
    expect(result).toBeNull()
  })

  // ── handleCreateEntity ──────────────────────────────────────────────

  it('handleCreateEntity creates cell and indexes in registry', async () => {
    const stub = { put: vi.fn().mockResolvedValue({ id: 'new1', type: 'Noun', data: { name: 'Customer' } }), delete: vi.fn() }
    const registry = { indexEntity: vi.fn().mockResolvedValue(undefined), deindexEntity: vi.fn().mockResolvedValue(undefined) }
    const result = await handleCreateEntity('Noun', 'tickets', { name: 'Customer' }, () => stub, registry)
    expect(result.id).toBeDefined()
    expect(result.type).toBe('Noun')
    expect(stub.put).toHaveBeenCalled()
    expect(registry.indexEntity).toHaveBeenCalledWith('Noun', expect.any(String), 'tickets')
  })

  // ── handleDeleteEntity ──────────────────────────────────────────────

  it('handleDeleteEntity removes cell and deindexes', async () => {
    const stub = { delete: vi.fn().mockResolvedValue({ id: 'e1' }), put: vi.fn() }
    const registry = { indexEntity: vi.fn().mockResolvedValue(undefined), deindexEntity: vi.fn().mockResolvedValue(undefined) }
    const result = await handleDeleteEntity('e1', stub, registry, 'Noun')
    expect(result).toEqual({ id: 'e1', deleted: true })
    expect(stub.delete).toHaveBeenCalled()
    expect(registry.deindexEntity).toHaveBeenCalledWith('Noun', 'e1')
  })

  it('handleDeleteEntity returns null when cell not found', async () => {
    const stub = { delete: vi.fn().mockResolvedValue(null), put: vi.fn() }
    const registry = { indexEntity: vi.fn().mockResolvedValue(undefined), deindexEntity: vi.fn().mockResolvedValue(undefined) }
    const result = await handleDeleteEntity('e1', stub, registry, 'Noun')
    expect(result).toBeNull()
    expect(registry.deindexEntity).not.toHaveBeenCalled()
  })

  // ── Broadcast hooks ─────────────────────────────────────────────────

  it('handleCreateEntity fires a create event on broadcast when provided', async () => {
    const stub = { put: vi.fn().mockResolvedValue({ id: 'ord-1', type: 'Order', data: { total: 10 } }), delete: vi.fn() }
    const registry = { indexEntity: vi.fn().mockResolvedValue(undefined), deindexEntity: vi.fn().mockResolvedValue(undefined) }
    const broadcast = { publish: vi.fn().mockResolvedValue(undefined) }
    await handleCreateEntity('Order', 'orders', { total: 10 }, () => stub, registry, 'ord-1', broadcast)
    expect(broadcast.publish).toHaveBeenCalledWith(expect.objectContaining({
      domain: 'orders',
      noun: 'Order',
      entityId: 'ord-1',
      operation: 'create',
      facts: { total: 10 },
    }))
  })

  it('handleCreateEntity tolerates broadcast failure — mutation still succeeds', async () => {
    const stub = { put: vi.fn().mockResolvedValue({ id: 'x', type: 'Noun', data: {} }), delete: vi.fn() }
    const registry = { indexEntity: vi.fn().mockResolvedValue(undefined), deindexEntity: vi.fn().mockResolvedValue(undefined) }
    const broadcast = { publish: vi.fn().mockRejectedValue(new Error('do down')) }
    const result = await handleCreateEntity('Noun', 'tickets', {}, () => stub, registry, 'x', broadcast)
    expect(result).toEqual({ id: 'x', type: 'Noun' })
  })

  it('handleCreateEntity skips broadcast when the stub is omitted', async () => {
    const stub = { put: vi.fn().mockResolvedValue({ id: 'a', type: 'Noun', data: {} }), delete: vi.fn() }
    const registry = { indexEntity: vi.fn().mockResolvedValue(undefined), deindexEntity: vi.fn().mockResolvedValue(undefined) }
    // No broadcast arg. Code path must not reference it.
    const result = await handleCreateEntity('Noun', 'x', {}, () => stub, registry)
    expect(result.type).toBe('Noun')
    expect(typeof result.id).toBe('string')
  })

  it('handleDeleteEntity fires a delete event on broadcast when provided', async () => {
    const stub = { delete: vi.fn().mockResolvedValue({ id: 'gone' }), put: vi.fn() }
    const registry = { indexEntity: vi.fn().mockResolvedValue(undefined), deindexEntity: vi.fn().mockResolvedValue(undefined) }
    const broadcast = { publish: vi.fn().mockResolvedValue(undefined) }
    await handleDeleteEntity('gone', stub, registry, 'Ticket', 'support', broadcast)
    expect(broadcast.publish).toHaveBeenCalledWith(expect.objectContaining({
      domain: 'support',
      noun: 'Ticket',
      entityId: 'gone',
      operation: 'delete',
    }))
  })

  it('handleDeleteEntity does NOT publish when cell was not found', async () => {
    const stub = { delete: vi.fn().mockResolvedValue(null), put: vi.fn() }
    const registry = { indexEntity: vi.fn().mockResolvedValue(undefined), deindexEntity: vi.fn().mockResolvedValue(undefined) }
    const broadcast = { publish: vi.fn().mockResolvedValue(undefined) }
    await handleDeleteEntity('nope', stub, registry, 'Noun', 'x', broadcast)
    expect(broadcast.publish).not.toHaveBeenCalled()
  })

  // ── populateDepthForEntity ──────────────────────────────────────────

  it('populateDepthForEntity resolves Id fields at depth=1', async () => {
    const entity = { id: 'n1', type: 'Noun', data: { name: 'Customer', factTypeId: 'gs1' } }
    const refStub = {
      get: vi.fn().mockResolvedValue({ id: 'gs1', type: 'FactType', data: { name: 'Tickets' } }),
    }
    const getStub = vi.fn().mockReturnValue(refStub)

    const populated = await populateDepthForEntity(entity, 1, getStub)

    expect(populated.factTypeId).toBe('gs1')
    expect(populated.factType).toEqual({ id: 'gs1', name: 'Tickets' })
    expect(getStub).toHaveBeenCalledWith('gs1')
  })

  it('populateDepthForEntity leaves data unchanged when no Id fields', async () => {
    const entity = { id: 'n1', type: 'Noun', data: { name: 'Customer', label: 'cust' } }
    const getStub = vi.fn()

    const populated = await populateDepthForEntity(entity, 1, getStub)

    expect(populated).toEqual({ name: 'Customer', label: 'cust' })
    expect(getStub).not.toHaveBeenCalled()
  })

  it('populateDepthForEntity leaves ID as-is when reference unreachable', async () => {
    const entity = { id: 'n1', type: 'Noun', data: { name: 'Customer', factTypeId: 'gs-gone' } }
    const refStub = {
      get: vi.fn().mockRejectedValue(new Error('DO unreachable')),
    }
    const getStub = vi.fn().mockReturnValue(refStub)

    const populated = await populateDepthForEntity(entity, 1, getStub)

    expect(populated.factTypeId).toBe('gs-gone')
    expect(populated.factType).toBeUndefined()
  })

  it('populateDepthForEntity returns data as-is at depth=0', async () => {
    const entity = { id: 'n1', type: 'Noun', data: { name: 'Customer', factTypeId: 'gs1' } }
    const getStub = vi.fn()

    const populated = await populateDepthForEntity(entity, 0, getStub)

    expect(populated).toEqual({ name: 'Customer', factTypeId: 'gs1' })
    expect(populated.factType).toBeUndefined()
    expect(getStub).not.toHaveBeenCalled()
  })

  it('populateDepthForEntity skips non-string Id fields', async () => {
    const entity = { id: 'n1', type: 'Noun', data: { name: 'Customer', someId: 42 } }
    const getStub = vi.fn()

    const populated = await populateDepthForEntity(entity, 1, getStub)

    expect(populated).toEqual({ name: 'Customer', someId: 42 })
    expect(getStub).not.toHaveBeenCalled()
  })

  it('populateDepthForEntity handles ref returning null (removed cell)', async () => {
    const entity = { id: 'n1', type: 'Noun', data: { name: 'Customer', factTypeId: 'gs-removed' } }
    const refStub = { get: vi.fn().mockResolvedValue(null) }
    const getStub = vi.fn().mockReturnValue(refStub)

    const populated = await populateDepthForEntity(entity, 1, getStub)

    expect(populated.factTypeId).toBe('gs-removed')
    expect(populated.factType).toBeUndefined()
  })

  // ── handleListEntities with depth ───────────────────────────────────

  it('handleListEntities populates depth=1 references in docs', async () => {
    const registry = { getEntityIds: vi.fn().mockResolvedValue(['n1']) }
    const nounStub = {
      get: vi.fn().mockResolvedValue({ id: 'n1', type: 'Noun', data: { name: 'Customer', factTypeId: 'gs1' } }),
    }
    const schemaStub = {
      get: vi.fn().mockResolvedValue({ id: 'gs1', type: 'FactType', data: { name: 'Tickets' } }),
    }
    const stubs = new Map<string, any>([
      ['n1', nounStub],
      ['gs1', schemaStub],
    ])

    const result = await handleListEntities('Noun', 'tickets', registry, (id) => stubs.get(id)!, { depth: 1 })

    expect(result.docs[0].data.factType).toEqual({ id: 'gs1', name: 'Tickets' })
    expect(result.docs[0].data.factTypeId).toBe('gs1')
  })

  it('handleGetEntity populates depth=1 references', async () => {
    const nounStub = {
      get: vi.fn().mockResolvedValue({ id: 'n1', type: 'Noun', data: { name: 'Customer', factTypeId: 'gs1' } }),
    }
    const schemaStub = {
      get: vi.fn().mockResolvedValue({ id: 'gs1', type: 'FactType', data: { name: 'Tickets' } }),
    }
    const stubs = new Map<string, any>([
      ['gs1', schemaStub],
    ])

    const result = await handleGetEntity(nounStub, { depth: 1, getStub: (id) => stubs.get(id)! })

    expect(result!.data.factType).toEqual({ id: 'gs1', name: 'Tickets' })
  })

})

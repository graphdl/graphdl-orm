import { describe, it, expect, vi } from 'vitest'
import { handleListEntities, handleGetEntity, handleCreateEntity, handleDeleteEntity } from './entity-routes'

describe('entity-routes', () => {
  // ── handleListEntities ──────────────────────────────────────────────

  it('handleListEntities returns entities by type from Registry fan-out', async () => {
    const registry = { getEntityIds: vi.fn().mockResolvedValue(['e1', 'e2']) }
    const entities = new Map([
      ['e1', { get: vi.fn().mockResolvedValue({ id: 'e1', type: 'Noun', data: { name: 'Customer' }, version: 1 }) }],
      ['e2', { get: vi.fn().mockResolvedValue({ id: 'e2', type: 'Noun', data: { name: 'Order' }, version: 1 }) }],
    ])
    const result = await handleListEntities('Noun', 'tickets', registry, (id) => entities.get(id)!)
    expect(result.docs).toHaveLength(2)
    expect(result.totalDocs).toBe(2)
  })

  it('handleGetEntity returns single entity by ID', async () => {
    const stub = { get: vi.fn().mockResolvedValue({ id: 'e1', type: 'Noun', data: { name: 'Customer' }, version: 1 }) }
    const result = await handleGetEntity(stub)
    expect(result).toBeDefined()
    expect(result!.data.name).toBe('Customer')
  })

  it('handleListEntities filters by domain', async () => {
    const registry = { getEntityIds: vi.fn().mockResolvedValue(['e1']) }
    const entities = new Map([
      ['e1', { get: vi.fn().mockResolvedValue({ id: 'e1', type: 'Noun', data: { name: 'Customer' }, version: 1 }) }],
    ])
    await handleListEntities('Noun', 'tickets', registry, (id) => entities.get(id)!)
    expect(registry.getEntityIds).toHaveBeenCalledWith('Noun', 'tickets')
  })

  it('handleListEntities returns warnings for unreachable DOs', async () => {
    const registry = { getEntityIds: vi.fn().mockResolvedValue(['e1', 'e2']) }
    const entities = new Map([
      ['e1', { get: vi.fn().mockResolvedValue({ id: 'e1', type: 'Noun', data: { name: 'A' }, version: 1 }) }],
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
        { get: vi.fn().mockResolvedValue({ id, type: 'Noun', data: { name: `N${id}` }, version: 1 }) },
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
      ['e1', { get: vi.fn().mockResolvedValue({ id: 'e1', type: 'Noun', data: { name: 'A' }, version: 1 }) }],
    ])
    const result = await handleListEntities('Noun', 'tickets', registry, (id) => entities.get(id)!)
    expect(result.limit).toBe(100)
    expect(result.page).toBe(1)
  })

  it('handleListEntities skips soft-deleted entities', async () => {
    const registry = { getEntityIds: vi.fn().mockResolvedValue(['e1', 'e2']) }
    const entities = new Map([
      ['e1', { get: vi.fn().mockResolvedValue({ id: 'e1', type: 'Noun', data: { name: 'A' }, version: 1 }) }],
      ['e2', { get: vi.fn().mockResolvedValue({ id: 'e2', type: 'Noun', data: { name: 'B' }, version: 1, deletedAt: '2024-01-01' }) }],
    ])
    const result = await handleListEntities('Noun', 'tickets', registry, (id) => entities.get(id)!)
    expect(result.docs).toHaveLength(1)
    expect(result.docs[0].id).toBe('e1')
  })

  // ── handleGetEntity ─────────────────────────────────────────────────

  it('handleGetEntity returns null for deleted entity', async () => {
    const stub = { get: vi.fn().mockResolvedValue({ id: 'e1', type: 'Noun', data: {}, version: 1, deletedAt: '2024-01-01' }) }
    const result = await handleGetEntity(stub)
    expect(result).toBeNull()
  })

  it('handleGetEntity returns null when entity not found', async () => {
    const stub = { get: vi.fn().mockResolvedValue(null) }
    const result = await handleGetEntity(stub)
    expect(result).toBeNull()
  })

  // ── handleCreateEntity ──────────────────────────────────────────────

  it('handleCreateEntity creates entity and indexes in registry', async () => {
    const stub = { put: vi.fn().mockResolvedValue({ id: 'new1', version: 1 }) }
    const registry = { indexEntity: vi.fn().mockResolvedValue(undefined) }
    const result = await handleCreateEntity('Noun', 'tickets', { name: 'Customer' }, () => stub, registry)
    expect(result.id).toBeDefined()
    expect(result.version).toBe(1)
    expect(stub.put).toHaveBeenCalled()
    expect(registry.indexEntity).toHaveBeenCalledWith('Noun', expect.any(String), 'tickets')
  })

  // ── handleDeleteEntity ──────────────────────────────────────────────

  it('handleDeleteEntity soft-deletes and deindexes', async () => {
    const stub = { delete: vi.fn().mockResolvedValue({ id: 'e1', deleted: true }) }
    const registry = { deindexEntity: vi.fn().mockResolvedValue(undefined) }
    const result = await handleDeleteEntity('e1', stub, registry, 'Noun')
    expect(result).toEqual({ id: 'e1', deleted: true })
    expect(stub.delete).toHaveBeenCalled()
    expect(registry.deindexEntity).toHaveBeenCalledWith('Noun', 'e1')
  })

  it('handleDeleteEntity returns null when entity not found', async () => {
    const stub = { delete: vi.fn().mockResolvedValue(null) }
    const registry = { deindexEntity: vi.fn().mockResolvedValue(undefined) }
    const result = await handleDeleteEntity('e1', stub, registry, 'Noun')
    expect(result).toBeNull()
    expect(registry.deindexEntity).not.toHaveBeenCalled()
  })
})

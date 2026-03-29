import { describe, it, expect, vi } from 'vitest'
import {
  handleListEntities,
  handleGetEntity,
  handleCreateEntity,
  handleDeleteEntity,
  populateDepthForEntity,
  buildEntityLinks,
} from './entity-routes'
type TransitionOption = { transitionId: string; event: string; eventTypeId: string; targetStatus: string; targetStatusId: string }

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

  it('handleGetEntity returns single entity by ID with _links', async () => {
    const stub = { get: vi.fn().mockResolvedValue({ id: 'e1', type: 'Noun', data: { name: 'Customer' }, version: 1 }) }
    const result = await handleGetEntity(stub)
    expect(result).toBeDefined()
    expect(result!.data.name).toBe('Customer')
    expect(result!._links).toBeDefined()
    expect(result!._links!.self).toEqual({ href: '/api/entities/Noun/e1' })
    expect(result!._links!.collection).toEqual({ href: '/api/entities/Noun' })
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

  // ── populateDepthForEntity ──────────────────────────────────────────

  it('populateDepthForEntity resolves Id fields at depth=1', async () => {
    const entity = { id: 'n1', type: 'Noun', data: { name: 'Customer', graphSchemaId: 'gs1' }, version: 1 }
    const refStub = {
      get: vi.fn().mockResolvedValue({ id: 'gs1', type: 'GraphSchema', data: { name: 'Tickets' }, version: 1 }),
    }
    const getStub = vi.fn().mockReturnValue(refStub)

    const populated = await populateDepthForEntity(entity, 1, getStub)

    expect(populated.graphSchemaId).toBe('gs1')
    expect(populated.graphSchema).toEqual({ id: 'gs1', name: 'Tickets' })
    expect(getStub).toHaveBeenCalledWith('gs1')
  })

  it('populateDepthForEntity leaves data unchanged when no Id fields', async () => {
    const entity = { id: 'n1', type: 'Noun', data: { name: 'Customer', label: 'cust' }, version: 1 }
    const getStub = vi.fn()

    const populated = await populateDepthForEntity(entity, 1, getStub)

    expect(populated).toEqual({ name: 'Customer', label: 'cust' })
    expect(getStub).not.toHaveBeenCalled()
  })

  it('populateDepthForEntity leaves ID as-is when reference unreachable', async () => {
    const entity = { id: 'n1', type: 'Noun', data: { name: 'Customer', graphSchemaId: 'gs-gone' }, version: 1 }
    const refStub = {
      get: vi.fn().mockRejectedValue(new Error('DO unreachable')),
    }
    const getStub = vi.fn().mockReturnValue(refStub)

    const populated = await populateDepthForEntity(entity, 1, getStub)

    expect(populated.graphSchemaId).toBe('gs-gone')
    expect(populated.graphSchema).toBeUndefined()
  })

  it('populateDepthForEntity returns data as-is at depth=0', async () => {
    const entity = { id: 'n1', type: 'Noun', data: { name: 'Customer', graphSchemaId: 'gs1' }, version: 1 }
    const getStub = vi.fn()

    const populated = await populateDepthForEntity(entity, 0, getStub)

    expect(populated).toEqual({ name: 'Customer', graphSchemaId: 'gs1' })
    expect(populated.graphSchema).toBeUndefined()
    expect(getStub).not.toHaveBeenCalled()
  })

  it('populateDepthForEntity skips non-string Id fields', async () => {
    const entity = { id: 'n1', type: 'Noun', data: { name: 'Customer', someId: 42 }, version: 1 }
    const getStub = vi.fn()

    const populated = await populateDepthForEntity(entity, 1, getStub)

    expect(populated).toEqual({ name: 'Customer', someId: 42 })
    expect(getStub).not.toHaveBeenCalled()
  })

  it('populateDepthForEntity handles ref returning null (deleted entity)', async () => {
    const entity = { id: 'n1', type: 'Noun', data: { name: 'Customer', graphSchemaId: 'gs-deleted' }, version: 1 }
    const refStub = { get: vi.fn().mockResolvedValue(null) }
    const getStub = vi.fn().mockReturnValue(refStub)

    const populated = await populateDepthForEntity(entity, 1, getStub)

    expect(populated.graphSchemaId).toBe('gs-deleted')
    expect(populated.graphSchema).toBeUndefined()
  })

  // ── handleListEntities with depth ───────────────────────────────────

  it('handleListEntities populates depth=1 references in docs', async () => {
    const registry = { getEntityIds: vi.fn().mockResolvedValue(['n1']) }
    const nounStub = {
      get: vi.fn().mockResolvedValue({ id: 'n1', type: 'Noun', data: { name: 'Customer', graphSchemaId: 'gs1' }, version: 1 }),
    }
    const schemaStub = {
      get: vi.fn().mockResolvedValue({ id: 'gs1', type: 'GraphSchema', data: { name: 'Tickets' }, version: 1 }),
    }
    const stubs = new Map<string, any>([
      ['n1', nounStub],
      ['gs1', schemaStub],
    ])

    const result = await handleListEntities('Noun', 'tickets', registry, (id) => stubs.get(id)!, { depth: 1 })

    expect(result.docs[0].data.graphSchema).toEqual({ id: 'gs1', name: 'Tickets' })
    expect(result.docs[0].data.graphSchemaId).toBe('gs1')
  })

  it('handleGetEntity populates depth=1 references', async () => {
    const nounStub = {
      get: vi.fn().mockResolvedValue({ id: 'n1', type: 'Noun', data: { name: 'Customer', graphSchemaId: 'gs1' }, version: 1 }),
    }
    const schemaStub = {
      get: vi.fn().mockResolvedValue({ id: 'gs1', type: 'GraphSchema', data: { name: 'Tickets' }, version: 1 }),
    }
    const stubs = new Map<string, any>([
      ['gs1', schemaStub],
    ])

    const result = await handleGetEntity(nounStub, { depth: 1, getStub: (id) => stubs.get(id)! })

    expect(result!.data.graphSchema).toEqual({ id: 'gs1', name: 'Tickets' })
  })

  // ── transitions folded into _links (AREST whitepaper) ─────────────

  describe('transitions folded into _links', () => {
    const mockTransitions: TransitionOption[] = [
      {
        transitionId: 't1',
        event: 'approve',
        eventTypeId: 'evt1',
        targetStatus: 'Approved',
        targetStatusId: 'status-approved',
      },
      {
        transitionId: 't2',
        event: 'reject',
        eventTypeId: 'evt2',
        targetStatus: 'Rejected',
        targetStatusId: 'status-rejected',
      },
    ]

    it('includes transition events as links when transitions provided', async () => {
      const stub = {
        get: vi.fn().mockResolvedValue({
          id: 'e1',
          type: 'SupportRequest',
          data: {
            title: 'Help',
            _status: 'Open',
            _statusId: 'status-open',
            _stateMachineDefinition: 'def1',
          },
          version: 1,
        }),
      }

      const result = await handleGetEntity(stub, {
        transitions: mockTransitions,
      })

      expect(result).toBeDefined()
      expect(result!._links).toBeDefined()
      expect(result!._links!.approve).toEqual({
        href: '/api/entities/SupportRequest/e1/transition',
        method: 'POST',
      })
      expect(result!._links!.reject).toEqual({
        href: '/api/entities/SupportRequest/e1/transition',
        method: 'POST',
      })
    })

    it('_links has only navigation links when no transitions', async () => {
      const stub = {
        get: vi.fn().mockResolvedValue({
          id: 'e1',
          type: 'Noun',
          data: { name: 'Customer' },
          version: 1,
        }),
      }

      const result = await handleGetEntity(stub, {})

      expect(result).toBeDefined()
      expect(result!._links).toBeDefined()
      expect(result!._links!.self).toEqual({ href: '/api/entities/Noun/e1' })
      expect(result!._links!.approve).toBeUndefined()
    })

    it('transition links include self and collection alongside events', async () => {
      const innerStub = {
        get: vi.fn().mockResolvedValue({
          id: 'e1', type: 'SupportRequest',
          data: { title: 'Help', _status: 'Open', _statusId: 'status-open', _stateMachineDefinition: 'def1' },
          version: 1,
        }),
      }
      const result = await handleGetEntity(innerStub, {
        transitions: mockTransitions,
      })

      // Navigation links
      expect(result!._links!.self).toEqual({ href: '/api/entities/SupportRequest/e1' })
      expect(result!._links!.collection).toEqual({ href: '/api/entities/SupportRequest' })
      // Transition links
      expect(result!._links!.approve).toBeDefined()
      expect(result!._links!.approve.method).toBe('POST')
    })

    it('omits transition links when none provided', async () => {
      const stub = {
        get: vi.fn().mockResolvedValue({
          id: 'e1',
          type: 'SupportRequest',
          data: {
            title: 'Help',
            _status: 'Open',
            _statusId: 'status-open',
            _stateMachineDefinition: 'def1',
          },
          version: 1,
        }),
      }

      const result = await handleGetEntity(stub)

      expect(result).toBeDefined()
      expect(result!._links).toBeDefined()
      expect(result!._links!.self).toBeDefined()
      // No transition links without transitions
      expect(result!._links!.approve).toBeUndefined()
    })
  })

  // ── buildEntityLinks ──────────────────────────────────────────────

  describe('buildEntityLinks', () => {
    it('returns self and collection links as objects', () => {
      const links = buildEntityLinks('Order', 'ord-1')
      expect(links.self).toEqual({ href: '/api/entities/Order/ord-1' })
      expect(links.collection).toEqual({ href: '/api/entities/Order' })
    })

    it('includes domain in collection link', () => {
      const links = buildEntityLinks('Order', 'ord-1', 'orders')
      expect(links.collection).toEqual({ href: '/api/entities/Order?domain=orders' })
    })

    it('includes transition events as POST links', () => {
      const transitions = [
        { transitionId: 't1', event: 'place', targetStatus: 'Placed', targetStatusId: 's1' },
        { transitionId: 't2', event: 'cancel', targetStatus: 'Cancelled', targetStatusId: 's2' },
      ]
      const links = buildEntityLinks('Order', 'ord-1', 'orders', transitions)
      expect(links.place).toEqual({
        href: '/api/entities/Order/ord-1/transition',
        method: 'POST',
      })
      expect(links.cancel).toEqual({
        href: '/api/entities/Order/ord-1/transition',
        method: 'POST',
      })
    })

    it('omits transition links when no transitions', () => {
      const links = buildEntityLinks('Order', 'ord-1', 'orders')
      expect(links.place).toBeUndefined()
    })
  })
})

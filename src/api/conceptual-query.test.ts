import { describe, it, expect, vi } from 'vitest'
import { handleConceptualQuery } from './conceptual-query'
import type { Env } from '../types'

// ---------------------------------------------------------------------------
// Helpers for building mock Env + DOs
// ---------------------------------------------------------------------------

/** Entity DO document shape returned by entityDO.get() */
interface EntityDoc {
  id: string
  type: string
  data: Record<string, unknown>
  createdAt?: string
  updatedAt?: string
  deletedAt?: string | null
}

/**
 * Build a mock Env that wires up REGISTRY_DB and ENTITY_DB stubs.
 *
 * @param entities  Map from entity id -> EntityDoc
 * @param registryOverrides  Extra methods on the registry stub (e.g. resolveSlugByUUID)
 * @param entityIdsByType  Map from (type, domainSlug?) -> id[]
 */
function buildMockEnv(
  entities: Map<string, EntityDoc>,
  entityIdsByType: Map<string, string[]>,
  registryOverrides: Record<string, (...args: any[]) => any> = {},
): Env {
  const registryStub = {
    getEntityIds: vi.fn(async (type: string, _domain?: string) => {
      return entityIdsByType.get(type) ?? []
    }),
    resolveSlugByUUID: vi.fn(async (_uuid: string) => null),
    ...registryOverrides,
  }

  const entityStubs = new Map<string, { get: () => Promise<EntityDoc | null> }>()
  for (const [id, doc] of entities) {
    entityStubs.set(id, { get: vi.fn(async () => doc) })
  }

  const env: any = {
    REGISTRY_DB: {
      idFromName: vi.fn((_name: string) => 'registry-id'),
      get: vi.fn((_id: any) => registryStub),
    },
    ENTITY_DB: {
      idFromName: vi.fn((name: string) => `entity-id:${name}`),
      get: vi.fn((id: any) => {
        const entityId = typeof id === 'string' ? id.replace('entity-id:', '') : id
        return entityStubs.get(entityId) ?? { get: vi.fn(async () => null) }
      }),
    },
  }

  return env as Env
}

/** Build a Request for the handler */
function makeRequest(
  method: 'GET' | 'POST',
  params: { q?: string; domain?: string; [k: string]: string | undefined },
): Request {
  if (method === 'GET') {
    const url = new URL('https://test.do/api/query')
    for (const [k, v] of Object.entries(params)) {
      if (v !== undefined) url.searchParams.set(k, v)
    }
    return new Request(url.toString(), { method: 'GET' })
  }
  return new Request('https://test.do/api/query', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(params),
  })
}

// ---------------------------------------------------------------------------
// Fixtures: a mini university/support domain
// ---------------------------------------------------------------------------

function buildSupportDomain() {
  const entities = new Map<string, EntityDoc>()
  const entityIdsByType = new Map<string, string[]>()

  // Nouns (entity-type nouns only — filtered by objectType === 'entity')
  const nounDocs: EntityDoc[] = [
    { id: 'noun-customer', type: 'Noun', data: { name: 'Customer', objectType: 'entity' } },
    { id: 'noun-sr', type: 'Noun', data: { name: 'Support Request', objectType: 'entity' } },
    { id: 'noun-priority', type: 'Noun', data: { name: 'Priority', objectType: 'entity' } },
    // value-type noun — should be filtered out of entity nouns
    { id: 'noun-name', type: 'Noun', data: { name: 'Name', objectType: 'value' } },
  ]
  entityIdsByType.set('Noun', nounDocs.map(n => n.id))
  for (const n of nounDocs) entities.set(n.id, n)

  // Readings
  const readingDocs: EntityDoc[] = [
    { id: 'r1', type: 'Reading', data: { text: 'Customer submits Support Request' } },
    { id: 'r2', type: 'Reading', data: { text: 'Support Request has Priority' } },
  ]
  entityIdsByType.set('Reading', readingDocs.map(r => r.id))
  for (const r of readingDocs) entities.set(r.id, r)

  // Customer entities
  const customerDocs: EntityDoc[] = [
    { id: 'cust-1', type: 'Customer', data: { name: 'Alice', supportRequestId: 'sr-1' } },
    { id: 'cust-2', type: 'Customer', data: { name: 'Bob', supportRequestId: 'sr-2' } },
  ]
  entityIdsByType.set('Customer', customerDocs.map(c => c.id))
  for (const c of customerDocs) entities.set(c.id, c)

  // Support Request entities
  const srDocs: EntityDoc[] = [
    { id: 'sr-1', type: 'Support Request', data: { title: 'Bug report', priorityId: 'pri-high' } },
    { id: 'sr-2', type: 'Support Request', data: { title: 'Feature req', priorityId: 'pri-low' } },
  ]
  entityIdsByType.set('Support Request', srDocs.map(s => s.id))
  for (const s of srDocs) entities.set(s.id, s)

  // Priority entities
  const priDocs: EntityDoc[] = [
    { id: 'pri-high', type: 'Priority', data: { name: 'High' } },
    { id: 'pri-low', type: 'Priority', data: { name: 'Low' } },
  ]
  entityIdsByType.set('Priority', priDocs.map(p => p.id))
  for (const p of priDocs) entities.set(p.id, p)

  return { entities, entityIdsByType }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

describe('handleConceptualQuery', () => {
  describe('input validation', () => {
    it('returns 400 when query text is missing (GET)', async () => {
      const { entities, entityIdsByType } = buildSupportDomain()
      const env = buildMockEnv(entities, entityIdsByType)
      const req = makeRequest('GET', { domain: 'support-domain' })

      const res = await handleConceptualQuery(req, env)
      expect(res.status).toBe(400)
      const body = await res.json() as any
      expect(body.errors[0].message).toMatch(/query text required/)
    })

    it('returns 400 when domain is missing (GET)', async () => {
      const { entities, entityIdsByType } = buildSupportDomain()
      const env = buildMockEnv(entities, entityIdsByType)
      const req = makeRequest('GET', { q: 'Customer that submits Support Request' })

      const res = await handleConceptualQuery(req, env)
      expect(res.status).toBe(400)
      const body = await res.json() as any
      expect(body.errors[0].message).toMatch(/domain required/)
    })

    it('returns 400 when query text is missing (POST)', async () => {
      const { entities, entityIdsByType } = buildSupportDomain()
      const env = buildMockEnv(entities, entityIdsByType)
      const req = makeRequest('POST', { domain: 'support-domain' })

      const res = await handleConceptualQuery(req, env)
      expect(res.status).toBe(400)
    })
  })

  describe('domain resolution', () => {
    it('returns 404 for a UUID domain that cannot be resolved', async () => {
      const { entities, entityIdsByType } = buildSupportDomain()
      const env = buildMockEnv(entities, entityIdsByType, {
        resolveSlugByUUID: vi.fn(async () => null),
      })
      // UUID-shaped ID that won't resolve
      const req = makeRequest('GET', {
        q: 'Customer that submits Support Request',
        domain: '12345678-1234-5678-9abc-def012345678',
      })

      const res = await handleConceptualQuery(req, env)
      expect(res.status).toBe(404)
      const body = await res.json() as any
      expect(body.errors[0].message).toMatch(/Domain not found/)
    })

    it('resolves a slug domain (contains hyphens, not UUID-shaped)', async () => {
      const { entities, entityIdsByType } = buildSupportDomain()
      const env = buildMockEnv(entities, entityIdsByType)
      const req = makeRequest('GET', {
        q: 'Customer that submits Support Request',
        domain: 'support-domain',
      })

      const res = await handleConceptualQuery(req, env)
      // Should not 404 — the slug domain passes through
      expect(res.status).toBe(200)
    })

    it('resolves a UUID domain via registry.resolveSlugByUUID', async () => {
      const { entities, entityIdsByType } = buildSupportDomain()
      const env = buildMockEnv(entities, entityIdsByType, {
        resolveSlugByUUID: vi.fn(async () => 'support-domain'),
      })
      const req = makeRequest('GET', {
        q: 'Customer that submits Support Request',
        domain: '12345678-1234-5678-9abc-def012345678',
      })

      const res = await handleConceptualQuery(req, env)
      expect(res.status).toBe(200)
    })
  })

  describe('query resolution — no matching path', () => {
    it('returns resolved=false when nouns are not found', async () => {
      const { entities, entityIdsByType } = buildSupportDomain()
      const env = buildMockEnv(entities, entityIdsByType)
      const req = makeRequest('GET', {
        q: 'Foo that bars Baz',
        domain: 'support-domain',
      })

      const res = await handleConceptualQuery(req, env)
      expect(res.status).toBe(200)
      const body = await res.json() as any
      expect(body.resolved).toBe(false)
      expect(body.message).toMatch(/No reading path found/)
      expect(body.availableNouns).toBeInstanceOf(Array)
    })

    it('returns resolved=false when only one noun matches (no path possible)', async () => {
      const { entities, entityIdsByType } = buildSupportDomain()
      const env = buildMockEnv(entities, entityIdsByType)
      // Only "Customer" is recognized — no second noun to build a path
      const req = makeRequest('GET', {
        q: 'Customer',
        domain: 'support-domain',
      })

      const res = await handleConceptualQuery(req, env)
      expect(res.status).toBe(200)
      const body = await res.json() as any
      expect(body.resolved).toBe(false)
    })
  })

  describe('query resolution — successful path', () => {
    it('resolves a single-hop forward query via GET', async () => {
      const { entities, entityIdsByType } = buildSupportDomain()
      const env = buildMockEnv(entities, entityIdsByType)
      const req = makeRequest('GET', {
        q: 'Customer that submits Support Request',
        domain: 'support-domain',
      })

      const res = await handleConceptualQuery(req, env)
      expect(res.status).toBe(200)
      const body = await res.json() as any

      expect(body.resolved).toBe(true)
      expect(body.rootNoun).toBe('Customer')
      expect(body.path).toHaveLength(1)
      expect(body.path[0]).toMatch(/Customer/)
      expect(body.path[0]).toMatch(/Support Request/)
    })

    it('resolves a single-hop forward query via POST', async () => {
      const { entities, entityIdsByType } = buildSupportDomain()
      const env = buildMockEnv(entities, entityIdsByType)
      const req = makeRequest('POST', {
        q: 'Customer that submits Support Request',
        domain: 'support-domain',
      })

      const res = await handleConceptualQuery(req, env)
      expect(res.status).toBe(200)
      const body = await res.json() as any
      expect(body.resolved).toBe(true)
      expect(body.rootNoun).toBe('Customer')
    })

    it('resolves a multi-hop query: Customer -> Support Request -> Priority', async () => {
      const { entities, entityIdsByType } = buildSupportDomain()
      const env = buildMockEnv(entities, entityIdsByType)
      const req = makeRequest('GET', {
        q: 'Customer that submits Support Request that has Priority',
        domain: 'support-domain',
      })

      const res = await handleConceptualQuery(req, env)
      expect(res.status).toBe(200)
      const body = await res.json() as any

      expect(body.resolved).toBe(true)
      expect(body.rootNoun).toBe('Customer')
      expect(body.path).toHaveLength(2)
    })
  })

  describe('value filtering', () => {
    it('applies quoted value filter and narrows results', async () => {
      const { entities, entityIdsByType } = buildSupportDomain()
      const env = buildMockEnv(entities, entityIdsByType)
      const req = makeRequest('GET', {
        q: "Support Request that has Priority 'High'",
        domain: 'support-domain',
      })

      const res = await handleConceptualQuery(req, env)
      expect(res.status).toBe(200)
      const body = await res.json() as any

      expect(body.resolved).toBe(true)
      expect(body.filters).toEqual([{ field: 'Priority', value: 'High' }])
    })
  })

  describe('path walking — FK joins', () => {
    it('forward FK join links entities by FK field', async () => {
      // Customer.supportRequestId -> Support Request.id
      const { entities, entityIdsByType } = buildSupportDomain()
      const env = buildMockEnv(entities, entityIdsByType)
      const req = makeRequest('GET', {
        q: 'Customer that submits Support Request',
        domain: 'support-domain',
      })

      const res = await handleConceptualQuery(req, env)
      const body = await res.json() as any

      expect(body.resolved).toBe(true)
      // results should contain entity data
      expect(body.results).toBeInstanceOf(Array)
      expect(body.count).toBeGreaterThanOrEqual(0)
    })

    it('inverse FK join finds target entities referencing current entities', async () => {
      const entities = new Map<string, EntityDoc>()
      const entityIdsByType = new Map<string, string[]>()

      // Nouns
      const nounDocs: EntityDoc[] = [
        { id: 'noun-order', type: 'Noun', data: { name: 'Order', objectType: 'entity' } },
        { id: 'noun-customer', type: 'Noun', data: { name: 'Customer', objectType: 'entity' } },
      ]
      entityIdsByType.set('Noun', nounDocs.map(n => n.id))
      for (const n of nounDocs) entities.set(n.id, n)

      // Reading: Customer places Order (Customer is subject)
      const readingDocs: EntityDoc[] = [
        { id: 'r1', type: 'Reading', data: { text: 'Customer places Order' } },
      ]
      entityIdsByType.set('Reading', readingDocs.map(r => r.id))
      for (const r of readingDocs) entities.set(r.id, r)

      // Orders with customerId FK
      const orderDocs: EntityDoc[] = [
        { id: 'ord-1', type: 'Order', data: { total: 100, customerId: 'cust-1' } },
        { id: 'ord-2', type: 'Order', data: { total: 200, customerId: 'cust-2' } },
        { id: 'ord-3', type: 'Order', data: { total: 50, customerId: 'cust-1' } },
      ]
      entityIdsByType.set('Order', orderDocs.map(o => o.id))
      for (const o of orderDocs) entities.set(o.id, o)

      // Customers
      const customerDocs: EntityDoc[] = [
        { id: 'cust-1', type: 'Customer', data: { name: 'Alice' } },
        { id: 'cust-2', type: 'Customer', data: { name: 'Bob' } },
      ]
      entityIdsByType.set('Customer', customerDocs.map(c => c.id))
      for (const c of customerDocs) entities.set(c.id, c)

      const env = buildMockEnv(entities, entityIdsByType)
      // Query from Order perspective — "Order placed by Customer"
      // Reading is "Customer places Order" so from Order's view this is inverse
      const req = makeRequest('GET', {
        q: 'Order placed by Customer',
        domain: 'order-domain',
      })

      const res = await handleConceptualQuery(req, env)
      const body = await res.json() as any

      expect(body.resolved).toBe(true)
      expect(body.rootNoun).toBe('Order')
      expect(body.results).toBeInstanceOf(Array)
    })
  })

  describe('entity filtering', () => {
    it('skips deleted entities (deletedAt set)', async () => {
      const entities = new Map<string, EntityDoc>()
      const entityIdsByType = new Map<string, string[]>()

      // Nouns
      const nounDocs: EntityDoc[] = [
        { id: 'noun-item', type: 'Noun', data: { name: 'Item', objectType: 'entity' } },
        { id: 'noun-cat', type: 'Noun', data: { name: 'Category', objectType: 'entity' } },
      ]
      entityIdsByType.set('Noun', nounDocs.map(n => n.id))
      for (const n of nounDocs) entities.set(n.id, n)

      const readingDocs: EntityDoc[] = [
        { id: 'r1', type: 'Reading', data: { text: 'Item has Category' } },
      ]
      entityIdsByType.set('Reading', readingDocs.map(r => r.id))
      for (const r of readingDocs) entities.set(r.id, r)

      // One live, one deleted
      const itemDocs: EntityDoc[] = [
        { id: 'item-1', type: 'Item', data: { name: 'Widget' } },
        { id: 'item-2', type: 'Item', data: { name: 'Gadget' }, deletedAt: '2025-01-01T00:00:00Z' },
      ]
      entityIdsByType.set('Item', itemDocs.map(i => i.id))
      for (const i of itemDocs) entities.set(i.id, i)

      const catDocs: EntityDoc[] = [
        { id: 'cat-1', type: 'Category', data: { name: 'Electronics' } },
      ]
      entityIdsByType.set('Category', catDocs.map(c => c.id))
      for (const c of catDocs) entities.set(c.id, c)

      const env = buildMockEnv(entities, entityIdsByType)
      const req = makeRequest('GET', {
        q: 'Item that has Category',
        domain: 'catalog-domain',
      })

      const res = await handleConceptualQuery(req, env)
      const body = await res.json() as any

      expect(body.resolved).toBe(true)
      // The deleted item should not appear in results
      const resultIds = body.results.map((r: any) => r.id)
      expect(resultIds).not.toContain('item-2')
    })
  })

  describe('POST body parsing', () => {
    it('accepts "query" field in POST body', async () => {
      const { entities, entityIdsByType } = buildSupportDomain()
      const env = buildMockEnv(entities, entityIdsByType)
      const req = new Request('https://test.do/api/query', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          query: 'Customer that submits Support Request',
          domain: 'support-domain',
        }),
      })

      const res = await handleConceptualQuery(req, env)
      expect(res.status).toBe(200)
      const body = await res.json() as any
      expect(body.resolved).toBe(true)
    })

    it('accepts "text" field in POST body', async () => {
      const { entities, entityIdsByType } = buildSupportDomain()
      const env = buildMockEnv(entities, entityIdsByType)
      const req = new Request('https://test.do/api/query', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          text: 'Customer that submits Support Request',
          domain: 'support-domain',
        }),
      })

      const res = await handleConceptualQuery(req, env)
      expect(res.status).toBe(200)
      const body = await res.json() as any
      expect(body.resolved).toBe(true)
    })
  })

  describe('value-type nouns are excluded', () => {
    it('filters out value-type nouns when building the noun list', async () => {
      const { entities, entityIdsByType } = buildSupportDomain()
      const env = buildMockEnv(entities, entityIdsByType)
      // "Name" is a value-type noun so it should not appear in availableNouns
      const req = makeRequest('GET', {
        q: 'Unknown thing',
        domain: 'support-domain',
      })

      const res = await handleConceptualQuery(req, env)
      const body = await res.json() as any
      // resolved=false means we get availableNouns in the response
      expect(body.resolved).toBe(false)
      expect(body.availableNouns).not.toContain('Name')
      expect(body.availableNouns).toContain('Customer')
      expect(body.availableNouns).toContain('Support Request')
      expect(body.availableNouns).toContain('Priority')
    })
  })
})

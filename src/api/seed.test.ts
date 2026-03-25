import { describe, it, expect, vi, beforeEach } from 'vitest'
import { handleSeed } from './seed'
import type { Env } from '../types'

// ---------------------------------------------------------------------------
// Mock helpers
// ---------------------------------------------------------------------------

/** Create a mock DO stub with chainable idFromName → get pattern. */
function mockDONamespace(stubFactory: (name: string) => any) {
  return {
    idFromName: vi.fn((name: string) => `id:${name}`),
    get: vi.fn((id: string) => stubFactory(id.replace('id:', ''))),
  }
}

/** Build a minimal mock Env with configurable DO stubs. */
function createMockEnv(overrides: {
  registry?: any
  domainDO?: (slug: string) => any
  entityDO?: (id: string) => any
} = {}): Env {
  const defaultRegistry = {
    listDomains: vi.fn(async () => []),
    getEntityIds: vi.fn(async () => []),
    indexNoun: vi.fn(async () => {}),
    indexEntity: vi.fn(async () => {}),
    registerDomain: vi.fn(async () => {}),
  }

  const registry = overrides.registry || defaultRegistry

  const defaultDomainDO = () => ({
    setDomainId: vi.fn(async () => {}),
    wipeAllData: vi.fn(async () => {}),
    createEntity: vi.fn(async () => ({})),
    applySchema: vi.fn(async () => ({})),
  })

  const defaultEntityDO = () => ({
    get: vi.fn(async () => null),
    put: vi.fn(async () => {}),
  })

  return {
    REGISTRY_DB: mockDONamespace(() => registry),
    DOMAIN_DB: mockDONamespace(overrides.domainDO || defaultDomainDO),
    ENTITY_DB: mockDONamespace(overrides.entityDO || defaultEntityDO),
    ENVIRONMENT: 'test',
  } as unknown as Env
}

/** Create a Request with JSON body. */
function jsonRequest(method: string, body?: any): Request {
  const init: RequestInit = { method }
  if (body !== undefined) {
    init.body = JSON.stringify(body)
    init.headers = { 'Content-Type': 'application/json' }
  }
  return new Request('http://localhost/seed', init)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

describe('handleSeed', () => {
  // ── Method routing ──────────────────────────────────────────────────

  describe('method routing', () => {
    it('returns 405 for unsupported methods', async () => {
      const env = createMockEnv()
      const res = await handleSeed(new Request('http://localhost/seed', { method: 'PUT' }), env)
      expect(res.status).toBe(405)
      const data = await res.json() as any
      expect(data.errors[0].message).toBe('Method not allowed')
    })

    it('routes GET to stats handler', async () => {
      const env = createMockEnv()
      const res = await handleSeed(new Request('http://localhost/seed', { method: 'GET' }), env)
      expect(res.status).toBe(200)
    })

    it('routes DELETE to wipe handler', async () => {
      const env = createMockEnv()
      const res = await handleSeed(new Request('http://localhost/seed', { method: 'DELETE' }), env)
      expect(res.status).toBe(200)
    })
  })

  // ── GET /seed — stats ───────────────────────────────────────────────

  describe('GET /seed (stats)', () => {
    it('returns zero totals when no domains exist', async () => {
      const registry = {
        listDomains: vi.fn(async () => []),
        getEntityIds: vi.fn(async () => []),
      }
      const env = createMockEnv({ registry })
      const res = await handleSeed(jsonRequest('GET'), env)
      expect(res.status).toBe(200)

      const data = await res.json() as any
      expect(data.totals).toEqual({
        domains: 0,
        nouns: 0,
        readings: 0,
        graphSchemas: 0,
        constraints: 0,
      })
      expect(data.perDomain).toEqual({})
    })

    it('aggregates counts across multiple domains', async () => {
      const registry = {
        listDomains: vi.fn(async () => ['university', 'hr']),
        getEntityIds: vi.fn(async (type: string, slug: string) => {
          const counts: Record<string, Record<string, string[]>> = {
            university: {
              Noun: ['n1', 'n2', 'n3'],
              Reading: ['r1', 'r2'],
              GraphSchema: ['gs1', 'gs2'],
              Constraint: ['c1'],
            },
            hr: {
              Noun: ['n4'],
              Reading: ['r3'],
              GraphSchema: ['gs3'],
              Constraint: [],
            },
          }
          return counts[slug]?.[type] || []
        }),
      }
      const env = createMockEnv({ registry })
      const res = await handleSeed(jsonRequest('GET'), env)
      const data = await res.json() as any

      expect(data.totals).toEqual({
        domains: 2,
        nouns: 4,
        readings: 3,
        graphSchemas: 3,
        constraints: 1,
      })
      expect(data.perDomain.university).toEqual({ nouns: 3, readings: 2 })
      expect(data.perDomain.hr).toEqual({ nouns: 1, readings: 1 })
    })
  })

  // ── DELETE /seed — wipe ─────────────────────────────────────────────

  describe('DELETE /seed (wipe)', () => {
    it('wipes all domain DOs and returns confirmation', async () => {
      const wipeFns = { university: vi.fn(async () => {}), hr: vi.fn(async () => {}) }
      const registry = {
        listDomains: vi.fn(async () => ['university', 'hr']),
      }
      const env = createMockEnv({
        registry,
        domainDO: (slug: string) => ({
          wipeAllData: wipeFns[slug as keyof typeof wipeFns] || vi.fn(async () => {}),
        }),
      })

      const res = await handleSeed(jsonRequest('DELETE'), env)
      expect(res.status).toBe(200)

      const data = await res.json() as any
      expect(data.message).toBe('All data wiped')
      expect(wipeFns.university).toHaveBeenCalledOnce()
      expect(wipeFns.hr).toHaveBeenCalledOnce()
    })

    it('handles empty domain list gracefully', async () => {
      const registry = { listDomains: vi.fn(async () => []) }
      const env = createMockEnv({ registry })

      const res = await handleSeed(jsonRequest('DELETE'), env)
      expect(res.status).toBe(200)
      const data = await res.json() as any
      expect(data.message).toBe('All data wiped')
    })
  })

  // ── POST /seed — error cases ────────────────────────────────────────

  describe('POST /seed (error handling)', () => {
    it('rejects unsupported seed type', async () => {
      const env = createMockEnv()
      const res = await handleSeed(jsonRequest('POST', { type: 'graphql' }), env)
      expect(res.status).toBe(400)
      const data = await res.json() as any
      expect(data.errors[0].message).toContain('Unsupported seed type')
    })

    it('rejects when no claims, text, or domains are provided', async () => {
      const env = createMockEnv()
      const res = await handleSeed(jsonRequest('POST', { type: 'claims' }), env)
      expect(res.status).toBe(400)
      const data = await res.json() as any
      expect(data.errors[0].message).toContain('Provide claims + domain')
    })
  })

  // ── POST /seed — text mode (parse + ingest) ─────────────────────────

  describe('POST /seed (text mode)', () => {
    it('parses FORML2 text and ingests claims for a single domain', async () => {
      const domainDO = {
        setDomainId: vi.fn(async () => {}),
        createEntity: vi.fn(async () => ({})),
        applySchema: vi.fn(async () => ({})),
      }
      const entityDO = {
        get: vi.fn(async () => null),
        put: vi.fn(async () => {}),
      }
      const registry = {
        listDomains: vi.fn(async () => []),
        getEntityIds: vi.fn(async () => []),
        indexNoun: vi.fn(async () => {}),
        indexEntity: vi.fn(async () => {}),
        registerDomain: vi.fn(async () => {}),
      }

      const env = createMockEnv({
        registry,
        domainDO: () => domainDO,
        entityDO: () => entityDO,
      })

      const forml2Text = `## Entity Types
Customer is an entity type.

## Value Types
Name is a value type.

## Fact Types
Customer has Name.
  Each Customer has at most one Name.`

      const res = await handleSeed(jsonRequest('POST', {
        type: 'claims',
        text: forml2Text,
        domain: 'test-domain',
      }), env)

      expect(res.status).toBe(200)
      const data = await res.json() as any

      // Should have ingested nouns and readings
      expect(data.nouns).toBeGreaterThanOrEqual(2) // Customer + Name
      expect(data.readings).toBeGreaterThanOrEqual(1) // Customer has Name
      expect(data.domainId).toBeDefined()

      // Domain DO should have been initialized
      expect(domainDO.setDomainId).toHaveBeenCalledWith('test-domain')

      // Registry should have had nouns indexed
      expect(registry.indexNoun).toHaveBeenCalled()
      expect(registry.registerDomain).toHaveBeenCalled()
    })
  })

  // ── POST /seed — pre-parsed claims ──────────────────────────────────

  describe('POST /seed (pre-parsed claims)', () => {
    it('ingests pre-parsed claims for a single domain', async () => {
      const domainDO = {
        setDomainId: vi.fn(async () => {}),
        createEntity: vi.fn(async () => ({})),
        applySchema: vi.fn(async () => ({})),
      }
      const entityDO = {
        get: vi.fn(async () => null),
        put: vi.fn(async () => {}),
      }
      const registry = {
        listDomains: vi.fn(async () => []),
        getEntityIds: vi.fn(async () => []),
        indexNoun: vi.fn(async () => {}),
        indexEntity: vi.fn(async () => {}),
        registerDomain: vi.fn(async () => {}),
      }

      const env = createMockEnv({
        registry,
        domainDO: () => domainDO,
        entityDO: () => entityDO,
      })

      const res = await handleSeed(jsonRequest('POST', {
        type: 'claims',
        domain: 'test-domain',
        domainId: 'test-domain',
        claims: {
          nouns: [
            { name: 'Student', objectType: 'entity' },
            { name: 'Student Nr', objectType: 'value', valueType: 'string' },
          ],
          readings: [
            { text: 'Student has Student Nr', nouns: ['Student', 'Student Nr'], predicate: 'has' },
          ],
          constraints: [
            { kind: 'UC', modality: 'Alethic', reading: 'Student has Student Nr', roles: [0] },
          ],
        },
      }), env)

      expect(res.status).toBe(200)
      const data = await res.json() as any

      expect(data.nouns).toBeGreaterThanOrEqual(2)
      expect(data.readings).toBeGreaterThanOrEqual(1)
      expect(data.domainId).toBeDefined()
    })

    it('rejects single-domain seed when neither domain nor domainId is provided', async () => {
      const env = createMockEnv()
      const res = await handleSeed(jsonRequest('POST', {
        type: 'claims',
        claims: {
          nouns: [{ name: 'Foo', objectType: 'entity' }],
          readings: [],
          constraints: [],
        },
      }), env)

      expect(res.status).toBe(400)
      const data = await res.json() as any
      expect(data.errors[0].message).toContain('domainId or domains[]')
    })
  })

  // ── POST /seed — bulk domains ───────────────────────────────────────

  describe('POST /seed (bulk domains)', () => {
    it('ingests multiple domains in parallel with text mode', async () => {
      const domainDOs: Record<string, any> = {}
      const makeDomainDO = (slug: string) => {
        if (!domainDOs[slug]) {
          domainDOs[slug] = {
            setDomainId: vi.fn(async () => {}),
            createEntity: vi.fn(async () => ({})),
            applySchema: vi.fn(async () => ({})),
          }
        }
        return domainDOs[slug]
      }

      const entityDO = {
        get: vi.fn(async () => null),
        put: vi.fn(async () => {}),
      }
      const registry = {
        listDomains: vi.fn(async () => []),
        getEntityIds: vi.fn(async () => []),
        indexNoun: vi.fn(async () => {}),
        indexEntity: vi.fn(async () => {}),
        registerDomain: vi.fn(async () => {}),
      }

      const env = createMockEnv({
        registry,
        domainDO: makeDomainDO,
        entityDO: () => entityDO,
      })

      const res = await handleSeed(jsonRequest('POST', {
        type: 'claims',
        domains: [
          {
            slug: 'university',
            text: `## Entity Types
Student is an entity type.
Course is an entity type.

## Fact Types
Student enrolls in Course.`,
          },
          {
            slug: 'hr',
            text: `## Entity Types
Employee is an entity type.

## Value Types
Name is a value type.

## Fact Types
Employee has Name.`,
          },
        ],
      }), env)

      expect(res.status).toBe(200)
      const data = await res.json() as any

      // Should return results for both domains
      expect(data.domains).toHaveLength(2)
      expect(data.domains[0].domain).toBe('university')
      expect(data.domains[1].domain).toBe('hr')

      // Both domain DOs should have been initialized
      expect(domainDOs['university'].setDomainId).toHaveBeenCalledWith('university')
      expect(domainDOs['hr'].setDomainId).toHaveBeenCalledWith('hr')

      // Registry should have been updated for both domains
      expect(registry.registerDomain).toHaveBeenCalledTimes(2)

      // Timings should be present
      expect(data.timings).toBeDefined()
    })

    it('handles domains with pre-parsed claims in bulk mode', async () => {
      const domainDO = {
        setDomainId: vi.fn(async () => {}),
        createEntity: vi.fn(async () => ({})),
        applySchema: vi.fn(async () => ({})),
      }
      const entityDO = {
        get: vi.fn(async () => null),
        put: vi.fn(async () => {}),
      }
      const registry = {
        listDomains: vi.fn(async () => []),
        getEntityIds: vi.fn(async () => []),
        indexNoun: vi.fn(async () => {}),
        indexEntity: vi.fn(async () => {}),
        registerDomain: vi.fn(async () => {}),
      }

      const env = createMockEnv({
        registry,
        domainDO: () => domainDO,
        entityDO: () => entityDO,
      })

      const res = await handleSeed(jsonRequest('POST', {
        type: 'claims',
        domains: [
          {
            slug: 'finance',
            claims: {
              nouns: [
                { name: 'Account', objectType: 'entity' },
                { name: 'Balance', objectType: 'value', valueType: 'number' },
              ],
              readings: [
                { text: 'Account has Balance', nouns: ['Account', 'Balance'], predicate: 'has' },
              ],
              constraints: [],
            },
          },
        ],
      }), env)

      expect(res.status).toBe(200)
      const data = await res.json() as any
      expect(data.domains).toHaveLength(1)
      expect(data.domains[0].domain).toBe('finance')
    })

    it('handles domains with empty claims (no text, no claims)', async () => {
      const domainDO = {
        setDomainId: vi.fn(async () => {}),
        createEntity: vi.fn(async () => ({})),
        applySchema: vi.fn(async () => ({})),
      }
      const entityDO = {
        get: vi.fn(async () => null),
        put: vi.fn(async () => {}),
      }
      const registry = {
        listDomains: vi.fn(async () => []),
        getEntityIds: vi.fn(async () => []),
        indexNoun: vi.fn(async () => {}),
        indexEntity: vi.fn(async () => {}),
        registerDomain: vi.fn(async () => {}),
      }

      const env = createMockEnv({
        registry,
        domainDO: () => domainDO,
        entityDO: () => entityDO,
      })

      const res = await handleSeed(jsonRequest('POST', {
        type: 'claims',
        domains: [{ slug: 'empty-domain' }],
      }), env)

      expect(res.status).toBe(200)
      const data = await res.json() as any
      expect(data.domains).toHaveLength(1)
      expect(data.domains[0].domain).toBe('empty-domain')
      expect(data.domains[0].nouns).toBe(0)
      expect(data.domains[0].readings).toBe(0)
    })
  })

  // ── ensureDomain (tested via seed flow effects) ─────────────────────

  describe('ensureDomain (via seed flow)', () => {
    it('creates a new domain entity when none exists', async () => {
      const putFn = vi.fn(async () => {})
      const domainDO = {
        setDomainId: vi.fn(async () => {}),
        createEntity: vi.fn(async () => ({})),
        applySchema: vi.fn(async () => ({})),
      }
      const registry = {
        listDomains: vi.fn(async () => []),
        getEntityIds: vi.fn(async () => []),
        indexNoun: vi.fn(async () => {}),
        indexEntity: vi.fn(async () => {}),
        registerDomain: vi.fn(async () => {}),
      }

      const env = createMockEnv({
        registry,
        domainDO: () => domainDO,
        entityDO: () => ({
          get: vi.fn(async () => null),
          put: putFn,
        }),
      })

      const res = await handleSeed(jsonRequest('POST', {
        type: 'claims',
        domain: 'new-domain',
        domainId: 'new-domain',
        claims: {
          nouns: [{ name: 'Thing', objectType: 'entity' }],
          readings: [],
          constraints: [],
        },
      }), env)

      expect(res.status).toBe(200)

      // ensureDomain should have created an entity via EntityDB.put
      expect(putFn).toHaveBeenCalled()
      const putCall = putFn.mock.calls[0][0]
      expect(putCall.type).toBe('Domain')
      expect(putCall.data.domainSlug).toBe('new-domain')

      // Registry should have indexed the new domain entity
      expect(registry.indexEntity).toHaveBeenCalledWith(
        'Domain',
        expect.any(String),
        'new-domain',
      )
    })

    it('reuses an existing domain entity when one is found', async () => {
      const existingId = 'existing-uuid-123'
      const putFn = vi.fn(async () => {})
      const domainDO = {
        setDomainId: vi.fn(async () => {}),
        createEntity: vi.fn(async () => ({})),
        applySchema: vi.fn(async () => ({})),
      }
      const registry = {
        listDomains: vi.fn(async () => []),
        getEntityIds: vi.fn(async (type: string) => {
          if (type === 'Domain') return [existingId]
          return []
        }),
        indexNoun: vi.fn(async () => {}),
        indexEntity: vi.fn(async () => {}),
        registerDomain: vi.fn(async () => {}),
      }

      const env = createMockEnv({
        registry,
        domainDO: () => domainDO,
        entityDO: () => ({
          get: vi.fn(async () => ({
            id: existingId,
            data: { domainSlug: 'existing-domain', name: 'Existing Domain' },
          })),
          put: putFn,
        }),
      })

      const res = await handleSeed(jsonRequest('POST', {
        type: 'claims',
        domain: 'existing-domain',
        domainId: 'existing-domain',
        claims: {
          nouns: [{ name: 'Widget', objectType: 'entity' }],
          readings: [],
          constraints: [],
        },
      }), env)

      expect(res.status).toBe(200)
      const data = await res.json() as any

      // Should have used the existing domain UUID
      expect(data.domainId).toBe(existingId)

      // put should NOT have been called — domain already exists
      expect(putFn).not.toHaveBeenCalled()
    })
  })

  // ── POST /seed — bulk with instance facts ───────────────────────────

  describe('POST /seed (bulk with instance facts)', () => {
    it('processes instance facts in phase 2 after metamodel seeding', async () => {
      const createEntityFn = vi.fn(async () => ({}))
      const domainDO = {
        setDomainId: vi.fn(async () => {}),
        createEntity: createEntityFn,
        applySchema: vi.fn(async () => ({})),
      }
      const entityDO = {
        get: vi.fn(async () => null),
        put: vi.fn(async () => {}),
      }
      const registry = {
        listDomains: vi.fn(async () => []),
        getEntityIds: vi.fn(async () => []),
        indexNoun: vi.fn(async () => {}),
        indexEntity: vi.fn(async () => {}),
        registerDomain: vi.fn(async () => {}),
      }

      const env = createMockEnv({
        registry,
        domainDO: () => domainDO,
        entityDO: () => entityDO,
      })

      const res = await handleSeed(jsonRequest('POST', {
        type: 'claims',
        domains: [
          {
            slug: 'statuses',
            claims: {
              nouns: [
                { name: 'Status', objectType: 'entity' },
                { name: 'Display Color', objectType: 'value', valueType: 'string' },
              ],
              readings: [
                { text: 'Status has Display Color', nouns: ['Status', 'Display Color'], predicate: 'has' },
              ],
              constraints: [],
              facts: [
                { entity: 'Status', entityValue: 'Active', valueType: 'Display Color', value: 'green' },
                { entity: 'Status', entityValue: 'Inactive', valueType: 'Display Color', value: 'gray' },
              ],
            },
          },
        ],
      }), env)

      expect(res.status).toBe(200)

      // createEntity should have been called for each fact
      expect(createEntityFn).toHaveBeenCalledTimes(2)

      // First fact: Status 'Active' with displayColor 'green'
      expect(createEntityFn).toHaveBeenCalledWith(
        expect.any(String), // domainUUID
        'Status',
        { displayColor: 'green' },
        'Active',
      )

      // Second fact: Status 'Inactive' with displayColor 'gray'
      expect(createEntityFn).toHaveBeenCalledWith(
        expect.any(String),
        'Status',
        { displayColor: 'gray' },
        'Inactive',
      )
    })
  })
})

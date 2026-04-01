/**
 * Seed endpoint tests.
 *
 * The seed pipeline: raw markdown → parseReadings (WASM) → materializeBatch → cells in D.
 * Tests mock the WASM parseReadings and the Registry.
 */

import { describe, it, expect, vi, beforeEach } from 'vitest'
import { handleSeed } from './seed'
import type { Env } from '../types'

// Mock the WASM engine — seed.ts uses parseReadingsWithNouns
vi.mock('./engine', () => {
  const parse = (markdown: string, domain: string) => {
    const entities: any[] = []
    for (const line of markdown.split('\n')) {
      const m = line.match(/^(\w[\w\s]*?)\s+is an entity type/i)
      if (m) {
        entities.push({
          id: `${domain}:${m[1].trim()}`,
          type: 'Noun',
          domain,
          data: { name: m[1].trim(), domain, objectType: 'entity' },
        })
      }
      const rm = line.match(/^(\w[\w\s]*?)\s+has\s+(\w[\w\s]*?)\.?\s*$/i)
      if (rm) {
        entities.push({
          id: `${domain}:reading:${rm[1].trim()}_${rm[2].trim()}`,
          type: 'Reading',
          domain,
          data: { text: line.trim(), domain },
        })
      }
    }
    return entities
  }
  return {
    parseReadings: vi.fn(parse),
    parseReadingsWithNouns: vi.fn((markdown: string, domain: string, _existingNouns: string) => parse(markdown, domain)),
    ensureWasm: vi.fn(),
  }
})

function mockDONamespace(factory: (...args: any[]) => any) {
  return {
    idFromName: vi.fn((name: string) => name),
    get: vi.fn((_id: any) => factory()),
  } as unknown as DurableObjectNamespace
}

function createMockEnv(overrides: { registry?: any } = {}): Env {
  const defaultRegistry = {
    listDomains: vi.fn(async () => []),
    getEntityIds: vi.fn(async () => []),
    indexNoun: vi.fn(async () => {}),
    registerDomain: vi.fn(async () => {}),
    materializeBatch: vi.fn(async () => ({ materialized: 0, failed: [] })),
    wipeAll: vi.fn(async () => {}),
  }
  const registry = overrides.registry || defaultRegistry

  return {
    REGISTRY_DB: mockDONamespace(() => registry),
    DOMAIN_DB: mockDONamespace(() => ({})),
    ENTITY_DB: mockDONamespace(() => ({ get: vi.fn(async () => null), put: vi.fn(async () => {}) })),
  } as unknown as Env
}

function jsonRequest(method: string, body?: any): Request {
  return new Request('https://test.do/seed', {
    method,
    headers: body ? { 'Content-Type': 'application/json' } : {},
    body: body ? JSON.stringify(body) : undefined,
  })
}

describe('handleSeed', () => {
  beforeEach(() => { vi.clearAllMocks() })

  describe('GET /seed (stats)', () => {
    it('returns zero stats when no domains', async () => {
      const env = createMockEnv()
      const res = await handleSeed(jsonRequest('GET'), env)
      expect(res.status).toBe(200)
      const data = await res.json() as any
      expect(data.totals.domains).toBe(0)
    })

    it('returns per-domain stats from registry', async () => {
      const registry = {
        listDomains: vi.fn(async () => ['core', 'support']),
        getEntityIds: vi.fn(async (type: string, domain: string) => {
          if (type === 'Noun' && domain === 'core') return ['a', 'b']
          if (type === 'Reading' && domain === 'core') return ['r1']
          return []
        }),
        wipeAll: vi.fn(async () => {}),
      }
      const env = createMockEnv({ registry })
      const res = await handleSeed(jsonRequest('GET'), env)
      const data = await res.json() as any
      expect(data.totals.domains).toBe(2)
      expect(data.totals.nouns).toBe(2)
      expect(data.perDomain.core.nouns).toBe(2)
    })
  })

  describe('DELETE /seed (wipe)', () => {
    it('wipes registry and returns confirmation', async () => {
      const wipeAll = vi.fn(async () => {})
      const registry = { listDomains: vi.fn(async () => []), wipeAll }
      const env = createMockEnv({ registry })
      const res = await handleSeed(jsonRequest('DELETE'), env)
      expect(res.status).toBe(200)
      expect(wipeAll).toHaveBeenCalledOnce()
    })
  })

  describe('POST /seed (parse via ρ)', () => {
    it('parses readings via WASM and materializes entities', async () => {
      const registry = {
        listDomains: vi.fn(async () => []),
        getEntityIds: vi.fn(async () => []),
        indexNoun: vi.fn(async () => {}),
        registerDomain: vi.fn(async () => {}),
        materializeBatch: vi.fn(async () => ({ materialized: 0, failed: [] })),
        wipeAll: vi.fn(async () => {}),
      }
      const env = createMockEnv({ registry })

      const res = await handleSeed(jsonRequest('POST', {
        domain: 'test',
        text: 'Student is an entity type.\nCourse is an entity type.\nStudent has Name.',
      }), env)

      expect(res.status).toBe(200)
      const data = await res.json() as any
      expect(data.domains).toHaveLength(1)
      expect(data.domains[0].domain).toBe('test')
      expect(data.domains[0].nouns).toBe(2) // Student, Course
      expect(data.domains[0].readings).toBe(1) // Student has Name
      expect(registry.registerDomain).toHaveBeenCalledWith('test', 'test', 'private')
      expect(registry.materializeBatch).toHaveBeenCalled()
    })

    it('returns 400 with no readings', async () => {
      const env = createMockEnv()
      const res = await handleSeed(jsonRequest('POST', {}), env)
      expect(res.status).toBe(400)
    })

    it('handles multiple domains in JSON body', async () => {
      const registry = {
        listDomains: vi.fn(async () => []),
        getEntityIds: vi.fn(async () => []),
        indexNoun: vi.fn(async () => {}),
        registerDomain: vi.fn(async () => {}),
        materializeBatch: vi.fn(async () => ({ materialized: 0, failed: [] })),
        wipeAll: vi.fn(async () => {}),
      }
      const env = createMockEnv({ registry })

      const res = await handleSeed(jsonRequest('POST', {
        domains: [
          { slug: 'university', text: 'Student is an entity type.' },
          { slug: 'hr', text: 'Employee is an entity type.' },
        ],
      }), env)

      expect(res.status).toBe(200)
      const data = await res.json() as any
      expect(data.domains).toHaveLength(2)
      expect(registry.registerDomain).toHaveBeenCalledTimes(2)
    })
  })
})

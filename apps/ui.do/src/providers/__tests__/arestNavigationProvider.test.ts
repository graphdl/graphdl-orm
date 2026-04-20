/**
 * arestNavigationProvider tests.
 *
 * Pulls the resource list + menu from the worker's OpenAPI document
 * (per #117). OpenAPI is served at `/api/openapi.json?app=<name>` —
 * the navigation provider treats the JSON as authoritative: any top-
 * level path `/arest/{slug}` becomes a resource named `{slug}`; the
 * plural label comes from the path, the singular is the slug minus a
 * trailing 's' as a fallback (the real data comes from x-singular /
 * summary when present).
 */
import { afterEach, describe, expect, it, vi } from 'vitest'
import { createArestNavigationProvider } from '../arestNavigationProvider'

function stubFetch(responder: (url: string) => Response): string[] {
  const recorded: string[] = []
  vi.stubGlobal('fetch', async (input: RequestInfo | URL) => {
    const url = typeof input === 'string' ? input : input instanceof URL ? input.toString() : input.url
    recorded.push(url)
    return responder(url)
  })
  return recorded
}

function json(payload: unknown, status = 200): Response {
  return new Response(JSON.stringify(payload), {
    status,
    headers: { 'Content-Type': 'application/json' },
  })
}

describe('arestNavigationProvider', () => {
  const baseUrl = 'https://ui.auto.dev/arest'

  afterEach(() => { vi.unstubAllGlobals() })

  describe('resources', () => {
    it('extracts collection slugs from /arest/{slug} paths in openapi.json', async () => {
      const provider = createArestNavigationProvider({ baseUrl })
      stubFetch(() => json({
        openapi: '3.1.0',
        info: { title: 'AREST' },
        paths: {
          '/arest/organizations': { get: { summary: 'List organizations' } },
          '/arest/organizations/{id}': { get: {} },
          '/arest/support-requests': {
            get: { summary: 'List support requests', 'x-singular': 'Support Request' },
          },
          '/arest/support-requests/{id}': { get: {} },
          '/arest/domains': { get: {} },
        },
      }))

      const resources = await provider.resources()
      expect(resources).toHaveLength(3)

      const names = resources.map((r) => r.name).sort()
      expect(names).toEqual(['domains', 'organizations', 'support-requests'])

      const support = resources.find((r) => r.name === 'support-requests')!
      expect(support.labelPlural).toBe('Support Requests')
      // x-singular wins over the trailing-s heuristic
      expect(support.label).toBe('Support Request')
    })

    it('falls back to trailing-s singularization when x-singular is absent', async () => {
      const provider = createArestNavigationProvider({ baseUrl })
      stubFetch(() => json({
        paths: {
          '/arest/organizations': { get: {} },
        },
      }))
      const resources = await provider.resources()
      expect(resources[0].label).toBe('Organization')
      expect(resources[0].labelPlural).toBe('Organizations')
    })

    it('ignores non-/arest/ paths and template paths at depth > 1', async () => {
      const provider = createArestNavigationProvider({ baseUrl })
      stubFetch(() => json({
        paths: {
          '/api/entity': { post: {} },              // legacy /api/ route
          '/arest/': { get: {} },                   // root
          '/arest/organizations': { get: {} },      // yes
          '/arest/organizations/{id}': { get: {} }, // skip — entity, not collection
          '/arest/organizations/{id}/domains': { get: {} }, // skip — child collection
        },
      }))
      const resources = await provider.resources()
      expect(resources.map((r) => r.name)).toEqual(['organizations'])
    })

    it('hits the openapi.json endpoint on the configured baseUrl', async () => {
      const recorded = stubFetch(() => json({ paths: {} }))
      const provider = createArestNavigationProvider({ baseUrl, app: 'ui.do' })
      await provider.resources()
      expect(recorded[0]).toContain('/api/openapi.json')
      expect(recorded[0]).toContain('app=ui.do')
    })

    it('caches resources across calls until explicitly refreshed', async () => {
      const recorded = stubFetch(() => json({
        paths: { '/arest/organizations': { get: {} } },
      }))
      const provider = createArestNavigationProvider({ baseUrl })

      const a = await provider.resources()
      const b = await provider.resources()

      expect(recorded).toHaveLength(1) // cached
      expect(a).toEqual(b)
    })

    it('returns an empty array when openapi.json returns 404', async () => {
      stubFetch(() => json({ errors: [{ message: 'no app' }] }, 404))
      const provider = createArestNavigationProvider({ baseUrl })
      await expect(provider.resources()).resolves.toEqual([])
    })
  })

  describe('menu', () => {
    it('builds menu items from the resource list (title=labelPlural, url=/{slug})', async () => {
      stubFetch(() => json({
        paths: {
          '/arest/organizations': { get: {} },
          '/arest/support-requests': { get: {} },
        },
      }))
      const provider = createArestNavigationProvider({ baseUrl })

      const menu = await provider.menu()
      expect(menu).toEqual([
        { title: 'Organizations', url: '/organizations' },
        { title: 'Support Requests', url: '/support-requests' },
      ])
    })
  })
})

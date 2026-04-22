/**
 * ExternalSystem resource (#343) tests.
 *
 * Covers:
 *   - extractExternalSystemsFromDoc: discover mounted systems by
 *     walking `/external/{system}/types` paths in the OpenAPI doc.
 *   - createExternalSystemResource: build a ResourceDefinition that
 *     mounts a list view (fetches types list) and show view (fetches
 *     per-type BrowseResponse).
 *   - useExternalSystems: combined hook that fetches the doc and
 *     returns one ResourceDefinition per mounted system.
 */
import { afterEach, describe, expect, it, vi } from 'vitest'
import { render, screen, waitFor } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import type { ReactNode } from 'react'

vi.mock('@mdxui/admin', () => ({
  useCurrentResource: () => ({ resourceName: 'schema.org', recordId: 'Person', action: 'show' as const }),
  ListView: ({ title, children }: any) => <div data-testid="list-container"><h1>{title}</h1>{children}</div>,
  ShowView: ({ title, children }: any) => <div data-testid="show-container"><h1>{title}</h1>{children}</div>,
  EditView: ({ title, children }: any) => <div data-testid="edit-container"><h1>{title}</h1>{children}</div>,
  CreateView: ({ title, children }: any) => <div data-testid="create-container"><h1>{title}</h1>{children}</div>,
}))

import {
  extractExternalSystemsFromDoc,
  createExternalSystemResource,
  useExternalSystems,
} from '../ExternalSystem'

const baseUrl = 'https://ui.auto.dev/arest'

const docWithSchemaOrg = {
  openapi: '3.1.0',
  paths: {
    '/organizations': { get: {} },
    '/external/schema.org/types': { get: {} },
    '/external/schema.org/types/{name}': { get: {} },
  },
  components: { schemas: {} },
}

const docWithMultipleSystems = {
  openapi: '3.1.0',
  paths: {
    '/external/schema.org/types': { get: {} },
    '/external/schema.org/types/{name}': { get: {} },
    '/external/dcmi/types': { get: {} },
    '/external/dcmi/types/{name}': { get: {} },
  },
  components: { schemas: {} },
}

const docWithoutExternal = {
  openapi: '3.1.0',
  paths: { '/organizations': { get: {} } },
  components: { schemas: {} },
}

interface Recorded { url: string; method: string }

function stubFetch(handler: (req: Recorded) => Response): Recorded[] {
  const recorded: Recorded[] = []
  vi.stubGlobal('fetch', async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = typeof input === 'string' ? input : input instanceof URL ? input.toString() : input.url
    const method = (init?.method ?? 'GET').toUpperCase()
    recorded.push({ url, method })
    return handler({ url, method })
  })
  return recorded
}

function json(payload: unknown, status = 200): Response {
  return new Response(JSON.stringify(payload), { status, headers: { 'Content-Type': 'application/json' } })
}

function wrap(node: ReactNode) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false, staleTime: 0 } } })
  return <QueryClientProvider client={client}>{node}</QueryClientProvider>
}

afterEach(() => { vi.unstubAllGlobals() })

describe('extractExternalSystemsFromDoc', () => {
  it('pulls system names out of /external/{system}/types paths', () => {
    expect(extractExternalSystemsFromDoc(docWithSchemaOrg)).toEqual(['schema.org'])
  })

  it('dedupes across collection + item paths', () => {
    expect(extractExternalSystemsFromDoc(docWithMultipleSystems).sort())
      .toEqual(['dcmi', 'schema.org'])
  })

  it('returns empty when no /external paths exist', () => {
    expect(extractExternalSystemsFromDoc(docWithoutExternal)).toEqual([])
  })

  it('tolerates missing paths / non-object docs', () => {
    expect(extractExternalSystemsFromDoc(null)).toEqual([])
    expect(extractExternalSystemsFromDoc({})).toEqual([])
    expect(extractExternalSystemsFromDoc({ paths: null })).toEqual([])
  })
})

describe('createExternalSystemResource', () => {
  it('produces a ResourceDefinition whose name is the system slug', () => {
    const def = createExternalSystemResource('schema.org', { baseUrl })
    // URL-safe name: dots → dashes so router paths work.
    expect(def.name).toBe('external-schema-org')
    expect(def.options?.label).toBe('schema.org')
    expect(typeof def.list).toBe('function')
    expect(typeof def.show).toBe('function')
    expect(def.create).toBeUndefined()
    expect(def.edit).toBeUndefined()
  })

  it('list component fetches /external/{system}/types', async () => {
    const recorded = stubFetch((req) => {
      if (req.url.includes('/types/Person')) return json({})
      if (req.url.endsWith('/external/schema.org/types')) {
        return json(['Thing', 'Person', 'Organization'])
      }
      return json({})
    })

    const def = createExternalSystemResource('schema.org', { baseUrl })
    const List = def.list!
    render(wrap(<List />))
    await waitFor(() => {
      const hit = recorded.find((r) => r.url.endsWith('/external/schema.org/types'))
      expect(hit).toBeDefined()
    })
  })

  it('show component fetches the per-type BrowseResponse', async () => {
    const recorded = stubFetch((req) => {
      if (req.url.endsWith('/external/schema.org/types/Person')) {
        return json({
          type: 'Person',
          supertypes: ['Thing'],
          subtypes: ['Patient'],
          properties: [
            { name: 'name', range: 'Text' },
            { name: 'birthDate', range: 'Date' },
          ],
        })
      }
      return json({})
    })

    const def = createExternalSystemResource('schema.org', { baseUrl })
    const Show = def.show!
    render(wrap(<Show />))
    await waitFor(() => {
      const hit = recorded.find((r) => r.url.endsWith('/external/schema.org/types/Person'))
      expect(hit).toBeDefined()
    })
    // Show view renders at least the type name and inherited properties.
    await waitFor(() => {
      expect(screen.getByText(/Person/)).toBeDefined()
      expect(screen.getByText(/birthDate/)).toBeDefined()
    })
  })
})

describe('useExternalSystems', () => {
  function Probe({ opts }: { opts: { baseUrl: string; app?: string } }) {
    const { resources, isLoading } = useExternalSystems(opts)
    if (isLoading) return <p>loading</p>
    return (
      <ul data-testid="external-list">
        {resources.map((r) => (
          <li key={r.name} data-testid={`external-${r.name}`}>{r.name}:{r.options?.label}</li>
        ))}
      </ul>
    )
  }

  it('maps every /external/{system}/types path in the OpenAPI doc to a ResourceDefinition', async () => {
    stubFetch(() => json(docWithSchemaOrg))
    render(wrap(<Probe opts={{ baseUrl, app: 'ui.do' }} />))
    await waitFor(() => expect(screen.getByTestId('external-list')).toBeDefined())
    expect(screen.getByTestId('external-external-schema-org')).toBeDefined()
  })

  it('returns empty resources when no systems are mounted', async () => {
    stubFetch(() => json(docWithoutExternal))
    render(wrap(<Probe opts={{ baseUrl, app: 'ui.do' }} />))
    await waitFor(() => expect(screen.getByTestId('external-list')).toBeDefined())
    expect(screen.getByTestId('external-list').children.length).toBe(0)
  })
})

/**
 * createArestResource / useArestResources tests.
 *
 * Mocks @mdxui/admin's useCurrentResource (and the four view
 * containers — see GenericViews.test.tsx for why) so the
 * ResourceDefinition's components can render in jsdom without an
 * AdminRouter wrapper. Tests assert:
 *   - produced definition shape (name = slug, all four ComponentTypes,
 *     label defaults)
 *   - mounting each produced component makes the expected fetches
 *   - auto-discovery maps every entity schema to a ResourceDefinition
 *   - value schemas are skipped
 */
import { afterEach, describe, expect, it, vi } from 'vitest'
import { render, screen, waitFor } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import type { ReactNode } from 'react'

// Mock useCurrentResource so Show/Edit components render in tests
// without an AdminRouter. And mock the four container primitives for
// the same reason as GenericViews.test.tsx (mdxui/admin's emoji-mart
// transitive breaks strict Node ESM).
vi.mock('@mdxui/admin', () => ({
  useCurrentResource: () => ({ resourceName: 'organizations', recordId: 'acme', action: 'edit' as const }),
  ListView: ({ title, children }: any) => <div data-testid="list-container"><h1>{title}</h1>{children}</div>,
  ShowView: ({ title, children }: any) => <div data-testid="show-container"><h1>{title}</h1>{children}</div>,
  EditView: ({ title, children }: any) => <div data-testid="edit-container"><h1>{title}</h1>{children}</div>,
  CreateView: ({ title, children }: any) => <div data-testid="create-container"><h1>{title}</h1>{children}</div>,
}))

import { createArestResource } from '../createArestResource'
import { extractNounsFromDoc, useArestResources } from '../useArestResources'

const baseUrl = 'https://ui.auto.dev/arest'

const sampleDoc = {
  openapi: '3.1.0',
  components: {
    schemas: {
      Organization: {
        type: 'object',
        properties: {
          id: { type: 'string' },
          name: { type: 'string', title: 'Legal Name' },
        },
        required: ['name'],
      },
      SupportRequest: {
        type: 'object',
        properties: {
          id: { type: 'string' },
          title: { type: 'string' },
        },
      },
      // Value type — should be skipped.
      Tier: {
        type: 'string',
        enum: ['Starter', 'Pro', 'Enterprise'],
      },
    },
  },
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

describe('createArestResource', () => {
  it('produces a ResourceDefinition matching @mdxui/admin\'s shape', () => {
    const def = createArestResource('Support Request', { baseUrl })
    expect(def.name).toBe('support-requests')
    expect(typeof def.list).toBe('function')
    expect(typeof def.show).toBe('function')
    expect(typeof def.edit).toBe('function')
    expect(typeof def.create).toBe('function')
    expect(def.options?.label).toBe('Support Requests')
    // Component displayNames surface the noun for React devtools.
    expect((def.list as { displayName?: string }).displayName).toBe('Support RequestListView')
  })

  it('forwards label / icon / hideFromMenu overrides', () => {
    const icon = <span data-testid="icon" />
    const def = createArestResource('Organization', {
      baseUrl,
      label: 'Orgs',
      icon,
      hideFromMenu: true,
    })
    expect(def.options?.label).toBe('Orgs')
    expect(def.icon).toBe(icon)
    expect(def.options?.hideFromMenu).toBe(true)
  })

  it('list component fetches /arest/{slug} when rendered', async () => {
    const recorded = stubFetch((req) => {
      if (req.url.includes('/api/openapi.json')) return json(sampleDoc)
      return json({ data: [{ id: 'o1', name: 'Org 1' }], _links: {} })
    })

    const def = createArestResource('Organization', { baseUrl })
    const List = def.list!
    render(wrap(<List />))
    await waitFor(() => expect(screen.getByTestId('list-container')).toBeDefined())
    // GenericListView now sends pagination params by default, so the
    // URL is /arest/organizations?page=1&perPage=20. Match on the path
    // rather than exact end.
    const listFetch = recorded.find((r) => r.url.includes('/arest/organizations'))
    expect(listFetch).toBeDefined()
  })

  it('edit component picks the record id out of useCurrentResource', async () => {
    const recorded = stubFetch((req) => {
      if (req.url.includes('/api/openapi.json')) return json(sampleDoc)
      return json({ data: { id: 'acme', name: 'Acme' }, _links: {} })
    })

    const def = createArestResource('Organization', { baseUrl })
    const Edit = def.edit!
    render(wrap(<Edit />))
    // The mocked useCurrentResource returns recordId='acme', so the
    // edit view fetches /arest/organizations/acme.
    await waitFor(() => {
      const fetched = recorded.find((r) => r.url.endsWith('/arest/organizations/acme'))
      expect(fetched).toBeDefined()
    })
  })
})

describe('extractNounsFromDoc', () => {
  it('returns entity schema names and skips value types', () => {
    expect(extractNounsFromDoc(sampleDoc).sort()).toEqual(['Organization', 'SupportRequest'])
  })

  it('tolerates missing components / schemas', () => {
    expect(extractNounsFromDoc(null)).toEqual([])
    expect(extractNounsFromDoc({})).toEqual([])
    expect(extractNounsFromDoc({ components: {} })).toEqual([])
  })
})

describe('useArestResources', () => {
  function Probe({ opts }: { opts: { baseUrl: string; app?: string } }) {
    const { resources, isLoading } = useArestResources(opts)
    if (isLoading) return <p>loading</p>
    return (
      <ul data-testid="resource-list">
        {resources.map((r) => (
          <li key={r.name} data-testid={`resource-${r.name}`}>
            {r.name}:{r.options?.label}
          </li>
        ))}
      </ul>
    )
  }

  it('maps every entity schema in the OpenAPI doc to a ResourceDefinition', async () => {
    stubFetch(() => json(sampleDoc))
    render(wrap(<Probe opts={{ baseUrl, app: 'ui.do' }} />))
    await waitFor(() => expect(screen.getByTestId('resource-list')).toBeDefined())
    expect(screen.getByTestId('resource-organizations')).toBeDefined()
    expect(screen.getByTestId('resource-support-requests')).toBeDefined()
    // Value type Tier is skipped.
    expect(screen.queryByTestId('resource-tiers')).toBeNull()
    // Default plural label applied.
    expect(screen.getByTestId('resource-organizations').textContent).toContain('Organizations')
  })

  it('honours the optional `filter` prop — branding-driven noun scoping', async () => {
    stubFetch(() => json(sampleDoc))
    function FilteredProbe() {
      const { resources, isLoading } = useArestResources({
        baseUrl,
        app: 'support.do',
        filter: (n) => n === 'SupportRequest',
      })
      if (isLoading) return <p>loading</p>
      return (
        <ul data-testid="resource-list">
          {resources.map((r) => (
            <li key={r.name} data-testid={`resource-${r.name}`}>{r.name}</li>
          ))}
        </ul>
      )
    }
    render(wrap(<FilteredProbe />))
    await waitFor(() => expect(screen.getByTestId('resource-list')).toBeDefined())
    expect(screen.getByTestId('resource-support-requests')).toBeDefined()
    expect(screen.queryByTestId('resource-organizations')).toBeNull()
  })
})

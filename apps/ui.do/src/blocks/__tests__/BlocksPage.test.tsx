/**
 * BlocksPage tests — render AREST ui-domain rows through a block
 * registry. Confirms:
 *   - each row's `type` routes to the matching registry entry
 *   - rows are ordered by the `order` field
 *   - unknown types fall through to the fallback
 *   - default registry renders hero / features / text as sections
 *   - parent-scoped fetch maps to a child collection URL
 */
import { afterEach, describe, expect, it, vi } from 'vitest'
import { render, screen, waitFor } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import type { ReactNode } from 'react'
import { BlocksPage, DEFAULT_BLOCK_REGISTRY } from '../BlocksPage'

const baseUrl = 'https://ui.auto.dev/arest'

function stubFetch(handler: (url: string) => Response): string[] {
  const urls: string[] = []
  vi.stubGlobal('fetch', async (input: RequestInfo | URL) => {
    const url = typeof input === 'string' ? input : input instanceof URL ? input.toString() : input.url
    urls.push(url)
    return handler(url)
  })
  return urls
}

function json(payload: unknown, status = 200): Response {
  return new Response(JSON.stringify(payload), { status, headers: { 'Content-Type': 'application/json' } })
}

function wrap(node: ReactNode) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false, staleTime: 0 } } })
  return <QueryClientProvider client={client}>{node}</QueryClientProvider>
}

afterEach(() => { vi.unstubAllGlobals() })

describe('BlocksPage', () => {
  it('routes each row to the matching registry entry and orders by `order`', async () => {
    stubFetch(() => json({
      data: [
        { id: 's3', type: 'text',     order: 3, title: 'About',     body: 'Text body' },
        { id: 's1', type: 'hero',     order: 1, title: 'Welcome',   subtitle: 'Tagline' },
        { id: 's2', type: 'features', order: 2, title: 'Features',  items: [{ label: 'A' }, { label: 'B' }] },
      ],
      _links: {},
    }))

    render(wrap(<BlocksPage noun="Section" baseUrl={baseUrl} />))
    await waitFor(() => expect(screen.getByTestId('block-hero-s1')).toBeDefined())

    // Order the children by order field: s1 -> s2 -> s3.
    const page = screen.getByTestId('blocks-page')
    const sections = page.querySelectorAll('[data-block-type]')
    const kinds = Array.from(sections).map((el) => el.getAttribute('data-block-type'))
    expect(kinds).toEqual(['hero', 'features', 'text'])

    expect(screen.getByTestId('block-features-s2').textContent).toMatch(/A/)
    expect(screen.getByTestId('block-features-s2').textContent).toMatch(/B/)
  })

  it('uses the fallback when a row.type is not in the registry', async () => {
    stubFetch(() => json({
      data: [{ id: 'sx', type: 'gallery', order: 1, images: ['a.jpg'] }],
      _links: {},
    }))
    render(wrap(<BlocksPage noun="Section" baseUrl={baseUrl} />))
    await waitFor(() => expect(screen.getByTestId('block-unknown-sx')).toBeDefined())
  })

  it('accepts a custom registry so consumers can plug mdxui blocks', async () => {
    stubFetch(() => json({
      data: [{ id: 'c1', type: 'custom', order: 1, label: 'Custom block' }],
      _links: {},
    }))

    const registry = {
      ...DEFAULT_BLOCK_REGISTRY,
      custom: ({ row }: { row: Record<string, unknown> }) => (
        <section data-testid={`block-custom-${row.id as string}`}>
          {String(row.label)}
        </section>
      ),
    }

    render(wrap(<BlocksPage noun="Section" baseUrl={baseUrl} registry={registry} />))
    await waitFor(() => expect(screen.getByTestId('block-custom-c1').textContent).toBe('Custom block'))
  })

  it('scopes to a child collection under a parent entity when `parent` is passed', async () => {
    const urls = stubFetch((url) => {
      if (url.includes('/arest/pages/home')) {
        return json({ data: { id: 'home', title: 'Home' }, _links: {} })
      }
      if (url.includes('/arest/sections')) {
        return json({ data: [], _links: {} })
      }
      return json({ data: [], _links: {} })
    })

    render(wrap(
      <BlocksPage
        noun="Section"
        baseUrl={baseUrl}
        parent={{ noun: 'Page', id: 'home' }}
      />,
    ))

    await waitFor(() => expect(urls.some((u) => u.endsWith('/arest/pages/home'))).toBe(true))
    // The parent-scoped list goes through filter[belongs to Page] on the
    // sections collection — keeps the data provider path consistent.
    const listUrl = urls.find((u) => u.includes('/arest/sections?'))
    expect(listUrl).toBeDefined()
    // URLSearchParams encodes spaces as '+', so decode via the parser
    // rather than decodeURIComponent (which leaves '+' as-is).
    const parsed = new URL(listUrl!)
    expect(parsed.searchParams.get('filter[belongs to Page]')).toBe('home')
  })
})

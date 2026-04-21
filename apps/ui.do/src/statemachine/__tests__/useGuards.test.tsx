import { afterEach, describe, expect, it, vi } from 'vitest'
import { act, renderHook, waitFor } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import type { ReactNode } from 'react'
import { useGuards } from '../useGuards'

const baseUrl = 'https://ui.auto.dev/arest'

interface Recorded { url: string; method: string; body?: unknown }

function stubFetch(handler: (req: Recorded) => Response): Recorded[] {
  const recorded: Recorded[] = []
  vi.stubGlobal('fetch', async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = typeof input === 'string' ? input : input instanceof URL ? input.toString() : input.url
    const method = (init?.method ?? 'GET').toUpperCase()
    let body: unknown
    if (init?.body != null) {
      try { body = JSON.parse(init.body as string) } catch { body = init.body }
    }
    const req = { url, method, body }
    recorded.push(req)
    return handler(req)
  })
  return recorded
}

function json(payload: unknown, status = 200): Response {
  return new Response(JSON.stringify(payload), { status, headers: { 'Content-Type': 'application/json' } })
}

function wrap(client: QueryClient) {
  return function Wrapper({ children }: { children: ReactNode }) {
    return <QueryClientProvider client={client}>{children}</QueryClientProvider>
  }
}

afterEach(() => { vi.unstubAllGlobals() })

describe('useGuards', () => {
  it('fetches /arest/guards filtered by transition id', async () => {
    const recorded = stubFetch(() => json({
      data: [
        { id: 'payment-required', transition: 'ship', factType: 'Order_has_Payment' },
      ],
      _links: {},
    }))

    const client = new QueryClient({ defaultOptions: { queries: { retry: false, staleTime: 0 } } })
    const { result } = renderHook(
      () => useGuards('ship', { baseUrl }),
      { wrapper: wrap(client) },
    )
    await waitFor(() => expect(result.current.isLoading).toBe(false))

    expect(result.current.guards).toHaveLength(1)
    const url = new URL(recorded[0].url)
    expect(url.searchParams.get('filter[transition]')).toBe('ship')
  })

  it('addGuard POSTs the guard and sets the transition reference', async () => {
    const recorded = stubFetch((req) => {
      if (req.method === 'POST') {
        return json({ data: { id: 'needs-review', transition: 'approve' } }, 201)
      }
      return json({ data: [], _links: {} })
    })
    const client = new QueryClient({ defaultOptions: { queries: { retry: false, staleTime: 0 } } })
    const { result } = renderHook(
      () => useGuards('approve', { baseUrl }),
      { wrapper: wrap(client) },
    )
    await waitFor(() => expect(result.current.isLoading).toBe(false))

    await act(async () => {
      await result.current.addGuard({ id: 'needs-review' })
    })

    const post = recorded.find((r) => r.method === 'POST')
    expect(post).toBeDefined()
    expect((post!.body as { id: string; transition: string }).transition).toBe('approve')
  })

  it('deleteGuard DELETEs /arest/guards/<id>', async () => {
    const recorded = stubFetch((req) => {
      if (req.method === 'DELETE') return json({ data: { id: 'x' } })
      return json({ data: [], _links: {} })
    })
    const client = new QueryClient({ defaultOptions: { queries: { retry: false, staleTime: 0 } } })
    const { result } = renderHook(
      () => useGuards('ship', { baseUrl }),
      { wrapper: wrap(client) },
    )
    await waitFor(() => expect(result.current.isLoading).toBe(false))

    await act(async () => {
      await result.current.deleteGuard('payment-required')
    })
    const del = recorded.find((r) => r.method === 'DELETE')
    expect(del!.url).toBe('https://ui.auto.dev/arest/guards/payment-required')
  })
})

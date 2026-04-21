/**
 * useActions tests (#124).
 *
 * Exercises the wire contract against the real router endpoints
 * (src/api/router.ts:573–677):
 *   GET  /api/entities/{noun}/{id}/transitions
 *   POST /api/entities/{noun}/{id}/transition  body: { event }
 *
 * baseUrl is the /arest-rooted URL the data provider uses; the hook
 * strips /arest to reach the sibling /api route.
 */
import { afterEach, describe, expect, it, vi } from 'vitest'
import { act, renderHook, waitFor } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import type { ReactNode } from 'react'
import { useActions } from '../useActions'

interface Recorded {
  url: string
  method: string
  body?: unknown
  credentials?: RequestCredentials
}

function stubFetch(responder: (req: Recorded) => Response): Recorded[] {
  const recorded: Recorded[] = []
  vi.stubGlobal('fetch', async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = typeof input === 'string' ? input : input instanceof URL ? input.toString() : input.url
    const method = (init?.method ?? 'GET').toUpperCase()
    let body: unknown
    if (init?.body != null) {
      try { body = JSON.parse(init.body as string) } catch { body = init.body }
    }
    const req: Recorded = { url, method, body, credentials: init?.credentials }
    recorded.push(req)
    return responder(req)
  })
  return recorded
}

function json(payload: unknown, status = 200): Response {
  return new Response(JSON.stringify(payload), {
    status,
    headers: { 'Content-Type': 'application/json' },
  })
}

function makeWrapper(queryClient: QueryClient) {
  return function Wrapper({ children }: { children: ReactNode }) {
    return <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  }
}

function makeClient() {
  return new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: 0 } },
  })
}

const baseUrl = 'https://ui.auto.dev/arest'

describe('useActions', () => {
  afterEach(() => { vi.unstubAllGlobals() })

  it('GETs /api/entities/{noun}/{id}/transitions and maps to {name,to,label}', async () => {
    const recorded = stubFetch(() => json({
      currentStatus: 'In Cart',
      transitions: [
        { event: 'place', targetStatus: 'Placed' },
        { event: 'cancel', targetStatus: 'Cancelled', label: 'Cancel order' },
      ],
    }))
    const queryClient = makeClient()
    const { result } = renderHook(
      () => useActions('Order', 'ord-1', { baseUrl }),
      { wrapper: makeWrapper(queryClient) },
    )

    expect(result.current.isLoading).toBe(true)
    await waitFor(() => expect(result.current.isLoading).toBe(false))

    expect(recorded[0].url).toBe('https://ui.auto.dev/api/entities/Order/ord-1/transitions')
    expect(recorded[0].method).toBe('GET')
    expect(recorded[0].credentials).toBe('include')

    expect(result.current.currentStatus).toBe('In Cart')
    expect(result.current.actions).toEqual([
      { name: 'place', to: 'Placed', label: 'Place' },
      { name: 'cancel', to: 'Cancelled', label: 'Cancel order' },
    ])
  })

  it('URL-encodes multi-word nouns ("Support Request" -> "Support%20Request")', async () => {
    const recorded = stubFetch(() => json({ transitions: [] }))
    const queryClient = makeClient()
    renderHook(
      () => useActions('Support Request', 'sr-1', { baseUrl }),
      { wrapper: makeWrapper(queryClient) },
    )
    await waitFor(() => expect(recorded).toHaveLength(1))
    expect(recorded[0].url).toBe('https://ui.auto.dev/api/entities/Support%20Request/sr-1/transitions')
  })

  it('accepts a Thm-5 envelope shape from the transitions GET', async () => {
    stubFetch(() => json({
      data: { currentStatus: 'Placed', transitions: [{ event: 'ship', targetStatus: 'Shipped' }] },
      _links: {},
    }))
    const queryClient = makeClient()
    const { result } = renderHook(
      () => useActions('Order', 'ord-1', { baseUrl }),
      { wrapper: makeWrapper(queryClient) },
    )
    await waitFor(() => expect(result.current.isLoading).toBe(false))
    expect(result.current.currentStatus).toBe('Placed')
    expect(result.current.actions).toEqual([{ name: 'ship', to: 'Shipped', label: 'Ship' }])
  })

  it('dispatch POSTs /api/entities/{noun}/{id}/transition with {event} body and invalidates keys', async () => {
    const recorded = stubFetch((req) => {
      if (req.method === 'GET') return json({ transitions: [{ event: 'place', targetStatus: 'Placed' }] })
      return json({ id: 'ord-1', status: 'Placed', event: 'place', transitions: [] })
    })

    const queryClient = makeClient()
    queryClient.setQueryData(['arest', 'list', 'orders'], { data: [] })
    queryClient.setQueryData(['arest', 'one', 'orders', 'ord-1'], { data: {} })
    queryClient.setQueryData(['arest', 'reference', 'orders', 'customers', 'acme'], { data: [] })

    const { result } = renderHook(
      () => useActions('Order', 'ord-1', { baseUrl }),
      { wrapper: makeWrapper(queryClient) },
    )
    await waitFor(() => expect(result.current.isLoading).toBe(false))

    await act(async () => {
      await result.current.dispatch('place')
    })

    const post = recorded.find((r) => r.method === 'POST')
    expect(post).toBeDefined()
    expect(post!.url).toBe('https://ui.auto.dev/api/entities/Order/ord-1/transition')
    expect(post!.credentials).toBe('include')
    expect(post!.body).toEqual({ event: 'place' })

    expect(queryClient.getQueryState(['arest', 'list', 'orders'])?.isInvalidated).toBe(true)
    expect(queryClient.getQueryState(['arest', 'one', 'orders', 'ord-1'])?.isInvalidated).toBe(true)
    expect(queryClient.getQueryState(['arest', 'reference', 'orders', 'customers', 'acme'])?.isInvalidated).toBe(true)
  })

  it('dispatch forwards `domain` to the POST body when configured', async () => {
    const recorded = stubFetch((req) => {
      if (req.method === 'GET') return json({ transitions: [{ event: 'ship', targetStatus: 'Shipped' }] })
      return json({ id: 'ord-1', status: 'Shipped' })
    })
    const queryClient = makeClient()
    const { result } = renderHook(
      () => useActions('Order', 'ord-1', { baseUrl, domain: 'orders' }),
      { wrapper: makeWrapper(queryClient) },
    )
    await waitFor(() => expect(result.current.isLoading).toBe(false))
    await act(async () => { await result.current.dispatch('ship') })
    const post = recorded.find((r) => r.method === 'POST')!
    expect(post.body).toEqual({ event: 'ship', domain: 'orders' })
  })

  it('dispatch rejects with violation detail on 422', async () => {
    stubFetch((req) => {
      if (req.method === 'GET') return json({ transitions: [{ event: 'ship', targetStatus: 'Shipped' }] })
      return json({
        errors: [{ message: 'order has no payment' }],
        violations: [{
          reading: 'Each Order has exactly one Payment.',
          constraintId: 'c-pay',
          modality: 'alethic',
          detail: 'order has no payment',
        }],
      }, 422)
    })

    const queryClient = makeClient()
    const { result } = renderHook(
      () => useActions('Order', 'ord-1', { baseUrl }),
      { wrapper: makeWrapper(queryClient) },
    )
    await waitFor(() => expect(result.current.isLoading).toBe(false))

    await expect(
      act(() => result.current.dispatch('ship')),
    ).rejects.toThrow(/order has no payment/)
  })

  it('URL-encodes IDs with special characters', async () => {
    const recorded = stubFetch(() => json({ transitions: [] }))
    const queryClient = makeClient()
    renderHook(
      () => useActions('Order', 'orders:2026/05', { baseUrl }),
      { wrapper: makeWrapper(queryClient) },
    )
    await waitFor(() => expect(recorded).toHaveLength(1))
    expect(recorded[0].url).toBe('https://ui.auto.dev/api/entities/Order/orders%3A2026%2F05/transitions')
  })
})

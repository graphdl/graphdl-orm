/**
 * useActions tests (#124).
 *
 * Stubs globalThis.fetch with a recorder. Hook runs inside a
 * QueryClientProvider so invalidation can be asserted on the live
 * cache. Tests the wire contract end-to-end:
 *   GET  /arest/{slug}/{id}/actions -> available transitions
 *   POST /arest/{slug}/{id}/{action} -> fires the transition
 *   on success, invalidates list + one + reference keys
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

  it('GETs /arest/{slug}/{id}/actions and maps transitions to {name,to,label}', async () => {
    const recorded = stubFetch(() => json({
      data: {
        entity: 'ord-1',
        noun: 'Order',
        status: 'In Cart',
        transitions: [
          { event: 'place', targetStatus: 'Placed' },
          { event: 'cancel', targetStatus: 'Cancelled', label: 'Cancel order' },
        ],
      },
    }))
    const queryClient = makeClient()
    const { result } = renderHook(
      () => useActions('Order', 'ord-1', { baseUrl }),
      { wrapper: makeWrapper(queryClient) },
    )

    expect(result.current.isLoading).toBe(true)
    await waitFor(() => expect(result.current.isLoading).toBe(false))

    // Support Request slugifies to "support-requests"; "Order" → "orders".
    expect(recorded[0].url).toBe('https://ui.auto.dev/arest/orders/ord-1/actions')
    expect(recorded[0].method).toBe('GET')
    expect(recorded[0].credentials).toBe('include')

    expect(result.current.actions).toEqual([
      { name: 'place', to: 'Placed', label: 'Place' },
      { name: 'cancel', to: 'Cancelled', label: 'Cancel order' },
    ])
  })

  it('slugifies multi-word nouns ("Support Request" -> "support-requests")', async () => {
    const recorded = stubFetch(() => json({ data: { transitions: [] } }))
    const queryClient = makeClient()
    renderHook(
      () => useActions('Support Request', 'sr-1', { baseUrl }),
      { wrapper: makeWrapper(queryClient) },
    )
    await waitFor(() => expect(recorded).toHaveLength(1))
    expect(recorded[0].url).toBe('https://ui.auto.dev/arest/support-requests/sr-1/actions')
  })

  it('accepts a bare array transitions shape', async () => {
    // Some handlers return [{event, targetStatus}, ...] directly, without
    // the envelope. The hook tolerates both.
    stubFetch(() => json([
      { event: 'ship', targetStatus: 'Shipped' },
    ]))
    const queryClient = makeClient()
    const { result } = renderHook(
      () => useActions('Order', 'ord-1', { baseUrl }),
      { wrapper: makeWrapper(queryClient) },
    )
    await waitFor(() => expect(result.current.isLoading).toBe(false))
    expect(result.current.actions).toEqual([
      { name: 'ship', to: 'Shipped', label: 'Ship' },
    ])
  })

  it('dispatch POSTs /arest/{slug}/{id}/{action-name} and invalidates cache keys', async () => {
    const recorded = stubFetch((req) => {
      if (req.method === 'GET') {
        return json({ data: { transitions: [{ event: 'place', targetStatus: 'Placed' }] } })
      }
      return json({ data: { id: 'ord-1', status: 'Placed' } })
    })

    const queryClient = makeClient()
    // Seed queries so we can observe their invalidation post-dispatch.
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
    expect(post!.url).toBe('https://ui.auto.dev/arest/orders/ord-1/place')
    expect(post!.credentials).toBe('include')

    expect(queryClient.getQueryState(['arest', 'list', 'orders'])?.isInvalidated).toBe(true)
    expect(queryClient.getQueryState(['arest', 'one', 'orders', 'ord-1'])?.isInvalidated).toBe(true)
    expect(queryClient.getQueryState(['arest', 'reference', 'orders', 'customers', 'acme'])?.isInvalidated).toBe(true)
  })

  it('dispatch rejects with violation detail when worker returns 422', async () => {
    stubFetch((req) => {
      if (req.method === 'GET') {
        return json({ data: { transitions: [{ event: 'ship', targetStatus: 'Shipped' }] } })
      }
      return json({
        data: null,
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
    const recorded = stubFetch(() => json({ data: { transitions: [] } }))
    const queryClient = makeClient()
    renderHook(
      () => useActions('Order', 'orders:2026/05', { baseUrl }),
      { wrapper: makeWrapper(queryClient) },
    )
    await waitFor(() => expect(recorded).toHaveLength(1))
    expect(recorded[0].url).toBe('https://ui.auto.dev/arest/orders/orders%3A2026%2F05/actions')
  })
})

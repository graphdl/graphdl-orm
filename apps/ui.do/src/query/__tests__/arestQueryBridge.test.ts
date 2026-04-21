/**
 * arestQueryBridge — TanStack Query + SSE cache invalidation.
 *
 * Wires an EventSource subscriber that listens on /api/events and, on
 * every matching CellEvent, invalidates the TanStack Query keys that
 * describe the affected resource. The invalidation mapping is:
 *
 *   CellEvent{ noun: 'SupportRequest', entityId, operation }
 *     -> invalidate ['arest', 'list',   'support-requests']
 *     -> invalidate ['arest', 'one',    'support-requests', entityId]
 *     -> invalidate ['arest', 'reference', ...]  (conservative)
 *
 * Integration requirement (from task #123): a create via
 * dataProvider triggers an SSE message, the list-query cache
 * invalidates, and the list re-runs within 500ms.
 */
import { QueryClient, QueryObserver } from '@tanstack/react-query'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createArestQueryBridge, createArestQueryKeys } from '../arestQueryBridge'

class MockEventSource {
  static instances: MockEventSource[] = []
  url: string
  onmessage: ((e: MessageEvent) => void) | null = null
  onerror: ((e: Event) => void) | null = null
  readyState = 0
  closed = false

  constructor(url: string) {
    this.url = url
    MockEventSource.instances.push(this)
    // Simulate open on next microtask.
    queueMicrotask(() => {
      this.readyState = 1
    })
  }

  close(): void {
    this.closed = true
    this.readyState = 2
  }

  /** Test helper: simulate a data frame arriving on the stream. */
  emit(payload: unknown): void {
    const data = typeof payload === 'string' ? payload : JSON.stringify(payload)
    const event = { data } as MessageEvent
    this.onmessage?.(event)
  }
}

describe('createArestQueryKeys', () => {
  it('builds a stable list key', () => {
    const keys = createArestQueryKeys('organizations')
    expect(keys.list()).toEqual(['arest', 'list', 'organizations'])
    expect(keys.list({ page: 1 })).toEqual(['arest', 'list', 'organizations', { page: 1 }])
  })

  it('builds a stable one key', () => {
    const keys = createArestQueryKeys('organizations')
    expect(keys.one('acme')).toEqual(['arest', 'one', 'organizations', 'acme'])
  })

  it('builds a reference key', () => {
    const keys = createArestQueryKeys('support-requests')
    expect(keys.reference('organizations', 'acme')).toEqual([
      'arest', 'reference', 'support-requests', 'organizations', 'acme',
    ])
  })
})

describe('arestQueryBridge — SSE → queryClient.invalidateQueries', () => {
  let queryClient: QueryClient
  // `vi.spyOn` on a bound instance gets typed narrowly; using `vi.fn` and
  // an `any` cast lets us keep the spy's .mock.calls surface while
  // satisfying QueryClient's broad generic signature.
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  let invalidateSpy: any

  beforeEach(() => {
    MockEventSource.instances = []
    queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false, staleTime: 0 } },
    })
    const realInvalidate = queryClient.invalidateQueries.bind(queryClient)
    invalidateSpy = vi.fn((...args) => realInvalidate(...args))
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    ;(queryClient as any).invalidateQueries = invalidateSpy
    vi.stubGlobal('EventSource', MockEventSource)
  })

  afterEach(() => {
    vi.unstubAllGlobals()
  })

  it('opens an EventSource at {workerRoot}/api/events?domain=<domain>', () => {
    createArestQueryBridge({
      baseUrl: 'https://ui.auto.dev/arest',
      domain: 'organizations',
      queryClient,
    })
    expect(MockEventSource.instances).toHaveLength(1)
    const url = new URL(MockEventSource.instances[0].url)
    expect(url.origin + url.pathname).toBe('https://ui.auto.dev/api/events')
    expect(url.searchParams.get('domain')).toBe('organizations')
  })

  it('invalidates the matching collection list on create events', async () => {
    const bridge = createArestQueryBridge({
      baseUrl: 'https://ui.auto.dev/arest',
      domain: 'organizations',
      queryClient,
    })
    MockEventSource.instances[0].emit({
      domain: 'organizations',
      noun: 'Organization',
      entityId: 'acme',
      operation: 'create',
      facts: {},
      timestamp: Date.now(),
      sequence: 1,
      cellKey: 'Organization:acme',
    })

    await new Promise((r) => setTimeout(r, 10))
    expect(invalidateSpy).toHaveBeenCalledWith(
      expect.objectContaining({ queryKey: ['arest', 'list', 'organizations'] }),
    )
    bridge.close()
  })

  it('invalidates the per-entity key on update events', async () => {
    const bridge = createArestQueryBridge({
      baseUrl: 'https://ui.auto.dev/arest',
      domain: 'organizations',
      queryClient,
    })
    MockEventSource.instances[0].emit({
      domain: 'organizations',
      noun: 'Organization',
      entityId: 'acme',
      operation: 'update',
      facts: { name: 'Acme Inc.' },
      timestamp: Date.now(),
      sequence: 2,
      cellKey: 'Organization:acme',
    })
    await new Promise((r) => setTimeout(r, 10))
    const calls = (invalidateSpy.mock.calls as unknown[][]).map((c) => c[0])
    expect(calls).toContainEqual(expect.objectContaining({
      queryKey: ['arest', 'one', 'organizations', 'acme'],
    }))
    expect(calls).toContainEqual(expect.objectContaining({
      queryKey: ['arest', 'list', 'organizations'],
    }))
    bridge.close()
  })

  it('does NOT invalidate unrelated resources', async () => {
    createArestQueryBridge({
      baseUrl: 'https://ui.auto.dev/arest',
      domain: 'organizations',
      queryClient,
    })
    MockEventSource.instances[0].emit({
      domain: 'organizations',
      noun: 'SupportRequest',
      entityId: 'sr-1',
      operation: 'create',
      facts: {},
      timestamp: Date.now(),
      sequence: 3,
      cellKey: 'SupportRequest:sr-1',
    })
    await new Promise((r) => setTimeout(r, 10))
    const calls = (invalidateSpy.mock.calls as unknown[][]).map((c) => c[0])
    // Organizations list should not be touched.
    expect(calls).not.toContainEqual(expect.objectContaining({
      queryKey: ['arest', 'list', 'organizations'],
    }))
    // SupportRequests list SHOULD be invalidated.
    expect(calls).toContainEqual(expect.objectContaining({
      queryKey: ['arest', 'list', 'support-requests'],
    }))
  })

  it('ignores non-JSON SSE frames quietly', async () => {
    createArestQueryBridge({
      baseUrl: 'https://ui.auto.dev/arest',
      domain: 'organizations',
      queryClient,
    })
    MockEventSource.instances[0].emit(': keepalive')
    MockEventSource.instances[0].emit('not json')
    await new Promise((r) => setTimeout(r, 10))
    expect(invalidateSpy).not.toHaveBeenCalled()
  })

  it('close() tears down the EventSource', () => {
    const bridge = createArestQueryBridge({
      baseUrl: 'https://ui.auto.dev/arest',
      domain: 'organizations',
      queryClient,
    })
    expect(MockEventSource.instances[0].closed).toBe(false)
    bridge.close()
    expect(MockEventSource.instances[0].closed).toBe(true)
  })
})

describe('arestQueryBridge — sequence-number replay and reconnect', () => {
  beforeEach(() => {
    MockEventSource.instances = []
    vi.stubGlobal('EventSource', MockEventSource)
  })
  afterEach(() => {
    vi.unstubAllGlobals()
    vi.useRealTimers()
  })

  it('getLastSequence tracks the highest observed sequence number', async () => {
    const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } })
    const bridge = createArestQueryBridge({
      baseUrl: 'https://ui.auto.dev/arest',
      domain: 'organizations',
      queryClient,
    })
    expect(bridge.getLastSequence()).toBeNull()

    MockEventSource.instances[0].emit({
      domain: 'organizations', noun: 'Organization', entityId: 'a',
      operation: 'create', facts: {}, timestamp: 0, sequence: 7, cellKey: 'Organization:a',
    })
    expect(bridge.getLastSequence()).toBe(7)

    // Out-of-order late delivery mustn't rewind the cursor.
    MockEventSource.instances[0].emit({
      domain: 'organizations', noun: 'Organization', entityId: 'a',
      operation: 'update', facts: {}, timestamp: 0, sequence: 3, cellKey: 'Organization:a',
    })
    expect(bridge.getLastSequence()).toBe(7)

    MockEventSource.instances[0].emit({
      domain: 'organizations', noun: 'Organization', entityId: 'a',
      operation: 'update', facts: {}, timestamp: 0, sequence: 12, cellKey: 'Organization:a',
    })
    expect(bridge.getLastSequence()).toBe(12)
    bridge.close()
  })

  it('reopens EventSource with ?lastSequence=<N> after onerror', async () => {
    vi.useFakeTimers()
    const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } })
    const bridge = createArestQueryBridge({
      baseUrl: 'https://ui.auto.dev/arest',
      domain: 'organizations',
      queryClient,
      reconnect: { initialDelayMs: 10, maxDelayMs: 100 },
    })

    MockEventSource.instances[0].emit({
      domain: 'organizations', noun: 'Organization', entityId: 'a',
      operation: 'create', facts: {}, timestamp: 0, sequence: 5, cellKey: 'Organization:a',
    })
    expect(bridge.getLastSequence()).toBe(5)

    // Simulate a dropped connection.
    MockEventSource.instances[0].onerror?.(new Event('error'))
    expect(MockEventSource.instances[0].closed).toBe(true)

    // Advance the reconnect timer; a new EventSource should open.
    await vi.advanceTimersByTimeAsync(15)
    expect(MockEventSource.instances).toHaveLength(2)

    const replayUrl = new URL(MockEventSource.instances[1].url)
    expect(replayUrl.searchParams.get('domain')).toBe('organizations')
    expect(replayUrl.searchParams.get('lastSequence')).toBe('5')
    bridge.close()
  })

  it('backs off exponentially and caps at maxDelayMs', async () => {
    vi.useFakeTimers()
    const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } })
    const bridge = createArestQueryBridge({
      baseUrl: 'https://ui.auto.dev/arest',
      domain: 'organizations',
      queryClient,
      reconnect: { initialDelayMs: 10, maxDelayMs: 40 },
    })

    // First error -> 10ms delay
    MockEventSource.instances[0].onerror?.(new Event('error'))
    await vi.advanceTimersByTimeAsync(10)
    expect(MockEventSource.instances).toHaveLength(2)
    // Second error -> 20ms
    MockEventSource.instances[1].onerror?.(new Event('error'))
    await vi.advanceTimersByTimeAsync(20)
    expect(MockEventSource.instances).toHaveLength(3)
    // Third error -> 40ms (40ms cap reached, not 80ms)
    MockEventSource.instances[2].onerror?.(new Event('error'))
    await vi.advanceTimersByTimeAsync(40)
    expect(MockEventSource.instances).toHaveLength(4)
    // Fourth error -> still 40ms (at cap)
    MockEventSource.instances[3].onerror?.(new Event('error'))
    await vi.advanceTimersByTimeAsync(40)
    expect(MockEventSource.instances).toHaveLength(5)
    bridge.close()
  })

  it('close() cancels any pending reconnect timer', async () => {
    vi.useFakeTimers()
    const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } })
    const bridge = createArestQueryBridge({
      baseUrl: 'https://ui.auto.dev/arest',
      domain: 'organizations',
      queryClient,
      reconnect: { initialDelayMs: 100, maxDelayMs: 200 },
    })

    MockEventSource.instances[0].onerror?.(new Event('error'))
    bridge.close()
    // Even after the reconnect delay elapses, no new EventSource opens.
    await vi.advanceTimersByTimeAsync(500)
    expect(MockEventSource.instances).toHaveLength(1)
  })
})

describe('arestQueryBridge — schema-compile invalidation', () => {
  beforeEach(() => {
    MockEventSource.instances = []
    vi.stubGlobal('EventSource', MockEventSource)
  })
  afterEach(() => { vi.unstubAllGlobals() })

  it('invalidates ["arest","openapi"] on an event with operation=compile', async () => {
    const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } })
    const realInvalidate = queryClient.invalidateQueries.bind(queryClient)
    const spy = vi.fn((...args) => realInvalidate(...args))
    ;(queryClient as unknown as { invalidateQueries: unknown }).invalidateQueries = spy

    const bridge = createArestQueryBridge({
      baseUrl: 'https://ui.auto.dev/arest',
      domain: 'organizations',
      queryClient,
    })
    MockEventSource.instances[0].emit({
      domain: 'organizations',
      // A compile event's noun is the entity noun being recompiled.
      noun: 'Schema',
      entityId: 'organizations',
      // We treat operation === 'compile' as a schema-level event.
      operation: 'compile' as unknown as 'create',
      facts: {},
      timestamp: Date.now(),
      sequence: 100,
      cellKey: 'Schema:organizations',
    })
    await new Promise((r) => setTimeout(r, 10))
    const calls = (spy.mock.calls as unknown[][]).map((c) => c[0])
    expect(calls).toContainEqual(expect.objectContaining({ queryKey: ['arest', 'openapi'] }))
    // Entity-level list shouldn't be invalidated for a compile event.
    expect(calls).not.toContainEqual(expect.objectContaining({ queryKey: ['arest', 'list', 'schemas'] }))
    bridge.close()
  })

  it('invalidateSchemaOnCompile=false suppresses the compile-event override', async () => {
    const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } })
    const realInvalidate = queryClient.invalidateQueries.bind(queryClient)
    const spy = vi.fn((...args) => realInvalidate(...args))
    ;(queryClient as unknown as { invalidateQueries: unknown }).invalidateQueries = spy

    const bridge = createArestQueryBridge({
      baseUrl: 'https://ui.auto.dev/arest',
      domain: 'organizations',
      queryClient,
      invalidateSchemaOnCompile: false,
    })
    MockEventSource.instances[0].emit({
      domain: 'organizations',
      noun: 'Schema',
      entityId: 'x',
      operation: 'compile' as unknown as 'create',
      facts: {},
      timestamp: 0,
      sequence: 1,
      cellKey: 'Schema:x',
    })
    await new Promise((r) => setTimeout(r, 10))
    const calls = (spy.mock.calls as unknown[][]).map((c) => c[0])
    expect(calls).not.toContainEqual(expect.objectContaining({ queryKey: ['arest', 'openapi'] }))
    bridge.close()
  })
})

describe('arestQueryBridge integration — create then list auto-refreshes', () => {
  beforeEach(() => {
    MockEventSource.instances = []
    vi.stubGlobal('EventSource', MockEventSource)
  })
  afterEach(() => {
    vi.unstubAllGlobals()
  })

  it('list query re-runs after a create broadcast (< 500ms budget)', async () => {
    // Acceptance criterion from the task: "create via dataProvider,
    // observe the list query auto-refresh within 500 ms." We mount a
    // list query backed by a stubbed fetcher, emit a create event on
    // the SSE stream, and assert the fetcher runs again.
    const queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false, staleTime: 0, gcTime: 0 } },
    })

    let fetchCount = 0
    const fetcher = async () => {
      fetchCount += 1
      return [{ id: 'acme' }]
    }

    // Prime the list query.
    await queryClient.fetchQuery({
      queryKey: ['arest', 'list', 'organizations'],
      queryFn: fetcher,
    })
    expect(fetchCount).toBe(1)

    // Attach an observer so invalidation actually triggers a refetch.
    // Without an observer the query is "inactive" and invalidateQueries
    // just marks it stale. The task's acceptance test has a real list
    // component mounted; we simulate that with a QueryObserver (the
    // same primitive useQuery hook uses internally).
    const observer = new QueryObserver(queryClient, {
      queryKey: ['arest', 'list', 'organizations'],
      queryFn: fetcher,
      staleTime: 0,
    })
    const unsubscribe = observer.subscribe(() => {})

    const bridge = createArestQueryBridge({
      baseUrl: 'https://ui.auto.dev/arest',
      domain: 'organizations',
      queryClient,
    })

    const start = Date.now()
    MockEventSource.instances[0].emit({
      domain: 'organizations',
      noun: 'Organization',
      entityId: 'newco',
      operation: 'create',
      facts: {},
      timestamp: start,
      sequence: 42,
      cellKey: 'Organization:newco',
    })

    // Wait up to 500ms for the refetch to complete.
    const deadline = start + 500
    while (fetchCount < 2 && Date.now() < deadline) {
      await new Promise((r) => setTimeout(r, 10))
    }
    const elapsed = Date.now() - start

    expect(fetchCount).toBeGreaterThanOrEqual(2)
    expect(elapsed).toBeLessThan(500)

    unsubscribe()
    bridge.close()
  })
})

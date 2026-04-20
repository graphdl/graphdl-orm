/**
 * arestQueryBridge — TanStack Query cache + SSE invalidation loop.
 *
 * Opens an EventSource against the AREST worker's broadcast stream
 * (/api/events?domain=<domain>, per #113) and on every incoming
 * CellEvent invalidates the TanStack Query keys whose data the event
 * just changed. That lets list / one queries stay fresh without
 * polling: mutate through the dataProvider, the worker broadcasts,
 * every connected tab refreshes within the 500ms budget.
 *
 * Key scheme (see createArestQueryKeys):
 *   list       : ['arest', 'list',      resource]
 *   list(args) : ['arest', 'list',      resource, args]
 *   one        : ['arest', 'one',       resource, id]
 *   reference  : ['arest', 'reference', resource, target, targetId]
 *
 * Invalidation strategy on a CellEvent:
 *   - The affected noun -> slug becomes the resource key.
 *   - `['arest', 'list', resource]`                (prefix match)
 *   - `['arest', 'one',  resource, entityId]`      (exact)
 *   - `['arest', 'reference', resource]`           (prefix, conservative)
 *
 * We intentionally over-invalidate references — TanStack Query's
 * prefix match on `queryKey` means callers only re-fetch what they
 * have observers on, so the cost is bounded.
 */
import type { QueryClient, QueryKey } from '@tanstack/react-query'

// ── Query keys ──────────────────────────────────────────────────────

export interface ArestQueryKeys {
  list(params?: unknown): QueryKey
  one(id: string | number): QueryKey
  reference(target: string, targetId: string | number): QueryKey
}

export function createArestQueryKeys(resource: string): ArestQueryKeys {
  return {
    list(params?: unknown) {
      return params === undefined
        ? ['arest', 'list', resource]
        : ['arest', 'list', resource, params]
    },
    one(id: string | number) {
      return ['arest', 'one', resource, id]
    },
    reference(target: string, targetId: string | number) {
      return ['arest', 'reference', resource, target, targetId]
    },
  }
}

// ── CellEvent shape (mirrors src/broadcast-do.ts) ───────────────────

export interface CellEventPayload {
  domain: string
  noun: string
  entityId: string
  operation: 'create' | 'update' | 'delete' | 'transition'
  facts: Record<string, unknown>
  timestamp: number
  sequence: number
  cellKey: string
}

function isCellEvent(value: unknown): value is CellEventPayload {
  return (
    typeof value === 'object' &&
    value !== null &&
    typeof (value as CellEventPayload).noun === 'string' &&
    typeof (value as CellEventPayload).entityId === 'string' &&
    typeof (value as CellEventPayload).operation === 'string'
  )
}

/**
 * AREST's nounToSlug convention — mirrors src/api/arest-router.ts so
 * the client derives the same slug the worker exposes. PascalCase and
 * "Multi Word" forms are supported: "Support Request" -> "support-requests".
 */
export function nounToSlug(noun: string): string {
  // "SupportRequest" -> "Support Request"
  const withSpaces = noun.replace(/([a-z])([A-Z])/g, '$1 $2')
  return withSpaces.toLowerCase().replace(/ /g, '-') + 's'
}

// ── Bridge ──────────────────────────────────────────────────────────

export interface ArestQueryBridgeOptions {
  /** AREST worker base URL (e.g. https://ui.auto.dev/arest). */
  baseUrl: string
  /** Domain scope for the subscription filter. */
  domain: string
  /** The TanStack QueryClient whose cache should invalidate on events. */
  queryClient: QueryClient
  /**
   * Optional EventSource constructor — tests stub this. Defaults to
   * globalThis.EventSource which is available in browsers and jsdom.
   */
  EventSource?: typeof globalThis.EventSource
  /**
   * Callback for debugging — fired on every parsed event, success or
   * not. Useful to wire metrics; no-op by default.
   */
  onEvent?: (event: CellEventPayload | null, raw: string) => void
}

export interface ArestQueryBridge {
  /** Tear down the EventSource and stop invalidating. */
  close(): void
}

export function createArestQueryBridge(
  options: ArestQueryBridgeOptions,
): ArestQueryBridge {
  const { baseUrl, domain, queryClient } = options
  const EventSourceCtor = options.EventSource ?? globalThis.EventSource

  if (typeof EventSourceCtor !== 'function') {
    throw new Error('arestQueryBridge: EventSource is not available in this environment')
  }

  // Derive the worker root (strip /arest) so /api/events resolves.
  const trimmed = baseUrl.replace(/\/$/, '')
  const workerRoot = trimmed.endsWith('/arest')
    ? trimmed.slice(0, -'/arest'.length)
    : trimmed
  const eventsUrl = `${workerRoot}/api/events?domain=${encodeURIComponent(domain)}`

  const source = new EventSourceCtor(eventsUrl, { withCredentials: true })

  source.onmessage = (msg: MessageEvent) => {
    const raw = typeof msg.data === 'string' ? msg.data : ''
    let parsed: unknown
    try { parsed = JSON.parse(raw) } catch { parsed = null }

    if (!isCellEvent(parsed)) {
      options.onEvent?.(null, raw)
      return
    }

    options.onEvent?.(parsed, raw)
    const resource = nounToSlug(parsed.noun)

    // Prefix-invalidate the list so ['arest', 'list', resource, { ... }]
    // variants also refetch when there's an observer.
    queryClient.invalidateQueries({ queryKey: ['arest', 'list', resource] })

    // Exact-invalidate the specific entity's one-query.
    queryClient.invalidateQueries({ queryKey: ['arest', 'one', resource, parsed.entityId] })

    // Conservative: invalidate any reference queries that depend on
    // this resource. The prefix match bounds the blast radius to
    // queries that actually observe it.
    queryClient.invalidateQueries({ queryKey: ['arest', 'reference', resource] })
  }

  source.onerror = () => {
    // EventSource auto-reconnects by default — onerror fires during
    // reconnect attempts too. We deliberately don't close the stream
    // here; the caller's close() is the authoritative teardown.
  }

  return {
    close() { source.close() },
  }
}

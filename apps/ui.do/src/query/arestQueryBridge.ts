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
 * Sequence-number replay (whitepaper Corollary 4 — constraint
 * consensus from a deterministic event log requires continuity of
 * the stream): the bridge tracks the last seen CellEvent.sequence
 * and, on reconnect, re-opens the EventSource with
 * `?lastSequence=<N>` so the worker replays events > N. Without
 * this, missed events during a network blip would silently
 * desynchronise the client's cache from the authoritative state.
 *
 * Key scheme (see createArestQueryKeys):
 *   list       : ['arest', 'list',      resource]
 *   list(args) : ['arest', 'list',      resource, args]
 *   one        : ['arest', 'one',       resource, id]
 *   reference  : ['arest', 'reference', resource, target, targetId]
 *
 * Invalidation strategy on a CellEvent:
 *   - `['arest', 'list', resource]`                (prefix match)
 *   - `['arest', 'one',  resource, entityId]`      (exact)
 *   - `['arest', 'reference', resource]`           (prefix, conservative)
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
  /**
   * Reconnect backoff (ms). Tests override to keep test wall-time
   * short; production defaults are `initial: 1000, max: 30000`.
   */
  reconnect?: { initialDelayMs?: number; maxDelayMs?: number }
  /**
   * When true, the bridge additionally invalidates the OpenAPI
   * schema query on any event whose operation is 'compile' (schema
   * mutation). Lets useArestResources / useOpenApiSchema react to
   * compile events without a full page reload. Defaults to true.
   */
  invalidateSchemaOnCompile?: boolean
}

export interface ArestQueryBridge {
  /** Tear down the EventSource and stop invalidating. */
  close(): void
  /**
   * The last sequence number the bridge has observed. Resets on a
   * successful reconnect-and-replay. Exposed for tests / metrics.
   */
  getLastSequence(): number | null
}

export function createArestQueryBridge(
  options: ArestQueryBridgeOptions,
): ArestQueryBridge {
  const { baseUrl, domain, queryClient } = options
  const EventSourceCtor = options.EventSource ?? globalThis.EventSource
  const invalidateSchemaOnCompile = options.invalidateSchemaOnCompile ?? true

  if (typeof EventSourceCtor !== 'function') {
    throw new Error('arestQueryBridge: EventSource is not available in this environment')
  }

  // Derive the worker root (strip /arest) so /api/events resolves.
  const trimmed = baseUrl.replace(/\/$/, '')
  const workerRoot = trimmed.endsWith('/arest')
    ? trimmed.slice(0, -'/arest'.length)
    : trimmed

  const initialBackoffMs = options.reconnect?.initialDelayMs ?? 1000
  const maxBackoffMs = options.reconnect?.maxDelayMs ?? 30_000

  let lastSequence: number | null = null
  let source: InstanceType<typeof EventSourceCtor> | null = null
  let closed = false
  let reconnectTimer: ReturnType<typeof setTimeout> | null = null
  let currentBackoff = initialBackoffMs

  function buildUrl(): string {
    const params = new URLSearchParams({ domain })
    // Replay: ask the worker for every event strictly after the last
    // one we successfully processed. If the worker doesn't know this
    // query param, it'll ignore it and stream normally.
    if (lastSequence !== null) {
      params.set('lastSequence', String(lastSequence))
    }
    return `${workerRoot}/api/events?${params.toString()}`
  }

  function handleMessage(msg: MessageEvent): void {
    const raw = typeof msg.data === 'string' ? msg.data : ''
    let parsed: unknown
    try { parsed = JSON.parse(raw) } catch { parsed = null }

    if (!isCellEvent(parsed)) {
      options.onEvent?.(null, raw)
      return
    }

    options.onEvent?.(parsed, raw)
    // Track sequence monotonically — out-of-order events (late
    // deliveries) shouldn't rewind the replay cursor.
    if (typeof parsed.sequence === 'number' && (lastSequence === null || parsed.sequence > lastSequence)) {
      lastSequence = parsed.sequence
    }

    const resource = nounToSlug(parsed.noun)

    // Schema compile events invalidate the OpenAPI doc so the
    // resource list + per-noun schema re-fetch without a page
    // reload. The worker emits these with operation === 'compile'
    // (see compile.rs in the compile pipeline).
    if (invalidateSchemaOnCompile && (parsed.operation as string) === 'compile') {
      queryClient.invalidateQueries({ queryKey: ['arest', 'openapi'] })
      return
    }

    // Prefix-invalidate the list so ['arest', 'list', resource, { ... }]
    // variants also refetch when there's an observer.
    queryClient.invalidateQueries({ queryKey: ['arest', 'list', resource] })
    // Exact-invalidate the specific entity's one-query.
    queryClient.invalidateQueries({ queryKey: ['arest', 'one', resource, parsed.entityId] })
    // Conservative: invalidate any reference queries that depend on
    // this resource.
    queryClient.invalidateQueries({ queryKey: ['arest', 'reference', resource] })

    // Successful event receipt resets the backoff — reconnect now
    // happens quickly next time the socket breaks.
    currentBackoff = initialBackoffMs
  }

  function open(): void {
    if (closed) return
    const url = buildUrl()
    const s = new EventSourceCtor(url, { withCredentials: true })
    source = s
    s.onmessage = handleMessage
    s.onerror = () => {
      // EventSource's native reconnect doesn't carry `lastSequence`,
      // so we close the browser's auto-reconnect and reopen
      // ourselves with the current cursor.
      s.close()
      if (closed) return
      scheduleReconnect()
    }
  }

  function scheduleReconnect(): void {
    if (closed || reconnectTimer !== null) return
    const delay = currentBackoff
    currentBackoff = Math.min(maxBackoffMs, currentBackoff * 2)
    reconnectTimer = setTimeout(() => {
      reconnectTimer = null
      open()
    }, delay)
  }

  open()

  return {
    close() {
      closed = true
      if (reconnectTimer !== null) {
        clearTimeout(reconnectTimer)
        reconnectTimer = null
      }
      if (source) {
        source.close()
        source = null
      }
    },
    getLastSequence() { return lastSequence },
  }
}

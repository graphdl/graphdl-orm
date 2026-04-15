/**
 * BroadcastDO — the kernel's signal-delivery layer.
 *
 * One DO per scope (global for small deployments; per-App for large
 * multi-tenant ones). Holds a transient subscriber registry and fans
 * out cell-change events to matching subscribers.
 *
 * This module exposes the pure registry functions; the DO class at
 * the bottom wires them to the Cloudflare DurableObject runtime.
 * Tests exercise every branch without touching the DO runtime.
 *
 * Signal-delivery analogy: subscribe = sigaction, unsubscribe =
 * signal(SIG_DFL), publish = kill(), match = signal-mask lookup.
 */

import { DurableObject } from 'cloudflare:workers'

// ── Types ──────────────────────────────────────────────────────────────

/**
 * A subscription filter. Narrower filters receive fewer events.
 * `domain` is required; `noun` restricts to one noun type; `entityId`
 * restricts to one entity. All three match every event in the scope.
 */
export interface SubscriptionFilter {
  readonly domain: string
  readonly noun?: string
  readonly entityId?: string
}

/**
 * A cell-change event published by a mutation. The shape mirrors the
 * paper's `event` of type E: a typed tuple that the SM fold (Eq. 11)
 * or a subscriber's ρ-application consumes.
 */
export interface CellEvent {
  readonly domain: string
  readonly noun: string
  readonly entityId: string
  readonly operation: 'create' | 'update' | 'delete' | 'transition'
  readonly facts: Readonly<Record<string, unknown>>
  readonly timestamp: number
  /** Monotonic per-registry sequence number. Assigned by publish(). */
  readonly sequence: number
}

/**
 * A subscriber callback — invoked once per matching event.
 * Rejection/failure of delivery is the DO's concern; the registry
 * does not retry. Slow subscribers drop behind; very slow ones get
 * unsubscribed by the DO when their write buffer overflows.
 */
export type SubscriberCallback = (event: CellEvent) => void

interface Subscription {
  readonly id: string
  readonly filter: SubscriptionFilter
  readonly callback: SubscriberCallback
}

export interface Registry {
  subscribers: Map<string, Subscription>
  nextSubscriberId: number
  sequence: number
}

// ── Pure operations ────────────────────────────────────────────────────

export function createRegistry(): Registry {
  return { subscribers: new Map(), nextSubscriberId: 0, sequence: 0 }
}

export function subscribe(
  reg: Registry,
  filter: SubscriptionFilter,
  callback: SubscriberCallback,
): string {
  if (!filter.domain) {
    throw new Error('subscribe: filter.domain is required')
  }
  const id = `sub-${reg.nextSubscriberId++}`
  reg.subscribers.set(id, { id, filter, callback })
  return id
}

/**
 * Remove a subscription by id. Returns true if the subscription
 * existed and was removed; false otherwise. Idempotent.
 */
export function unsubscribe(reg: Registry, id: string): boolean {
  return reg.subscribers.delete(id)
}

/**
 * Enumerate the active subscription ids for introspection / metrics.
 * Order is insertion order (Map semantics).
 */
export function listSubscribers(reg: Registry): readonly string[] {
  return Array.from(reg.subscribers.keys())
}

/**
 * True iff the filter matches the event. Match semantics: the
 * subscriber sees every event in its declared domain, narrowed by
 * optional noun and entityId. Omitting a field means "all values".
 */
export function matches(filter: SubscriptionFilter, event: CellEvent): boolean {
  if (filter.domain !== event.domain) return false
  if (filter.noun !== undefined && filter.noun !== event.noun) return false
  if (filter.entityId !== undefined && filter.entityId !== event.entityId) return false
  return true
}

/**
 * Publish an event. Assigns a monotonic sequence number, invokes
 * every matching subscriber's callback, returns the assigned event.
 * Callbacks fire synchronously in subscription order; a throwing
 * callback does not abort fanout to the rest.
 */
export function publish(reg: Registry, event: Omit<CellEvent, 'sequence'>): CellEvent {
  const sequence = reg.sequence++
  const full: CellEvent = { ...event, sequence }
  for (const sub of reg.subscribers.values()) {
    if (!matches(sub.filter, full)) continue
    try {
      sub.callback(full)
    } catch {
      // A subscriber's failure doesn't stop the fanout. The DO
      // layer that owns the callback's transport (the SSE stream)
      // is responsible for unsubscribing dead subscribers.
    }
  }
  return full
}

// ── Server-Sent Events encoding ────────────────────────────────────────

/**
 * Format a CellEvent as an SSE frame.
 *
 * `data: <json>\n\n` per the SSE spec. The browser's EventSource API
 * calls onmessage with the parsed-or-raw JSON on each frame.
 */
export function formatSseFrame(event: CellEvent): string {
  return `data: ${JSON.stringify(event)}\n\n`
}

/**
 * Open an SSE response stream bound to a subscription on `reg`.
 *
 * Extracted from BroadcastDO.fetch() so tests can drive the full
 * subscribe → publish → stream-frame pipeline against a plain
 * Registry without needing the DurableObject runtime. The DO
 * class's fetch() is a thin wrapper.
 *
 * Contract:
 * - `?domain=X` required; `noun` and `entityId` optional narrowers.
 * - Returns `text/event-stream` with a 200 response.
 * - Emits `: connected sub=<id>\n\n` immediately so a client
 *   EventSource sees the connection open.
 * - Every subsequent matching publish arrives as `data: <json>\n\n`.
 * - request.signal.abort → unsubscribes and closes the writer.
 */
export async function openSseStream(reg: Registry, request: Request): Promise<Response> {
  const url = new URL(request.url)
  const domain = url.searchParams.get('domain')
  if (!domain) {
    return new Response('domain query parameter required', { status: 400 })
  }
  const noun = url.searchParams.get('noun') ?? undefined
  const entityId = url.searchParams.get('entityId') ?? undefined

  const { readable, writable } = new TransformStream<Uint8Array, Uint8Array>()
  const writer = writable.getWriter()
  const encoder = new TextEncoder()

  let subscriptionId = ''
  const callback: SubscriberCallback = (event) => {
    writer.write(encoder.encode(formatSseFrame(event))).catch(() => {
      if (subscriptionId) unsubscribe(reg, subscriptionId)
    })
  }

  subscriptionId = subscribe(reg, { domain, noun, entityId }, callback)

  writer.write(encoder.encode(`: connected sub=${subscriptionId}\n\n`))
    .catch(() => { /* client already gone */ })

  request.signal.addEventListener('abort', () => {
    unsubscribe(reg, subscriptionId)
    writer.close().catch(() => {})
  })

  return new Response(readable, {
    headers: {
      'Content-Type': 'text/event-stream',
      'Cache-Control': 'no-cache, no-transform',
      'Connection': 'keep-alive',
      'X-Accel-Buffering': 'no',
    },
  })
}

// ── Durable Object wrapper ─────────────────────────────────────────────

/**
 * BroadcastDO — the runtime wrapper around the pure registry.
 *
 * A single instance per scope. Exposes two entry points:
 * - RPC methods (subscribe/unsubscribe/publish/listSubscribers) for
 *   in-process code that wants programmatic access.
 * - A fetch() handler that opens an SSE response stream bound to a
 *   freshly-created subscription. When the client disconnects
 *   (request.signal.abort), the subscription is dropped and the
 *   stream is closed.
 *
 * The DO runtime serialises access so the registry itself does not
 * need internal locking.
 */
export class BroadcastDO extends DurableObject {
  private registry: Registry

  constructor(ctx: DurableObjectState, env: unknown) {
    super(ctx, env)
    this.registry = createRegistry()
  }

  /**
   * GET /events?domain=X&noun=Y&entityId=Z
   *
   * Opens an SSE stream bound to a subscription matching the query
   * filter. Closes the stream and unsubscribes on client disconnect.
   * The worker's /api/events route forwards here via doStub.fetch().
   */
  async fetch(request: Request): Promise<Response> {
    return openSseStream(this.registry, request)
  }

  /** Register a subscriber; returns the subscription id. */
  async subscribe(filter: SubscriptionFilter, callback: SubscriberCallback): Promise<string> {
    return subscribe(this.registry, filter, callback)
  }

  /**
   * RPC-friendly subscription registration — registers a filter with
   * a no-op callback. The subscription is tracked in listSubscribers
   * and removable via unsubscribe, but events never reach the caller
   * through this subscription because callbacks don't cross the DO
   * RPC boundary (they are function references, not serialisable).
   *
   * Used by MCP tools that want to register a client's interest and
   * track its lifecycle, but deliver events via a separate transport
   * (today: /api/events SSE). The filter can be re-used to attach an
   * SSE stream by making the same query on /api/events.
   */
  async registerFilter(filter: SubscriptionFilter): Promise<string> {
    return subscribe(this.registry, filter, () => { /* no-op across RPC */ })
  }

  /** Remove a subscription. Returns true if it existed. */
  async unsubscribe(id: string): Promise<boolean> {
    return unsubscribe(this.registry, id)
  }

  /** Current subscriber ids, insertion-ordered. */
  async listSubscribers(): Promise<readonly string[]> {
    return listSubscribers(this.registry)
  }

  /** Publish an event; returns the assigned event with its sequence number. */
  async publish(event: Omit<CellEvent, 'sequence'>): Promise<CellEvent> {
    return publish(this.registry, event)
  }
}

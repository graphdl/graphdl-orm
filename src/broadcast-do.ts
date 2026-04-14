/**
 * BroadcastDO — the kernel's signal-delivery layer.
 *
 * One DO per scope (global for small deployments; per-App for large
 * multi-tenant ones). Holds a transient subscriber registry and fans
 * out cell-change events to matching subscribers.
 *
 * This module exposes the pure registry functions; the DO class at the
 * bottom wires them to the Cloudflare DurableObject runtime.
 *
 * Scope of task #112: the in-memory registry + pure operations. Tests
 * exercise every branch without touching the DO runtime.
 * - #113 wires the /api/events SSE route that opens a subscriber conn
 * - #114 post-mutation hooks publish CellEvents
 * - #115 adds the BROADCAST binding + v2 migration to wrangler.jsonc
 * - #116 end-to-end smoke through the apis catch-all
 *
 * Under the OS-kernel reframe (docs/11): subscribe = sigaction,
 * unsubscribe = signal(SIG_DFL), publish = kill(), match = signal
 * mask lookup.
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

// ── Durable Object wrapper ─────────────────────────────────────────────

/**
 * BroadcastDO — the runtime wrapper around the pure registry.
 *
 * A single instance per scope. Callers interact via the methods below;
 * the DO runtime serialises access so the registry itself does not
 * need internal locking.
 */
export class BroadcastDO extends DurableObject {
  private registry: Registry

  constructor(ctx: DurableObjectState, env: unknown) {
    super(ctx, env)
    this.registry = createRegistry()
  }

  /** Register a subscriber; returns the subscription id. */
  async subscribe(filter: SubscriptionFilter, callback: SubscriberCallback): Promise<string> {
    return subscribe(this.registry, filter, callback)
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

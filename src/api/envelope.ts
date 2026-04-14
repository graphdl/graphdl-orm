/**
 * Theorem 5 response envelope.
 *
 * The OpenAPI generator declares every operation's response as
 *   { data, derived, violations, _links }
 * per AREST.tex Theorem 5's `repr(e, P, S)`. This module is the
 * runtime counterpart: the server handlers wrap their payloads in
 * the same shape so the published contract is true.
 *
 * Today `derived` and the `_links` substructures are empty by
 * default — handlers fill what they know. Full population (live
 * derivation traces, SM transition links, navigation URIs) grows
 * over time as each handler's implementation catches up to its
 * declared schema. Consumers can always read `data`, iterate
 * `violations` for user-facing errors, and read `_links` for
 * HATEOAS navigation when available.
 */

export interface Violation {
  readonly reading: string
  readonly constraintId: string
  readonly modality: 'alethic' | 'deontic'
  readonly detail?: string
}

export interface TransitionLink {
  readonly event: string
  readonly href: string
  readonly method: 'POST'
}

export interface Envelope<T> {
  readonly data: T
  readonly derived?: Record<string, unknown>
  readonly violations?: readonly Violation[]
  readonly _links: {
    readonly transitions?: readonly TransitionLink[]
    readonly navigation?: Record<string, string>
  }
}

export interface EnvelopeOptions {
  readonly derived?: Record<string, unknown>
  readonly violations?: readonly Violation[]
  readonly transitions?: readonly TransitionLink[]
  readonly navigation?: Record<string, string>
}

/**
 * Wrap a data payload in the Theorem 5 envelope shape.
 *
 * `data` is the caller's payload — an entity row, an array of rows,
 * or whatever the operation returns. Optional fields default to
 * empty so clients always see the declared shape.
 */
export function envelope<T>(data: T, opts?: EnvelopeOptions): Envelope<T> {
  return {
    data,
    derived: opts?.derived ?? {},
    violations: opts?.violations ?? [],
    _links: {
      transitions: opts?.transitions ?? [],
      navigation: opts?.navigation ?? {},
    },
  }
}

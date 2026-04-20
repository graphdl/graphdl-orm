/**
 * Federation — Citation provenance tests (E3 / #305).
 *
 * federatedFetch returns Citation provenance alongside the fetched
 * entity facts. Each ρ(populate_n) application yields a single
 * Citation describing origin (URI, retrieval date, authority type,
 * external system). The caller (server.ts / engine) emits paired
 * Fact cites Citation facts linking each returned entity fact to
 * that Citation.
 *
 * Authority Type 'Federated-Fetch' corresponds to the value added
 * to readings/instances.md's Authority Type enum.
 */

/// <reference types="node" />
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { federatedFetch, type FederationConfig, type FederatedFetchResult } from './federation'

const STRIPE_CONFIG: FederationConfig = {
  system: 'stripe',
  url: 'https://api.stripe.com/v1',
  uri: '/customers',
  header: 'Authorization',
  prefix: 'Bearer',
  noun: 'Stripe Customer',
  fields: ['Email', 'Name'],
}

function jsonResponse(body: unknown, ok = true, status = 200): Response {
  return {
    ok,
    status,
    statusText: ok ? 'OK' : 'Error',
    json: async () => body,
  } as Response
}

describe('federatedFetch — Citation provenance (E3 / #305)', () => {
  beforeEach(() => {
    vi.useFakeTimers()
    vi.setSystemTime(new Date('2026-04-20T12:00:00Z'))
  })
  afterEach(() => {
    vi.useRealTimers()
    vi.unstubAllGlobals()
  })

  it('returns a Citation with Authority Type Federated-Fetch', async () => {
    vi.stubGlobal('fetch', vi.fn(async () =>
      jsonResponse([{ id: 'cus_1', Email: 'a@x.com', Name: 'Alice' }])
    ))
    const result = await federatedFetch(STRIPE_CONFIG)
    expect(result.citation).toBeDefined()
    expect(result.citation!.authorityType).toBe('Federated-Fetch')
  })

  it('Citation names the external system', async () => {
    vi.stubGlobal('fetch', vi.fn(async () =>
      jsonResponse([{ id: 'cus_1', Email: 'a@x.com' }])
    ))
    const result = await federatedFetch(STRIPE_CONFIG)
    expect(result.citation!.externalSystem).toBe('stripe')
  })

  it('Citation records the fetch URL as URI', async () => {
    vi.stubGlobal('fetch', vi.fn(async () =>
      jsonResponse([{ id: 'cus_1' }])
    ))
    const result = await federatedFetch(STRIPE_CONFIG)
    expect(result.citation!.uri).toBe('https://api.stripe.com/v1/customers')
  })

  it('entity-scoped fetch embeds the entity id in Citation URI', async () => {
    vi.stubGlobal('fetch', vi.fn(async () =>
      jsonResponse({ id: 'cus_42', Email: 'x@y.com' })
    ))
    const result = await federatedFetch(STRIPE_CONFIG, 'cus_42')
    expect(result.citation!.uri).toBe('https://api.stripe.com/v1/customers/cus_42')
  })

  it('Citation retrieval date is ISO-8601 at the fetch time', async () => {
    vi.stubGlobal('fetch', vi.fn(async () =>
      jsonResponse([])
    ))
    const result = await federatedFetch(STRIPE_CONFIG)
    expect(result.citation!.retrievalDate).toBe('2026-04-20T12:00:00.000Z')
  })

  it('error response still produces a Citation (provenance of the error origin)', async () => {
    vi.stubGlobal('fetch', vi.fn(async () =>
      jsonResponse({ error: 'forbidden' }, false, 403)
    ))
    const result = await federatedFetch(STRIPE_CONFIG)
    expect(result.error).toBeDefined()
    expect(result.citation).toBeDefined()
    expect(result.citation!.authorityType).toBe('Federated-Fetch')
    expect(result.citation!.uri).toBe('https://api.stripe.com/v1/customers')
  })

  it('successful fetch still maps response to facts (no regression)', async () => {
    vi.stubGlobal('fetch', vi.fn(async () =>
      jsonResponse([
        { id: 'cus_1', Email: 'a@x.com', Name: 'Alice' },
        { id: 'cus_2', Email: 'b@x.com', Name: 'Bob' },
      ])
    ))
    const result = await federatedFetch(STRIPE_CONFIG)
    expect(result.count).toBe(2)
    expect(result.facts).toHaveLength(2)
    expect(result.facts[0]['Email']).toBe('a@x.com')
    expect(result.facts[0]['Stripe Customer']).toBe('cus_1')
  })
})

// ── Ingest-payload shape: maps FederatedFetchResult to the engine's
// federated_ingest:<noun> JSON contract (#305). One fact per (entity,
// field) pair because the engine's cells are keyed per fact type
// (Noun_verb_Role) and each FT holds the (Noun, Role) binding shape.
import { buildIngestPayload } from './federation'

describe('buildIngestPayload — federated_ingest FFI shape (E3 / #305)', () => {
  it('splits one entity record into one fact per field', () => {
    const result = {
      system: 'stripe',
      noun: 'Stripe Customer',
      count: 1,
      facts: [{ 'Stripe Customer': 'cus_1', Email: 'a@x.com', Name: 'Alice' }],
      citation: {
        uri: 'https://api.stripe.com/v1/customers',
        retrievalDate: '2026-04-20T12:00:00Z',
        authorityType: 'Federated-Fetch',
        externalSystem: 'stripe',
      },
      _meta: { url: 'https://api.stripe.com/v1/customers', worldAssumption: 'OWA' as const },
    }
    const payload = buildIngestPayload(result)
    expect(payload.externalSystem).toBe('stripe')
    expect(payload.url).toBe('https://api.stripe.com/v1/customers')
    expect(payload.retrievalDate).toBe('2026-04-20T12:00:00Z')
    expect(payload.facts).toHaveLength(2)
    expect(payload.facts).toContainEqual({
      factTypeId: 'Stripe_Customer_has_Email',
      bindings: { 'Stripe Customer': 'cus_1', Email: 'a@x.com' },
    })
    expect(payload.facts).toContainEqual({
      factTypeId: 'Stripe_Customer_has_Name',
      bindings: { 'Stripe Customer': 'cus_1', Name: 'Alice' },
    })
  })

  it('multi-word role names underscore in factTypeId but not in binding keys', () => {
    const result = {
      system: 'stripe',
      noun: 'Stripe Customer',
      count: 1,
      facts: [{ 'Stripe Customer': 'cus_1', 'Billing Address': '1 Main St' }],
      citation: {
        uri: 'https://api.stripe.com/v1/customers',
        retrievalDate: '2026-04-20T12:00:00Z',
        authorityType: 'Federated-Fetch',
        externalSystem: 'stripe',
      },
      _meta: { url: 'https://api.stripe.com/v1/customers', worldAssumption: 'OWA' as const },
    }
    const payload = buildIngestPayload(result)
    expect(payload.facts[0].factTypeId).toBe('Stripe_Customer_has_Billing_Address')
    expect(payload.facts[0].bindings['Billing Address']).toBe('1 Main St')
  })

  it('returns empty facts array when no citation is present', () => {
    const result = {
      system: 'stripe',
      noun: 'Stripe Customer',
      count: 0,
      facts: [],
      _meta: { url: '', worldAssumption: 'OWA' as const },
    }
    const payload = buildIngestPayload(result)
    expect(payload.facts).toHaveLength(0)
  })

  /// Error-path Citation: fetch returned an HTTP error, federatedFetch
  /// still emitted a Citation (origin of the error), but facts is
  /// empty. The ingest payload must still carry origin metadata so
  /// the engine can absorb the Citation by itself — downstream
  /// derivations over failed-fetch provenance need the Citation in P.
  it('preserves citation metadata when facts are empty (error response)', () => {
    const result = {
      system: 'stripe',
      noun: 'Stripe Customer',
      count: 0,
      facts: [],
      citation: {
        uri: 'https://api.stripe.com/v1/customers/cus_missing',
        retrievalDate: '2026-04-20T12:00:00Z',
        authorityType: 'Federated-Fetch',
        externalSystem: 'stripe',
      },
      _meta: { url: 'https://api.stripe.com/v1/customers/cus_missing', worldAssumption: 'OWA' as const },
      error: '404 Not Found',
    }
    const payload = buildIngestPayload(result)
    expect(payload.facts).toHaveLength(0)
    expect(payload.externalSystem).toBe('stripe')
    expect(payload.url).toBe('https://api.stripe.com/v1/customers/cus_missing')
    expect(payload.retrievalDate).toBe('2026-04-20T12:00:00Z')
  })
})

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

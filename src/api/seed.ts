import { json, error } from 'itty-router'
import type { Env } from '../types'
import { ingestClaims } from '../claims/ingest'
import type { ExtractedClaims } from '../claims/ingest'
import { parseFORML2 } from './parse'

// ── DO helpers ───────────────────────────────────────────────────────

/** Get a DomainDB DO stub for a specific domain slug. */
function getDomainDO(env: Env, domainSlug: string) {
  return env.DOMAIN_DB.get(env.DOMAIN_DB.idFromName(domainSlug))
}

/** Get the global RegistryDB DO stub. */
function getRegistryDO(env: Env) {
  return env.REGISTRY_DB.get(env.REGISTRY_DB.idFromName('global'))
}

// ── Seed endpoint ────────────────────────────────────────────────────

export async function handleSeed(request: Request, env: Env): Promise<Response> {
  if (request.method === 'GET') {
    return handleSeedGet(env)
  }

  if (request.method === 'DELETE') {
    return handleSeedDelete(env)
  }

  if (request.method === 'POST') {
    return handleSeedPost(request, env)
  }

  return error(405, { errors: [{ message: 'Method not allowed' }] })
}

// ── GET /seed — stats from per-domain DOs ────────────────────────────

async function handleSeedGet(env: Env): Promise<Response> {
  const registry = getRegistryDO(env) as any
  const domainSlugs: string[] = await registry.listDomains()

  if (!domainSlugs.length) {
    return json({
      totals: { domains: 0, nouns: 0, readings: 0, graphSchemas: 0, constraints: 0 },
      perDomain: {},
    })
  }

  // Query each domain DO in parallel for its stats
  const perDomainEntries = await Promise.all(
    domainSlugs.map(async (slug) => {
      const domainDO = getDomainDO(env, slug) as any
      const [nouns, readings, schemas, constraints] = await Promise.all([
        domainDO.findInCollection('nouns', {}, { limit: 0 }),
        domainDO.findInCollection('readings', {}, { limit: 0 }),
        domainDO.findInCollection('graph-schemas', {}, { limit: 0 }),
        domainDO.findInCollection('constraints', {}, { limit: 0 }),
      ])
      return {
        slug,
        nouns: nouns.totalDocs as number,
        readings: readings.totalDocs as number,
        graphSchemas: schemas.totalDocs as number,
        constraints: constraints.totalDocs as number,
      }
    })
  )

  const totals = {
    domains: domainSlugs.length,
    nouns: 0,
    readings: 0,
    graphSchemas: 0,
    constraints: 0,
  }
  const perDomain: Record<string, { nouns: number; readings: number }> = {}

  for (const entry of perDomainEntries) {
    totals.nouns += entry.nouns
    totals.readings += entry.readings
    totals.graphSchemas += entry.graphSchemas
    totals.constraints += entry.constraints
    perDomain[entry.slug] = { nouns: entry.nouns, readings: entry.readings }
  }

  return json({ totals, perDomain })
}

// ── DELETE /seed — wipe all domain DOs ──────────────────────────────

async function handleSeedDelete(env: Env): Promise<Response> {
  const registry = getRegistryDO(env) as any
  const domainSlugs: string[] = await registry.listDomains()

  // Wipe each domain DO in parallel
  await Promise.all(
    domainSlugs.map(async (slug) => {
      const domainDO = getDomainDO(env, slug) as any
      await domainDO.wipeAllData()
    })
  )

  return json({ message: 'All data wiped' })
}

// ── POST /seed — parallel per-domain ingestion ───────────────────────

async function handleSeedPost(request: Request, env: Env): Promise<Response> {
  const body = await request.json() as {
    type: string
    claims?: ExtractedClaims
    domain?: string
    domainId?: string
    domains?: Array<{ slug: string; name?: string; claims?: ExtractedClaims; text?: string }>
    /** Raw FORML2 text — parsed server-side */
    text?: string
  }

  if (body.type !== 'claims') {
    return error(400, { errors: [{ message: 'Unsupported seed type. Use type: "claims"' }] })
  }

  // Text mode: parse server-side, then seed
  if (body.text && body.domain) {
    const claims = parseFORML2(body.text, [])
    return handleSingleSeed(env, { claims, domain: body.domain, domainId: body.domain })
  }

  // Bulk: multiple domains — each can have claims or text
  if (body.domains?.length) {
    const resolved = body.domains.map(d => {
      if (d.claims) return d as { slug: string; name?: string; claims: ExtractedClaims }
      if (d.text) return { slug: d.slug, name: d.name, claims: parseFORML2(d.text, []) }
      return { slug: d.slug, name: d.name, claims: { nouns: [], readings: [], constraints: [] } as ExtractedClaims }
    })
    return handleBulkSeed(env, resolved)
  }

  // Single domain with pre-parsed claims
  if (body.claims) {
    return handleSingleSeed(env, body)
  }

  return error(400, { errors: [{ message: 'Provide claims + domain, text + domain, or domains[]' }] })
}

// ── Bulk multi-domain seeding ────────────────────────────────────────

async function handleBulkSeed(
  env: Env,
  domains: Array<{ slug: string; name?: string; claims: ExtractedClaims }>,
): Promise<Response> {
  const timings: Record<string, number> = {}
  const t = (label: string) => { timings[label] = Date.now() }

  t('start')
  const registry = getRegistryDO(env) as any

  // Phase 1: Seed each domain's metamodel in parallel (steps 1-5)
  t('phase1_start')
  const results = await Promise.all(
    domains.map(async (entry) => {
      const domainDO = getDomainDO(env, entry.slug) as any
      await domainDO.setDomainId(entry.slug)
      const domainRecord = await ensureDomain(domainDO, entry.slug, entry.name)
      const domainUUID = domainRecord.id as string
      const adapter = domainDO as any
      const claimsWithoutFacts = { ...entry.claims, facts: [] }
      const result = await ingestClaims(adapter, {
        claims: claimsWithoutFacts,
        domainId: domainUUID,
      })
      return { domain: entry.slug, domainId: domainUUID, ...result }
    })
  )
  t('phase1_end')

  // Build slug → UUID map from phase 1 results
  const slugToUUID = new Map<string, string>()
  for (const r of results) slugToUUID.set(r.domain, r.domainId)

  // Phase 1.5: Register domains + nouns in Registry (batched, not per-noun RPC)
  t('registry_start')
  await Promise.all(
    domains.map(async (entry) => {
      const uuid = slugToUUID.get(entry.slug)
      await registry.registerDomain(entry.slug, entry.slug, 'private', uuid)
    })
  )
  // Batch all noun indexing: collect all noun→domain pairs, then index
  const nounPairs: Array<[string, string]> = []
  for (const entry of domains) {
    for (const noun of entry.claims.nouns) {
      nounPairs.push([noun.name, entry.slug])
    }
  }
  // Index in parallel batches of 50
  for (let i = 0; i < nounPairs.length; i += 50) {
    const batch = nounPairs.slice(i, i + 50)
    await Promise.all(batch.map(([name, slug]) => registry.indexNoun(name, slug)))
  }
  t('registry_end')

  // Phase 2: Process instance facts (after all metamodels are seeded)
  t('phase2_start')
  // Apply schema for all domains in parallel first
  await Promise.all(
    domains.filter(e => e.claims.facts?.length).map(async (entry) => {
      const domainDO = getDomainDO(env, entry.slug) as any
      const uuid = slugToUUID.get(entry.slug) || entry.slug
      try { await domainDO.applySchema(uuid) } catch {}
    })
  )
  t('schema_applied')

  // Process facts in parallel per domain
  await Promise.all(
    domains.filter(e => e.claims.facts?.length).map(async (entry) => {
      const domainDO = getDomainDO(env, entry.slug) as any
      const uuid = slugToUUID.get(entry.slug) || entry.slug
      for (const fact of entry.claims.facts!) {
        try {
          const entityName = fact.entity || ''
          const entityRef = fact.entityValue || ''
          const fieldValues: Record<string, string> = {}
          if (fact.entity && fact.valueType && fact.value) {
            const fieldName = fact.valueType
              .split(' ')
              .map((w: string, i: number) =>
                i === 0 ? w.toLowerCase() : w.charAt(0).toUpperCase() + w.slice(1).toLowerCase()
              )
              .join('')
            fieldValues[fieldName] = fact.value
          }
          if (entityName) {
            await domainDO.createEntity(uuid, entityName, fieldValues, entityRef)
          }
        } catch { /* best-effort */ }
      }
    })
  )
  t('phase2_end')

  // Build timing report
  const report: Record<string, string> = {}
  const s = timings['start']
  for (const [k, v] of Object.entries(timings)) {
    if (k !== 'start') report[k] = `${v - s}ms`
  }

  return json({ domains: results, timings: report })
}

// ── Single domain seeding ────────────────────────────────────────────

async function handleSingleSeed(
  env: Env,
  body: { claims?: ExtractedClaims; domain?: string; domainId?: string },
): Promise<Response> {
  const slug = body.domain
  const rawId = body.domainId
  if (!slug && !rawId) {
    return error(400, { errors: [{ message: 'domainId or domains[] required' }] })
  }

  const domainSlug = slug || rawId!
  const domainDO = getDomainDO(env, domainSlug) as any
  await domainDO.setDomainId(domainSlug)

  // Ensure domain record exists in this DO
  const domainRecord = await ensureDomain(domainDO, domainSlug)
  const domainUUID = domainRecord.id as string

  const adapter = domainDO as any
  const result = await ingestClaims(adapter, { claims: body.claims!, domainId: domainUUID })

  // Register in the global registry
  const registry = getRegistryDO(env) as any
  for (const noun of body.claims!.nouns) {
    await registry.indexNoun(noun.name, domainSlug)
  }
  await registry.registerDomain(domainSlug, domainSlug)

  return json({ ...result, domainId: domainUUID })
}

// ── Domain record helper ─────────────────────────────────────────────

async function ensureDomain(db: any, slug: string, name?: string): Promise<Record<string, any>> {
  const existing = await db.findInCollection('domains', {
    domainSlug: { equals: slug },
  }, { limit: 1 })

  if (existing.docs.length) return existing.docs[0]

  try {
    return await db.createInCollection('domains', {
      domainSlug: slug,
      name: name || slug,
      visibility: 'private',
    })
  } catch {
    // UNIQUE constraint race: another concurrent call created it first
    const retry = await db.findInCollection('domains', {
      domainSlug: { equals: slug },
    }, { limit: 1 })
    if (retry.docs.length) return retry.docs[0]
    throw new Error(`Failed to create or find domain: ${slug}`)
  }
}

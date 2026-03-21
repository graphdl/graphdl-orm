import { json, error } from 'itty-router'
import type { Env } from '../types'
import { ingestClaims } from '../claims/ingest'
import type { ExtractedClaims } from '../claims/ingest'
import { createDomainAdapter } from '../do-adapter'

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
    domains?: Array<{ slug: string; name?: string; claims: ExtractedClaims }>
  }

  if (body.type !== 'claims') {
    return error(400, { errors: [{ message: 'Unsupported seed type. Use type: "claims"' }] })
  }

  // Bulk: multiple domains in one call — each gets its own DO
  if (body.domains?.length) {
    return handleBulkSeed(env, body.domains)
  }

  // Single domain
  if (body.claims) {
    return handleSingleSeed(env, body)
  }

  return error(400, { errors: [{ message: 'Provide claims + domainId, or domains[]' }] })
}

// ── Bulk multi-domain seeding ────────────────────────────────────────

async function handleBulkSeed(
  env: Env,
  domains: Array<{ slug: string; name?: string; claims: ExtractedClaims }>,
): Promise<Response> {
  const registry = getRegistryDO(env) as any

  // Phase 1: Seed each domain's metamodel in parallel (steps 1-5)
  // Each domain gets its own DomainDB DO — no shared state needed
  const results = await Promise.all(
    domains.map(async (entry) => {
      const domainDO = getDomainDO(env, entry.slug) as any
      await domainDO.setDomainId(entry.slug)

      // Ensure domain record exists in this DO
      await ensureDomain(domainDO, entry.slug, entry.name)

      // Use adapter to make DomainDB look like GraphDLDBLike
      const adapter = createDomainAdapter(domainDO)

      // Ingest metamodel only (nouns, readings, constraints, transitions — no facts)
      const claimsWithoutFacts = { ...entry.claims, facts: [] }
      const result = await ingestClaims(adapter, {
        claims: claimsWithoutFacts,
        domainId: entry.slug,
      })

      // Index nouns in the global registry for cross-domain resolution
      for (const noun of entry.claims.nouns) {
        await registry.indexNoun(noun.name, entry.slug)
      }
      await registry.registerDomain(entry.slug, entry.slug)

      return { domain: entry.slug, domainId: entry.slug, ...result }
    })
  )

  // Phase 2: Process instance facts for all domains (after all metamodels are seeded)
  // Facts can reference nouns from other domains via the Registry
  for (const entry of domains) {
    if (!entry.claims.facts?.length) continue

    const domainDO = getDomainDO(env, entry.slug) as any

    // Apply schema before creating entity instances
    try { await domainDO.applySchema(entry.slug) } catch { /* may fail if no readings yet */ }

    // Process each fact via createEntity on the DomainDB DO
    for (const fact of entry.claims.facts) {
      try {
        const entityName = fact.entity || ''
        const entityRef = fact.entityValue || ''
        const fieldValues: Record<string, string> = {}

        if (fact.entity && fact.valueType) {
          if (fact.value) {
            const fieldName = fact.valueType
              .split(' ')
              .map((w: string, i: number) =>
                i === 0 ? w.toLowerCase() : w.charAt(0).toUpperCase() + w.slice(1).toLowerCase()
              )
              .join('')
            fieldValues[fieldName] = fact.value
          }
        }

        if (entityName) {
          await domainDO.createEntity(entry.slug, entityName, fieldValues, entityRef)
        }
      } catch { /* best-effort fact creation */ }
    }
  }

  return json({ domains: results })
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
  await ensureDomain(domainDO, domainSlug)

  const adapter = createDomainAdapter(domainDO)
  const result = await ingestClaims(adapter, { claims: body.claims!, domainId: domainSlug })

  // Register in the global registry
  const registry = getRegistryDO(env) as any
  for (const noun of body.claims!.nouns) {
    await registry.indexNoun(noun.name, domainSlug)
  }
  await registry.registerDomain(domainSlug, domainSlug)

  return json({ ...result, domainId: domainSlug })
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

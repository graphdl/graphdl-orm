/**
 * POST /api/claims — ingest structured claims into one or more domains.
 *
 * Single domain:
 *   { claims: ExtractedClaims, domain: "slug" }
 *   { claims: ExtractedClaims, domainId: "uuid" }
 *
 * Batch (multiple domains):
 *   { domains: [{ slug: "a", claims: {...} }, { slug: "b", claims: {...} }] }
 *
 * Legacy format (type: "claims" wrapper) also accepted for backwards compatibility.
 */

import { json, error } from 'itty-router'
import type { Env } from '../types'
import { ingestClaims, ingestProject } from '../claims/ingest'
import type { ExtractedClaims } from '../claims/ingest'

function getDB(env: Env) {
  const id = env.DOMAIN_DB.idFromName('graphdl-primary')
  return env.DOMAIN_DB.get(id)
}

async function ensureDomain(db: any, slug: string, name?: string): Promise<Record<string, any>> {
  const existing = await db.findInCollection('domains', {
    domainSlug: { equals: slug },
  }, { limit: 1 })

  if (existing.docs.length) return existing.docs[0]

  return db.createInCollection('domains', {
    domainSlug: slug,
    name: name || slug,
    visibility: 'private',
  })
}

export async function handleClaims(request: Request, env: Env): Promise<Response> {
  if (request.method !== 'POST') {
    return error(405, { errors: [{ message: 'POST required' }] })
  }

  const db = getDB(env)
  const body = await request.json() as Record<string, any>

  // Unwrap legacy { type: "claims", ... } wrapper
  const claims: ExtractedClaims | undefined = body.claims
  const domains: Array<{ slug: string; name?: string; claims: ExtractedClaims }> | undefined =
    body.domains
  const domainSlug: string | undefined = body.domain
  const domainId: string | undefined = body.domainId

  // Batch: multiple domains
  if (domains?.length) {
    // Ensure all domain records exist first
    const domainRecords = await Promise.all(
      domains.map(entry => ensureDomain(db as any, entry.slug, entry.name))
    )
    // Build input for ingestProject
    const projectDomains = domains.map((entry, i) => ({
      domainId: domainRecords[i].id as string,
      claims: entry.claims,
    }))
    const projectResult = await ingestProject(db as any, projectDomains)
    // Flatten into the existing response shape
    const results = domains.map((entry, i) => {
      const domainId = domainRecords[i].id as string
      const r = projectResult.domains.get(domainId)!
      return { domain: entry.slug, domainId, ...r }
    })
    return json({ domains: results })
  }

  // Single domain
  if (claims) {
    if (!domainSlug && !domainId) {
      return error(400, { errors: [{ message: 'domain or domainId required' }] })
    }

    const domain = domainSlug
      ? await ensureDomain(db as any, domainSlug)
      : { id: domainId }
    const result = await ingestClaims(db as any, { claims, domainId: domain!.id })
    return json({ ...result, domainId: domain!.id })
  }

  return error(400, { errors: [{ message: 'Provide claims + domain, or domains[]' }] })
}

/**
 * GET /api/stats — domain statistics (nouns, readings, constraints per domain).
 */
export async function handleStats(request: Request, env: Env): Promise<Response> {
  const db = getDB(env) as any

  const [allNouns, allReadings, allDomains, allSchemas, allConstraints] = await Promise.all([
    db.findInCollection('nouns', {}, { limit: 0 }),
    db.findInCollection('readings', {}, { limit: 0 }),
    db.findInCollection('domains', {}, { limit: 100 }),
    db.findInCollection('graph-schemas', {}, { limit: 0 }),
    db.findInCollection('constraints', {}, { limit: 0 }),
  ])

  const perDomain: Record<string, { nouns: number; readings: number }> = {}
  for (const d of allDomains.docs) {
    const slug = (d.domainSlug || d.id) as string
    const [nouns, readings] = await Promise.all([
      db.findInCollection('nouns', { domain: { equals: d.id } }, { limit: 0 }),
      db.findInCollection('readings', { domain: { equals: d.id } }, { limit: 0 }),
    ])
    perDomain[slug] = { nouns: nouns.totalDocs, readings: readings.totalDocs }
  }

  return json({
    totals: {
      domains: allDomains.totalDocs,
      nouns: allNouns.totalDocs,
      readings: allReadings.totalDocs,
      graphSchemas: allSchemas.totalDocs,
      constraints: allConstraints.totalDocs,
    },
    perDomain,
  })
}

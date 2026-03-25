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
import { persistViolations } from '../worker/outcomes'
import { materializeBatch } from '../worker/materialize'
import { ensureDomain } from './ensure-domain'

function getDB(env: Env) {
  const id = env.DOMAIN_DB.idFromName('graphdl-primary')
  return env.DOMAIN_DB.get(id)
}

function getRegistry(env: Env) {
  return env.REGISTRY_DB.get(env.REGISTRY_DB.idFromName('global'))
}

export async function handleClaims(request: Request, env: Env): Promise<Response> {
  if (request.method !== 'POST') {
    return error(405, { errors: [{ message: 'POST required' }] })
  }

  const db = getDB(env)
  const registry = getRegistry(env) as any
  const body = await request.json() as Record<string, any>

  // Unwrap legacy { type: "claims", ... } wrapper
  const claims: ExtractedClaims | undefined = body.claims
  const domains: Array<{ slug: string; name?: string; claims: ExtractedClaims }> | undefined =
    body.domains
  const domainSlug: string | undefined = body.domain
  const domainId: string | undefined = body.domainId

  // Batch: multiple domains
  if (domains?.length) {
    // Ensure all domain records exist first via Registry+EntityDB
    const domainRecords = await Promise.all(
      domains.map(entry => ensureDomain(env, registry, entry.slug, entry.name))
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
      ? await ensureDomain(env, registry, domainSlug)
      : { id: domainId }
    const result = await ingestClaims(db as any, { claims, domainId: domain!.id })

    // Materialize batch entities to EntityDB DOs + Registry
    // Batch entities use domain UUID internally; Registry indexes by slug
    const resolvedSlug = domainSlug || (domain as any)?.domainSlug || domain!.id
    if (result.batch?.entities?.length) {
      const nonViolationEntities = result.batch.entities
        .filter((e: any) => e.type !== 'Violation')
        .map((e: any) => ({ ...e, domain: resolvedSlug }))
      if (nonViolationEntities.length > 0) {
        const materializeResult = await materializeBatch(
          nonViolationEntities,
          (id: string) => env.ENTITY_DB.get(env.ENTITY_DB.idFromName(id)) as any,
          registry,
        )
        if (materializeResult.failed.length > 0) {
          console.error('Materialization failures:', materializeResult.failed)
        }
      }
    }

    // Persist violations as Violation entities (best-effort)
    if (result.batch?.entities) {
      const violationEntities = result.batch.entities.filter((e: any) => e.type === 'Violation')
      if (violationEntities.length > 0) {
        persistViolations(env, violationEntities.map((e: any) => ({
          domain: domainSlug || domain!.id,
          constraintId: e.data?.constraintId || null,
          text: e.data?.text || e.data?.message || 'CSDP validation violation',
          severity: e.data?.severity || 'error',
          functionId: e.data?.functionId || null,
        }))).catch(() => { /* best-effort */ })
      }
    }

    return json({ ...result, domainId: domain!.id })
  }

  return error(400, { errors: [{ message: 'Provide claims + domain, or domains[]' }] })
}

/**
 * GET /api/stats — domain statistics (nouns, readings, constraints per domain).
 * Uses Registry entity counts instead of DomainDB queries.
 */
export async function handleStats(request: Request, env: Env): Promise<Response> {
  const registry = getRegistry(env) as any

  const domainSlugs: string[] = await registry.listDomains()

  // Count all entities globally (no domain filter)
  const [allNounIds, allReadingIds, allSchemaIds, allConstraintIds] = await Promise.all([
    registry.getEntityIds('Noun') as Promise<string[]>,
    registry.getEntityIds('Reading') as Promise<string[]>,
    registry.getEntityIds('GraphSchema') as Promise<string[]>,
    registry.getEntityIds('Constraint') as Promise<string[]>,
  ])

  // Per-domain counts
  const perDomain: Record<string, { nouns: number; readings: number }> = {}
  await Promise.all(
    domainSlugs.map(async (slug: string) => {
      const [nounIds, readingIds] = await Promise.all([
        registry.getEntityIds('Noun', slug) as Promise<string[]>,
        registry.getEntityIds('Reading', slug) as Promise<string[]>,
      ])
      perDomain[slug] = { nouns: nounIds.length, readings: readingIds.length }
    })
  )

  return json({
    totals: {
      domains: domainSlugs.length,
      nouns: allNounIds.length,
      readings: allReadingIds.length,
      graphSchemas: allSchemaIds.length,
      constraints: allConstraintIds.length,
    },
    perDomain,
  })
}

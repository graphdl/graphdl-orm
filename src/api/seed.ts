/**
 * Seed endpoint — parse readings via ρ (WASM), materialize as cells in D.
 *
 * Per the paper:
 *   parse: R → Φ (Theorem 2)
 *   Each reading becomes entities (Noun, Reading, Constraint, Graph Schema, Role)
 *   Entities are cells in D, indexed in Registry (P = ↑FILE:D)
 *
 * The Rust WASM engine (parse_forml2.rs) is the ONLY parser.
 * No TS parsing. No procedural entity construction.
 */

import { json, error } from 'itty-router'
import type { Env } from '../types'
import { parseReadings } from './engine'

function getRegistryDO(env: Env) {
  return env.REGISTRY_DB.get(env.REGISTRY_DB.idFromName('global'))
}

export async function handleSeed(request: Request, env: Env): Promise<Response> {
  if (request.method === 'GET') return handleSeedGet(env)
  if (request.method === 'DELETE') return handleSeedDelete(env)
  if (request.method === 'POST') return handleSeedPost(request, env)
  return error(405, { errors: [{ message: 'Method not allowed' }] })
}

// ── GET /seed — stats from Registry ─────────────────────────────────

async function handleSeedGet(env: Env): Promise<Response> {
  const registry = getRegistryDO(env) as any
  const domainSlugs: string[] = await registry.listDomains()

  if (!domainSlugs.length) {
    return json({
      totals: { domains: 0, nouns: 0, readings: 0, graphSchemas: 0, constraints: 0 },
      perDomain: {},
    })
  }

  const perDomainEntries = await Promise.all(
    domainSlugs.map(async (slug) => {
      const [nounIds, readingIds, schemaIds, constraintIds] = await Promise.all([
        registry.getEntityIds('Noun', slug) as Promise<string[]>,
        registry.getEntityIds('Reading', slug) as Promise<string[]>,
        registry.getEntityIds('Graph Schema', slug) as Promise<string[]>,
        registry.getEntityIds('Constraint', slug) as Promise<string[]>,
      ])
      return { slug, nouns: nounIds.length, readings: readingIds.length, graphSchemas: schemaIds.length, constraints: constraintIds.length }
    })
  )

  const totals = { domains: domainSlugs.length, nouns: 0, readings: 0, graphSchemas: 0, constraints: 0 }
  const perDomain: Record<string, { nouns: number; readings: number }> = {}
  for (const e of perDomainEntries) {
    totals.nouns += e.nouns
    totals.readings += e.readings
    totals.graphSchemas += e.graphSchemas
    totals.constraints += e.constraints
    perDomain[e.slug] = { nouns: e.nouns, readings: e.readings }
  }

  return json({ totals, perDomain })
}

// ── DELETE /seed — wipe population ──────────────────────────────────

async function handleSeedDelete(env: Env): Promise<Response> {
  const registry = getRegistryDO(env) as any
  await registry.wipeAll()
  return json({ message: 'All data wiped' })
}

// ── POST /seed — parse readings via ρ, materialize as cells ─────────

async function handleSeedPost(request: Request, env: Env): Promise<Response> {
  const contentType = request.headers.get('content-type') || ''

  // Collect raw markdown texts by domain slug
  const rawDomains: Array<{ slug: string; text: string }> = []

  if (contentType.includes('multipart/form-data')) {
    const formData = await request.formData()
    for (const [name, value] of formData.entries()) {
      if (!(value instanceof File) && typeof value !== 'string') continue
      const text = value instanceof File ? await value.text() : value
      if (!text.trim()) continue
      rawDomains.push({ slug: name.replace(/\.md$/i, ''), text })
    }
  } else {
    const body = await request.json() as any
    if (body.text && body.domain) {
      rawDomains.push({ slug: body.domain, text: body.text })
    } else if (body.domains?.length) {
      for (const d of body.domains) {
        if (d.text) rawDomains.push({ slug: d.slug, text: d.text })
      }
    }
  }

  if (rawDomains.length === 0) {
    return error(400, { errors: [{ message: 'No readings provided' }] })
  }

  const registry = getRegistryDO(env) as any
  const results: Array<{ domain: string; entities: number; nouns: number; readings: number; errors: string[] }> = []

  // For each domain: parse via ρ (WASM), register domain, materialize cells
  for (const { slug, text } of rawDomains) {
    const errors: string[] = []

    // parse: R → Φ via WASM (the ONLY parser)
    let entities: Array<{ id: string; type: string; domain: string; data: Record<string, unknown> }>
    try {
      entities = parseReadings(text, slug)
    } catch (e) {
      errors.push(`parse error: ${e}`)
      results.push({ domain: slug, entities: 0, nouns: 0, readings: 0, errors })
      continue
    }

    // Register domain in Registry
    await registry.registerDomain(slug, slug, 'private')

    // Index nouns for cross-domain reference
    const nounEntities = entities.filter(e => e.type === 'Noun')
    for (const noun of nounEntities) {
      const name = noun.data.name as string
      if (name) await registry.indexNoun(name, slug)
    }

    // Materialize all entities as cells in D
    if (entities.length > 0) {
      await registry.materializeBatch(entities)
    }

    const readingEntities = entities.filter(e => e.type === 'Reading')
    results.push({
      domain: slug,
      entities: entities.length,
      nouns: nounEntities.length,
      readings: readingEntities.length,
      errors,
    })
  }

  return json({ domains: results })
}

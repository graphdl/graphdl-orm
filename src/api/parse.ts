/**
 * Parse endpoint — compile ∘ parse: readings → cells in D.
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
import { parseReadings, parseReadingsWithNouns } from './engine'

function getRegistryDO(env: Env) {
  return env.REGISTRY_DB.get(env.REGISTRY_DB.idFromName('global'))
}

export async function handleParse(request: Request, env: Env): Promise<Response> {
  if (request.method === 'GET') return handleParseGet(env)
  if (request.method === 'DELETE') return handleParseDelete(env)
  if (request.method === 'POST') return handleParsePost(request, env)
  return error(405, { errors: [{ message: 'Method not allowed' }] })
}

// ── GET /parse — stats from Registry ─────────────────────────────────

async function handleParseGet(env: Env): Promise<Response> {
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

// ── DELETE /parse — wipe population ──────────────────────────────────

async function handleParseDelete(env: Env): Promise<Response> {
  const registry = getRegistryDO(env) as any
  await registry.wipeAll()
  return json({ message: 'All data wiped' })
}

// ── POST /parse — parse readings via ρ, materialize as cells ─────────

async function handleParsePost(request: Request, env: Env): Promise<Response> {
  const contentType = request.headers.get('content-type') || ''

  // Collect raw markdown texts by domain slug
  const rawDomains: Array<{ slug: string; text: string }> = []

  if (contentType.includes('multipart/form-data')) {
    const formData = await request.formData()
    for (const [name, value] of formData.entries()) {
      if (!((value as any) instanceof File) && typeof value !== 'string') continue
      const text = (value as any) instanceof File ? await (value as any).text() : value as string
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

  // Domains are NORMA tabs, not separate universes. Nouns are global.
  // Load existing noun definitions from Noun entities for cross-domain resolution.
  let existingNounsJson = '{}'
  try {
    const allDomains: string[] = await registry.listDomains()
    const mergedNouns: Record<string, any> = {}
    const nounSets = await Promise.allSettled(
      allDomains.map(async (d: string) => {
        const nounIds: string[] = await registry.getEntityIds('Noun', d)
        const settled = await Promise.allSettled(
          nounIds.map(async (id: string) => {
            const stub = env.ENTITY_DB.get(env.ENTITY_DB.idFromName(id)) as any
            const cell = await stub.get()
            return cell ? { id: cell.id, ...cell.data } : null
          }),
        )
        return settled
          .filter((r): r is PromiseFulfilledResult<any> => r.status === 'fulfilled' && r.value)
          .map(r => r.value)
      }),
    )
    // Merge all noun definitions from all domains into NounDef-shaped objects.
    // NounDef only has objectType and worldAssumption.
    nounSets
      .filter((r): r is PromiseFulfilledResult<any[]> => r.status === 'fulfilled')
      .flatMap(r => r.value)
      .forEach((n: any) => {
        const name = n.name || n.id
        if (!name) return
        mergedNouns[name] = {
          objectType: n.objectType || 'entity',
          worldAssumption: 'closed',
        }
      })
    existingNounsJson = JSON.stringify(mergedNouns)
  } catch {}

  // Parse each domain with the full noun context from all existing domains.
  for (const { slug, text } of rawDomains) {
    const errors: string[] = []
    let entities: Array<{ id: string; type: string; domain: string; data: Record<string, unknown> }>
    try {
      entities = parseReadingsWithNouns(text, slug, existingNounsJson)
    } catch (e) {
      errors.push(`parse error: ${e}`)
      results.push({ domain: slug, entities: 0, nouns: 0, readings: 0, errors })
      continue
    }

    await registry.registerDomain(slug, slug, 'private')

    const nounEntities = entities.filter(e => e.type === 'Noun')
    await Promise.all(
      nounEntities.map(async (noun) => {
        const name = noun.data.name as string
        return name ? registry.indexNoun(name, slug) : null
      }),
    )

    if (entities.length > 0) {
      await registry.materializeBatch(entities)
    }

    // ↓DEFS — store raw readings (the source) and runtime bindings
    const defsData: Record<string, string> = {}
    defsData['*:read'] = 'local'
    defsData['*:readDetail'] = 'local'
    defsData['*:create'] = 'local'
    defsData['readings'] = text
    const instanceFactEntities = entities.filter(e => e.type === 'Instance Fact')
    for (const ife of instanceFactEntities) {
      const d = ife.data as any
      if (d.subjectNoun === 'Noun' && d.objectNoun === 'External System') {
        defsData[`${d.subjectValue}:read`] = 'external'
        defsData[`${d.subjectValue}:readDetail`] = 'external'
      }
    }
    const defsStub = env.ENTITY_DB.get(env.ENTITY_DB.idFromName(`defs:${slug}`)) as any
    await defsStub.put({ id: `defs:${slug}`, type: 'Defs', domain: slug, data: defsData })

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

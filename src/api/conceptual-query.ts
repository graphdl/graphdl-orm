/**
 * Conceptual Query API handler.
 *
 * Takes natural language queries like:
 *   "Customer that submits Support Request that has Priority 'High'"
 * Resolves through reading paths, fans out to Entity DOs, returns results.
 */
import { json, error } from 'itty-router'
import type { Env } from '../types'
import { resolveConceptualQuery } from '../derivation/conceptual-query'
import type { QueryPathStep } from '../derivation/conceptual-query'

interface NounDoc {
  id: string
  name: string
  objectType: 'entity' | 'value'
}

interface ReadingDoc {
  id: string
  text: string
}

/**
 * Execute a conceptual query against live DOs.
 *
 * Algorithm:
 * 1. Load nouns and readings from the domain
 * 2. Resolve the query text into a reading path
 * 3. Walk the path: fetch root entities, then follow each hop via FK joins
 * 4. Apply value filters
 * 5. Return matching entities
 */
export async function handleConceptualQuery(request: Request, env: Env) {
  const url = new URL(request.url)
  let queryText: string | null = null
  let domainId: string | null = null

  if (request.method === 'POST') {
    const body = await request.json() as Record<string, any>
    queryText = body.q || body.query || body.text
    domainId = body.domain
  } else {
    queryText = url.searchParams.get('q')
    domainId = url.searchParams.get('domain')
  }

  if (!queryText) return error(400, { errors: [{ message: 'query text required (q param or body.q)' }] })
  if (!domainId) return error(400, { errors: [{ message: 'domain required' }] })

  // Load nouns and readings via Registry+EntityDB fan-out
  const registry = env.REGISTRY_DB.get(env.REGISTRY_DB.idFromName('global')) as any
  const domainSlug = await resolveDomainSlug(env, registry, domainId)
  if (!domainSlug) return error(404, { errors: [{ message: `Domain not found: ${domainId}` }] })

  const [nounIds, readingIds] = await Promise.all([
    registry.getEntityIds('Noun', domainSlug) as Promise<string[]>,
    registry.getEntityIds('Reading', domainSlug) as Promise<string[]>,
  ])
  const [nounEntities, readingEntities] = await Promise.all([
    Promise.all(nounIds.map(id => (env.ENTITY_DB.get(env.ENTITY_DB.idFromName(id)) as any).get())),
    Promise.all(readingIds.map(id => (env.ENTITY_DB.get(env.ENTITY_DB.idFromName(id)) as any).get())),
  ])

  const nounDocs: NounDoc[] = nounEntities
    .filter(Boolean)
    .map((e: any) => ({ id: e.id, name: e.data?.name, objectType: e.data?.objectType || e.data?.object_type }))
    .filter((n: any) => n.objectType === 'entity')
  const readingDocs: ReadingDoc[] = readingEntities
    .filter(Boolean)
    .map((e: any) => ({ id: e.id, text: e.data?.text }))

  const nouns = nounDocs.filter((n: any) => n.name && typeof n.name === 'string').map(n => ({ name: n.name, id: n.id }))
  const readings = readingDocs.filter((r: any) => r.text && typeof r.text === 'string').map(r => {
    // Extract noun names from reading text using the known noun list
    const readingNouns: string[] = []
    const sorted = [...nouns].sort((a, b) => b.name.length - a.name.length)
    let remaining = r.text
    for (const noun of sorted) {
      if (remaining.includes(noun.name)) {
        readingNouns.push(noun.name)
        remaining = remaining.replace(noun.name, '\0'.repeat(noun.name.length))
      }
    }
    // Extract predicate between first two nouns
    let predicate = ''
    if (readingNouns.length >= 2) {
      const start = r.text.indexOf(readingNouns[0]) + readingNouns[0].length
      const end = r.text.indexOf(readingNouns[1], start)
      if (end > start) predicate = r.text.slice(start, end).trim()
    }
    return { text: r.text, nouns: readingNouns.filter(Boolean) }
  })

  // Resolve the conceptual query — resolveConceptualQuery takes string[] for nouns
  const nounNames = nouns.map(n => n.name)
  const resolved = resolveConceptualQuery(queryText, nounNames, readings)

  if (resolved.path.length === 0) {
    return json({
      query: queryText,
      resolved: false,
      message: 'No reading path found for this query. Check that the nouns and relationships are declared in the domain.',
      availableNouns: nouns.map(n => n.name),
    })
  }

  // Walk the path against live Entity DOs (reuse the registry stub from above)
  const results = await walkPath(env, registry, resolved.path, resolved.filters)

  return json({
    query: queryText,
    resolved: true,
    path: resolved.path.map(s => `${s.from} ${s.inverse ? '<-' : '->'} ${s.predicate} ${s.inverse ? '<-' : '->'} ${s.to}`),
    rootNoun: resolved.rootNoun,
    filters: resolved.filters,
    results,
    count: results.length,
  })
}

/**
 * Walk a reading path against live Entity DOs.
 * Starts by fetching all entities of the root noun type,
 * then joins through each hop using FK relationships.
 */
async function walkPath(
  env: Env,
  registry: any,
  path: QueryPathStep[],
  filters: Array<{ field: string; value: string }>,
): Promise<Record<string, unknown>[]> {
  if (path.length === 0) return []

  const rootNoun = path[0].from
  const rootIds: string[] = await registry.getEntityIds(rootNoun)

  // Fetch root entities
  let currentEntities = await fetchEntities(env, rootIds)

  // Walk each hop
  for (const step of path) {
    const targetNoun = step.to
    const targetIds: string[] = await registry.getEntityIds(targetNoun)

    if (targetIds.length === 0) continue

    const targetEntities = await fetchEntities(env, targetIds)

    if (step.inverse) {
      // Inverse: target entities reference current entities
      // Find target entities that have a FK pointing to any current entity
      const currentIds = new Set(currentEntities.map(e => e.id as string))
      const fkField = toFKField(step.from)
      currentEntities = targetEntities.filter(te => {
        const fkValue = te[fkField] || te[step.from.toLowerCase().replace(/\s+/g, '_') + '_id']
        return fkValue && currentIds.has(fkValue as string)
      })
    } else {
      // Forward: current entities reference target entities
      // For "has" predicates, the target value is a column on the current entity
      if (step.predicate === 'has') {
        // Value type relationship: the target noun name is a column
        // Apply filters to current entities directly
        // No join needed — the value is on the entity
      } else {
        // Entity relationship: join via FK
        const fkField = toFKField(targetNoun)
        const targetMap = new Map(targetEntities.map(e => [e.id as string, e]))
        const joined: Record<string, unknown>[] = []
        for (const entity of currentEntities) {
          const fkValue = entity[fkField] || entity[targetNoun.toLowerCase().replace(/\s+/g, '_') + '_id']
          if (fkValue && targetMap.has(fkValue as string)) {
            joined.push({ ...entity, [`_${targetNoun}`]: targetMap.get(fkValue as string) })
          }
        }
        currentEntities = joined.length > 0 ? joined : targetEntities
      }
    }
  }

  // Apply value filters
  for (const filter of filters) {
    const fieldName = toColumnName(filter.field)
    currentEntities = currentEntities.filter(e => {
      const val = e[fieldName] || e[filter.field] || e[filter.field.toLowerCase()]
      return val !== undefined && String(val) === filter.value
    })
  }

  return currentEntities
}

/** Fetch entities from Entity DOs by ID */
async function fetchEntities(
  env: Env,
  entityIds: string[],
): Promise<Record<string, unknown>[]> {
  const results: Record<string, unknown>[] = []

  // Batch in groups of 50
  for (let i = 0; i < entityIds.length; i += 50) {
    const batch = entityIds.slice(i, i + 50)
    const batchResults = await Promise.all(
      batch.map(async (id) => {
        const entityDO = env.ENTITY_DB.get(env.ENTITY_DB.idFromName(id)) as any
        try {
          const entity = await entityDO.get()
          if (entity && !entity.deletedAt) {
            return { id: entity.id, type: entity.type, ...entity.data, createdAt: entity.createdAt, updatedAt: entity.updatedAt }
          }
        } catch { /* entity DO may not exist */ }
        return null
      })
    )
    for (const r of batchResults) {
      if (r) results.push(r)
    }
  }

  return results
}

/** Convert noun name to FK field name: "Support Request" → "supportRequestId" */
function toFKField(noun: string): string {
  const parts = noun.split(/\s+/)
  return parts[0].toLowerCase() + parts.slice(1).map(w => w.charAt(0).toUpperCase() + w.slice(1)).join('') + 'Id'
}

/** Convert noun name to column name: "Priority" → "priority", "Support Request" → "support_request" */
function toColumnName(noun: string): string {
  return noun.toLowerCase().replace(/\s+/g, '_')
}

/** Resolve a domain ID (UUID or slug) to a domain slug string */
async function resolveDomainSlug(env: Env, registry: any, domainId: string): Promise<string | null> {
  // Slug (contains hyphens, not UUID-shaped)
  if (domainId.includes('-') && !domainId.match(/^[0-9a-f]{8}-[0-9a-f]{4}-/)) {
    return domainId
  }

  // UUID — try registry
  try {
    const slug: string | null = await registry.resolveSlugByUUID(domainId)
    if (slug) return slug
  } catch { /* fall through */ }

  return null
}

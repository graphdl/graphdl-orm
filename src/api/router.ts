import { AutoRouter, json, error } from 'itty-router'
import type { Env } from '../types'
import { parseQueryOptions } from './collections'
import { nounToSlug, nounToTable, resolveSlugToNoun } from '../collections'
import { handleSeed } from './seed'
import { handleGenerate } from './generate'
import { handleParse } from './parse'
import { handleParseOrm } from './parse-orm'
import { handleVerify } from './verify'
import { handleEvaluate, handleSynthesize } from './evaluate'
import { handleConceptualQuery } from './conceptual-query'
import { induceConstraints } from '../csdp/induce'
import { handleListEntities, handleGetEntity, handleCreateEntity, handleDeleteEntity, buildEntityLinks } from './entity-routes'
import { loadDomainSchema, loadDomainAndPopulation, getTransitions, applyCommand, querySchema, forwardChain, getNounSchemas } from './engine'

// ── Collection slug → noun type resolution ───────────────────────────
// Resolved dynamically from the Registry via nounToSlug convention.
// Noun entities are seeded from readings — no hardcoded maps.

// ── DO helpers ───────────────────────────────────────────────────────

/** Get an EntityDB DO stub for the given entity ID. */
function getEntityDO(env: Env, entityId: string): DurableObjectStub {
  const id = env.ENTITY_DB.idFromName(entityId)
  return env.ENTITY_DB.get(id)
}

/** Get a RegistryDB DO stub for the given scope (e.g. app/org/global). */
function getRegistryDO(env: Env, scope: string): DurableObjectStub {
  const id = env.REGISTRY_DB.idFromName(scope)
  return env.REGISTRY_DB.get(id)
}

/**
 * Get the primary DomainDB DO stub.
 * Returns the primary DomainDB DO stub for generators cache and legacy operations.
 * Uses a single "primary" instance for all collection/metamodel operations.
 */
function getPrimaryDB(env: Env): DurableObjectStub {
  const id = env.DOMAIN_DB.idFromName('graphdl-primary')
  return env.DOMAIN_DB.get(id)
}

export const router = AutoRouter()

// ── Health ───────────────────────────────────────────────────────────
router.get('/health', () => json({ status: 'ok', version: '0.1.0' }))

// ── Debug: entity counts by type from Registry ──────────────────────
router.post('/api/debug/arest/:domain', async (request, env: Env) => {
  const { domain } = request.params
  const body = await request.json() as any
  const registry = getRegistryDO(env, 'global') as any
  const getStub = (id: string) => getEntityDO(env, id) as any
  const populationJson = await loadDomainAndPopulation(registry, getStub, domain)
  const result = applyCommand(body, populationJson)
  return json(result)
})

router.get('/api/debug/compiled/:domain', async (request, env: Env) => {
  const { domain } = request.params
  const registry = getRegistryDO(env, 'global') as any
  await loadDomainSchema(registry, (id: string) => getEntityDO(env, id) as any, domain)
  const { debug_compiled_state: debugState } = await import('../../crates/fol-engine/pkg/fol_engine.js')
  return json(JSON.parse(debugState()))
})

router.get('/api/debug/schema/:domain', async (request, env: Env) => {
  const { domain } = request.params
  const registry = getRegistryDO(env, 'global') as any
  const { buildSchemaFromEntities } = await import('../generate/schema-from-entities')
  const fetchEntities = async (type: string, d: string) => {
    const ids: string[] = await registry.getEntityIds(type, d)
    const results: Array<{ id: string; type: string; data: Record<string, unknown> }> = []
    await Promise.allSettled(ids.map(async (id: string) => {
      const entity = await (getEntityDO(env, id) as any).get()
      if (entity && !entity.deletedAt) results.push({ id: entity.id, type: entity.type, data: entity.data })
    }))
    return results
  }
  const schema = await buildSchemaFromEntities(domain, fetchEntities)
  return json({
    domain,
    nounCount: Object.keys(schema.nouns).length,
    factTypeCount: Object.keys(schema.factTypes).length,
    constraintCount: schema.constraints.length,
    stateMachines: Object.entries(schema.stateMachines).map(([k, v]) => ({
      id: k, nounName: (v as any).nounName, statuses: (v as any).statuses, transitions: (v as any).transitions,
    })),
  })
})

router.get('/debug/table/:table', async (request, env: Env) => {
  const { table } = request.params
  const url = new URL(request.url)
  const domain = url.searchParams.get('domain') || undefined

  const registry = getRegistryDO(env, 'global') as any
  const counts = await registry.getEntityCounts(domain) as Array<{ nounType: string; count: number }>

  // Resolve: accept noun type name, collection slug, or table name
  const entityType = counts.find(c => c.nounType === table)?.nounType
    || counts.find(c => nounToSlug(c.nounType) === table || nounToTable(c.nounType) === table)?.nounType

  if (entityType) {
    const ids = await registry.getEntityIds(entityType, domain) as string[]
    return json({ table, entityType, domain: domain || 'all', count: ids.length, entityIds: ids })
  }

  return json({ table, domain: domain || 'all', counts })
})

// ── WebSocket (live events) ──────────────────────────────────────────
router.get('/ws', async (request, env: Env) => {
  if (request.headers.get('Upgrade') !== 'websocket') {
    return error(426, { errors: [{ message: 'WebSocket upgrade required' }] })
  }
  const url = new URL(request.url)
  const domain = url.searchParams.get('domain') || 'all'
  const stub = getPrimaryDB(env)
  return stub.fetch(new Request(`https://graphdl-orm/ws?domain=${domain}`, {
    headers: request.headers,
  }))
})

// ── Generate ────────────────────────────────────────────────────────
router.post('/api/generate', handleGenerate)
router.post('/api/evaluate', handleEvaluate)
router.post('/api/synthesize', (request, env) => handleSynthesize(request, env))

// ── Induction (discover constraints from population) ─────────────────
router.post('/api/induce', async (request) => {
  const body = await request.json() as { ir?: any; population?: any }
  if (!body.ir || !body.population) {
    return error(400, { errors: [{ message: 'ir and population are required' }] })
  }
  const result = induceConstraints(JSON.stringify(body.ir), JSON.stringify(body.population))
  return json(result)
})

// ── Facts (instance-level graph creation) ────────────────────────────
router.post('/api/facts', async (request, env: Env) => {
  const body = await request.json() as {
    domainId: string
    graphSchemaId?: string
    bindings?: Array<{ nounId: string; value?: string; resourceId?: string }>
    // Batch mode: resolve reading text + noun names
    facts?: Array<{
      reading: string
      bindings: Array<{ noun: string; value?: string; resourceId?: string }>
    }>
  }
  if (!body.domainId) {
    return error(400, { errors: [{ message: 'domainId required' }] })
  }

  const db = getPrimaryDB(env) as any

  // Single fact with pre-resolved IDs
  if (body.graphSchemaId && body.bindings?.length) {
    const result = await db.createFact(body.domainId, body.graphSchemaId, body.bindings)
    return json(result, { status: 201 })
  }

  // Batch mode: resolve reading text + noun names → IDs
  if (!body.facts?.length) {
    return error(400, { errors: [{ message: 'graphSchemaId+bindings or facts[] required' }] })
  }

  // Load nouns and readings for this domain via Registry+EntityDB fan-out
  const registry = getRegistryDO(env, 'global') as any

  const [nounIds, readingIds] = await Promise.all([
    registry.getEntityIds('Noun', body.domainId) as Promise<string[]>,
    registry.getEntityIds('Reading', body.domainId) as Promise<string[]>,
  ])

  const [nounSettled, readingSettled] = await Promise.all([
    Promise.allSettled(nounIds.map(async (nid: string) => {
      const stub = getEntityDO(env, nid) as any
      return stub.get()
    })),
    Promise.allSettled(readingIds.map(async (rid: string) => {
      const stub = getEntityDO(env, rid) as any
      return stub.get()
    })),
  ])

  const nounByName = new Map<string, any>()
  for (const r of nounSettled) {
    if (r.status === 'fulfilled' && r.value && !r.value.deletedAt) {
      const n = { id: r.value.id, ...r.value.data }
      if (n.name) nounByName.set(n.name, n)
    }
  }

  const schemaByReading = new Map<string, string>()
  for (const r of readingSettled) {
    if (r.status === 'fulfilled' && r.value && !r.value.deletedAt) {
      const rd = r.value.data
      if (rd.text && rd.graphSchema) schemaByReading.set(rd.text, rd.graphSchema)
    }
  }

  const results: any[] = []
  const errors: string[] = []

  for (const fact of body.facts) {
    try {
      const graphSchemaId = schemaByReading.get(fact.reading)
      if (!graphSchemaId) {
        errors.push(`Reading not found: "${fact.reading}"`)
        continue
      }

      const resolvedBindings = fact.bindings.map(b => {
        const noun = nounByName.get(b.noun)
        if (!noun) return null
        return { nounId: noun.id, value: b.value, resourceId: b.resourceId }
      }).filter(Boolean)

      if (resolvedBindings.length < 2) {
        errors.push(`Not enough nouns resolved for: "${fact.reading}"`)
        continue
      }

      const result = await db.createFact(body.domainId, graphSchemaId, resolvedBindings)
      results.push(result)
    } catch (err: any) {
      errors.push(`"${fact.reading}": ${err.message}`)
    }
  }

  return json({ facts: results, errors }, { status: 201 })
})

// ── Entity (high-level: create noun instance with fields) ────────────
router.post('/api/entity', async (request, env: Env) => {
  const body = await request.json() as {
    noun: string        // noun name, e.g. "SupportRequest"
    domain: string      // domain ID
    fields: Record<string, string | string[] | Record<string, any>>  // string, array, or nested entity
    reference?: string  // display reference
    createdBy?: string  // creator identity
  }
  if (!body.noun || !body.domain || !body.fields) {
    return error(400, { errors: [{ message: 'noun, domain, and fields required' }] })
  }

  const registry = getRegistryDO(env, 'global') as any

  // Access control: engine evaluates "User can access Domain iff..." derivation rules

  // Extract array-of-objects fields (child entities) BEFORE creating the parent.
  // Children are created as separate Entity DOs to keep each DO = one entity.
  const childArrays: Array<{ fieldName: string; childNoun: string; items: Record<string, any>[] }> = []
  const parentFields: Record<string, any> = {}

  for (const [fieldName, fieldValue] of Object.entries(body.fields)) {
    if (Array.isArray(fieldValue) && fieldValue.length > 0 && typeof fieldValue[0] === 'object') {
      const singular = fieldName.replace(/s$/, '')
      const childNoun = singular.charAt(0).toUpperCase() + singular.slice(1)
      childArrays.push({ fieldName, childNoun, items: fieldValue as Record<string, any>[] })
    } else {
      parentFields[fieldName] = fieldValue
    }
  }

  // AREST: one command, one function application, one state transfer.
  const getStub = (id: string) => getEntityDO(env, id) as any
  let populationJson = JSON.stringify({ facts: {} })
  populationJson = await loadDomainAndPopulation(registry, getStub, body.domain)

  const arestResult = applyCommand({
    type: 'createEntity',
    noun: body.noun,
    domain: body.domain,
    id: null, // resolved below from reference scheme
    fields: parentFields,
  }, populationJson)

  if (arestResult.rejected) {
    return error(422, { errors: arestResult.violations.map((v: any) => ({ message: v.detail, constraintId: v.constraintId })) })
  }

  // Persist all entities from the AREST result (Resource + State Machine + Violations)
  for (const entity of arestResult.entities) {
    const eid = entity.id || crypto.randomUUID()
    const eDO = getEntityDO(env, eid) as any
    await eDO.put({ id: eid, type: entity.type, data: entity.data })
    await registry.indexEntity(entity.type, eid, body.domain)
  }

  // The primary entity ID — first entity in the result
  const primaryEntity = arestResult.entities[0]
  const parentId = primaryEntity?.id || crypto.randomUUID()

  // Create children as separate Entity DOs
  if (childArrays.length > 0) {
    const parentFkField = body.noun.charAt(0).toLowerCase() + body.noun.slice(1)
    for (const { childNoun, items } of childArrays) {
      for (const childFields of items) {
        const childId = crypto.randomUUID()
        const childDO = getEntityDO(env, childId) as any
        await childDO.put({ id: childId, type: childNoun, data: { ...childFields, [parentFkField]: parentId } })
        await registry.indexEntity(childNoun, childId, body.domain)
      }
    }
  }

  return json({
    id: parentId,
    noun: body.noun,
    domain: body.domain,
    status: arestResult.status,
    transitions: arestResult.transitions,
    ...(arestResult.derivedCount > 0 && { derived: arestResult.derivedCount }),
    ...(arestResult.violations.length > 0 && { deonticWarnings: arestResult.violations }),
  }, { status: 201 })
})

// ── Entity state machine transitions ─────────────────────────────────
// GET available transitions
router.get('/api/entities/:noun/:id/transitions', async (request, env: Env) => {
  const { noun, id } = request.params
  const entityDO = getEntityDO(env, id) as any
  const entity = await entityDO.get()
  if (!entity) return error(404, { errors: [{ message: 'Not Found' }] })

  if (!entity.data?._status) {
    return json({ transitions: [], message: 'Entity has no state machine' })
  }

  // Valid transitions from the engine's compiled state machine
  const registry = getRegistryDO(env, 'global') as any
  let transitions: Array<{ event: string; targetStatus: string }> = []
  try {
    const reqUrl = new URL(request.url)
    const domainSlug = reqUrl.searchParams.get('domain') || entity.data._domain as string || ''
    await loadDomainSchema(registry, (eid: string) => getEntityDO(env, eid) as any, domainSlug)
    transitions = getTransitions(entity.type, entity.data._status as string)
      .map(t => ({ event: t.event, targetStatus: t.to }))
  } catch { /* schema load failed */ }

  return json({
    currentStatus: entity.data._status,
    transitions,
  })
})

// POST fire a transition event — AREST command
router.post('/api/entities/:noun/:id/transition', async (request, env: Env) => {
  const { noun, id } = request.params
  const body = await request.json() as { event: string; domain?: string }
  if (!body.event) return error(400, { errors: [{ message: 'event required' }] })

  const entityDO = getEntityDO(env, id) as any
  const entity = await entityDO.get()
  if (!entity) return error(404, { errors: [{ message: 'Not Found' }] })

  const domainSlug = body.domain || entity.data?._domain as string || entity.data?.domain as string || ''
  const registry = getRegistryDO(env, 'global') as any
  const getStub = (eid: string) => getEntityDO(env, eid) as any

  // Find the State Machine instance for this entity
  const smIds: string[] = await registry.getEntityIds('State Machine', domainSlug)
  let currentStatus: string | undefined
  let smEntityId: string | undefined
  for (const smId of smIds) {
    try {
      const sm = await getStub(smId).get()
      if (sm?.data?.forResource === id) {
        currentStatus = sm.data.currentlyInStatus as string
        smEntityId = smId
        break
      }
    } catch { continue }
  }

  if (!currentStatus || !smEntityId) {
    return error(400, { errors: [{ message: 'Entity has no state machine' }] })
  }

  // AREST: apply transition command
  const populationJson = await loadDomainAndPopulation(registry, getStub, domainSlug)
  const cmd = {
    type: 'transition' as const,
    entityId: id,
    event: body.event,
    currentStatus,
    domain: domainSlug,
  }
  console.log('AREST transition cmd:', JSON.stringify(cmd))
  const arestResult = applyCommand(cmd, populationJson)
  console.log('AREST transition result:', JSON.stringify({ status: arestResult.status, entities: arestResult.entities?.length, error: (arestResult as any).error }))

  if (arestResult.rejected) {
    return error(422, { errors: arestResult.violations.map((v: any) => ({ message: v.detail })) })
  }

  if (!arestResult.status) {
    return error(400, { errors: [{
      message: `Invalid transition: event '${body.event}' not available from status '${currentStatus}'`,
    }], debug: { arestResult, cmd } })
  }

  // Update the State Machine entity's current status
  await getStub(smEntityId).patch({ currentlyInStatus: arestResult.status })

  // Persist any Event entities from the AREST result
  for (const e of arestResult.entities) {
    const eid = e.id || crypto.randomUUID()
    await getStub(eid).put({ id: eid, type: e.type, data: e.data })
    await registry.indexEntity(e.type, eid, domainSlug)
  }

  return json({
    id,
    noun,
    previousStatus: currentStatus,
    status: arestResult.status,
    event: body.event,
    transitions: arestResult.transitions,
  })
})

// ── Entity queries (fan-out via pure handlers) ───────────────────────
router.get('/api/entities/:noun', async (request, env: Env) => {
  const { noun } = request.params
  const url = new URL(request.url)
  const domainId = url.searchParams.get('domain')
  if (!domainId) return error(400, { errors: [{ message: 'domain query param required' }] })

  const limit = parseInt(url.searchParams.get('limit') || '100', 10)
  const page = parseInt(url.searchParams.get('page') || '1', 10)

  const registry = getRegistryDO(env, 'global') as any
  const result = await handleListEntities(
    noun,
    domainId,
    registry,
    (id) => getEntityDO(env, id) as any,
    { limit, page },
  )

  // Flatten entity data for backwards compat (spread data into top level)
  const docs = result.docs.map((e) => ({
    id: e.id,
    type: e.type,
    ...e.data,
    version: e.version,
    createdAt: e.createdAt,
    updatedAt: e.updatedAt,
    _links: buildEntityLinks(e.type, e.id, domainId || undefined),
  }))

  // Client-side filtering by where params
  const where: Record<string, any> = {}
  for (const [key, val] of url.searchParams.entries()) {
    const m = key.match(/^where\[(\w+)\](?:\[(\w+)\])?$/)
    if (m) {
      const field = m[1]
      const op = m[2] || 'equals'
      where[field] = { [op]: val }
    }
  }
  let filtered = docs
  for (const [field, condition] of Object.entries(where)) {
    if (typeof condition === 'object' && condition !== null && 'equals' in condition) {
      filtered = filtered.filter(doc => (doc as any)[field] === condition.equals)
    }
  }

  return json({
    docs: filtered,
    totalDocs: result.totalDocs,
    limit: result.limit,
    page: result.page,
    totalPages: result.totalPages,
    hasNextPage: result.hasNextPage,
    hasPrevPage: result.hasPrevPage,
    ...(result.warnings && { warnings: result.warnings }),
    ...(result._links && { _links: result._links }),
  })
})

router.get('/api/entities/:noun/:id', async (request, env: Env) => {
  const { id } = request.params
  const url = new URL(request.url)
  const domainSlug = url.searchParams.get('domain') || undefined

  // Load domain schema and resolve transitions from engine's compiled state machine
  let transitions: Array<{ transitionId: string; event: string; targetStatus: string; targetStatusId: string }> = []
  try {
    const registry = getRegistryDO(env, 'global') as any
    await loadDomainSchema(registry, (eid: string) => getEntityDO(env, eid) as any, domainSlug || '')
    const entityStub = getEntityDO(env, id) as any
    const raw = await entityStub.get()
    if (raw?.data?._status) {
      transitions = getTransitions(raw.type, raw.data._status as string)
        .map(t => ({ transitionId: '', event: t.event, targetStatus: t.to, targetStatusId: '' }))
    }
  } catch { /* best-effort */ }

  const entity = await handleGetEntity(getEntityDO(env, id) as any, {
    domain: domainSlug,
    transitions,
  })
  if (!entity) return error(404, { errors: [{ message: 'Not Found' }] })

  return json({
    id: entity.id,
    type: entity.type,
    ...entity.data,
    version: entity.version,
    createdAt: entity.createdAt,
    updatedAt: entity.updatedAt,
    ...(entity._links && { _links: entity._links }),
  })
})

router.patch('/api/entities/:noun/:id', async (request, env: Env) => {
  const { id } = request.params
  const body = await request.json() as Record<string, any>

  const entityDO = getEntityDO(env, id) as any
  const result = await entityDO.patch(body)

  if (!result) return error(404, { errors: [{ message: 'Not Found' }] })
  return json(result)
})

router.delete('/api/entities/:noun/:id', async (request, env: Env) => {
  const { noun, id } = request.params

  const registry = getRegistryDO(env, 'global') as any
  const result = await handleDeleteEntity(id, getEntityDO(env, id) as any, registry, noun)

  if (!result) return error(404, { errors: [{ message: 'Not Found' }] })
  return json(result)
})

// ── Collection CRUD ──────────────────────────────────────────────────

// ── Fact Query: partial application of graph schemas ────────────────
// GET /api/facts/:schema?domain=X&bind[NounA]=value&target=NounB
// A query IS a partially applied Construction. Bind some roles, get the free roles.
// The engine loads the domain schema, federates to populate facts, forward-chains
// derivation rules, then applies Filter(predicate) to return matching values.

router.get('/api/facts/:schema', async (request, env: Env) => {
  const { schema } = request.params
  const url = new URL(request.url)
  const domainSlug = url.searchParams.get('domain')
  const targetRole = url.searchParams.get('target')

  if (!domainSlug) return error(400, { errors: [{ message: 'domain query param required' }] })
  if (!targetRole) return error(400, { errors: [{ message: 'target query param required (the role to extract)' }] })

  // Collect bind[NounName]=value pairs from query string
  const bindings: Array<[string, string]> = []
  for (const [key, value] of url.searchParams.entries()) {
    const match = key.match(/^bind\[(.+)\]$/)
    if (match) {
      bindings.push([match[1], value])
    }
  }

  try {
    const registry = getRegistryDO(env, 'global') as any
    const getStub = (eid: string) => getEntityDO(env, eid) as any

    // Load domain schema + build federated population
    const populationJson = await loadDomainAndPopulation(registry, getStub, domainSlug)

    // Forward-chain derivation rules (including joins)
    const derived = forwardChain(populationJson)

    // Merge derived facts into population
    const population = JSON.parse(populationJson)
    for (const fact of derived) {
      if (!population.facts[fact.factTypeId]) population.facts[fact.factTypeId] = []
      population.facts[fact.factTypeId].push({
        factTypeId: fact.factTypeId,
        bindings: fact.bindings,
      })
    }
    const enrichedPopJson = JSON.stringify(population)

    // Resolve the schema ID — try exact match, then reading-format match
    // The schema param can be a graph schema ID or a reading like "Vehicle is resolved to Chrome Style Candidate"
    const schemaId = decodeURIComponent(schema)

    // Resolve role names to 1-indexed role positions using the compiled schema.
    // The schema's role_names array maps position → noun name.
    // We need to find the schema in the compiled model to get its role_names.
    // Try synthesize_noun for the first bound noun to discover the schema's roles.
    let roleNames: string[] = []
    try {
      // Use the schema ID to look up role names from the compiled model
      // getNounSchemas returns schemas where a noun plays role 0
      // We need a more direct lookup — use the population's fact type to infer roles
      const popFacts = population.facts[schemaId]
      if (popFacts && popFacts.length > 0) {
        roleNames = popFacts[0].bindings.map((b: [string, string]) => b[0])
      }
    } catch { /* fall through to index-based */ }

    const filterBindings: Array<[number, string]> = []
    for (const [noun, value] of bindings) {
      const idx = parseInt(noun)
      if (!isNaN(idx)) {
        filterBindings.push([idx, value])
      } else {
        // Map noun name to 1-indexed role position
        const roleIdx = roleNames.indexOf(noun)
        if (roleIdx >= 0) {
          filterBindings.push([roleIdx + 1, value])
        }
      }
    }

    // Resolve target role: name or 1-indexed number
    const targetIdx = parseInt(targetRole)
    let targetRoleIndex: number
    if (!isNaN(targetIdx)) {
      targetRoleIndex = targetIdx
    } else {
      const idx = roleNames.indexOf(targetRole)
      targetRoleIndex = idx >= 0 ? idx + 1 : 2
    }

    const result = querySchema(schemaId, targetRoleIndex, filterBindings, enrichedPopJson)

    return json({
      schema: schemaId,
      target: targetRole,
      bindings: Object.fromEntries(bindings),
      matches: result.matches,
      count: result.count,
      derived: derived.length,
      _links: {
        self: { href: url.pathname + url.search },
        domain: { href: `/api/entities/Domain?domain=${domainSlug}` },
      },
    })
  } catch (e: any) {
    return error(500, { errors: [{ message: e.message || 'Query failed' }] })
  }
})

/** GET /api/:collection — list/find */
// ── Conceptual Query (before generic :collection routes) ─────────────
router.get('/api/query', handleConceptualQuery)
router.post('/api/query', handleConceptualQuery)

// ── Named API routes (before generic :collection catch-all) ──────────
router.post('/api/claims', async (request, env: Env, ctx: ExecutionContext) => {
  const { handleClaims } = await import('./claims')
  return handleClaims(request, env, ctx)
})
router.all('/api/seed', handleSeed)
router.get('/api/stats', async (request, env: Env) => {
  const { handleStats } = await import('./claims')
  return handleStats(request, env)
})
router.all('/api/parse', handleParse)
router.all('/api/parse/orm', handleParseOrm)
router.all('/api/verify', handleVerify)

router.get('/api/:collection', async (request, env: Env) => {
  const { collection } = request.params

  // Generators collection stays in DomainDB (SQL table, not entity-per-DO)
  if (collection === 'generators') {
    const url = new URL(request.url)
    const { where, limit, page, sort } = parseQueryOptions(url.searchParams)
    const db = getPrimaryDB(env) as any
    const result = await db.findInCollection(collection, where, { limit, page, sort })
    return json({
      docs: result.docs,
      totalDocs: result.totalDocs,
      limit: result.limit,
      page: result.page,
      totalPages: Math.ceil(result.totalDocs / result.limit),
      hasNextPage: result.hasNextPage,
      hasPrevPage: result.page > 1,
      pagingCounter: (result.page - 1) * result.limit + 1,
    })
  }

  // Resolve collection slug to noun type dynamically from Registry
  const registry = getRegistryDO(env, 'global') as any
  const entityType = await resolveSlugToNoun(registry, collection)
  if (!entityType) {
    return error(404, { errors: [{ message: `Collection "${collection}" not found` }] })
  }

  const url = new URL(request.url)
  const { where, limit: qLimit, page: qPage } = parseQueryOptions(url.searchParams)
  const domainId = where?.domain?.equals || where?.['domain.domainSlug']?.equals || url.searchParams.get('domain') || ''
  const result = await handleListEntities(
    entityType,
    domainId,
    registry,
    (id) => getEntityDO(env, id) as any,
    { limit: qLimit, page: qPage },
  )

  // Access control: engine evaluates "User can access Domain iff..." derivation rules

  // Flatten entity data for backwards compat (spread data into top level)
  const docs = result.docs.map((e) => ({
    id: e.id,
    type: e.type,
    ...e.data,
    version: e.version,
    createdAt: e.createdAt,
    updatedAt: e.updatedAt,
    _links: buildEntityLinks(e.type, e.id, domainId || undefined),
  }))

  // Client-side filtering by where params
  let filtered = docs
  if (where) {
    for (const [field, condition] of Object.entries(where)) {
      if (field === 'domain' || field === 'domain.domainSlug') continue // already used for routing
      if (typeof condition === 'object' && condition !== null && 'equals' in condition) {
        filtered = filtered.filter(doc => (doc as any)[field] === (condition as any).equals)
      }
    }
  }

  return json({
    docs: filtered,
    totalDocs: result.totalDocs,
    limit: result.limit,
    page: result.page,
    totalPages: result.totalPages,
    hasNextPage: result.hasNextPage,
    hasPrevPage: result.hasPrevPage,
    pagingCounter: (result.page - 1) * result.limit + 1,
    ...(result.warnings && { warnings: result.warnings }),
    ...(result._links && { _links: result._links }),
  })
})

/** GET /api/:collection/:id — get by ID */
router.get('/api/:collection/:id', async (request, env: Env) => {
  const { collection, id } = request.params

  if (collection === 'generators') {
    const db = getPrimaryDB(env) as any
    const doc = await db.getFromCollection(collection, id)
    if (!doc) return error(404, { errors: [{ message: 'Not Found' }] })
    return json(doc)
  }

  const registry = getRegistryDO(env, 'global') as any
  const entityType = await resolveSlugToNoun(registry, collection)
  if (!entityType) {
    return error(404, { errors: [{ message: `Collection "${collection}" not found` }] })
  }

  const url = new URL(request.url)
  const domainSlug = url.searchParams.get('domain') || undefined

  // Resolve transitions from engine
  let collTransitions: Array<{ transitionId: string; event: string; targetStatus: string; targetStatusId: string }> = []
  try {
    await loadDomainSchema(registry, (eid: string) => getEntityDO(env, eid) as any, domainSlug || '')
    const raw = await (getEntityDO(env, id) as any).get()
    if (raw?.data?._status) {
      collTransitions = getTransitions(raw.type, raw.data._status as string)
        .map((t: any) => ({ transitionId: '', event: t.event, targetStatus: t.to, targetStatusId: '' }))
    }
  } catch { /* best-effort */ }

  const entity = await handleGetEntity(getEntityDO(env, id) as any, {
    domain: domainSlug,
    transitions: collTransitions,
  })
  if (!entity) return error(404, { errors: [{ message: 'Not Found' }] })
  return json({
    id: entity.id,
    type: entity.type,
    ...entity.data,
    version: entity.version,
    createdAt: entity.createdAt,
    updatedAt: entity.updatedAt,
    ...(entity._links && { _links: entity._links }),
  })
})

/** POST /api/:collection — create */
router.post('/api/:collection', async (request, env: Env) => {
  const { collection } = request.params
  const body = await request.json() as Record<string, any>

  if (collection === 'generators') {
    const db = getPrimaryDB(env) as any
    const doc = await db.createInCollection(collection, body)
    return json({ doc, message: 'Created successfully' }, { status: 201 })
  }

  const registry = getRegistryDO(env, 'global') as any
  const entityType = await resolveSlugToNoun(registry, collection)
  if (!entityType) {
    return error(404, { errors: [{ message: `Collection "${collection}" not found` }] })
  }

  const domain = body.domain || ''
  const result = await handleCreateEntity(
    entityType,
    domain,
    body,
    (id) => getEntityDO(env, id) as any,
    registry,
  )
  return json({ doc: { id: result.id, ...body }, message: 'Created successfully', ...(result._links && { _links: result._links }) }, { status: 201 })
})

/** PATCH /api/:collection/:id — update */
router.patch('/api/:collection/:id', async (request, env: Env) => {
  const { collection, id } = request.params
  const body = await request.json() as Record<string, any>

  if (collection === 'generators') {
    const db = getPrimaryDB(env) as any
    const doc = await db.updateInCollection(collection, id, body)
    if (!doc) return error(404, { errors: [{ message: 'Not Found' }] })
    return json({ doc, message: 'Updated successfully' })
  }

  const registry = getRegistryDO(env, 'global') as any
  const entityType = await resolveSlugToNoun(registry, collection)
  if (!entityType) {
    return error(404, { errors: [{ message: `Collection "${collection}" not found` }] })
  }

  const entityDO = getEntityDO(env, id) as any
  const result = await entityDO.patch(body)
  if (!result) return error(404, { errors: [{ message: 'Not Found' }] })
  return json({ doc: result, message: 'Updated successfully' })
})

/** DELETE /api/:collection/:id — delete */
router.delete('/api/:collection/:id', async (request, env: Env) => {
  const { collection, id } = request.params

  if (collection === 'generators') {
    const db = getPrimaryDB(env) as any
    const result = await db.deleteFromCollection(collection, id)
    if (!result.deleted) return error(404, { errors: [{ message: 'Not Found' }] })
    return json({ id, message: 'Deleted successfully' })
  }

  const registry = getRegistryDO(env, 'global') as any
  const entityType = await resolveSlugToNoun(registry, collection)
  if (!entityType) {
    return error(404, { errors: [{ message: `Collection "${collection}" not found` }] })
  }

  const result = await handleDeleteEntity(id, getEntityDO(env, id) as any, registry, entityType)
  if (!result) return error(404, { errors: [{ message: 'Not Found' }] })
  return json({ id, message: 'Deleted successfully' })
})


// ── Domain wipe (metamodel only — nouns, readings, constraints, roles, etc.) ──
// Deindex from Registry — EntityDB DOs become orphaned. Background cleanup.
router.delete('/api/domains/:domainId/metamodel', async (request, env: Env, ctx: ExecutionContext) => {
  const { domainId } = request.params
  const registry = getRegistryDO(env, 'global') as any

  // Count before wiping
  const entities = await registry.getAllEntityIdsForDomain(domainId) as Array<{ nounType: string; entityId: string }>

  // Deindex — entities become orphaned, instant
  const deindexedEntities = await registry.deindexEntitiesForDomain(domainId) as number
  const deindexedNouns = await registry.deindexNounsForDomain(domainId) as number

  // Clear generators cache
  try {
    const db = getPrimaryDB(env) as any
    await db.wipeDomainMetamodel(domainId)
  } catch { /* best effort */ }

  // Background: soft-delete orphaned DOs
  if (entities.length > 0) {
    const BATCH = 50
    ctx.waitUntil((async () => {
      for (let i = 0; i < entities.length; i += BATCH) {
        const chunk = entities.slice(i, i + BATCH)
        await Promise.allSettled(
          chunk.map(({ entityId }) => (getEntityDO(env, entityId) as any).delete()),
        )
      }
    })())
  }

  return json({
    deleted: true,
    domainId,
    counts: {
      entitiesOrphaned: entities.length,
      entitiesDeindexed: deindexedEntities,
      nounsDeindexed: deindexedNouns,
      cleanupScheduled: entities.length > 0,
    },
  })
})

// ── Full reset (delete ALL data from ALL tables) ────────────────────
// Wipe the Registry index — EntityDB DOs become orphaned (unreachable).
// Background process cleans up orphaned DOs after response is sent.
router.delete('/api/reset', async (request, env: Env, ctx: ExecutionContext) => {
  const registry = getRegistryDO(env, 'global') as any

  // Collect entity IDs before wiping
  const entities = await registry.getAllEntityIds() as Array<{ nounType: string; entityId: string }>
  const entityCount = entities.length

  // Wipe all Registry tables — instant
  await registry.wipeAll()

  // Wipe DomainDB SQL tables (generators, etc.)
  try {
    const db = getPrimaryDB(env) as any
    await db.wipeAllData()
  } catch { /* best effort */ }

  // Background: soft-delete orphaned DOs (fire and forget)
  if (entities.length > 0) {
    const BATCH = 50
    ctx.waitUntil((async () => {
      for (let i = 0; i < entities.length; i += BATCH) {
        const chunk = entities.slice(i, i + BATCH)
        await Promise.allSettled(
          chunk.map(({ entityId }) => (getEntityDO(env, entityId) as any).delete()),
        )
      }
    })())
  }

  return json({
    reset: true,
    counts: {
      entitiesOrphaned: entityCount,
      cleanupScheduled: entityCount > 0,
    },
  })
})

// ── 404 fallback ─────────────────────────────────────────────────────
router.all('*', () => error(404, { errors: [{ message: 'Not Found' }] }))

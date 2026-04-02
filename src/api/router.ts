import { AutoRouter, json, error } from 'itty-router'
import type { Env } from '../types'
import { parseQueryOptions } from './collections'
import { nounToSlug, nounToTable, resolveSlugToNoun } from '../collections'
import { handleSeed } from './seed'
import { handleEvaluate, handleSynthesize } from './evaluate'
import { handleListEntities, handleGetEntity, handleCreateEntity, handleDeleteEntity, buildEntityLinks } from './entity-routes'
import { loadDomainSchema, loadDomainAndPopulation, buildPopulation, getTransitions, applyCommand, querySchema, forwardChain, getNounSchemas, deriveViewMetadata, deriveNavContext, getTopLevelNouns, computeRMAP, getCachedIR } from './engine'
import { system } from './system'
import { handleArestRequest } from './arest-router'

// ── Collection slug → noun type resolution ───────────────────────────
// Resolved dynamically from the Registry via nounToSlug convention.
// Noun entities are seeded from readings — no hardcoded maps.

// ── DO helpers ───────────────────────────────────────────────────────

/** Get an EntityDB DO stub for the given entity ID. */
function getEntityDO(env: Env, entityId: string): DurableObjectStub {
  const id = env.ENTITY_DB.idFromName(entityId)
  return env.ENTITY_DB.get(id)
}

/** Decode URL-encoded entity ID from route params. Entity IDs contain colons (domain:name). */
function decodeId(params: Record<string, string>): string {
  return decodeURIComponent(params.id || '')
}

/** Get a RegistryDB DO stub for the given scope (e.g. app/org/global). */
function getRegistryDO(env: Env, scope: string): DurableObjectStub {
  const id = env.REGISTRY_DB.idFromName(scope)
  return env.REGISTRY_DB.get(id)
}


export const router = AutoRouter()

// ── Health ───────────────────────────────────────────────────────────
router.get('/health', () => json({ status: 'ok', version: '0.1.0' }))

// ── Domain Connection: store secrets for External System access ──────
// Per core.md: Domain connects to External System with Secret Reference.
// Secrets are stored in the Domain entity's DO via connectSystem().
router.post('/api/connect/:domain/:system', async (request, env: Env) => {
  const { domain, system } = request.params
  const body = await request.json() as any
  const secret = body?.secret
  if (!secret) return error(400, { errors: [{ message: 'secret required' }] })

  const domainDO = getEntityDO(env, `domain-secrets:${domain}`) as any
  await domainDO.connectSystem(system, secret)
  return json({ connected: true, domain, system })
})

router.get('/api/connect/:domain', async (request, env: Env) => {
  const { domain } = request.params
  const domainDO = getEntityDO(env, `domain-secrets:${domain}`) as any
  const systems = await domainDO.connectedSystems()
  return json({ domain, connectedSystems: systems })
})

// REMOVED: /api/access is replaced by GET /arest/ (root resource with org membership links).
// See handleArestRequest in arest-router.ts.

// ── Derivation trace for a domain ────────────────────────────────────
// Per the paper: derivation chains are recorded and available on demand.
// Returns the full forward-chained derivation trace for a domain's population.
router.get('/api/trace/:domain', async (request, env: Env) => {
  const domain = decodeURIComponent(request.params.domain)
  const registry = getRegistryDO(env, 'global') as any
  const getStub = (id: string) => getEntityDO(env, id) as any

  await loadDomainSchema(registry, getStub, domain).catch(() => {})
  const popJson = await buildPopulation(registry, getStub, domain)
  let derived: Array<{ factTypeId: string; reading: string; bindings: Array<[string, string]>; derivedBy: string }> = []
  try { derived = forwardChain(popJson) } catch {}

  return json({
    domain,
    derivedFacts: derived.length,
    trace: derived.map(fact => ({
      rule: fact.derivedBy,
      reading: fact.reading,
      bindings: fact.bindings,
    })),
  })
})

// ── Debug: population fact type IDs ──────────────────────────────────
router.get('/api/debug/population/:domain', async (request, env: Env) => {
  const domain = decodeURIComponent(request.params.domain)
  const registry = getRegistryDO(env, 'global') as any
  const getStub = (id: string) => getEntityDO(env, id) as any
  await loadDomainSchema(registry, getStub, domain).catch(() => {})
  const popJson = await buildPopulation(registry, getStub, domain)
  const pop = JSON.parse(popJson) as { facts: Record<string, any[]> }
  const summary = Object.entries(pop.facts).map(([ftId, facts]) => ({ ftId, count: facts.length }))
  return json({ domain, factTypes: summary })
})

// ── Debug: entity counts by type from Registry ──────────────────────
router.get('/api/debug/counts/:domain', async (request, env: Env) => {
  const { domain } = request.params
  const registry = getRegistryDO(env, 'global') as any
  const counts = await registry.getEntityCounts(domain)
  return json({ domain, counts })
})

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
  const domain = decodeURIComponent(request.params.domain)
  const registry = getRegistryDO(env, 'global') as any
  await loadDomainSchema(registry, (id: string) => getEntityDO(env, id) as any, domain)
  const { debug_compiled_state } = await import('../../crates/fol-engine/pkg/fol_engine.js')
  const schema = JSON.parse(debug_compiled_state())
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

// RMAP: show cell partitioning derived from UC structure (Halpin, Ch. 17).
router.get('/api/debug/rmap/:domain', async (request, env: Env) => {
  const domain = decodeURIComponent(request.params.domain)
  const registry = getRegistryDO(env, 'global') as any
  const getStub = (id: string) => getEntityDO(env, id) as any
  await loadDomainSchema(registry, getStub, domain).catch(() => {})
  const tables = computeRMAP()
  return json({ domain, tables })
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

// ── Evaluate / Synthesize (WASM engine) ─────────────────────────────
router.post('/api/evaluate', handleEvaluate)
router.post('/api/synthesize', (request, env) => handleSynthesize(request, env))

// ── Induction (discover constraints from population) — uses WASM engine
router.post('/api/induce', async (request) => {
  const body = await request.json() as { ir?: any; population?: any }
  if (!body.ir || !body.population) {
    return error(400, { errors: [{ message: 'ir and population are required' }] })
  }
  const { induce_from_population } = await import('../../crates/fol-engine/pkg/fol_engine.js')
  const result = induce_from_population(JSON.stringify(body.population))
  return json(result)
})

// Entity creation goes through POST /api/entities/:noun (the AREST command path).

router.post('/api/_placeholder', async (_request: Request, _env: Env) => {
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
    if (r.status === 'fulfilled' && r.value) {
      const n = { id: r.value.id, ...r.value.data }
      if (n.name) nounByName.set(n.name, n)
    }
  }

  const schemaByReading = new Map<string, string>()
  for (const r of readingSettled) {
    if (r.status === 'fulfilled' && r.value) {
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
  const noun = decodeURIComponent(request.params.noun); const id = decodeURIComponent(request.params.id)
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
  const noun = decodeURIComponent(request.params.noun); const id = decodeURIComponent(request.params.id)
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

  // Update the State Machine entity's current status via cell store (↓n)
  const smCell = await getStub(smEntityId).get()
  if (smCell) {
    await getStub(smEntityId).put({ id: smCell.id, type: smCell.type, data: { ...smCell.data, currentlyInStatus: arestResult.status } })
  }

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
// POST /api/entities/:noun — create entity via direct DO write
router.post('/api/entities/:noun', async (request, env: Env) => {
  const noun = decodeURIComponent(request.params.noun)
  const body = await request.json() as { domain?: string; data?: Record<string, unknown>; id?: string }
  const domain = body.domain || new URL(request.url).searchParams.get('domain') || ''
  if (!domain) return error(400, { errors: [{ message: 'domain required in body or query param' }] })

  const entityId = body.id || crypto.randomUUID()
  const entityDO = getEntityDO(env, entityId) as any
  await entityDO.put({ id: entityId, type: noun, data: body.data || {} })

  const registry = getRegistryDO(env, 'global') as any
  await registry.indexEntity(noun, entityId, domain)

  const cell = await entityDO.get()
  return json({
    id: cell.id,
    type: cell.type,
    ...cell.data,
    _links: buildEntityLinks(noun, entityId, domain),
  }, { status: 201 })
})

router.get('/api/entities/:noun', async (request, env: Env) => {
  const noun = decodeURIComponent(request.params.noun)
  const url = new URL(request.url)
  const domainId = url.searchParams.get('domain')
  if (!domainId) return error(400, { errors: [{ message: 'domain query param required' }] })

  const userEmail = request.headers.get('x-user-email') || ''

  const params: Record<string, string> = {}
  url.searchParams.forEach((v, k) => { params[k] = v })

  const registry = getRegistryDO(env, 'global') as any
  const getStub = (id: string) => getEntityDO(env, id) as any

  // SYSTEM:x = (ρ (↑entity(x) : D)) : ↑op(x)
  const result = await system(noun, domainId || '', 'read', params, { registry, getStub })

  // Enrich with view metadata (derived from ρ, not procedural)
  const viewMeta = await deriveViewMetadata(registry, getStub, noun, domainId).catch(() => null)
  const navCtx = userEmail ? await deriveNavContext(registry, getStub, userEmail, domainId, noun).catch(() => null) : null

  // When listing Nouns, enrich each doc with _topLevel derived from cached IR
  let topLevelInfo: { topLevel: Set<string> } | null = null
  if (noun === 'Noun' && domainId) {
    const irJson = await getCachedIR(getStub, registry, domainId)
    if (irJson) {
      topLevelInfo = getTopLevelNouns(irJson)
    }
  }

  const body = result.body as any
  const docs = (body.docs || []).map((e: any) => ({
    ...e,
    ...(topLevelInfo ? { _topLevel: topLevelInfo.topLevel.has(e.name || e.id) } : {}),
    _links: buildEntityLinks(e.type || noun, e.id, domainId || undefined),
  }))

  // Where-param filtering
  const where: Record<string, any> = {}
  url.searchParams.forEach((val, key) => {
    const m = key.match(/^where\[(\w+)\](?:\[(\w+)\])?$/)
    if (m) {
      where[m[1]] = { [m[2] || 'equals']: val }
    }
  })
  const filtered = Object.entries(where).reduce(
    (acc, [field, condition]) =>
      (typeof condition === 'object' && condition !== null && 'equals' in condition)
        ? acc.filter((doc: any) => doc[field] === condition.equals)
        : acc,
    docs,
  )

  return json({
    docs: filtered,
    totalDocs: body.totalDocs,
    limit: body.limit,
    page: body.page,
    totalPages: body.totalPages,
    hasNextPage: body.hasNextPage,
    hasPrevPage: body.hasPrevPage,
    _view: viewMeta,
    ...(navCtx && { _nav: navCtx }),
  })
})

router.get('/api/entities/:noun/:id', async (request, env: Env) => {
  const noun = decodeURIComponent(request.params.noun)
  const id = decodeURIComponent(request.params.id)
  const url = new URL(request.url)
  const domainSlug = url.searchParams.get('domain') || ''

  const registry = getRegistryDO(env, 'global') as any
  const getStub = (eid: string) => getEntityDO(env, eid) as any

  // SYSTEM:x = (ρ (↑entity(x) : D)) : ↑op(x)
  const result = await system(noun, domainSlug, 'readDetail', { _id: id, domain: domainSlug }, { registry, getStub })

  const entity = result.body as any
  const viewMeta = domainSlug
    ? await deriveViewMetadata(registry, getStub, noun, domainSlug).catch(() => null)
    : null

  return json({
    ...entity,
    ...(viewMeta ? { _view: { ...viewMeta, type: 'DetailView' } } : {}),
    _links: {
      self: { href: `/api/entities/${encodeURIComponent(noun)}/${id}` },
      collection: { href: `/api/entities/${encodeURIComponent(noun)}?domain=${domainSlug}` },
    },
  }, { status: result.status })
})

// PATCH on entities: ↓n with merged data (cell store replaces contents)
router.patch('/api/entities/:noun/:id', async (request, env: Env) => {
  const { id } = request.params
  const body = await request.json() as Record<string, any>

  const entityDO = getEntityDO(env, id) as any
  const existing = await entityDO.get()
  if (!existing) return error(404, { errors: [{ message: 'Not Found' }] })

  const merged = { ...existing.data, ...body }
  const result = await entityDO.put({ id: existing.id, type: existing.type, data: merged })
  return json({ id: result.id, type: result.type, ...result.data })
})

router.delete('/api/entities/:noun/:id', async (request, env: Env) => {
  const noun = decodeURIComponent(request.params.noun); const id = decodeURIComponent(request.params.id)

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
// Conceptual query — uses WASM query_schema_wasm (θ₁ operations on P)

// ── Conceptual Query — θ₁ operations on P via WASM ─────────────────
// GET/POST /api/query?domain=X&q=<natural language query>
// Uses query_schema_wasm to evaluate partial application of graph schemas.
router.all('/api/query', async (request, env: Env) => {
  const url = new URL(request.url)
  let domain: string | undefined
  let query: string | undefined

  if (request.method === 'POST') {
    const body = await request.json() as any
    domain = body.domain || body.domainId
    query = body.query || body.q
  } else {
    domain = url.searchParams.get('domain') || undefined
    query = url.searchParams.get('q') || url.searchParams.get('query') || undefined
  }

  if (!domain || !query) {
    return error(400, { errors: [{ message: 'domain and q (query) are required' }] })
  }

  const registry = getRegistryDO(env, 'global') as any
  const getStub = (id: string) => getEntityDO(env, id) as any

  // Load schema + build population, then query via WASM
  await loadDomainSchema(registry, getStub, domain)
  const populationJson = await loadDomainAndPopulation(registry, getStub, domain)

  // Use WASM prove_goal for the query
  const { prove_goal } = await import('../../crates/fol-engine/pkg/fol_engine.js')
  const result = prove_goal(query, JSON.parse(populationJson), 'closed')

  return json(result)
})

// Seed is the ONLY ingestion path — readings → WASM parse → cells in D
router.all('/api/seed', handleSeed)

router.get('/api/:collection', async (request, env: Env) => {
  const collection = decodeURIComponent(request.params.collection)

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

  // Flatten cell data into top level
  const docs = result.docs.map((e) => ({
    id: e.id,
    type: e.type,
    ...e.data,
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
  const collection = decodeURIComponent(request.params.collection); const id = decodeURIComponent(request.params.id)

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
    ...(entity._links && { _links: entity._links }),
  })
})

/** POST /api/:collection — create */
router.post('/api/:collection', async (request, env: Env) => {
  const collection = decodeURIComponent(request.params.collection)
  const body = await request.json() as Record<string, any>

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
  const collection = decodeURIComponent(request.params.collection); const id = decodeURIComponent(request.params.id)
  const body = await request.json() as Record<string, any>

  const registry = getRegistryDO(env, 'global') as any
  const entityType = await resolveSlugToNoun(registry, collection)
  if (!entityType) {
    return error(404, { errors: [{ message: `Collection "${collection}" not found` }] })
  }

  const entityDO = getEntityDO(env, id) as any
  const existing = await entityDO.get()
  if (!existing) return error(404, { errors: [{ message: 'Not Found' }] })
  const merged = { ...existing.data, ...body }
  const result = await entityDO.put({ id: existing.id, type: existing.type, data: merged })
  return json({ doc: { id: result.id, type: result.type, ...result.data }, message: 'Updated successfully' })
})

/** DELETE /api/:collection/:id — delete */
router.delete('/api/:collection/:id', async (request, env: Env) => {
  const collection = decodeURIComponent(request.params.collection); const id = decodeURIComponent(request.params.id)

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

// ── HATEOAS API: /arest/ routes ─────────────────────────────────────
// All /arest/ paths resolve against the constraint graph derived from IR.
// This is additive — the old /api/ routes remain untouched.

async function handleArestRoute(request: Request, env: Env) {
  const url = new URL(request.url)
  const userEmail = request.headers.get('x-user-email') || ''

  // Load domain IR for constraint graph (default: organizations)
  const registryDO = getRegistryDO(env, 'global') as any
  const irDomain = url.searchParams.get('ir_domain') || 'organizations'
  const getStubLocal = (id: string) => getEntityDO(env, id) as any
  try { await loadDomainSchema(registryDO, getStubLocal, irDomain) } catch {}
  const irJson = await getCachedIR(getStubLocal, registryDO, irDomain)
  const ir = irJson ? JSON.parse(irJson) : null

  if (!ir) return json({ error: 'No schema loaded' }, { status: 500 })

  // Debug: return IR shape if ?debug=ir
  if (url.searchParams.get('debug') === 'ir') {
    return json({
      factTypeKeys: Object.keys(ir.factTypes || ir.fact_types || {}),
      constraintCount: (ir.constraints || []).length,
      nounKeys: Object.keys(ir.nouns || {}),
      nounDetails: Object.fromEntries(
        Object.entries(ir.nouns || {}).map(([k, v]: [string, any]) => [k, { objectType: v.objectType, superType: v.superType }])
      ),
      instanceFacts: (ir.generalInstanceFacts || []).length,
    })
  }

  // Build population for root resource (needed to resolve org memberships)
  let population = { facts: {} as Record<string, any[]> }
  if (url.pathname === '/arest/' || url.pathname === '/arest') {
    try {
      const popJson = await buildPopulation(registryDO, (id: string) => getEntityDO(env, id) as any, 'organizations')
      try { forwardChain(popJson) } catch {}
      population = JSON.parse(popJson)
    } catch {}
  }

  const result = await handleArestRequest({
    path: url.pathname,
    method: request.method,
    ir,
    registry: registryDO,
    getStub: (id: string) => getEntityDO(env, id) as any,
    userEmail,
    population,
  })

  if (!result) return error(404, { errors: [{ message: 'Not Found' }] })
  return json(result)
}

router.get('/arest', async (request, env: Env) => {
  return handleArestRoute(request, env)
})

router.get('/arest/*', async (request, env: Env) => {
  return handleArestRoute(request, env)
})

// ── 404 fallback ─────────────────────────────────────────────────────
router.all('*', () => error(404, { errors: [{ message: 'Not Found' }] }))

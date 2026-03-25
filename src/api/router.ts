import { AutoRouter, json, error } from 'itty-router'
import type { Env } from '../types'
import { parseQueryOptions } from './collections'
import { COLLECTION_TABLE_MAP, NOUN_TABLE_MAP } from '../collections'
import { handleSeed } from './seed'
import { handleGenerate } from './generate'
import { handleParse } from './parse'
import { handleParseOrm } from './parse-orm'
import { handleVerify } from './verify'
import { handleEvaluate, handleSynthesize } from './evaluate'
import { getInitialState, getValidTransitions, applyTransition } from '../worker/state-machine'
import { handleConceptualQuery } from './conceptual-query'
import { deriveOnWrite } from '../worker/derive-on-write'
import { induceConstraints } from '../csdp/induce'
import { handleListEntities, handleGetEntity, handleCreateEntity, handleDeleteEntity } from './entity-routes'

// ── Reverse mapping: collection slug → entity type name ──────────────
// Built by inverting NOUN_TABLE_MAP (type→table) and joining with
// COLLECTION_TABLE_MAP (slug→table). Only metamodel collections that
// appear in NOUN_TABLE_MAP get an entry — unknown slugs return 404.
const TABLE_TO_ENTITY_TYPE: Record<string, string> = {}
for (const [typeName, tableName] of Object.entries(NOUN_TABLE_MAP)) {
  TABLE_TO_ENTITY_TYPE[tableName] = typeName
}
const COLLECTION_TO_ENTITY_TYPE: Record<string, string> = {}
for (const [slug, tableName] of Object.entries(COLLECTION_TABLE_MAP)) {
  if (TABLE_TO_ENTITY_TYPE[tableName]) {
    COLLECTION_TO_ENTITY_TYPE[slug] = TABLE_TO_ENTITY_TYPE[tableName]
  }
}

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
 * Replaces the old getLegacyDB that referenced GraphDLDB.
 * Uses a single "primary" instance for all collection/metamodel operations.
 */
function getPrimaryDB(env: Env): DurableObjectStub {
  const id = env.DOMAIN_DB.idFromName('graphdl-primary')
  return env.DOMAIN_DB.get(id)
}

export const router = AutoRouter()

// ── Health ───────────────────────────────────────────────────────────
router.get('/health', () => json({ status: 'ok', version: '0.1.0' }))

// ── Debug: table schema inspection ──────────────────────────────────
router.get('/debug/table/:table', async (request, env: Env) => {
  const { table } = request.params
  const db = getPrimaryDB(env) as any
  const info = await db.inspectTable(table)
  return json(info)
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

  // Load nouns and readings for this domain
  const nouns = await db.findInCollection('nouns', { domain: { equals: body.domainId } }, { limit: 1000 })
  const nounByName = new Map<string, any>()
  for (const n of nouns.docs) nounByName.set(n.name, n)

  const readings = await db.findInCollection('readings', { domain: { equals: body.domainId } }, { limit: 1000 })
  const schemaByReading = new Map<string, string>()
  for (const r of readings.docs) {
    if (r.text && r.graphSchema) schemaByReading.set(r.text, r.graphSchema)
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

  // Look up state machine definition for this noun type via Registry+EntityDB fan-out
  const smInit = await getInitialState(
    registry,
    (id) => getEntityDO(env, id) as any,
    body.noun,
    body.domain,
  )

  // Create parent entity in its own EntityDB DO
  const parentId = crypto.randomUUID()
  const parentData = {
    ...parentFields,
    ...(body.reference && { reference: body.reference }),
    ...(body.createdBy && { createdBy: body.createdBy }),
    // Initialize state machine at initial status
    ...(smInit && {
      _status: smInit.initialStatus,
      _statusId: smInit.initialStatusId,
      _stateMachineDefinition: smInit.definitionId,
    }),
  }
  const entityDO = getEntityDO(env, parentId) as any
  const putResult = await entityDO.put({ id: parentId, type: body.noun, data: parentData })

  // Index the entity in the Registry
  await registry.indexEntity(body.noun, parentId)

  // Fire derivation rules (best-effort, don't block on failure)
  const deriveGetStub = (eid: string) => getEntityDO(env, eid) as any
  let derivedCount = 0
  try {
    const deriveResult = await deriveOnWrite({
      entity: { id: parentId, type: body.noun, data: parentData },
      loadDerivationRules: async () => {
        const readingIds: string[] = await registry.getEntityIds('Reading', body.domain)
        const settled = await Promise.allSettled(readingIds.map(async (rid: string) => {
          const stub = deriveGetStub(rid)
          return stub.get()
        }))
        return settled
          .filter((r: any) => r.status === 'fulfilled' && r.value && !r.value.deletedAt)
          .map((r: any) => r.value.data)
          .filter((d: any) => d.text?.includes(':='))
      },
      loadNouns: async () => {
        const nounIds: string[] = await registry.getEntityIds('Noun', body.domain)
        const settled = await Promise.allSettled(nounIds.map(async (nid: string) => {
          const stub = deriveGetStub(nid)
          return stub.get()
        }))
        return settled
          .filter((r: any) => r.status === 'fulfilled' && r.value && !r.value.deletedAt)
          .map((r: any) => r.value.data.name)
          .filter(Boolean)
      },
      loadRelatedFacts: async (nounType: string) => {
        const ids: string[] = await registry.getEntityIds(nounType)
        const entities: Array<{ id: string; type: string; data: Record<string, unknown> }> = []
        await Promise.all(ids.slice(0, 50).map(async (id: string) => {
          try {
            const eDO = getEntityDO(env, id) as any
            const e = await eDO.get()
            if (e && !e.deletedAt) entities.push({ id: e.id, type: e.type, data: e.data })
          } catch {}
        }))
        return entities
      },
      writeDerivedFact: async (entityId: string, field: string, value: string) => {
        const eDO = getEntityDO(env, entityId) as any
        await eDO.patch({ [field]: value })
      },
    })
    derivedCount = deriveResult.derivedCount
  } catch { /* derivation is best-effort */ }

  const result: Record<string, any> = {
    id: parentId,
    noun: body.noun,
    domain: body.domain,
    version: putResult.version,
    ...(smInit && { status: smInit.initialStatus }),
    ...(derivedCount > 0 && { derived: derivedCount }),
  }

  // Create children as separate Entity DOs, each with FK back to parent
  if (childArrays.length > 0) {
    const parentFkField = body.noun.charAt(0).toLowerCase() + body.noun.slice(1)
    const children: any[] = []
    for (const { childNoun, items } of childArrays) {
      for (const childFields of items) {
        try {
          const childId = crypto.randomUUID()
          const childData = { ...childFields, [parentFkField]: parentId }
          const childDO = getEntityDO(env, childId) as any
          await childDO.put({ id: childId, type: childNoun, data: childData })
          await registry.indexEntity(childNoun, childId)
          children.push({ noun: childNoun, id: childId })
        } catch (err: any) {
          children.push({ noun: childNoun, error: err.message })
        }
      }
    }
    result.children = children
  }

  return json(result, { status: 201 })
})

// ── Entity state machine transitions ─────────────────────────────────
// GET available transitions
router.get('/api/entities/:noun/:id/transitions', async (request, env: Env) => {
  const { noun, id } = request.params
  const entityDO = getEntityDO(env, id) as any
  const entity = await entityDO.get()
  if (!entity) return error(404, { errors: [{ message: 'Not Found' }] })

  if (!entity.data?._stateMachineDefinition || !entity.data?._statusId) {
    return json({ transitions: [], message: 'Entity has no state machine' })
  }

  const url = new URL(request.url)
  const domainSlug = url.searchParams.get('domain') || entity.data._domain as string || undefined
  const smRegistry = getRegistryDO(env, 'global') as any
  const smGetStub = (eid: string) => getEntityDO(env, eid) as any
  const options = await getValidTransitions(
    smRegistry, smGetStub,
    entity.data._stateMachineDefinition as string,
    entity.data._statusId as string,
    domainSlug,
  )

  return json({
    currentStatus: entity.data._status,
    transitions: options.map(o => ({ event: o.event, targetStatus: o.targetStatus })),
  })
})

// POST fire a transition event
router.post('/api/entities/:noun/:id/transition', async (request, env: Env) => {
  const { noun, id } = request.params
  const body = await request.json() as { event: string; domain?: string }
  if (!body.event) return error(400, { errors: [{ message: 'event required' }] })

  const entityDO = getEntityDO(env, id) as any
  const entity = await entityDO.get()
  if (!entity) return error(404, { errors: [{ message: 'Not Found' }] })

  if (!entity.data?._stateMachineDefinition || !entity.data?._statusId) {
    return error(400, { errors: [{ message: 'Entity has no state machine' }] })
  }

  const domainSlug = body.domain || entity.data._domain as string || undefined
  const smRegistry = getRegistryDO(env, 'global') as any
  const smGetStub = (eid: string) => getEntityDO(env, eid) as any
  const result = await applyTransition(
    smRegistry, smGetStub,
    entity.data._stateMachineDefinition as string,
    entity.data._statusId as string,
    body.event,
    domainSlug,
  )

  if (!result) {
    const options = await getValidTransitions(
      smRegistry, smGetStub,
      entity.data._stateMachineDefinition as string,
      entity.data._statusId as string,
      domainSlug,
    )
    return error(400, { errors: [{
      message: `Invalid transition: event '${body.event}' not available from status '${entity.data._status}'`,
      validEvents: options.map(o => o.event),
    }] })
  }

  // Update entity with new status
  await entityDO.patch({
    _status: result.newStatus,
    _statusId: result.newStatusId,
  })

  return json({
    id,
    noun,
    previousStatus: result.previousStatus,
    status: result.newStatus,
    event: result.event,
    transitionId: result.transitionId,
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
      filtered = filtered.filter(doc => doc[field] === condition.equals)
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
  })
})

router.get('/api/entities/:noun/:id', async (request, env: Env) => {
  const { id } = request.params

  const entity = await handleGetEntity(getEntityDO(env, id) as any)
  if (!entity) return error(404, { errors: [{ message: 'Not Found' }] })

  return json({
    id: entity.id,
    type: entity.type,
    ...entity.data,
    version: entity.version,
    createdAt: entity.createdAt,
    updatedAt: entity.updatedAt,
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

/** GET /api/:collection — list/find */
// ── Conceptual Query (before generic :collection routes) ─────────────
router.get('/api/query', handleConceptualQuery)
router.post('/api/query', handleConceptualQuery)

// ── Claims ingestion & stats (before generic :collection routes) ─────
router.post('/api/claims', async (request, env: Env) => {
  const { handleClaims } = await import('./claims')
  return handleClaims(request, env)
})
router.get('/api/stats', async (request, env: Env) => {
  const { handleStats } = await import('./claims')
  return handleStats(request, env)
})

router.get('/api/:collection', async (request, env: Env) => {
  const { collection } = request.params
  if (!COLLECTION_TABLE_MAP[collection]) {
    return error(404, { errors: [{ message: `Collection "${collection}" not found` }] })
  }

  // Delegate to entity-type handler if this collection maps to a known entity type
  const entityType = COLLECTION_TO_ENTITY_TYPE[collection]
  if (entityType) {
    const url = new URL(request.url)
    const { where, limit: qLimit, page: qPage } = parseQueryOptions(url.searchParams)
    const domainId = where?.domain?.equals || where?.['domain.domainSlug']?.equals || url.searchParams.get('domain') || ''
    const registry = getRegistryDO(env, 'global') as any
    const result = await handleListEntities(
      entityType,
      domainId,
      registry,
      (id) => getEntityDO(env, id) as any,
      { limit: qLimit, page: qPage },
    )

    // Flatten entity data for backwards compat (spread data into top level)
    const docs = result.docs.map((e) => ({
      id: e.id,
      type: e.type,
      ...e.data,
      version: e.version,
      createdAt: e.createdAt,
      updatedAt: e.updatedAt,
    }))

    // Client-side filtering by where params
    let filtered = docs
    if (where) {
      for (const [field, condition] of Object.entries(where)) {
        if (field === 'domain' || field === 'domain.domainSlug') continue // already used for routing
        if (typeof condition === 'object' && condition !== null && 'equals' in condition) {
          filtered = filtered.filter(doc => doc[field] === (condition as any).equals)
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
    })
  }

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

  // No entity-type mapping and not generators — 404
  return error(404, { errors: [{ message: `Collection "${collection}" has no entity-type handler` }] })
})

/** GET /api/:collection/:id — get by ID */
router.get('/api/:collection/:id', async (request, env: Env) => {
  const { collection, id } = request.params
  if (!COLLECTION_TABLE_MAP[collection]) {
    return error(404, { errors: [{ message: `Collection "${collection}" not found` }] })
  }

  // Delegate to entity-type handler if this collection maps to a known entity type
  const entityType = COLLECTION_TO_ENTITY_TYPE[collection]
  if (entityType) {
    const entity = await handleGetEntity(getEntityDO(env, id) as any)
    if (!entity) return error(404, { errors: [{ message: 'Not Found' }] })
    return json({
      id: entity.id,
      type: entity.type,
      ...entity.data,
      version: entity.version,
      createdAt: entity.createdAt,
      updatedAt: entity.updatedAt,
    })
  }

  // Generators collection stays in DomainDB
  if (collection === 'generators') {
    const db = getPrimaryDB(env) as any
    const doc = await db.getFromCollection(collection, id)
    if (!doc) return error(404, { errors: [{ message: 'Not Found' }] })
    return json(doc)
  }

  return error(404, { errors: [{ message: `Collection "${collection}" has no entity-type handler` }] })
})

/** POST /api/:collection — create */
router.post('/api/:collection', async (request, env: Env) => {
  const { collection } = request.params
  if (!COLLECTION_TABLE_MAP[collection]) {
    return error(404, { errors: [{ message: `Collection "${collection}" not found` }] })
  }

  const body = await request.json() as Record<string, any>

  // Delegate to entity-type handler if this collection maps to a known entity type
  const entityType = COLLECTION_TO_ENTITY_TYPE[collection]
  if (entityType) {
    const domain = body.domain || ''
    const registry = getRegistryDO(env, 'global') as any
    const result = await handleCreateEntity(
      entityType,
      domain,
      body,
      (id) => getEntityDO(env, id) as any,
      registry,
    )
    return json({ doc: { id: result.id, ...body }, message: 'Created successfully' }, { status: 201 })
  }

  // Generators collection stays in DomainDB
  if (collection === 'generators') {
    const db = getPrimaryDB(env) as any
    const doc = await db.createInCollection(collection, body)
    return json({ doc, message: 'Created successfully' }, { status: 201 })
  }

  return error(404, { errors: [{ message: `Collection "${collection}" has no entity-type handler` }] })
})

/** PATCH /api/:collection/:id — update */
router.patch('/api/:collection/:id', async (request, env: Env) => {
  const { collection, id } = request.params
  if (!COLLECTION_TABLE_MAP[collection]) {
    return error(404, { errors: [{ message: `Collection "${collection}" not found` }] })
  }

  const body = await request.json() as Record<string, any>

  // Delegate to entity-type handler if this collection maps to a known entity type
  const entityType = COLLECTION_TO_ENTITY_TYPE[collection]
  if (entityType) {
    const entityDO = getEntityDO(env, id) as any
    const result = await entityDO.patch(body)
    if (!result) return error(404, { errors: [{ message: 'Not Found' }] })
    return json({ doc: result, message: 'Updated successfully' })
  }

  // Generators collection stays in DomainDB
  if (collection === 'generators') {
    const db = getPrimaryDB(env) as any
    const doc = await db.updateInCollection(collection, id, body)
    if (!doc) return error(404, { errors: [{ message: 'Not Found' }] })
    return json({ doc, message: 'Updated successfully' })
  }

  return error(404, { errors: [{ message: `Collection "${collection}" has no entity-type handler` }] })
})

/** DELETE /api/:collection/:id — delete */
router.delete('/api/:collection/:id', async (request, env: Env) => {
  const { collection, id } = request.params
  if (!COLLECTION_TABLE_MAP[collection]) {
    return error(404, { errors: [{ message: `Collection "${collection}" not found` }] })
  }

  // Delegate to entity-type handler if this collection maps to a known entity type
  const entityType = COLLECTION_TO_ENTITY_TYPE[collection]
  if (entityType) {
    const registry = getRegistryDO(env, 'global') as any
    const result = await handleDeleteEntity(id, getEntityDO(env, id) as any, registry, entityType)
    if (!result) return error(404, { errors: [{ message: 'Not Found' }] })
    return json({ id, message: 'Deleted successfully' })
  }

  // Generators collection stays in DomainDB
  if (collection === 'generators') {
    const db = getPrimaryDB(env) as any
    const result = await db.deleteFromCollection(collection, id)
    if (!result.deleted) return error(404, { errors: [{ message: 'Not Found' }] })
    return json({ id, message: 'Deleted successfully' })
  }

  return error(404, { errors: [{ message: `Collection "${collection}" has no entity-type handler` }] })
})

// ── Legacy aliases (backwards compat for /seed and /claims without /api prefix) ──
router.all('/seed', handleSeed)
router.all('/claims', handleSeed)

// ── Domain wipe (metamodel only — nouns, readings, constraints, roles, etc.) ──
router.delete('/api/domains/:domainId/metamodel', async (request, env: Env) => {
  const { domainId } = request.params
  const db = getPrimaryDB(env) as any
  const result = await db.wipeDomainMetamodel(domainId)
  return json(result)
})

// ── Full reset (delete ALL data from ALL tables) ────────────────────
router.delete('/api/reset', async (request, env: Env) => {
  const db = getPrimaryDB(env) as any
  await db.wipeAllData()
  return json({ reset: true })
})

// ── Parse / Verify ──────────────────────────────────────────────────
router.all('/parse', handleParse)
router.all('/parse/orm', handleParseOrm)
router.all('/verify', handleVerify)

// ── State Machine RPC ────────────────────────────────────────────────
router.get('/api/state/*', async (request, env: Env) => {
  const { handleGetState } = await import('./state')
  return handleGetState(request, env)
})
router.post('/api/state/*', async (request, env: Env) => {
  const { handleSendEvent } = await import('./state')
  return handleSendEvent(request, env)
})
router.delete('/api/state/*', async (request, env: Env) => {
  const { handleDeleteState } = await import('./state')
  return handleDeleteState(request, env)
})

// ── 404 fallback ─────────────────────────────────────────────────────
router.all('*', () => error(404, { errors: [{ message: 'Not Found' }] }))

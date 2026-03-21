import { AutoRouter, json, error } from 'itty-router'
import type { Env } from '../types'
import { parseQueryOptions } from './collections'
import { COLLECTION_TABLE_MAP, FIELD_MAP, FK_TARGET_TABLE, REVERSE_FK_MAP } from '../collections'
import { handleSeed } from './seed'
import { handleGenerate } from './generate'
import { handleParse } from './parse'
import { handleParseOrm } from './parse-orm'
import { handleVerify } from './verify'
import { handleEvaluate, handleSynthesize } from './evaluate'
import { createWithHook, refreshNouns, type HookContext, COLLECTION_HOOKS } from '../hooks'
import { getInitialState, getValidTransitions, applyTransition } from '../worker/state-machine'
import { handleConceptualQuery } from './conceptual-query'

// ── DO helpers ───────────────────────────────────────────────────────

/** Get a DomainDB DO stub for the given domain slug. */
function getDomainDO(env: Env, domainSlug: string): DurableObjectStub {
  const id = env.DOMAIN_DB.idFromName(domainSlug)
  return env.DOMAIN_DB.get(id)
}

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

/**
 * Resolve which DomainDB DO to query based on the domain context in the request.
 * Checks where[domain][equals] or body.domain for a domain UUID or slug.
 * Uses the Registry to find which DomainDB DO has that domain.
 * Falls back to getPrimaryDB if no domain context is found.
 */
/**
 * Result of domain resolution — includes the DO stub and whether a specific
 * domain was resolved (so the caller can strip the domain filter from queries).
 */
interface DomainResolution {
  db: DurableObjectStub
  resolved: boolean  // true = routed to a per-domain DO (strip domain filter)
}

async function resolveDomainDB(env: Env, where?: Record<string, any>, body?: Record<string, any>): Promise<DomainResolution> {
  const domainId = where?.domain?.equals || where?.['domain.domainSlug']?.equals || body?.domain

  if (domainId && typeof domainId === 'string') {
    // If it looks like a slug (has hyphens, not a UUID pattern), use directly
    if (domainId.includes('-') && !domainId.match(/^[0-9a-f]{8}-[0-9a-f]{4}-/)) {
      return { db: getDomainDO(env, domainId), resolved: true }
    }

    // It's a UUID — try Registry first, then primary DO for slug lookup
    const registry = getRegistryDO(env, 'global') as any
    try {
      const slug: string | null = await registry.resolveSlugByUUID(domainId)
      if (slug) return { db: getDomainDO(env, slug), resolved: true }
    } catch { /* fall through */ }

    // Fallback: look up slug from primary DO's domains table
    const primary = getPrimaryDB(env) as any
    try {
      const result = await primary.findInCollection('domains', { id: { equals: domainId } }, { limit: 1 })
      if (result.docs.length) {
        const slug = result.docs[0].domainSlug || result.docs[0].domain_slug
        if (slug) return { db: getDomainDO(env, slug), resolved: true }
      }
    } catch { /* fall through */ }
  }

  // No domain context — fall back to primary
  return { db: getPrimaryDB(env), resolved: false }
}

/** Strip domain filter from where clause — not needed when querying a per-domain DO. */
function stripDomainFilter(where: Record<string, any>): Record<string, any> {
  const filtered = { ...where }
  delete filtered.domain
  delete filtered['domain.domainSlug']
  delete filtered['domain.organization']
  return filtered
}

export const router = AutoRouter()

// ── Depth population ────────────────────────────────────────────────

/**
 * Resolve FK columns in a doc into populated objects (like Payload's depth).
 * For each FK column (e.g. event_type_id), fetch the target row and replace
 * the Payload field (e.g. eventType) with the full object.
 */
async function populateDepth(
  db: any,
  doc: Record<string, unknown>,
  table: string,
  currentDepth: number,
  maxDepth: number,
): Promise<Record<string, unknown>> {
  if (currentDepth >= maxDepth) return doc

  const fieldMap = FIELD_MAP[table] || {}
  // Build reverse: SQL column → Payload field name
  const reverseFieldMap: Record<string, string> = {}
  for (const [payloadName, sqlCol] of Object.entries(fieldMap)) {
    reverseFieldMap[sqlCol] = payloadName
  }

  const populated = { ...doc }

  for (const [sqlCol, targetTable] of Object.entries(FK_TARGET_TABLE)) {
    // Find the Payload field name that maps to this FK column
    const payloadField = reverseFieldMap[sqlCol]
    if (!payloadField || !(payloadField in populated)) continue

    const fkValue = populated[payloadField]
    if (!fkValue || typeof fkValue !== 'string') continue

    // Look up the FK slug for the target collection
    const targetSlug = Object.entries(COLLECTION_TABLE_MAP).find(([, t]) => t === targetTable)?.[0]
    if (!targetSlug) continue

    try {
      const related = await db.getFromCollection(targetSlug, fkValue)
      if (related) {
        const populatedRelated = await populateDepth(db, related, targetTable, currentDepth + 1, maxDepth)
        populated[payloadField] = populatedRelated
      }
    } catch {
      // If lookup fails, keep the ID
    }
  }

  // Reverse FK population (has-many relationships, e.g. app.domains)
  const reverseFKs = REVERSE_FK_MAP[table]
  if (reverseFKs) {
    for (const [payloadField, { childCollection, fkColumn }] of Object.entries(reverseFKs)) {
      try {
        const children = await db.findInCollection(childCollection, {
          [fkColumn]: { equals: populated.id },
        }, { limit: 100 })
        const childTable = COLLECTION_TABLE_MAP[childCollection]
        if (childTable && currentDepth + 1 < maxDepth) {
          populated[payloadField] = await Promise.all(
            children.docs.map((child: Record<string, unknown>) =>
              populateDepth(db, child, childTable, currentDepth + 1, maxDepth)
            )
          )
        } else {
          populated[payloadField] = children.docs
        }
      } catch {
        // If reverse lookup fails, skip
      }
    }
  }

  return populated
}

/** Populate depth for an array of docs. */
async function populateDocs(
  db: any,
  docs: Record<string, unknown>[],
  collectionSlug: string,
  depth: number,
): Promise<Record<string, unknown>[]> {
  if (depth <= 0) return docs
  const table = COLLECTION_TABLE_MAP[collectionSlug]
  if (!table) return docs
  return Promise.all(docs.map(doc => populateDepth(db, doc, table, 0, depth)))
}

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

  // Look up state machine definition for this noun type
  const domainDO = getDomainDO(env, body.domain) as any
  const smInit = await getInitialState(domainDO, body.noun, body.domain)

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

  const result: Record<string, any> = {
    id: parentId,
    noun: body.noun,
    domain: body.domain,
    version: putResult.version,
    ...(smInit && { status: smInit.initialStatus }),
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
  const domainSlug = url.searchParams.get('domain') || entity.data._domain || 'global'
  const domainDO = getDomainDO(env, domainSlug) as any
  const options = await getValidTransitions(domainDO, entity.data._stateMachineDefinition, entity.data._statusId)

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

  const domainSlug = body.domain || entity.data._domain || 'global'
  const domainDO = getDomainDO(env, domainSlug) as any
  const result = await applyTransition(
    domainDO,
    entity.data._stateMachineDefinition,
    entity.data._statusId,
    body.event,
  )

  if (!result) {
    const options = await getValidTransitions(domainDO, entity.data._stateMachineDefinition, entity.data._statusId)
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

// ── Entity queries (3NF tables) ───────────────────────────────────────
router.get('/api/entities/:noun', async (request, env: Env) => {
  const { noun } = request.params
  const url = new URL(request.url)
  const domainId = url.searchParams.get('domain')
  if (!domainId) return error(400, { errors: [{ message: 'domain query param required' }] })

  const limit = parseInt(url.searchParams.get('limit') || '100', 10)
  const page = parseInt(url.searchParams.get('page') || '1', 10)

  // Parse where params: where[field][op]=value
  const where: Record<string, any> = {}
  for (const [key, val] of url.searchParams.entries()) {
    const m = key.match(/^where\[(\w+)\](?:\[(\w+)\])?$/)
    if (m) {
      const field = m[1]
      const op = m[2] || 'equals'
      where[field] = { [op]: val }
    }
  }

  // Get all entity IDs for this noun type from the Registry
  const registry = getRegistryDO(env, 'global') as any
  const allIds: string[] = await registry.getEntityIds(noun)

  // Fetch each entity from its EntityDB DO
  const allDocs: Record<string, unknown>[] = []
  await Promise.all(
    allIds.map(async (entityId: string) => {
      const entityDO = getEntityDO(env, entityId) as any
      const entity = await entityDO.get()
      if (entity && !entity.deletedAt) {
        allDocs.push({ id: entity.id, type: entity.type, ...entity.data, version: entity.version, createdAt: entity.createdAt, updatedAt: entity.updatedAt })
      }
    })
  )

  // Client-side filtering by where params
  let filtered = allDocs
  for (const [field, condition] of Object.entries(where)) {
    if (typeof condition === 'object' && condition !== null && 'equals' in condition) {
      filtered = filtered.filter(doc => doc[field] === condition.equals)
    }
  }

  // Pagination
  const totalDocs = filtered.length
  const offset = (page - 1) * limit
  const docs = limit > 0 ? filtered.slice(offset, offset + limit) : filtered

  return json({
    docs,
    totalDocs,
    limit,
    page,
    totalPages: limit > 0 ? Math.ceil(totalDocs / limit) : 1,
    hasNextPage: limit > 0 && offset + limit < totalDocs,
    hasPrevPage: page > 1,
  })
})

router.get('/api/entities/:noun/:id', async (request, env: Env) => {
  const { id } = request.params

  const entityDO = getEntityDO(env, id) as any
  const entity = await entityDO.get()

  if (!entity || entity.deletedAt) {
    return error(404, { errors: [{ message: 'Not Found' }] })
  }

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

  const entityDO = getEntityDO(env, id) as any
  const result = await entityDO.delete()

  if (!result) return error(404, { errors: [{ message: 'Not Found' }] })

  // Deindex the entity in the Registry
  const registry = getRegistryDO(env, 'global') as any
  await registry.deindexEntity(noun, id)

  return json({ id, deleted: true })
})

// ── Collection CRUD ──────────────────────────────────────────────────

/** GET /api/:collection — list/find */
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

  const url = new URL(request.url)
  const { where, limit, page, sort, depth } = parseQueryOptions(url.searchParams)

  // Route to the correct DomainDB based on domain context
  const { db, resolved } = await resolveDomainDB(env, where)
  const queryWhere = resolved ? stripDomainFilter(where || {}) : where
  const result = await (db as any).findInCollection(collection, queryWhere, { limit, page, sort })
  const docs = await populateDocs(db as any, result.docs, collection, depth)

  return json({
    docs,
    totalDocs: result.totalDocs,
    limit: result.limit,
    page: result.page,
    totalPages: Math.ceil(result.totalDocs / result.limit),
    hasNextPage: result.hasNextPage,
    hasPrevPage: result.page > 1,
    pagingCounter: (result.page - 1) * result.limit + 1,
  })
})

/** GET /api/:collection/:id — get by ID */
router.get('/api/:collection/:id', async (request, env: Env) => {
  const { collection, id } = request.params
  if (!COLLECTION_TABLE_MAP[collection]) {
    return error(404, { errors: [{ message: `Collection "${collection}" not found` }] })
  }

  const url = new URL(request.url)
  const depth = parseInt(url.searchParams.get('depth') || '0', 10)
  const { where } = parseQueryOptions(url.searchParams)

  const { db } = await resolveDomainDB(env, where)
  const doc = await (db as any).getFromCollection(collection, id)

  if (!doc) return error(404, { errors: [{ message: 'Not Found' }] })
  const populated = depth > 0 ? await populateDepth(db as any, doc, COLLECTION_TABLE_MAP[collection], 0, depth) : doc
  return json(populated)
})

/** POST /api/:collection — create */
router.post('/api/:collection', async (request, env: Env) => {
  const { collection } = request.params
  if (!COLLECTION_TABLE_MAP[collection]) {
    return error(404, { errors: [{ message: `Collection "${collection}" not found` }] })
  }

  const body = await request.json() as Record<string, any>
  const { db } = await resolveDomainDB(env, undefined, body)

  // If a hook exists for this collection, use createWithHook
  if (COLLECTION_HOOKS[collection]) {
    const domainId = body.domain || ''
    const allNouns = domainId ? await refreshNouns(db, domainId) : []
    const context: HookContext = { domainId, allNouns }
    const { doc, hookResult } = await createWithHook(db, collection, body, context)
    return json({
      doc,
      message: 'Created successfully',
      ...(Object.keys(hookResult.created).length > 0 && { created: hookResult.created }),
      ...(hookResult.warnings.length > 0 && { warnings: hookResult.warnings }),
    }, { status: 201 })
  }

  // No hook — standard create
  const doc = await (db as any).createInCollection(collection, body)
  return json({ doc, message: 'Created successfully' }, { status: 201 })
})

/** PATCH /api/:collection/:id — update */
router.patch('/api/:collection/:id', async (request, env: Env) => {
  const { collection, id } = request.params
  if (!COLLECTION_TABLE_MAP[collection]) {
    return error(404, { errors: [{ message: `Collection "${collection}" not found` }] })
  }

  const body = await request.json() as Record<string, any>
  const { db } = await resolveDomainDB(env, undefined, body)
  const doc = await (db as any).updateInCollection(collection, id, body)

  if (!doc) return error(404, { errors: [{ message: 'Not Found' }] })
  return json({ doc, message: 'Updated successfully' })
})

/** DELETE /api/:collection/:id — delete */
router.delete('/api/:collection/:id', async (request, env: Env) => {
  const { collection, id } = request.params
  if (!COLLECTION_TABLE_MAP[collection]) {
    return error(404, { errors: [{ message: `Collection "${collection}" not found` }] })
  }

  const { db } = await resolveDomainDB(env)
  const result = await (db as any).deleteFromCollection(collection, id)

  if (!result.deleted) return error(404, { errors: [{ message: 'Not Found' }] })
  return json({ id, message: 'Deleted successfully', ...(result.cascaded && { cascaded: result.cascaded }) })
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

// ── Conceptual Query ────────────────────────────────────────────────
router.get('/api/query', handleConceptualQuery)
router.post('/api/query', handleConceptualQuery)

// ── 404 fallback ─────────────────────────────────────────────────────
router.all('*', () => error(404, { errors: [{ message: 'Not Found' }] }))

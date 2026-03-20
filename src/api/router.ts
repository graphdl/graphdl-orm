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

/**
 * Get the GraphDL DO stub. One DO per system (for now, single instance).
 * Later: one DO per domain per tenant.
 */
function getDB(env: Env): DurableObjectStub {
  const id = env.GRAPHDL_DB.idFromName('graphdl-primary')
  return env.GRAPHDL_DB.get(id)
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
  const db = getDB(env) as any
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
  const id = env.GRAPHDL_DB.idFromName('graphdl-primary')
  const stub = env.GRAPHDL_DB.get(id)
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

  const db = getDB(env) as any

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

  const db = getDB(env) as any

  // Extract array-of-objects fields (child entities) BEFORE creating the parent.
  // Children are created as separate top-level createEntity calls to avoid
  // Cloudflare DO implicit transaction/savepoint issues with nested writes.
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

  // Create parent entity (without array-of-objects fields)
  const result = await db.createEntity(body.domain, body.noun, parentFields, body.reference, body.createdBy)

  // Create children as separate top-level calls, each with FK back to parent
  if (childArrays.length > 0 && result.id) {
    const parentFkField = body.noun.charAt(0).toLowerCase() + body.noun.slice(1)
    const children: any[] = []
    for (const { childNoun, items } of childArrays) {
      for (const childFields of items) {
        try {
          const childResult = await db.createEntity(
            body.domain, childNoun,
            { ...childFields, [parentFkField]: result.id },
          )
          children.push({ noun: childNoun, id: childResult.id })
        } catch (err: any) {
          children.push({ noun: childNoun, error: err.message })
        }
      }
    }
    result.children = children
  }

  return json(result, { status: 201 })
})

// ── Entity queries (3NF tables) ───────────────────────────────────────
router.get('/api/entities/:noun', async (request, env: Env) => {
  const { noun } = request.params
  const url = new URL(request.url)
  const domainId = url.searchParams.get('domain')
  if (!domainId) return error(400, { errors: [{ message: 'domain query param required' }] })

  const limit = parseInt(url.searchParams.get('limit') || '100', 10)
  const page = parseInt(url.searchParams.get('page') || '1', 10)
  const sort = url.searchParams.get('sort') || '-createdAt'

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

  const depth = parseInt(url.searchParams.get('depth') || '0', 10)

  const db = getDB(env) as any
  const result = await db.queryEntities(domainId, noun, { where, sort, limit, page })

  if (depth > 0) {
    result.docs = await Promise.all(
      result.docs.map((doc: Record<string, unknown>) => db.populateEntity(domainId, noun, doc))
    )
  }
  return json(result)
})

router.get('/api/entities/:noun/:id', async (request, env: Env) => {
  const { noun, id } = request.params
  const url = new URL(request.url)
  const domainId = url.searchParams.get('domain')
  if (!domainId) return error(400, { errors: [{ message: 'domain query param required' }] })
  const depth = parseInt(url.searchParams.get('depth') || '0', 10)

  const db = getDB(env) as any
  const result = await db.queryEntities(domainId, noun, { where: { id: { equals: id } }, limit: 1 })
  if (!result.docs.length) return error(404, { errors: [{ message: 'Not Found' }] })

  let doc = result.docs[0]
  if (depth > 0) {
    doc = await db.populateEntity(domainId, noun, doc)
  }
  return json(doc)
})

router.patch('/api/entities/:noun/:id', async (request, env: Env) => {
  const { noun, id } = request.params
  const url = new URL(request.url)
  const domainId = url.searchParams.get('domain')
  if (!domainId) return error(400, { errors: [{ message: 'domain query param required' }] })

  const body = await request.json() as Record<string, any>
  const db = getDB(env) as any
  const result = await db.updateEntity(domainId, noun, id, body)
  if (!result) return error(404, { errors: [{ message: 'Not Found' }] })
  return json(result)
})

router.delete('/api/entities/:noun/:id', async (request, env: Env) => {
  const { noun, id } = request.params
  const url = new URL(request.url)
  const domainId = url.searchParams.get('domain')
  if (!domainId) return error(400, { errors: [{ message: 'domain query param required' }] })

  const db = getDB(env) as any
  const result = await db.deleteEntity(domainId, noun, id)
  if (!result.deleted) return error(404, { errors: [{ message: 'Not Found' }] })
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

  const db = getDB(env) as any
  const result = await db.findInCollection(collection, where, { limit, page, sort })
  const docs = await populateDocs(db, result.docs, collection, depth)

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

  const db = getDB(env) as any
  const doc = await db.getFromCollection(collection, id)

  if (!doc) return error(404, { errors: [{ message: 'Not Found' }] })
  const populated = depth > 0 ? await populateDepth(db, doc, COLLECTION_TABLE_MAP[collection], 0, depth) : doc
  return json(populated)
})

/** POST /api/:collection — create */
router.post('/api/:collection', async (request, env: Env) => {
  const { collection } = request.params
  if (!COLLECTION_TABLE_MAP[collection]) {
    return error(404, { errors: [{ message: `Collection "${collection}" not found` }] })
  }

  const body = await request.json() as Record<string, any>
  const db = getDB(env) as any

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
  const doc = await db.createInCollection(collection, body)
  return json({ doc, message: 'Created successfully' }, { status: 201 })
})

/** PATCH /api/:collection/:id — update */
router.patch('/api/:collection/:id', async (request, env: Env) => {
  const { collection, id } = request.params
  if (!COLLECTION_TABLE_MAP[collection]) {
    return error(404, { errors: [{ message: `Collection "${collection}" not found` }] })
  }

  const body = await request.json() as Record<string, any>
  const db = getDB(env) as any
  const doc = await db.updateInCollection(collection, id, body)

  if (!doc) return error(404, { errors: [{ message: 'Not Found' }] })
  return json({ doc, message: 'Updated successfully' })
})

/** DELETE /api/:collection/:id — delete */
router.delete('/api/:collection/:id', async (request, env: Env) => {
  const { collection, id } = request.params
  if (!COLLECTION_TABLE_MAP[collection]) {
    return error(404, { errors: [{ message: `Collection "${collection}" not found` }] })
  }

  const db = getDB(env) as any
  const result = await db.deleteFromCollection(collection, id)

  if (!result.deleted) return error(404, { errors: [{ message: 'Not Found' }] })
  return json({ id, message: 'Deleted successfully', ...(result.cascaded && { cascaded: result.cascaded }) })
})

// ── Legacy aliases (backwards compat for /seed and /claims without /api prefix) ──
router.all('/seed', handleSeed)
router.all('/claims', handleSeed)

// ── Domain wipe (metamodel only — nouns, readings, constraints, roles, etc.) ──
router.delete('/api/domains/:domainId/metamodel', async (request, env: Env) => {
  const { domainId } = request.params
  const db = getDB(env) as any
  const result = await db.wipeDomainMetamodel(domainId)
  return json(result)
})

// ── Full reset (delete ALL data from ALL tables) ────────────────────
router.delete('/api/reset', async (request, env: Env) => {
  const db = getDB(env) as any
  const tables = [
    'guard_runs', 'events', 'state_machines', 'resource_roles', 'graph_citations',
    'graphs', 'resources', 'citations', 'generators',
    'completions', 'agents', 'agent_definitions', 'models',
    'constraint_spans', 'constraints', 'roles', 'readings', 'graph_schemas',
    'functions', 'verbs', 'guards', 'transitions', 'statuses',
    'event_types', 'streams', 'state_machine_definitions',
    'nouns', 'domains', 'apps', 'org_memberships', 'organizations',
  ]
  const counts: Record<string, number> = {}
  for (const table of tables) {
    try {
      const result = db.sql.exec(`SELECT count(*) as c FROM ${table}`).toArray()
      const count = result[0]?.c || 0
      if (count > 0) {
        db.sql.exec(`DELETE FROM ${table}`)
        counts[table] = count
      }
    } catch { /* table may not exist */ }
  }
  return json({ reset: true, deleted: counts })
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

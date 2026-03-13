import { AutoRouter, json, error } from 'itty-router'
import type { Env } from '../types'
import { parseQueryOptions } from './collections'
import { COLLECTION_TABLE_MAP, FIELD_MAP, FK_TARGET_TABLE, REVERSE_FK_MAP } from '../collections'
import { handleSeed } from './seed'
import { handleGenerate } from './generate'
import { handleParse } from './parse'
import { handleVerify } from './verify'
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

// ── Generate ────────────────────────────────────────────────────────
router.post('/api/generate', handleGenerate)

// ── Collection CRUD ──────────────────────────────────────────────────

/** GET /api/:collection — list/find */
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

// ── Seed / Claims ───────────────────────────────────────────────────
router.all('/seed', handleSeed)
router.all('/claims', handleSeed) // Alias used by apis worker

// ── Parse / Verify ──────────────────────────────────────────────────
router.all('/parse', handleParse)
router.all('/verify', handleVerify)

// ── 404 fallback ─────────────────────────────────────────────────────
router.all('*', () => error(404, { errors: [{ message: 'Not Found' }] }))

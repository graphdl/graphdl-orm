import { AutoRouter, json, error } from 'itty-router'
import type { Env } from '../types'
import { parseQueryOptions } from './collections'
import { COLLECTION_TABLE_MAP } from '../collections'

/**
 * Get the GraphDL DO stub. One DO per system (for now, single instance).
 * Later: one DO per domain per tenant.
 */
function getDB(env: Env): DurableObjectStub {
  const id = env.GRAPHDL_DB.idFromName('graphdl-primary')
  return env.GRAPHDL_DB.get(id)
}

export const router = AutoRouter()

// ── Health ───────────────────────────────────────────────────────────
router.get('/health', () => json({ status: 'ok', version: '0.1.0' }))

// ── Collection CRUD ──────────────────────────────────────────────────

/** GET /api/:collection — list/find */
router.get('/api/:collection', async (request, env: Env) => {
  const { collection } = request.params
  if (!COLLECTION_TABLE_MAP[collection]) {
    return error(404, { errors: [{ message: `Collection "${collection}" not found` }] })
  }

  const url = new URL(request.url)
  const { where, limit, page, sort } = parseQueryOptions(url.searchParams)

  const db = getDB(env) as any
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
})

/** GET /api/:collection/:id — get by ID */
router.get('/api/:collection/:id', async (request, env: Env) => {
  const { collection, id } = request.params
  if (!COLLECTION_TABLE_MAP[collection]) {
    return error(404, { errors: [{ message: `Collection "${collection}" not found` }] })
  }

  const db = getDB(env) as any
  const doc = await db.getFromCollection(collection, id)

  if (!doc) return error(404, { errors: [{ message: 'Not Found' }] })
  return json(doc)
})

/** POST /api/:collection — create */
router.post('/api/:collection', async (request, env: Env) => {
  const { collection } = request.params
  if (!COLLECTION_TABLE_MAP[collection]) {
    return error(404, { errors: [{ message: `Collection "${collection}" not found` }] })
  }

  const body = await request.json() as Record<string, any>
  const db = getDB(env) as any
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
  return json({ id, message: 'Deleted successfully' })
})

// ── 404 fallback ─────────────────────────────────────────────────────
router.all('*', () => error(404, { errors: [{ message: 'Not Found' }] }))

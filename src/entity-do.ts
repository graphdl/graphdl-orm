/**
 * EntityDB — pure functions for a lightweight Durable Object that holds a
 * single entity instance. Each DO = one row.
 *
 * Exports pure functions that take a `SqlLike` interface so they can be
 * unit tested without the Cloudflare runtime.
 */

export interface EntityData {
  id: string
  type: string
  data: Record<string, unknown>
}

export interface SqlLike {
  exec(query: string, ...params: any[]): { toArray(): any[] }
}

/**
 * Creates entity + events tables with CREATE TABLE IF NOT EXISTS.
 */
export function initEntitySchema(sql: SqlLike): void {
  sql.exec(`CREATE TABLE IF NOT EXISTS entity (
    id TEXT PRIMARY KEY,
    type TEXT NOT NULL,
    data TEXT NOT NULL,
    version INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    deleted_at TEXT
  )`)

  sql.exec(`CREATE TABLE IF NOT EXISTS events (
    id TEXT PRIMARY KEY,
    timestamp TEXT NOT NULL DEFAULT (datetime('now')),
    operation TEXT NOT NULL,
    data TEXT,
    prev TEXT
  )`)
}

/**
 * Inserts an entity row and logs a 'create' CDC event.
 */
export function createEntity(sql: SqlLike, input: EntityData): { id: string; version: number } {
  const now = new Date().toISOString()
  const dataJson = JSON.stringify(input.data)

  sql.exec(
    `INSERT INTO entity (id, type, data, version, created_at, updated_at, deleted_at) VALUES (?, ?, ?, ?, ?, ?, ?)`,
    input.id,
    input.type,
    dataJson,
    1,
    now,
    now,
    null,
  )

  const eventId = crypto.randomUUID()
  sql.exec(
    `INSERT INTO events (id, timestamp, operation, data, prev) VALUES (?, ?, ?, ?, ?)`,
    eventId,
    now,
    'create',
    dataJson,
    null,
  )

  return { id: input.id, version: 1 }
}

/**
 * Returns the entity or null if the table is empty.
 */
export function getEntity(sql: SqlLike): {
  id: string
  type: string
  data: Record<string, unknown>
  version: number
  createdAt: string | null
  updatedAt: string | null
  deletedAt: string | null
} | null {
  const rows = sql.exec(`SELECT * FROM entity`).toArray()
  if (rows.length === 0) return null

  const row = rows[0] as Record<string, any>
  return {
    id: row.id,
    type: row.type,
    data: typeof row.data === 'string' ? JSON.parse(row.data) : row.data,
    version: row.version,
    createdAt: row.created_at ?? null,
    updatedAt: row.updated_at ?? null,
    deletedAt: row.deleted_at ?? null,
  }
}

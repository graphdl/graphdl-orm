/**
 * EntityDB — pure functions for a lightweight Durable Object that holds a
 * single entity instance. Each DO = one row.
 *
 * Exports pure functions that take a `SqlLike` interface so they can be
 * unit tested without the Cloudflare runtime.
 */

import { DurableObject } from 'cloudflare:workers'

export interface EntityData {
  id: string
  type: string
  data: Record<string, unknown>
}

export interface EventRecord {
  id: string
  timestamp: string
  operation: string
  data: string | null
  prev: string | null
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

/**
 * Updates an entity by shallow-merging fields into the existing data.
 * Increments version, updates updated_at, and logs a CDC 'update' event.
 * Returns null if no entity exists.
 */
export function updateEntity(sql: SqlLike, fields: Record<string, unknown>): { id: string; version: number } | null {
  const existing = getEntity(sql)
  if (!existing) return null

  const now = new Date().toISOString()
  const prevData = existing.data
  const newData = { ...prevData, ...fields }
  const newVersion = existing.version + 1
  const newDataJson = JSON.stringify(newData)
  const prevDataJson = JSON.stringify(prevData)

  sql.exec(
    `UPDATE entity SET data = ?, version = ?, updated_at = ? WHERE id = ?`,
    newDataJson,
    newVersion,
    now,
    existing.id,
  )

  const eventId = crypto.randomUUID()
  sql.exec(
    `INSERT INTO events (id, timestamp, operation, data, prev) VALUES (?, ?, ?, ?, ?)`,
    eventId,
    now,
    'update',
    newDataJson,
    prevDataJson,
  )

  return { id: existing.id, version: newVersion }
}

/**
 * Soft-deletes an entity by setting deleted_at and updated_at.
 * Logs a CDC 'delete' event. Returns null if no entity exists.
 */
export function deleteEntity(sql: SqlLike): { id: string; deleted: boolean } | null {
  const existing = getEntity(sql)
  if (!existing) return null

  const now = new Date().toISOString()
  const prevDataJson = JSON.stringify(existing.data)

  sql.exec(
    `UPDATE entity SET deleted_at = ?, updated_at = ? WHERE id = ?`,
    now,
    now,
    existing.id,
  )

  const eventId = crypto.randomUUID()
  sql.exec(
    `INSERT INTO events (id, timestamp, operation, data, prev) VALUES (?, ?, ?, ?, ?)`,
    eventId,
    now,
    'delete',
    null,
    prevDataJson,
  )

  return { id: existing.id, deleted: true }
}

/**
 * Returns events in reverse chronological order (newest first).
 * If `since` is provided, only returns events after that timestamp.
 */
export function getEvents(sql: SqlLike, since?: string): EventRecord[] {
  let rows: any[]
  if (since) {
    rows = sql.exec(
      `SELECT * FROM events WHERE timestamp > ? ORDER BY timestamp DESC`,
      since,
    ).toArray()
  } else {
    rows = sql.exec(
      `SELECT * FROM events ORDER BY timestamp DESC`,
    ).toArray()
  }

  return rows.map((row: any) => ({
    id: row.id,
    timestamp: row.timestamp,
    operation: row.operation,
    data: row.data ?? null,
    prev: row.prev ?? null,
  }))
}

// =========================================================================
// Durable Object class
// =========================================================================

export class EntityDB extends DurableObject {
  private initialized = false

  private ensureInit(): void {
    if (this.initialized) return
    initEntitySchema(this.ctx.storage.sql)
    this.initialized = true
  }

  async get(): Promise<ReturnType<typeof getEntity>> {
    this.ensureInit()
    return getEntity(this.ctx.storage.sql)
  }

  async put(input: EntityData): Promise<{ id: string; version: number }> {
    this.ensureInit()
    const existing = getEntity(this.ctx.storage.sql)
    if (existing) {
      return updateEntity(this.ctx.storage.sql, input.data as Record<string, unknown>)!
    }
    return createEntity(this.ctx.storage.sql, input)
  }

  async patch(fields: Record<string, unknown>): Promise<{ id: string; version: number } | null> {
    this.ensureInit()
    return updateEntity(this.ctx.storage.sql, fields)
  }

  async delete(): Promise<{ id: string; deleted: boolean } | null> {
    this.ensureInit()
    return deleteEntity(this.ctx.storage.sql)
  }

  async events(since?: string): Promise<EventRecord[]> {
    this.ensureInit()
    return getEvents(this.ctx.storage.sql, since)
  }
}

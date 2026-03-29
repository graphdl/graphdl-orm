/**
 * EntityDB — a Durable Object that holds one entity as a 3NF row.
 *
 * RMAP generates the schema from readings. The DO stores the row.
 * Facts are projections: α(project_column) applied to the row's fields.
 * Each column is a fact type. Each cell value is a role binding.
 *
 * Storage  = the 3NF row (RMAP output)
 * Facts    = α(project) applied to the row (computed, not stored separately)
 * Query    = Filter(predicate) ∘ α(load) applied to entity references
 */

import { DurableObject } from 'cloudflare:workers'
import type { SqlLike } from './sql-like'
export type { SqlLike } from './sql-like'

// ── Types ───────────────────────────────────────────────────────────

/** A fact: a graph schema instance with role bindings (projected from the row). */
export interface Fact {
  graphSchemaId: string
  bindings: Array<[string, string]>
}

/** The entity's 3NF row. */
export interface EntityRow {
  id: string
  noun: string
  domain: string
  fields: Record<string, string>
  version: number
  createdAt: string
  updatedAt: string
  deletedAt: string | null
}

export interface EventRecord {
  id: string
  timestamp: string
  operation: string
  data: string | null
  prev: string | null
}

// ── Schema ──────────────────────────────────────────────────────────

export function initEntitySchema(sql: SqlLike): void {
  // The 3NF row. Fields stored as JSON — RMAP column projection is computed.
  sql.exec(`CREATE TABLE IF NOT EXISTS entity (
    id TEXT PRIMARY KEY,
    noun TEXT NOT NULL,
    domain TEXT NOT NULL,
    fields TEXT NOT NULL DEFAULT '{}',
    version INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    deleted_at TEXT
  )`)

  // Migration: add fields column to DOs created before it existed
  try {
    sql.exec(`ALTER TABLE entity ADD COLUMN fields TEXT NOT NULL DEFAULT '{}'`)
  } catch {
    // Column already exists — expected
  }

  sql.exec(`CREATE TABLE IF NOT EXISTS events (
    id TEXT PRIMARY KEY,
    timestamp TEXT NOT NULL DEFAULT (datetime('now')),
    operation TEXT NOT NULL,
    data TEXT,
    prev TEXT
  )`)
}

// ── Row Operations ──────────────────────────────────────────────────

/** Create the entity row. */
export function createEntity(
  sql: SqlLike, id: string, noun: string, domain: string, fields?: Record<string, string>,
): EntityRow {
  const now = new Date().toISOString()
  const fieldsJson = JSON.stringify(fields || {})
  sql.exec(
    `INSERT OR REPLACE INTO entity (id, noun, domain, fields, version, created_at, updated_at, deleted_at) VALUES (?, ?, ?, ?, 1, ?, ?, ?)`,
    id, noun, domain, fieldsJson, now, now, null,
  )
  const eventId = crypto.randomUUID()
  sql.exec(
    `INSERT INTO events (id, timestamp, operation, data, prev) VALUES (?, ?, ?, ?, ?)`,
    eventId, now, 'create', fieldsJson, null,
  )
  return { id, noun, domain, fields: fields || {}, version: 1, createdAt: now, updatedAt: now, deletedAt: null }
}

/** Get the entity row. */
export function getEntity(sql: SqlLike): EntityRow | null {
  const rows = sql.exec(`SELECT * FROM entity`).toArray()
  if (rows.length === 0) return null
  const row = rows[0] as Record<string, any>
  return {
    id: row.id,
    noun: row.noun,
    domain: row.domain,
    fields: typeof row.fields === 'string' ? JSON.parse(row.fields) : (row.fields || {}),
    version: row.version,
    createdAt: row.created_at ?? null,
    updatedAt: row.updated_at ?? null,
    deletedAt: row.deleted_at ?? null,
  }
}

/** Update fields by shallow merge. Returns null if no entity exists. */
export function updateEntity(sql: SqlLike, newFields: Record<string, string>): EntityRow | null {
  const entity = getEntity(sql)
  if (!entity) return null
  const now = new Date().toISOString()
  const prevFields = entity.fields
  const merged = { ...prevFields, ...newFields }
  const mergedJson = JSON.stringify(merged)
  const prevJson = JSON.stringify(prevFields)
  sql.exec(
    `UPDATE entity SET fields = ?, version = version + 1, updated_at = ? WHERE id = ?`,
    mergedJson, now, entity.id,
  )
  const eventId = crypto.randomUUID()
  sql.exec(
    `INSERT INTO events (id, timestamp, operation, data, prev) VALUES (?, ?, ?, ?, ?)`,
    eventId, now, 'update', mergedJson, prevJson,
  )
  return { ...entity, fields: merged, version: entity.version + 1, updatedAt: now }
}

/** Soft-delete. */
export function deleteEntity(sql: SqlLike): { id: string; deleted: boolean } | null {
  const entity = getEntity(sql)
  if (!entity) return null
  const now = new Date().toISOString()
  sql.exec(`UPDATE entity SET deleted_at = ?, updated_at = ? WHERE id = ?`, now, now, entity.id)
  const eventId = crypto.randomUUID()
  sql.exec(
    `INSERT INTO events (id, timestamp, operation, data, prev) VALUES (?, ?, ?, ?, ?)`,
    eventId, now, 'delete', null, JSON.stringify(entity.fields),
  )
  return { id: entity.id, deleted: true }
}

// ── Fact Projection ─────────────────────────────────────────────────
// Facts are NOT stored. They are projections of the 3NF row.
// α(project_column) applied to the row's fields.

/** Project the row into facts. Each field becomes a graph schema instance. */
export function getFacts(sql: SqlLike): Fact[] {
  const entity = getEntity(sql)
  if (!entity) return []
  return Object.entries(entity.fields)
    .filter(([_, v]) => v !== null && v !== undefined && v !== '')
    .map(([field, value]) => ({
      graphSchemaId: `${entity.noun} has ${field}`,
      bindings: [[entity.noun, entity.id], [field, value]],
    }))
}

/** Project facts for a specific graph schema (field). */
export function getFactsBySchema(sql: SqlLike, graphSchemaId: string): Fact[] {
  return getFacts(sql).filter(f => f.graphSchemaId === graphSchemaId)
}

/** Convert to Population-compatible structure. */
export function toPopulation(sql: SqlLike): Record<string, Array<{ factTypeId: string; bindings: Array<[string, string]> }>> {
  const facts = getFacts(sql)
  const population: Record<string, Array<{ factTypeId: string; bindings: Array<[string, string]> }>> = {}
  for (const fact of facts) {
    if (!population[fact.graphSchemaId]) population[fact.graphSchemaId] = []
    population[fact.graphSchemaId].push({ factTypeId: fact.graphSchemaId, bindings: fact.bindings })
  }
  return population
}

// ── Events ──────────────────────────────────────────────────────────

export function getEvents(sql: SqlLike, since?: string): EventRecord[] {
  let rows: any[]
  if (since) {
    rows = sql.exec(`SELECT * FROM events WHERE timestamp > ? ORDER BY timestamp DESC`, since).toArray()
  } else {
    rows = sql.exec(`SELECT * FROM events ORDER BY timestamp DESC`).toArray()
  }
  return rows.map((row: any) => ({
    id: row.id, timestamp: row.timestamp, operation: row.operation,
    data: row.data ?? null, prev: row.prev ?? null,
  }))
}

// ── Secrets ─────────────────────────────────────────────────────────

export function initSecretSchema(sql: SqlLike): void {
  sql.exec(`CREATE TABLE IF NOT EXISTS secrets (
    system TEXT NOT NULL, value TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (system)
  )`)
}

export function storeSecret(sql: SqlLike, system: string, value: string): void {
  const now = new Date().toISOString()
  sql.exec(
    `INSERT INTO secrets (system, value, created_at, updated_at) VALUES (?, ?, ?, ?)
     ON CONFLICT(system) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at`,
    system, value, now, now,
  )
}

export function resolveSecret(sql: SqlLike, system: string): string | null {
  const rows = sql.exec(`SELECT value FROM secrets WHERE system = ?`, system).toArray()
  if (rows.length === 0) return null
  return (rows[0] as any).value
}

export function deleteSecret(sql: SqlLike, system: string): boolean {
  sql.exec(`DELETE FROM secrets WHERE system = ?`, system)
  return true
}

export function listConnectedSystems(sql: SqlLike): string[] {
  return sql.exec(`SELECT system FROM secrets ORDER BY system`).toArray().map((r: any) => r.system)
}

// ── Durable Object ──────────────────────────────────────────────────

export class EntityDB extends DurableObject {
  private initialized = false

  private ensureInit(): void {
    if (this.initialized) return
    initEntitySchema(this.ctx.storage.sql)
    initSecretSchema(this.ctx.storage.sql)
    this.initialized = true
  }

  async get(): Promise<EntityRow | null> {
    this.ensureInit()
    return getEntity(this.ctx.storage.sql)
  }

  async put(input: { id: string; type: string; data: Record<string, unknown> }): Promise<void> {
    this.ensureInit()
    const existing = getEntity(this.ctx.storage.sql)
    const fields: Record<string, string> = {}
    for (const [k, v] of Object.entries(input.data)) {
      if (v !== null && v !== undefined) fields[k] = String(v)
    }
    if (existing) {
      updateEntity(this.ctx.storage.sql, fields)
    } else {
      createEntity(this.ctx.storage.sql, input.id, input.type, fields.domain || '', fields)
    }
  }

  async patch(newFields: Record<string, string>): Promise<EntityRow | null> {
    this.ensureInit()
    return updateEntity(this.ctx.storage.sql, newFields)
  }

  async delete(): Promise<{ id: string; deleted: boolean } | null> {
    this.ensureInit()
    return deleteEntity(this.ctx.storage.sql)
  }

  async getFacts(): Promise<Fact[]> {
    this.ensureInit()
    return getFacts(this.ctx.storage.sql)
  }

  async getFactsBySchema(graphSchemaId: string): Promise<Fact[]> {
    this.ensureInit()
    return getFactsBySchema(this.ctx.storage.sql, graphSchemaId)
  }

  async toPopulation(): Promise<Record<string, Array<{ factTypeId: string; bindings: Array<[string, string]> }>>> {
    this.ensureInit()
    return toPopulation(this.ctx.storage.sql)
  }

  async events(since?: string): Promise<EventRecord[]> {
    this.ensureInit()
    return getEvents(this.ctx.storage.sql, since)
  }

  async connectSystem(system: string, secret: string): Promise<void> {
    this.ensureInit()
    storeSecret(this.ctx.storage.sql, system, secret)
  }

  async resolveSystemSecret(system: string): Promise<string | null> {
    this.ensureInit()
    return resolveSecret(this.ctx.storage.sql, system)
  }

  async disconnectSystem(system: string): Promise<boolean> {
    this.ensureInit()
    return deleteSecret(this.ctx.storage.sql, system)
  }

  async connectedSystems(): Promise<string[]> {
    this.ensureInit()
    return listConnectedSystems(this.ctx.storage.sql)
  }
}

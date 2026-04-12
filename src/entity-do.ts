/**
 * EntityDB — a Durable Object that IS a cell: ⟨CELL, id, contents⟩.
 *
 * One DO instance per entity id. This is what gives per-entity
 * writer isolation (Definition 2, cell isolation): commands on
 * different entities land on different DOs and run concurrently;
 * commands on the same entity serialize through its DO. Cross-entity
 * metadata (the population index, schema cache, domain secrets)
 * lives in RegistryDB — one per scope — so it isn't contended
 * against entity writes.
 *
 * Per the AREST whitepaper (Sec. 14.3):
 *   - Each entity is a cell in state D
 *   - ↑n : D → c  (fetch — get the cell's contents)
 *   - ↓n : ⟨x, D⟩ → D'  (store — replace the cell's contents)
 *
 * The cell's contents = { id, type, data } where:
 *   - id   = reference scheme (cell name)
 *   - type = noun type (ORM 2 entity type)
 *   - data = record of role bindings (field → value)
 *
 * Facts are projections: α(project_column) applied to the cell's data.
 * Each field is a fact type. Each value is a role binding.
 *
 * Traceability (created_at, updated_at, version, audit trail) is modeled
 * as readings in the metamodel — Event entities are cells in D, not a
 * procedural side-channel. See readings/instances.md:
 *   "Event occurred at Timestamp."
 *   "Event is of Event Type."
 *   "Event triggered Transition in State Machine."
 */

import { DurableObject } from 'cloudflare:workers'
import type { SqlLike } from './sql-like'
export type { SqlLike } from './sql-like'

// ── Types ───────────────────────────────────────────────────────────

/** The cell's contents — what ↑n returns. */
export interface CellContents {
  id: string
  type: string
  data: Record<string, unknown>
}

/** A fact: a fact type instance with role bindings (projected from the cell). */
export interface Fact {
  graphSchemaId: string
  bindings: Array<[string, string]>
}

// ── Schema ──────────────────────────────────────────────────────────

export function initCellSchema(sql: SqlLike): void {
  sql.exec(`CREATE TABLE IF NOT EXISTS cell (
    id TEXT PRIMARY KEY,
    type TEXT NOT NULL,
    data TEXT NOT NULL DEFAULT '{}'
  )`)

  // Migration from old entity table: if entity table exists, migrate data
  try {
    const rows = sql.exec(`SELECT id, noun, fields FROM entity LIMIT 1`).toArray()
    if (rows.length > 0) {
      const row = rows[0] as Record<string, any>
      const data = typeof row.fields === 'string' ? row.fields : JSON.stringify(row.fields || {})
      sql.exec(
        `INSERT OR REPLACE INTO cell (id, type, data) VALUES (?, ?, ?)`,
        row.id, row.noun, data,
      )
      sql.exec(`DROP TABLE entity`)
    }
  } catch {
    // No old entity table — expected for new DOs
  }

  // Drop legacy events table — traceability is modeled as Event entities in the population
  try { sql.exec(`DROP TABLE events`) } catch { /* doesn't exist */ }
}

// ── Cell Operations (↑n / ↓n) ──────────────────────────────────────

/** ↑n — fetch the cell's contents. */
export function fetchCell(sql: SqlLike): CellContents | null {
  const rows = sql.exec(`SELECT id, type, data FROM cell`).toArray()
  if (rows.length === 0) return null
  const row = rows[0] as Record<string, any>
  return {
    id: row.id,
    type: row.type,
    data: typeof row.data === 'string' ? JSON.parse(row.data) : (row.data || {}),
  }
}

/** ↓n — store new contents into the cell. */
export function storeCell(
  sql: SqlLike, id: string, type: string, data: Record<string, unknown>,
): CellContents {
  const dataJson = JSON.stringify(data)
  sql.exec(
    `INSERT OR REPLACE INTO cell (id, type, data) VALUES (?, ?, ?)`,
    id, type, dataJson,
  )
  return { id, type, data }
}

/** Remove the cell entirely (hard delete). */
export function removeCell(sql: SqlLike): { id: string } | null {
  const cell = fetchCell(sql)
  if (!cell) return null
  sql.exec(`DELETE FROM cell`)
  return { id: cell.id }
}

// ── Fact Projection ─────────────────────────────────────────────────
// Facts are NOT stored. They are projections of the cell's data.
// α(project_column) applied to the data record.

/** Project the cell into facts. Each field becomes a fact type instance. */
export function getFacts(sql: SqlLike): Fact[] {
  const cell = fetchCell(sql)
  if (!cell) return []
  return Object.entries(cell.data)
    .filter(([_, v]) => v !== null && v !== undefined && v !== '')
    .map(([field, value]) => ({
      graphSchemaId: `${cell.type} has ${field}`,
      bindings: [[cell.type, cell.id], [field, String(value)]],
    }))
}

/** Project facts for a specific fact type (field). */
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

// ── Secrets (infrastructure, not domain facts) ─────────────────────
// API keys, OAuth tokens, connection strings for external systems.
// Not part of the population P — these are infrastructure config.

export function initSecretSchema(sql: SqlLike): void {
  sql.exec(`CREATE TABLE IF NOT EXISTS secrets (
    system TEXT PRIMARY KEY,
    value TEXT NOT NULL
  )`)
}

export function storeSecret(sql: SqlLike, system: string, value: string): void {
  sql.exec(
    `INSERT OR REPLACE INTO secrets (system, value) VALUES (?, ?)`,
    system, value,
  )
}

export function resolveSecret(sql: SqlLike, system: string): string | null {
  const rows = sql.exec(`SELECT value FROM secrets WHERE system = ?`, system).toArray()
  if (rows.length === 0) return null
  return (rows[0] as any).value
}

export function deleteSecret(sql: SqlLike, system: string): void {
  sql.exec(`DELETE FROM secrets WHERE system = ?`, system)
}

export function listConnectedSystems(sql: SqlLike): string[] {
  return sql.exec(`SELECT system FROM secrets ORDER BY system`).toArray().map((r: any) => r.system)
}

// ── Durable Object ──────────────────────────────────────────────────

export class EntityDB extends DurableObject {
  private initialized = false

  private ensureInit(): void {
    if (this.initialized) return
    initCellSchema(this.ctx.storage.sql)
    initSecretSchema(this.ctx.storage.sql)
    this.initialized = true
  }

  /** ↑n — fetch the cell. Returns { id, type, data } or null. */
  async get(): Promise<CellContents | null> {
    this.ensureInit()
    return fetchCell(this.ctx.storage.sql)
  }

  /** ↓n — store the cell. Merges with existing data (idempotent across domains). */
  async put(input: { id: string; type: string; data: Record<string, unknown> }): Promise<CellContents> {
    this.ensureInit()
    const existing = fetchCell(this.ctx.storage.sql)
    const merged: Record<string, unknown> = existing ? { ...existing.data } : {}
    for (const [k, v] of Object.entries(input.data)) {
      if (v !== null && v !== undefined) merged[k] = v
    }
    return storeCell(this.ctx.storage.sql, input.id, input.type, merged)
  }

  /** Remove the cell entirely. */
  async delete(): Promise<{ id: string } | null> {
    this.ensureInit()
    return removeCell(this.ctx.storage.sql)
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

  // ── Secret storage (infrastructure) ────────────────────────────────

  async connectSystem(system: string, secret: string): Promise<void> {
    this.ensureInit()
    storeSecret(this.ctx.storage.sql, system, secret)
  }

  async resolveSystemSecret(system: string): Promise<string | null> {
    this.ensureInit()
    return resolveSecret(this.ctx.storage.sql, system)
  }

  async disconnectSystem(system: string): Promise<void> {
    this.ensureInit()
    deleteSecret(this.ctx.storage.sql, system)
  }

  async connectedSystems(): Promise<string[]> {
    this.ensureInit()
    return listConnectedSystems(this.ctx.storage.sql)
  }
}

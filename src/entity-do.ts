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
import {
  type CellAddress,
  type TenantMasterKey,
  cellSeal,
  cellOpen,
  deriveTenantMasterKey,
  rotateCell,
} from './cell-encryption'
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

// ── Cell-level encryption (#659) ───────────────────────────────────
//
// `storeCellSealed` / `fetchCellSealed` are the cell_seal / cell_open
// pair the EntityDB reaches for whenever a tenant master is bound at
// the DO scope. The wire shape stored in the SQLite TEXT column is a
// magic prefix + base64 of the sealed envelope:
//
//     "ARESTAEAD1:" + base64(NONCE | ciphertext | tag)
//
// The prefix is what lets `fetchCell` /
// `fetchCellSealed` distinguish encrypted from plaintext rows during
// a migration window — if the prefix is absent we treat the row as
// legacy plaintext JSON. Production deployments enable encryption
// uniformly so the legacy path is a no-op once the migration window
// closes; until then it keeps mixed-shape DBs readable.
//
// Address shape: scope = "worker", domain = the EntityDB's noun type
// (e.g. "Order"), cellName = the entity id (e.g. "ord-42"), version
// = 0 today (a future commit can wire this to the per-row monotonic
// version per #558 for replay-defence on hot-swapped masters).

/** Sealed-row magic prefix on the SQLite TEXT column. */
export const SEALED_CELL_PREFIX = 'ARESTAEAD1:'

/** Build a CellAddress from the EntityDB's notion of (type, id). */
export function cellAddressFor(type: string, id: string): CellAddress {
  return {
    scope: 'worker',
    domain: type,
    cellName: id,
    version: 0,
  }
}

/** ↑n — fetch the cell, decrypting if the row carries the sealed
 *  prefix. Returns the same shape as `fetchCell` so callers can
 *  swap the helper without touching their consumers. */
export async function fetchCellSealed(
  sql: SqlLike,
  master: TenantMasterKey,
): Promise<CellContents | null> {
  const rows = sql.exec(`SELECT id, type, data FROM cell`).toArray()
  if (rows.length === 0) return null
  const row = rows[0] as Record<string, any>
  const dataField: unknown = row.data
  let data: Record<string, unknown>
  if (typeof dataField === 'string' && dataField.startsWith(SEALED_CELL_PREFIX)) {
    const sealed = base64ToBytes(dataField.slice(SEALED_CELL_PREFIX.length))
    const address = cellAddressFor(row.type as string, row.id as string)
    const opened = await cellOpen(master, address, sealed)
    const json = new TextDecoder().decode(opened)
    data = JSON.parse(json)
  } else if (typeof dataField === 'string') {
    // Legacy plaintext row — read as-is during migration window.
    data = JSON.parse(dataField || '{}')
  } else {
    data = (dataField as Record<string, unknown>) ?? {}
  }
  return {
    id: row.id,
    type: row.type,
    data,
  }
}

/** ↓n — store new contents into the cell, sealing the JSON-encoded
 *  data column with the per-tenant master before the SQL write.
 *  The encrypted bytes go into the same `data` TEXT column, prefixed
 *  with `SEALED_CELL_PREFIX` so `fetchCellSealed` / `fetchCell` can
 *  tell encrypted rows from legacy plaintext. */
export async function storeCellSealed(
  sql: SqlLike,
  master: TenantMasterKey,
  id: string,
  type: string,
  data: Record<string, unknown>,
): Promise<CellContents> {
  const json = JSON.stringify(data)
  const address = cellAddressFor(type, id)
  const sealed = await cellSeal(master, address, json)
  const blob = SEALED_CELL_PREFIX + bytesToBase64(sealed)
  sql.exec(
    `INSERT OR REPLACE INTO cell (id, type, data) VALUES (?, ?, ?)`,
    id,
    type,
    blob,
  )
  return { id, type, data }
}

// Inline base64 helpers — `cell-encryption.ts` keeps them private; we
// duplicate the few lines here rather than re-exporting because the
// SQL column round-trip is the only place outside the encryption
// module that needs the raw conversion.
function bytesToBase64(bytes: Uint8Array): string {
  let binary = ''
  const CHUNK = 0x8000
  for (let i = 0; i < bytes.length; i += CHUNK) {
    binary += String.fromCharCode(
      ...bytes.subarray(i, Math.min(i + CHUNK, bytes.length)),
    )
  }
  return btoa(binary)
}
function base64ToBytes(b64: string): Uint8Array {
  const binary = atob(b64)
  const out = new Uint8Array(binary.length)
  for (let i = 0; i < binary.length; i++) out[i] = binary.charCodeAt(i)
  return out
}

// ── Fact Projection ─────────────────────────────────────────────────
// Facts are NOT stored. They are projections of the cell's data.
// α(project_column) applied to the data record.

/** Project a cell value (already fetched + decrypted) into facts.
 *  Pure function — split out so the encrypted DO methods can call it
 *  after `fetchCellSealed` without re-deriving the master. */
export function factsFromCell(cell: CellContents | null): Fact[] {
  if (!cell) return []
  return Object.entries(cell.data)
    .filter(([_, v]) => v !== null && v !== undefined && v !== '')
    .map(([field, value]) => ({
      graphSchemaId: `${cell.type} has ${field}`,
      bindings: [[cell.type, cell.id], [field, String(value)]],
    }))
}

/** Project the cell into facts. Each field becomes a fact type instance. */
export function getFacts(sql: SqlLike): Fact[] {
  return factsFromCell(fetchCell(sql))
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
  /** Lazily-derived per-tenant master. `null` until the first call
   *  that actually needs to seal/open — derivation reaches Web
   *  Crypto's `crypto.subtle` and is async, so we can't do it in
   *  `ensureInit` (which is sync) and shouldn't pay the cost on
   *  every request. */
  private master: TenantMasterKey | null = null

  private ensureInit(): void {
    if (this.initialized) return
    initCellSchema(this.ctx.storage.sql)
    initSecretSchema(this.ctx.storage.sql)
    this.initialized = true
  }

  /** Resolve the per-tenant master from the
   *  `TENANT_MASTER_SEED` Worker secret + this DO's id (which is
   *  the tenant-scoped routing key the dispatcher derived). Memoised
   *  per DO instance.
   *
   *  Returns `null` if the secret is not bound — callers fall back
   *  to the legacy plaintext path so a stripped-down dev build (no
   *  `wrangler secret put TENANT_MASTER_SEED` step) keeps working
   *  without source surgery. Production deployments must set the
   *  secret; absence of the secret in prod is a deploy-time bug. */
  private async getMaster(): Promise<TenantMasterKey | null> {
    if (this.master) return this.master
    const env = this.env as { TENANT_MASTER_SEED?: string } | undefined
    const seed = env?.TENANT_MASTER_SEED
    if (!seed) return null
    // The DO's id name is the tenant routing key (per-cell DO mapping
    // #217). Use it as the salt so each tenant derives a distinct
    // master from the same shared seed.
    const tenantSalt = this.ctx.id.toString()
    const m = await deriveTenantMasterKey(seed, tenantSalt)
    this.master = m
    return m
  }

  /** ↑n — fetch the cell. Returns { id, type, data } or null. */
  async get(): Promise<CellContents | null> {
    this.ensureInit()
    const master = await this.getMaster()
    if (master) {
      return fetchCellSealed(this.ctx.storage.sql, master)
    }
    return fetchCell(this.ctx.storage.sql)
  }

  /** ↓n — store the cell. Merges with existing data (idempotent across domains). */
  async put(input: { id: string; type: string; data: Record<string, unknown> }): Promise<CellContents> {
    this.ensureInit()
    const master = await this.getMaster()
    const existing = master
      ? await fetchCellSealed(this.ctx.storage.sql, master)
      : fetchCell(this.ctx.storage.sql)
    const merged: Record<string, unknown> = existing ? { ...existing.data } : {}
    for (const [k, v] of Object.entries(input.data)) {
      if (v !== null && v !== undefined) merged[k] = v
    }
    if (master) {
      return storeCellSealed(this.ctx.storage.sql, master, input.id, input.type, merged)
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
    const master = await this.getMaster()
    if (master) {
      const cell = await fetchCellSealed(this.ctx.storage.sql, master)
      return factsFromCell(cell)
    }
    return getFacts(this.ctx.storage.sql)
  }

  async getFactsBySchema(graphSchemaId: string): Promise<Fact[]> {
    this.ensureInit()
    const master = await this.getMaster()
    if (master) {
      const cell = await fetchCellSealed(this.ctx.storage.sql, master)
      return factsFromCell(cell).filter(f => f.graphSchemaId === graphSchemaId)
    }
    return getFactsBySchema(this.ctx.storage.sql, graphSchemaId)
  }

  async toPopulation(): Promise<Record<string, Array<{ factTypeId: string; bindings: Array<[string, string]> }>>> {
    this.ensureInit()
    const master = await this.getMaster()
    if (master) {
      const cell = await fetchCellSealed(this.ctx.storage.sql, master)
      const facts = factsFromCell(cell)
      const population: Record<string, Array<{ factTypeId: string; bindings: Array<[string, string]> }>> = {}
      for (const fact of facts) {
        if (!population[fact.graphSchemaId]) population[fact.graphSchemaId] = []
        population[fact.graphSchemaId].push({ factTypeId: fact.graphSchemaId, bindings: fact.bindings })
      }
      return population
    }
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

  // ── Tenant master rotation (#662) ─────────────────────────────────
  //
  // Rotate THIS DO's sealed row from `oldSeed`/`oldSalt` → `newSeed`/
  // `newSalt`. The orchestrator (worker.ts / RegistryDB rotation
  // path) holds the per-tenant write lock for the duration of the
  // walk; this method performs the per-cell atomic swap inside the
  // DO's single-writer scope.
  //
  // Returns:
  //   - `{ ok: true, rotated: true }` on a clean rotation
  //   - `{ ok: true, rotated: false }` when the row is empty / legacy
  //     plaintext / not in our `SEALED_CELL_PREFIX` form (no-op)
  //   - `{ ok: false, kind: 'truncated' | 'auth' }` when the old master
  //     cannot open the row — the row is left untouched, operator
  //     decides whether to retry, zeroize, or accept the loss.
  //
  // The two seeds + two salts are passed explicitly rather than
  // derived from `env`: during rotation the orchestrator has both
  // masters in hand (TENANT_MASTER_SEED + TENANT_MASTER_SEED_v2).
  // After rotation completes the operator promotes v2 → v1 and the
  // DO's `getMaster` resolves transparently to the new key.
  async rotateMaster(args: {
    oldSeed: string | Uint8Array
    oldSalt: string | Uint8Array
    newSeed: string | Uint8Array
    newSalt: string | Uint8Array
  }): Promise<
    | { ok: true; rotated: boolean }
    | { ok: false; kind: 'truncated' | 'auth' }
  > {
    this.ensureInit()
    const rows = this.ctx.storage.sql
      .exec(`SELECT id, type, data FROM cell`)
      .toArray()
    if (rows.length === 0) {
      return { ok: true, rotated: false }
    }
    const row = rows[0] as Record<string, any>
    const dataField = row.data as unknown
    if (typeof dataField !== 'string' || !dataField.startsWith(SEALED_CELL_PREFIX)) {
      // Legacy plaintext or empty — no rotation needed.
      return { ok: true, rotated: false }
    }
    const oldMaster = await deriveTenantMasterKey(args.oldSeed, args.oldSalt)
    const newMaster = await deriveTenantMasterKey(args.newSeed, args.newSalt)
    const sealed = base64ToBytes(dataField.slice(SEALED_CELL_PREFIX.length))
    const address = cellAddressFor(row.type as string, row.id as string)
    let newSealed: Uint8Array
    try {
      newSealed = await rotateCell(oldMaster, newMaster, address, sealed)
    } catch (e) {
      // Old master could not open the row — surface the kind so the
      // orchestrator can collect it into the rotation report.
      const kind = (e as { kind?: 'truncated' | 'auth' }).kind ?? 'auth'
      return { ok: false, kind }
    }
    // Atomic swap: write the new sealed envelope back. The DO's
    // single-writer guarantee means no concurrent put/get on this DO
    // can interleave between the read above and the write below.
    const blob = SEALED_CELL_PREFIX + bytesToBase64(newSealed)
    this.ctx.storage.sql.exec(
      `INSERT OR REPLACE INTO cell (id, type, data) VALUES (?, ?, ?)`,
      row.id,
      row.type,
      blob,
    )
    // Invalidate the memoised master so subsequent calls re-derive
    // from whichever seed `env` exposes (the orchestrator promotes
    // v2 → v1 after the walk completes).
    this.master = null
    return { ok: true, rotated: true }
  }
}

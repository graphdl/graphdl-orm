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
import { compileDomainReadings, freezeHandle, thawHandle, release_domain, system } from './api/engine'
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
  // The `version` column is the per-cell monotonic counter used as
  // part of the AEAD AAD (#661 / #558). Each successful sealed write
  // bumps it; a captured-then-replayed older sealed envelope at the
  // same `(scope, domain, cell_name)` fails to decrypt because the
  // current row's version no longer matches the captured ciphertext's
  // AAD. Default 0 — the first sealed write bumps to 1.
  sql.exec(`CREATE TABLE IF NOT EXISTS cell (
    id TEXT PRIMARY KEY,
    type TEXT NOT NULL,
    data TEXT NOT NULL DEFAULT '{}',
    version INTEGER NOT NULL DEFAULT 0
  )`)

  // Schema migration: existing DOs created before #661 don't have the
  // `version` column. ALTER TABLE ADD COLUMN is idempotent-by-trial:
  // SQLite throws "duplicate column name" if it already exists, which
  // we swallow. Pre-existing rows pick up the DEFAULT 0 — matching
  // the "all existing cells are at version 0" baseline the task brief
  // calls out.
  try {
    sql.exec(`ALTER TABLE cell ADD COLUMN version INTEGER NOT NULL DEFAULT 0`)
  } catch {
    // Column already exists — expected when the table was created
    // fresh by the CREATE TABLE above (with the column already in
    // its declaration).
  }

  // Migration from old entity table: if entity table exists, migrate data.
  //
  // #766 (#721-followup-c) note: this SQL write does NOT route through
  // the per-DO engine apply path. `initCellSchema` is a sync function
  // called from `EntityDB.ensureInit`, which runs before the engine
  // handle is hydrated (`hydrateEngine` is async and only kicked off
  // by the first user-facing instance method). Routing this one-time
  // legacy-table migration through engine apply would require either
  // making `initCellSchema` async + reaching the engine handle from a
  // free function, or deferring the migration until the first call —
  // both heavier than the migration warrants. The migrated cell will
  // pick up an engine-side chain entry on its first write through
  // `put()` / `writeCellThroughEngine`.
  try {
    const rows = sql.exec(`SELECT id, noun, fields FROM entity LIMIT 1`).toArray()
    if (rows.length > 0) {
      const row = rows[0] as Record<string, any>
      const data = typeof row.fields === 'string' ? row.fields : JSON.stringify(row.fields || {})
      sql.exec(
        `INSERT OR REPLACE INTO cell (id, type, data, version) VALUES (?, ?, ?, ?)`,
        row.id, row.noun, data, 0,
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

// ── Cell-level encryption (#659 / #661) ────────────────────────────
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
// = the per-cell monotonic counter from the row's `version` column
// (#661 / #558). Each successful sealed write bumps the counter; a
// captured-then-replayed older sealed envelope at the same address
// fails decrypt because the persisted version (now N+1) no longer
// matches the captured ciphertext's AAD (which carries N).

/** Sealed-row magic prefix on the SQLite TEXT column. */
export const SEALED_CELL_PREFIX = 'ARESTAEAD1:'

/** Build a CellAddress from the EntityDB's notion of (type, id) plus
 *  the per-row monotonic version (#661). Pre-existing cells default
 *  to version 0 (matching the schema column DEFAULT); the first
 *  successful sealed write through `storeCellSealed` bumps to 1. */
export function cellAddressFor(type: string, id: string, version: number = 0): CellAddress {
  return {
    scope: 'worker',
    domain: type,
    cellName: id,
    version,
  }
}

/** ↑n — fetch the cell, decrypting if the row carries the sealed
 *  prefix. Returns the same shape as `fetchCell` so callers can
 *  swap the helper without touching their consumers.
 *
 *  The persisted `version` column is read alongside the sealed bytes
 *  and folded into the CellAddress before `cellOpen` — without it the
 *  AEAD opener would derive a different per-cell HKDF key (since the
 *  AAD includes the version) and every read after the first write
 *  would surface as `CellAeadError(auth)`. */
export async function fetchCellSealed(
  sql: SqlLike,
  master: TenantMasterKey,
): Promise<CellContents | null> {
  const rows = sql.exec(`SELECT id, type, data, version FROM cell`).toArray()
  if (rows.length === 0) return null
  const row = rows[0] as Record<string, any>
  const dataField: unknown = row.data
  // Coerce the persisted version. Older DOs that pre-date the
  // schema change return undefined for the column; treat those as
  // version 0 (legacy baseline). SQLite returns INTEGER as `number`
  // through the workerd binding.
  const persistedVersion = typeof row.version === 'number' && Number.isFinite(row.version)
    ? row.version
    : 0
  let data: Record<string, unknown>
  if (typeof dataField === 'string' && dataField.startsWith(SEALED_CELL_PREFIX)) {
    const sealed = base64ToBytes(dataField.slice(SEALED_CELL_PREFIX.length))
    const address = cellAddressFor(row.type as string, row.id as string, persistedVersion)
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
 *  tell encrypted rows from legacy plaintext.
 *
 *  ## Atomic version + sealed write (#661)
 *
 *  Read the current `version` column for this row, bump by 1, build
 *  the CellAddress with the NEW version, seal under that address,
 *  and persist `(version, sealed)` in a single
 *  `INSERT OR REPLACE INTO cell` call. The DO is single-writer by
 *  Cloudflare's design, so the read-modify-write window cannot
 *  observe a concurrent put on the same row; the single SQL UPSERT
 *  commits both halves together (the sealed bytes and the new
 *  version), preserving the invariant that the persisted version
 *  always matches the AAD the sealed bytes were produced under. */
export async function storeCellSealed(
  sql: SqlLike,
  master: TenantMasterKey,
  id: string,
  type: string,
  data: Record<string, unknown>,
): Promise<CellContents> {
  // Read the current version for this row (if any). A fresh cell
  // returns 0 rows; a re-write returns the existing row's stamp.
  // The DO is single-writer, so this read-then-write window is
  // atomic with respect to any other operation on the same DO.
  const existing = sql.exec(`SELECT version FROM cell WHERE id = ?`, id).toArray()
  const prevVersion = existing.length > 0 && typeof (existing[0] as any).version === 'number'
    ? (existing[0] as any).version as number
    : 0
  const nextVersion = prevVersion + 1

  const json = JSON.stringify(data)
  const address = cellAddressFor(type, id, nextVersion)
  const sealed = await cellSeal(master, address, json)
  const blob = SEALED_CELL_PREFIX + bytesToBase64(sealed)
  // Persist sealed bytes + new version atomically. INSERT OR REPLACE
  // is one statement; SQLite commits it as a single row write, so
  // we cannot end up in a state where the new sealed bytes were
  // committed but the version bump was not (or vice versa).
  sql.exec(
    `INSERT OR REPLACE INTO cell (id, type, data, version) VALUES (?, ?, ?, ?)`,
    id,
    type,
    blob,
    nextVersion,
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

/** DO storage key under which each EntityDB persists its engine
 *  freeze image (#764). Constant — every EntityDB DO uses the same
 *  key inside its own private storage namespace. */
export const ENGINE_STATE_STORAGE_KEY = 'engine_state_bytes'

export class EntityDB extends DurableObject {
  private initialized = false
  /** Lazily-derived per-tenant master. `null` until the first call
   *  that actually needs to seal/open — derivation reaches Web
   *  Crypto's `crypto.subtle` and is async, so we can't do it in
   *  `ensureInit` (which is sync) and shouldn't pay the cost on
   *  every request. */
  private master: TenantMasterKey | null = null

  /** Per-DO engine handle (#764, #721-followup-a). `-1` until the
   *  first call hydrates it. Each EntityDB instance IS a cell per
   *  the whitepaper (§3.3, §202): its engine state is the per-cell
   *  fold `D_n' = foldl μ_n D_n E_n`. Sibling tasks #765/#766/#767
   *  route cell reads/writes through this handle; this task only
   *  delivers the lifecycle layer.
   *
   *  The handle is RESERVED across the DO's lifetime — Cloudflare
   *  evicts the isolate after some idle period and a fresh isolate
   *  re-hydrates from `state.storage` via `hydrateEngine`. The
   *  legacy direct-SQL paths (`fetchCell` / `storeCell` / etc.) are
   *  unaffected by this field; they continue to operate on
   *  `ctx.storage.sql` directly until #765/#766 swap them out. */
  private engineHandle = -1

  /** In-flight hydrate promise. Concurrent invocations on a fresh
   *  isolate must NOT race on `compileDomainReadings` /
   *  `thawHandle` — only one `compileDomainReadings` should run per
   *  DO instance. We memoise the promise so any concurrent caller
   *  awaits the same hydrate work. Cloudflare's
   *  `state.blockConcurrencyWhile` would be the canonical primitive
   *  here, but that has to live in the constructor; doing it
   *  lazily is equally correct because the DO is single-threaded
   *  inside one isolate (concurrent fetches enter the JS event
   *  loop one at a time, so the second caller observes the first's
   *  in-flight promise before it ever calls `hydrateEngine` itself). */
  private hydrateInFlight: Promise<void> | null = null

  private ensureInit(): void {
    if (this.initialized) return
    initCellSchema(this.ctx.storage.sql)
    initSecretSchema(this.ctx.storage.sql)
    this.initialized = true
  }

  /** Idempotent hydrate: ensure `engineHandle` is allocated and
   *  loaded with the persisted freeze image (if any). Safe to call
   *  on every public method; cheap after the first call (just an
   *  early-return on the cached handle).
   *
   *  Concurrency: two concurrent `await this.hydrateEngine()` calls
   *  on a cold isolate share one in-flight promise — the second
   *  caller sees `hydrateInFlight` non-null and awaits it instead
   *  of double-allocating. Cloudflare's
   *  `state.blockConcurrencyWhile` is the canonical equivalent
   *  primitive but is only callable from the constructor; deferring
   *  to lazy hydrate is the same shape because the DO is
   *  single-isolate-single-threaded and the JS event loop
   *  serialises the field reads. */
  protected async hydrateEngine(): Promise<void> {
    if (this.engineHandle >= 0) return
    if (this.hydrateInFlight) return this.hydrateInFlight
    this.hydrateInFlight = (async () => {
      // Re-check inside the critical section in case a concurrent
      // caller raced us through the early return above (defensive
      // — under Cloudflare's single-thread model this can't happen,
      // but it costs nothing and keeps the invariant local).
      if (this.engineHandle >= 0) return
      const handle = compileDomainReadings()
      const persisted = await this.ctx.storage.get<string>(
        ENGINE_STATE_STORAGE_KEY,
      )
      if (typeof persisted === 'string' && persisted.length > 0) {
        // Best-effort hydrate: a malformed / cross-version freeze
        // image returns `false` and we keep the freshly-allocated
        // empty engine. Sibling task #769 adds explicit migration;
        // this task's contract is "lifecycle wired", not "every
        // possible freeze image is recoverable".
        thawHandle(handle, persisted)
      }
      this.engineHandle = handle
    })()
    try {
      await this.hydrateInFlight
    } finally {
      this.hydrateInFlight = null
    }
  }

  /** Snapshot the per-DO engine state and write it back to DO
   *  storage. Called by sibling tasks #766/#767 after every
   *  state-mutating engine call (apply / transition). Public for
   *  those siblings; the lifecycle test below also drives it
   *  directly to verify the persistence path. */
  protected async persistEngineState(): Promise<void> {
    if (this.engineHandle < 0) return
    const hex = freezeHandle(this.engineHandle)
    await this.ctx.storage.put(ENGINE_STATE_STORAGE_KEY, hex)
  }

  /** Route a cell write through the per-DO engine's `apply` system
   *  verb (#766, #721-followup-c).
   *
   *  ## Current behaviour (#766) and pending engine lift
   *
   *  `system(h, "apply", JSON)` dispatches to
   *  `crates/arest/src/ast.rs:platform_apply_command`, which evaluates
   *  the command via `apply_command_defs` and returns the resulting
   *  `CommandResult` wrapped as `Object::atom(JSON.stringify(result))`.
   *  The outer write dispatcher (`crates/arest/src/lib.rs:2048`
   *  `system_impl`) recognises a `WriterResult::CommitDelta` ONLY when
   *  the result is a Map shaped `{__state_delta, __result}` —
   *  `platform_apply_command` does not return that shape today, so
   *  the current dispatch resolves to `NoCommit` and D is NOT
   *  mutated. The chain therefore does NOT extend on this call.
   *
   *  The wiring below is intentional: keeping the helper hot on
   *  every `put()` means the moment the engine lift lands (return
   *  the raw delta carrier from `platform_apply_command`, or expose
   *  a new `raw_store` SystemVerb), the EntityDB write path inherits
   *  chain semantics with zero call-site changes. The persist-after-
   *  apply call already keeps the freeze image fresh, the WASM handle
   *  is reused, and the verb shape (`createEntity`/`updateEntity`)
   *  matches the rest of the worker (`src/api/entity-routes.ts`).
   *
   *  Whitepaper anchor (AREST.tex §202, §462 eq:cellfold): one writer
   *  per cell, chain version-of-record. Pre-#766 the worker EntityDB
   *  stamped each cell with its own SQL `cell.version` column — the
   *  divergent sidecar §3.3 warns against. The full migration off
   *  `cell.version` lands once the engine surface lift (above) +
   *  sibling tasks #765 (engine reads) and #767 (cell_pin →
   *  CellAddress.version_id) close out — #768 then drops the column.
   *
   *  Returns the parsed engine response so callers can surface
   *  `entities`/`violations`/etc. The response is best-effort: a
   *  malformed engine reply (parse error) is swallowed because the
   *  SQL write at the call site is the authoritative store until
   *  #765 routes reads through engine fetch.
   *
   *  Sibling-task contract: do NOT call this from rotation
   *  (`rotateMaster`) — rotation re-encrypts the cell's bytes under a
   *  new master while preserving the AAD version field; bumping the
   *  engine's chain here would put the new sealed bytes
   *  (AAD=oldVersion) out of sync with the engine's version stamp
   *  (newVersion+1), causing `cellOpen` to fail the next read after
   *  #767 lands. The migration path (`initCellSchema`'s
   *  legacy-`entity`-table branch) also skips this helper because
   *  the engine handle isn't allocated until first instance-method
   *  call. */
  protected async writeCellThroughEngine(
    operation: 'create' | 'update',
    type: string,
    id: string,
    fields: Record<string, unknown>,
  ): Promise<unknown> {
    await this.hydrateEngine()
    // Coerce field values to strings — engine `Command::CreateEntity`
    // (`crates/arest/src/command.rs:46`) declares `fields:
    // hashbrown::HashMap<String, String>`. JSON bool/number passed
    // raw would fail the deserializer.
    const stringFields: Record<string, string> = {}
    for (const [k, v] of Object.entries(fields)) {
      if (v === null || v === undefined) continue
      stringFields[k] = typeof v === 'string' ? v : String(v)
    }
    const command = operation === 'create'
      ? {
          type: 'createEntity',
          noun: type,
          domain: '',
          id,
          fields: stringFields,
        }
      : {
          type: 'updateEntity',
          noun: type,
          domain: '',
          entityId: id,
          fields: stringFields,
        }
    // The wrapped envelope shape — `platform_apply_command`
    // (crates/arest/src/ast.rs:2750) accepts both
    // `{command, population}` and raw command JSON; we use the
    // wrapper for parity with `engine.applyCommand` so a future
    // refactor can collapse the two call sites.
    const envelope = JSON.stringify({ command, population: '' })
    const raw = system(this.engineHandle, 'apply', envelope)
    await this.persistEngineState()
    try {
      return JSON.parse(raw)
    } catch {
      // Bottom / malformed envelope — the engine still committed the
      // delta into its chain (we just can't parse the JSON envelope
      // back). Returning `null` keeps the helper total; the SQL
      // write at the call site remains authoritative for
      // backward-compat readers.
      return null
    }
  }

  /** Test hook — exposes the hydrate path to the unit suite without
   *  having to drive it through one of the user-facing methods.
   *  Returns the engine handle (always `>= 0` after the call).
   *  Marked with the `__test_` prefix so production callers don't
   *  reach for it by accident. */
  async __test_hydrate(): Promise<number> {
    await this.hydrateEngine()
    return this.engineHandle
  }

  /** Test hook — exposes the freeze + persist path so the lifecycle
   *  test can drive a write-back without waiting for sibling tasks
   *  to land. Returns the hex blob that was written. */
  async __test_persist(): Promise<string> {
    await this.hydrateEngine()
    await this.persistEngineState()
    const stored = await this.ctx.storage.get<string>(
      ENGINE_STATE_STORAGE_KEY,
    )
    return stored ?? ''
  }

  /** Test hook — releases the engine handle (mimics isolate
   *  eviction). The next `hydrateEngine` re-allocates and re-thaws
   *  from DO storage. */
  async __test_evict(): Promise<void> {
    if (this.engineHandle >= 0) {
      release_domain(this.engineHandle)
      this.engineHandle = -1
    }
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
    // ── Engine apply (#766, #721-followup-c) ─────────────────────
    // Route the write through the per-DO engine BEFORE the SQL
    // write so the engine path is the authoritative version-of-
    // record per AREST.tex §202, §462 eq:cellfold. The helper docs
    // on `writeCellThroughEngine` capture the current limitation:
    // `system(h, "apply", …)` is functionally evaluated but does NOT
    // mutate D today (the wrapper at platform_apply_command turns
    // the delta carrier into a JSON-string atom that the outer
    // dispatcher classifies as NoCommit). The wiring is hot anyway
    // so chain semantics land here automatically once the engine
    // surface lift (or a `raw_store` SystemVerb) ships.
    //
    // The SQL write below (storeCellSealed/storeCell) stays as
    // backward-compat scaffolding — sibling task #765 routes reads
    // through `system(h, "fetch", ...)`, after which the SQL
    // payload column becomes redundant. #768 then drops the
    // `cell.version` SQL column once #767 sources CellAddress's
    // version field from `cell_pin` instead of the row stamp.
    //
    // Best-effort: a thrown engine error doesn't abort the SQL
    // write. Until #765 lands, reads still source from SQL, so a
    // missing engine apply is recoverable. We log to console.warn
    // for ops visibility without breaking the request.
    const isUpdate = existing !== null
    try {
      await this.writeCellThroughEngine(
        isUpdate ? 'update' : 'create',
        input.type,
        input.id,
        merged,
      )
    } catch (e) {
      // Engine apply failure is non-fatal during the migration
      // window. The SQL write below is the authoritative store
      // until #765/#767/#768 lands.
      // eslint-disable-next-line no-console
      console.warn('EntityDB.put: engine apply failed, falling back to SQL-only write:', e)
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
      .exec(`SELECT id, type, data, version FROM cell`)
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
    // The persisted version IS the AAD the sealed bytes were produced
    // under (#661). Rotation re-seals the recovered plaintext at the
    // SAME address (same version) but under the new master — the
    // version field stays put, only the master derivation changes.
    const persistedVersion = typeof row.version === 'number' && Number.isFinite(row.version)
      ? row.version
      : 0
    const address = cellAddressFor(row.type as string, row.id as string, persistedVersion)
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
    // The version is preserved — rotation is master-only, not
    // content-mutating, so bumping the version stamp here would
    // wrongly invalidate the just-produced sealed bytes against
    // their own AAD.
    //
    // #766 (#721-followup-c) note: this SQL write does NOT route
    // through `writeCellThroughEngine`. Engine apply would mint a new
    // VersionEntry via `merge_delta` (S1b #718) and bump the chain's
    // version_id from N to N+1. The rotated sealed bytes here carry
    // AAD=N (preserved per the contract above); after #767 lands and
    // sources `CellAddress.version` from `system(h, "cell_pin", …)`,
    // a chain at N+1 against AAD=N would fail every subsequent
    // `cellOpen` for this cell. Rotation must therefore stay
    // engine-silent — it's a key swap on the persistence layer, not
    // a logical mutation of the cell's contents.
    const blob = SEALED_CELL_PREFIX + bytesToBase64(newSealed)
    this.ctx.storage.sql.exec(
      `INSERT OR REPLACE INTO cell (id, type, data, version) VALUES (?, ?, ?, ?)`,
      row.id,
      row.type,
      blob,
      persistedVersion,
    )
    // Invalidate the memoised master so subsequent calls re-derive
    // from whichever seed `env` exposes (the orchestrator promotes
    // v2 → v1 after the walk completes).
    this.master = null
    return { ok: true, rotated: true }
  }
}

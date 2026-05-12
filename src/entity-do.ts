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
import { compileDomainReadings, freezeHandle, thawHandle, release_domain, system, callCellPin, callFetchCell } from './api/engine'
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
  // ⟨id, type, data⟩ — the cell's contents per AREST.tex §202. The
  // chain (per-DO engine) IS the version-of-record per §462 eq:cellfold
  // and §472 ("hash chain provides ordering"); cell.version was the
  // divergent SQL sidecar §3.3 warns against. As of #768 the column is
  // gone — `aadVersionFor` sources the AEAD AAD `version` field from
  // `system(h, "cell_pin", cellName)` exclusively (#767 / S1e + #770).
  sql.exec(`CREATE TABLE IF NOT EXISTS cell (
    id TEXT PRIMARY KEY,
    type TEXT NOT NULL,
    data TEXT NOT NULL DEFAULT '{}'
  )`)

  // ── Migration: drop the legacy `version` column (#768, #721-followup-e) ──
  //
  // Pre-#768 DOs persisted a per-cell monotonic counter in `cell.version`
  // and folded it into the AEAD AAD. With the engine chain shipped
  // (#764–#770), that counter is the divergent sidecar AREST.tex §202
  // / eq:cellfold rules out — chain head from `cell_pin` is now the
  // canonical source. Drop the column on existing DOs; idempotent-by-trial
  // (SQLite raises "no such column" once the column has already been
  // dropped, or when CREATE TABLE above ran fresh without it).
  try {
    sql.exec(`ALTER TABLE cell DROP COLUMN version`)
  } catch {
    // Column already absent — expected on fresh DOs (CREATE TABLE above
    // omits `version`) and on second-call idempotency for migrated DOs.
  }
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

/** ↑n via engine — try the per-DO engine `fetch_cell` system verb
 *  (#765, S1c eq:cellfold) for the cell named `cellName`, with SQL
 *  fallback through `fetchCell` for cells the engine does not yet
 *  know about (legacy cells written before #766 wired engine apply,
 *  or cells whose direct-SQL writes — e.g. `rotateMaster` — are
 *  intentionally engine-silent so the chain version stays consistent
 *  with the preserved AAD version on rotated bytes).
 *
 *  Engine-first means a chain-resident cell's contents come straight
 *  from the engine snapshot instead of the worker's SQLite sidecar —
 *  this is the read-side closure of the chain-as-version-of-record
 *  contract that #766 (write path) and #767 (AEAD AAD source) were
 *  building toward.
 *
 *  Engine return shape adaptation: `system(h, "fetch_cell", name)`
 *  hands back the cell's contents JSON via the engine's
 *  `to_json_string` (atom payloads → JSON string, Maps → object,
 *  Seqs → array). Worker EntityDB cells are conventionally written
 *  through `EntityDB.put` → `writeCellThroughEngine` → `createEntity`
 *  — that path stores facts under fact-type cells (e.g.
 *  `Order_has_total`) and does NOT today populate a per-entity-id
 *  cell shaped `{id, type, data}`. So for `cellName = entity-id`
 *  the engine returns `⊥` for those cells and the SQL fallback
 *  fires — exactly the "class (b)/(c) legacy cells" path the brief
 *  calls out.
 *
 *  When a future engine surface DOES register an entity-id cell with
 *  the `{id, type, data}` shape, the JSON we parse here matches
 *  `CellContents` directly. Defensive: anything that isn't shaped
 *  like `CellContents` (or that fails the JSON envelope check) routes
 *  through the SQL fallback so the DO never surfaces a malformed
 *  payload to its caller.
 *
 *  Both args default to the no-engine-bound case so legacy callers
 *  (the un-encrypted `EntityDB.get` path with no master, or unit
 *  tests driving `fetchCell` directly) keep working without surgery. */
export function fetchCellViaEngine(
  sql: SqlLike,
  engineHandle: number = -1,
  cellName: string = '',
): CellContents | null {
  if (engineHandle >= 0 && cellName.length > 0) {
    const fromEngine = callFetchCell(engineHandle, cellName)
    const adapted = adaptEngineCellPayload(fromEngine)
    if (adapted !== null) return adapted
  }
  return fetchCell(sql)
}

/** Best-effort coercion of a `callFetchCell` return value into the
 *  `CellContents` shape. Returns `null` for any payload that isn't
 *  recognisably a `{id, type, data}` envelope — caller falls back
 *  to SQL.
 *
 *  The coercion is intentionally narrow: a future engine surface that
 *  stores entity cells will produce this exact shape. Until then,
 *  every entity-id-keyed `fetch_cell` returns the engine's `⊥` (which
 *  `callFetchCell` already maps to `null`) and the body of this helper
 *  is dead code. Keeping the shape check explicit means the moment
 *  the engine grows entity-cell semantics, reads route through the
 *  engine path with zero call-site changes. */
function adaptEngineCellPayload(payload: unknown): CellContents | null {
  if (!payload || typeof payload !== 'object' || Array.isArray(payload)) return null
  const m = payload as Record<string, unknown>
  if (typeof m.id !== 'string' || typeof m.type !== 'string') return null
  const data = m.data
  if (data !== null && data !== undefined && typeof data !== 'object') return null
  return {
    id: m.id,
    type: m.type,
    data: (data as Record<string, unknown>) ?? {},
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
 *  the chain head version_id sourced from `system(h, "cell_pin", id)`
 *  (#767 / S1e + #770). With the per-DO engine chain shipped, the
 *  chain IS the version stamp per AREST.tex §462 eq:cellfold and §472
 *  ("hash chain provides ordering") — so the worker no longer mints a
 *  parallel SQL counter (the divergent sidecar §3.3 / §202 warns against).
 *
 *  Pre-#770 cells (chain has no entry for `id` yet) default to version 0
 *  via `aadVersionFor`'s fallback; the first successful sealed write
 *  through `storeCellSealed` materialises a chain entry through the
 *  engine apply path (#766) and the AAD `version` jumps to whatever
 *  chain head `cell_pin` reports next. */
export function cellAddressFor(type: string, id: string, version: number = 0): CellAddress {
  return {
    scope: 'worker',
    domain: type,
    cellName: id,
    version,
  }
}

/** Resolve the AAD `version` field for a sealed cell from the per-DO
 *  engine's `cell_pin` chain head for `cellName` (#767 / S1e + #770).
 *
 *  As of #768 there is no SQL fallback — the worker `cell.version`
 *  column is gone. When `engineHandle < 0` (no engine bound, e.g. a
 *  unit test exercising the legacy-only path before hydrate) or when
 *  the engine has no chain entry yet for this cell, the helper returns
 *  `0` (the eq:cellfold "empty fold" baseline). Once the cell's first
 *  sealed write lands on the chain via #766's `writeCellThroughEngine`,
 *  `cell_pin` reports the chain head and subsequent reads pick it up.
 *
 *  Note: the AEAD AAD binds version_id by construction (S1i #725) —
 *  this helper sources the field from the chain so the worker matches
 *  the engine's storage version_id exactly. */
export function aadVersionFor(
  engineHandle: number,
  cellName: string,
): number {
  if (engineHandle < 0) return 0
  const pinned = callCellPin(engineHandle, cellName)
  return pinned ?? 0
}

/** ↑n — fetch the cell, decrypting via AEAD. Returns the same shape
 *  as `fetchCell` so callers can swap the helper without touching
 *  their consumers.
 *
 *  ## Fail-loud contract (#888 / part of #777 + #779 A-9)
 *
 *  AEAD encoding is part of the CellSource adapter's payload codec;
 *  the worker MUST NOT branch on it. Pre-#888 a row missing the
 *  `SEALED_CELL_PREFIX` was silently parsed as legacy plaintext —
 *  a security foot-gun in a supposedly-encrypted store (#779 A-9).
 *  After #888 cells either decode via AEAD or fail loudly: a missing
 *  prefix throws naming the affected cell, and `cellOpen` failures
 *  propagate untouched so the operator sees the broken-row condition
 *  instead of a quiet plaintext leak.
 *
 *  ## AAD version source (#767 / S1e + #768)
 *
 *  The AAD `version` field is sourced from the engine's `cell_pin`
 *  chain head for this cell — per AREST.tex §462 eq:cellfold and §472
 *  the chain IS the version stamp. As of #768 there is no SQL fallback
 *  (the `cell.version` column has been dropped); when the engine has
 *  no chain entry yet, `aadVersionFor` returns `0` (the empty-fold
 *  baseline). Pass `-1` for `engineHandle` in the no-engine path
 *  (legacy unit tests) — the AAD then resolves to `0`, matching what
 *  `storeCellSealed` would have sealed under at the same call. */
export async function fetchCellSealed(
  sql: SqlLike,
  master: TenantMasterKey,
  engineHandle: number = -1,
): Promise<CellContents | null> {
  const rows = sql.exec(`SELECT id, type, data FROM cell`).toArray()
  if (rows.length === 0) return null
  const row = rows[0] as Record<string, any>
  const dataField: unknown = row.data
  if (typeof dataField !== 'string' || !dataField.startsWith(SEALED_CELL_PREFIX)) {
    // Fail-loud: no plaintext fallback. A supposedly-encrypted store
    // that silently surfaced raw JSON for a missing prefix is the
    // exact foot-gun #779 A-9 flagged. Throw naming the cell so the
    // operator can locate it; downstream callers (EntityDB.get,
    // rotateMaster, fact projection) propagate the failure rather
    // than degrade to plaintext.
    const id = typeof row.id === 'string' ? row.id : '<unknown>'
    throw new Error(
      `fetchCellSealed: cell "${id}" is not sealed (missing ${SEALED_CELL_PREFIX} prefix). ` +
        'Encrypted-store reads cannot fall back to plaintext (#888 / #779 A-9). ' +
        'Cause: a pre-encryption legacy write, an out-of-band SQL edit, or a ' +
        'truncated row. Investigate the row before attempting recovery; do not ' +
        'silently re-seal contents whose provenance is unknown.',
    )
  }
  const sealed = base64ToBytes(dataField.slice(SEALED_CELL_PREFIX.length))
  const aadVersion = aadVersionFor(engineHandle, row.id as string)
  const address = cellAddressFor(row.type as string, row.id as string, aadVersion)
  const opened = await cellOpen(master, address, sealed)
  const json = new TextDecoder().decode(opened)
  const data = JSON.parse(json) as Record<string, unknown>
  return {
    id: row.id,
    type: row.type,
    data,
  }
}

/** ↑n via engine — try the per-DO engine `fetch_cell` system verb
 *  (#765, S1c eq:cellfold) for the sealed-cell path, with the existing
 *  `fetchCellSealed` (decrypt-from-SQL) as the fallback for cells the
 *  engine does not yet know about.
 *
 *  ## Why route encrypted reads through the engine
 *
 *  Per the chain-as-version-of-record contract (#766/#767), a cell
 *  written through `EntityDB.put` lands in the engine's chain via
 *  `apply` (currently as facts under fact-type cells, not as a
 *  per-entity envelope — see `fetchCellViaEngine` for the shape
 *  details). When/if the engine surface grows entity-keyed cells,
 *  reads of those cells should source from the engine snapshot
 *  rather than the worker's encrypted SQL row. The wiring lands
 *  here so the moment that surface ships, encrypted reads inherit
 *  it for free.
 *
 *  ## Today's behaviour: SQL fallback dominant
 *
 *  Today's engine stores by fact-type, not by entity-id, so
 *  `callFetchCell(handle, entityId)` returns null for all current
 *  worker cells. The fallback always fires and the encrypted SQL
 *  decrypt path runs as before — no behaviour change for the live
 *  EntityDB. The engine call is cheap (a snapshot read under a
 *  shared lock per `is_read_only_op("fetch_cell")`) so the wiring
 *  cost is negligible.
 *
 *  ## Backward-compat for class (b)/(c) cells
 *
 *  - (a) Cells written via #766 engine apply path: chain-resident.
 *    `fetch_cell` returns contents when the surface supports it
 *    (today: still ⊥ for entity ids).
 *  - (b) Legacy cells written by direct SQL pre-#766: engine has no
 *    chain entry. `fetch_cell` returns ⊥ → fallback opens the SQL
 *    sealed bytes.
 *  - (c) `rotateMaster` re-sealed bytes (engine-silent by design):
 *    same fallback path as (b).
 *
 *  All three classes remain readable across the migration window
 *  before #768 drops the `cell.version` SQL column. */
export async function fetchCellSealedViaEngine(
  sql: SqlLike,
  master: TenantMasterKey,
  engineHandle: number = -1,
  cellName: string = '',
): Promise<CellContents | null> {
  if (engineHandle >= 0 && cellName.length > 0) {
    const fromEngine = callFetchCell(engineHandle, cellName)
    const adapted = adaptEngineCellPayload(fromEngine)
    if (adapted !== null) return adapted
  }
  return fetchCellSealed(sql, master, engineHandle)
}

/** ↓n — store new contents into the cell, sealing the JSON-encoded
 *  data column with the per-tenant master before the SQL write.
 *  The encrypted bytes go into the same `data` TEXT column, prefixed
 *  with `SEALED_CELL_PREFIX` so `fetchCellSealed` / `fetchCell` can
 *  tell encrypted rows from legacy plaintext.
 *
 *  ## AAD version source (#767 / S1e + #768 + #770)
 *
 *  The CellAddress AAD `version` is sourced from the engine's
 *  `cell_pin` chain head for this cell — per AREST.tex §462 eq:cellfold
 *  and §472 the chain IS the version stamp. Caller (`EntityDB.put`)
 *  routes the write through `writeCellThroughEngine` BEFORE invoking
 *  this helper, so the chain has already extended and `cell_pin`
 *  reports the new head; this seal then binds that head into the AAD
 *  and the matching `fetchCellSealed` path (which sources the same
 *  head) recovers the plaintext.
 *
 *  Pre-#770 ordering note: when the engine has no chain entry for the
 *  cell yet (e.g. apply path didn't materialise an entity cell — the
 *  pre-#770 fact-keyed-only world), `aadVersionFor` returns `0` and
 *  read/write both bind `0` so the round-trip still works; once #770's
 *  per-entity chain materialisation lands the live behaviour the AAD
 *  threads non-zero version_ids. */
export async function storeCellSealed(
  sql: SqlLike,
  master: TenantMasterKey,
  id: string,
  type: string,
  data: Record<string, unknown>,
  engineHandle: number = -1,
): Promise<CellContents> {
  const json = JSON.stringify(data)
  // AAD version comes straight from the chain head. The engine apply
  // call (in `EntityDB.put`) runs BEFORE us so the chain has already
  // extended; `fetchCellSealed`'s mirror call to `aadVersionFor` will
  // surface the same head and the AEAD round-trip closes.
  const aadVersion = aadVersionFor(engineHandle, id)
  const address = cellAddressFor(type, id, aadVersion)
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
   *  legacy direct-SQL paths (`fetchCell` / `storeCell` / etc.)
   *  remain alongside the engine path; #765/#766 routed reads/writes
   *  through engine, but removing the SQL fall-back is gated on
   *  Sweep-6e/6f (#801, #802). */
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
   *  #767 lands. */
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
   *  Hard-fails (throws) if the secret is missing in production
   *  (`Sweep-8a` / task #809). The legacy "silently fall back to
   *  plaintext" behaviour was a 1.0 security foot-gun: an
   *  "encrypted" DO that quietly stored cell payloads in cleartext
   *  whenever the deploy step forgot `wrangler secret put
   *  TENANT_MASTER_SEED`. Production cannot run plaintext.
   *
   *  Plaintext is only permitted under an explicit dev opt-in:
   *  `AREST_ALLOW_PLAINTEXT === '1'` AND `ENVIRONMENT !==
   *  'production'`. Tests that exercise the EntityDB encrypted
   *  path MUST set `TENANT_MASTER_SEED` in the mocked env. */
  private async getMaster(): Promise<TenantMasterKey | null> {
    if (this.master) return this.master
    const env = this.env as
      | {
          TENANT_MASTER_SEED?: string
          ENVIRONMENT?: string
          AREST_ALLOW_PLAINTEXT?: string
        }
      | undefined
    const seed = env?.TENANT_MASTER_SEED
    if (!seed) {
      const isProd = env?.ENVIRONMENT === 'production'
      const plaintextOptIn = env?.AREST_ALLOW_PLAINTEXT === '1'
      if (isProd || !plaintextOptIn) {
        throw new Error(
          'EntityDB.getMaster: TENANT_MASTER_SEED is not bound. ' +
            'Cell storage cannot fall back to plaintext in production. ' +
            'Fix: run `wrangler secret put TENANT_MASTER_SEED` (and re-deploy) ' +
            'so per-tenant masters can be derived. ' +
            'For local dev only, you may opt in to the legacy plaintext ' +
            'path by setting both `ENVIRONMENT != "production"` and ' +
            '`AREST_ALLOW_PLAINTEXT=1` in your env (never do this in prod).',
        )
      }
      // Dev-only legacy plaintext path: explicit opt-in + non-prod.
      return null
    }
    // The DO's id name is the tenant routing key (per-cell DO mapping
    // #217). Use it as the salt so each tenant derives a distinct
    // master from the same shared seed.
    const tenantSalt = this.ctx.id.toString()
    const m = await deriveTenantMasterKey(seed, tenantSalt)
    this.master = m
    return m
  }

  /** ↑n — fetch the cell. Returns { id, type, data } or null.
   *
   *  Hydrates the per-DO engine so:
   *    - the sealed-row path can source the AEAD AAD `version` field
   *      from `cell_pin` (#767/S1e) instead of the worker SQL counter,
   *    - the read can route through `system(h, "fetch_cell", name)`
   *      (#765) for chain-resident cells, with the SQL `SELECT id,
   *      type, data` as the fallback for cells the engine does not
   *      yet know about (legacy class (b) + rotated class (c) — see
   *      `fetchCellViaEngine`/`fetchCellSealedViaEngine`).
   *
   *  The `cellName` we hand the engine is `this.ctx.id.toString()` —
   *  the DO's routing identifier (the cellKey-formatted name from
   *  `src/api/cell-key.ts`). Today's engine surface stores facts
   *  under fact-type cells (`Order_has_total` etc.), not under
   *  entity-id, so `callFetchCell` returns `⊥` for these names and
   *  the SQL fallback fires. The wiring lands so that the moment
   *  the engine grows entity-keyed cells, reads inherit the
   *  chain-as-version-of-record path with zero call-site changes. */
  async get(): Promise<CellContents | null> {
    this.ensureInit()
    const master = await this.getMaster()
    await this.hydrateEngine()
    const cellName = this.ctx.id.toString()
    if (master) {
      return fetchCellSealedViaEngine(
        this.ctx.storage.sql, master, this.engineHandle, cellName,
      )
    }
    return fetchCellViaEngine(
      this.ctx.storage.sql, this.engineHandle, cellName,
    )
  }

  /** ↓n — store the cell. Merges with existing data (idempotent across domains).
   *
   *  Hydrates the per-DO engine so the sealed-row path can source the
   *  AEAD AAD `version` field from `cell_pin` (#767/S1e), and so the
   *  read of existing contents (for the merge) routes through
   *  `system(h, "fetch_cell", name)` (#765) with SQL fallback. */
  async put(input: { id: string; type: string; data: Record<string, unknown> }): Promise<CellContents> {
    this.ensureInit()
    const master = await this.getMaster()
    await this.hydrateEngine()
    const cellName = this.ctx.id.toString()
    const existing = master
      ? await fetchCellSealedViaEngine(
          this.ctx.storage.sql, master, this.engineHandle, cellName,
        )
      : fetchCellViaEngine(
          this.ctx.storage.sql, this.engineHandle, cellName,
        )
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
      // Engine apply failure is non-fatal: the SQL write below
      // remains the authoritative store. Removing this fall-back is
      // tracked under Sweep-6e/6f (#801, #802) — gated on engine-only
      // path stabilization.
      // eslint-disable-next-line no-console
      console.warn('EntityDB.put: engine apply failed, falling back to SQL-only write:', e)
    }
    if (master) {
      return storeCellSealed(this.ctx.storage.sql, master, input.id, input.type, merged, this.engineHandle)
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
      await this.hydrateEngine()
      const cell = await fetchCellSealed(this.ctx.storage.sql, master, this.engineHandle)
      return factsFromCell(cell)
    }
    return getFacts(this.ctx.storage.sql)
  }

  async getFactsBySchema(graphSchemaId: string): Promise<Fact[]> {
    this.ensureInit()
    const master = await this.getMaster()
    if (master) {
      await this.hydrateEngine()
      const cell = await fetchCellSealed(this.ctx.storage.sql, master, this.engineHandle)
      return factsFromCell(cell).filter(f => f.graphSchemaId === graphSchemaId)
    }
    return getFactsBySchema(this.ctx.storage.sql, graphSchemaId)
  }

  async toPopulation(): Promise<Record<string, Array<{ factTypeId: string; bindings: Array<[string, string]> }>>> {
    this.ensureInit()
    const master = await this.getMaster()
    if (master) {
      await this.hydrateEngine()
      const cell = await fetchCellSealed(this.ctx.storage.sql, master, this.engineHandle)
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
  //   - `{ ok: true, rotated: false }` when the cell row is absent
  //     (nothing to rotate)
  //   - `{ ok: false, kind: 'truncated' | 'auth' }` when the row is
  //     present but cannot be opened under the old master — either
  //     because the sealed envelope is missing (#888 / #779 A-9: a
  //     row without the `SEALED_CELL_PREFIX` is no longer treated as
  //     a benign no-op; it surfaces as `truncated`) or because the
  //     AEAD tag check fails. The row is left untouched; operator
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
    // Hydrate the per-DO engine FIRST so:
    //   - the AAD `version` field can be sourced from `cell_pin`
    //     (#767/S1e) — matching whatever the sealing path used to
    //     mint the AAD,
    //   - the cell-contents read can route through
    //     `system(h, "fetch_cell", name)` (#765) for engine-resident
    //     cells, with the SQL row as the fallback (which rotation
    //     ALWAYS still requires for the sealed bytes column —
    //     engine returns plaintext contents, but rotation needs the
    //     raw AEAD envelope to decrypt+re-encrypt under the new
    //     master). The engine probe is the read-pattern bookend per
    //     #765's contract; in practice rotation cells are always in
    //     the (c) class (engine-silent rewrites) so the probe
    //     surfaces `⊥` and the SQL row is authoritative anyway.
    await this.hydrateEngine()
    const cellName = this.ctx.id.toString()
    // Engine-first probe per #765 read-pattern contract. Even when
    // the probe returns non-null (e.g. a future surface registers an
    // entity-id cell), rotation MUST still read the SQL row — the
    // engine returns plaintext cell contents but rotation needs the
    // SEALED bytes (data column) and the persisted version stamp
    // (version column) to decrypt+re-encrypt under the new master.
    // In practice rotation cells are always class (c) (engine-silent
    // rewrites preserved per the task brief), so the probe surfaces
    // `⊥` and the SQL row is authoritative. The probe stays wired
    // so the moment a sibling task adds an engine surface for the
    // sealed envelope (or the rotated bytes start landing in the
    // chain through a key-rotation-safe apply), we can collapse the
    // SQL fallback.
    if (this.engineHandle >= 0) {
      // Result intentionally discarded — informational probe only.
      // Keeps the call-site shape uniform with `EntityDB.get`/`put`.
      callFetchCell(this.engineHandle, cellName)
    }
    const rows = this.ctx.storage.sql
      .exec(`SELECT id, type, data FROM cell`)
      .toArray()
    if (rows.length === 0) {
      return { ok: true, rotated: false }
    }
    const row = rows[0] as Record<string, any>
    const dataField = row.data as unknown
    if (typeof dataField !== 'string' || !dataField.startsWith(SEALED_CELL_PREFIX)) {
      // Fail-loud: a row that exists but lacks the sealed prefix is
      // not a benign no-op — it's a corrupted or pre-encryption cell
      // that the operator must inspect before rotation. Surfacing
      // `truncated` flows the broken-row condition into the rotation
      // orchestrator's report instead of silently skipping (#888 /
      // #779 A-9: the plaintext-fallback foot-gun on rotation).
      return { ok: false, kind: 'truncated' }
    }
    const oldMaster = await deriveTenantMasterKey(args.oldSeed, args.oldSalt)
    const newMaster = await deriveTenantMasterKey(args.newSeed, args.newSalt)
    const sealed = base64ToBytes(dataField.slice(SEALED_CELL_PREFIX.length))
    // AAD source for rotation is the engine's chain head per #768 +
    // #770. With the worker `cell.version` column gone, the chain IS
    // the canonical version stamp (AREST.tex §462 eq:cellfold / §472).
    // Rotation does NOT extend the chain — it's a key swap on the
    // persistence layer, not a logical mutation of the cell's contents
    // — so the chain head observed here is the SAME version_id the
    // sealed bytes were originally produced under, which is exactly
    // what AEAD AAD reconstruction needs. The next `EntityDB.get` call
    // sources the same `cell_pin` value when computing the AAD for the
    // re-encrypted bytes; round-trip closes.
    const aadVersion = aadVersionFor(this.engineHandle, row.id as string)
    const address = cellAddressFor(row.type as string, row.id as string, aadVersion)
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
    //
    // Engine-silent by design — `writeCellThroughEngine` would mint a
    // new VersionEntry via `merge_delta` (S1b #718) and bump the
    // chain's version_id from N to N+1, while the rotated sealed
    // bytes still carry AAD=N. With AAD now sourced exclusively from
    // `cell_pin` (#768), a chain at N+1 against AAD=N would fail
    // every subsequent `cellOpen` for this cell. Rotation MUST stay
    // engine-silent so the chain head + AAD version stay in lockstep.
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

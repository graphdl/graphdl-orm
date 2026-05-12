/**
 * EntityDB engine-routed cell reads (#765, #721-followup-b)
 *
 * #764 stood up the per-DO engine handle + freeze/thaw lifecycle.
 * #766 wired the productive cell-write path (`EntityDB.put`) through
 * the engine's `apply` system verb. #767 sourced CellAddress's AAD
 * `version` field from the engine's `cell_pin`. #765 closes the
 * read-side bookend: cell reads route through `system(h, "fetch_cell",
 * name)` (engine prereq commit 11fdc47c) with SQL fallback for cells
 * the engine does not yet know about.
 *
 * The chain-as-version-of-record contract (AREST.tex §202, §462
 * eq:cellfold) requires that BOTH writes AND reads talk to the engine
 * snapshot, otherwise the worker's per-cell SQLite sidecar drifts
 * from the engine's chain. This suite pins the read-path wiring:
 *
 *   1. Engine-resident cells: `callFetchCell` returns the contents
 *      directly (mocked here because today's engine stores facts
 *      under fact-type cells, not under entity-id; the WIRE
 *      contract is what we assert).
 *   2. Class (b) legacy cells: `callFetchCell` returns null (⊥) →
 *      SQL fallback opens the legacy plaintext / sealed row.
 *   3. Class (c) `rotateMaster`-rewritten cells: same fallback path
 *      as (b) — engine-silent rotation preserves chain version.
 *   4. Missing cells: read returns null gracefully (no throw) on
 *      both engine-bound and engine-unbound paths.
 *
 * The wiring lands so the moment a sibling task adds an engine
 * surface for entity-keyed cells (per the brief: "When/if the engine
 * surface grows entity-keyed cells…"), reads inherit the
 * chain-as-version-of-record path with zero call-site changes.
 */

import { describe, it, expect, vi, beforeEach } from 'vitest'
import { EntityDB, fetchCell, fetchCellViaEngine, fetchCellSealedViaEngine, storeCell } from './entity-do'
import { deriveTenantMasterKey } from './cell-encryption'

// ── Mock DO state (mirrors entity-do-engine-apply.test.ts shape) ────
//
// The mock SQL surface tracks INSERT OR REPLACE rows in-memory so
// `EntityDB.put` followed by `EntityDB.get` returns the prior write.
// Same shape as the apply-write suite — kept verbatim so the two
// suites stay in lockstep when either evolves.

interface MockStorage {
  data: Map<string, unknown>
  get<T = unknown>(key: string): Promise<T | undefined>
  put(key: string, value: unknown): Promise<void>
  delete(key: string): Promise<void>
  sql: { exec(query: string, ...params: any[]): { toArray(): unknown[] } }
}

function createMockStorage(): MockStorage {
  const tables: Record<string, any[]> = {}
  return {
    data: new Map<string, unknown>(),
    async get<T = unknown>(key: string): Promise<T | undefined> {
      return this.data.get(key) as T | undefined
    },
    async put(key: string, value: unknown): Promise<void> {
      this.data.set(key, value)
    },
    async delete(key: string): Promise<void> {
      this.data.delete(key)
    },
    sql: {
      exec(query: string, ...params: any[]) {
        const n = query.replace(/\s+/g, ' ').trim()
        if (/^CREATE/i.test(n)) {
          const m = n.match(/(?:TABLE|INDEX)\s+(?:IF\s+NOT\s+EXISTS\s+)?(\w+)/i)
          if (m && !tables[m[1]]) tables[m[1]] = []
          return { toArray: () => [] }
        }
        if (/^ALTER/i.test(n)) {
          throw new Error('column already exists')
        }
        if (/^DROP/i.test(n)) return { toArray: () => [] }
        if (/INSERT OR REPLACE/i.test(n)) {
          const m = n.match(/INSERT OR REPLACE INTO (\w+)\s*\(([^)]+)\)\s*VALUES/i)
          if (m) {
            const t = m[1]
            const cols = m[2].split(',').map(c => c.trim())
            if (!tables[t]) tables[t] = []
            const row: any = {}
            cols.forEach((c, i) => row[c] = params[i])
            const idx = tables[t].findIndex((r: any) => r[cols[0]] === row[cols[0]])
            if (idx >= 0) tables[t][idx] = row
            else tables[t].push(row)
          }
          return { toArray: () => [] }
        }
        if (/^DELETE FROM cell$/i.test(n)) {
          tables['cell'] = []
          return { toArray: () => [] }
        }
        if (/^SELECT id, noun, fields FROM entity/i.test(n)) {
          return { toArray: () => [] }
        }
        if (/^SELECT id, type, data FROM cell$/i.test(n)) {
          return { toArray: () => [...(tables['cell'] || [])] }
        }
        if (/^SELECT id, type, data, version FROM cell/i.test(n)) {
          return { toArray: () => [...(tables['cell'] || [])] }
        }
        if (/^SELECT version FROM cell WHERE id = \?/i.test(n)) {
          return {
            toArray: () => (tables['cell'] || [])
              .filter((r: any) => r.id === params[0])
              .map((r: any) => ({ version: r.version ?? 0 })),
          }
        }
        return { toArray: () => [] }
      },
    },
  }
}

interface MockCtx {
  id: { toString(): string }
  storage: MockStorage
}

function createMockCtx(idName = 'test-cell-id'): MockCtx {
  return {
    id: { toString: () => idName },
    storage: createMockStorage(),
  }
}

function makeEntityDB(ctx: MockCtx, env: Record<string, unknown> = {}): EntityDB {
  const db = new (EntityDB as unknown as new () => EntityDB)()
  ;(db as unknown as { ctx: MockCtx }).ctx = ctx
  // Post-#888/#902 (`getMaster` fail-loud), an env without
  // `TENANT_MASTER_SEED` and without the explicit dev-only
  // `AREST_ALLOW_PLAINTEXT=1` opt-in throws on the first `db.put` /
  // `db.get`. The #765 cases below predate that hardening and were
  // written against the legacy plaintext-default path; they pin the
  // engine-routed read wiring (engine-payload preference, SQL
  // fallback for class (b)/(c) legacy cells, missing-cell ⊥
  // graceful-null), NOT the AEAD AAD agreement contract (that's the
  // #803 sealed-path case in the round-trip suite, and the
  // sealed-cell SQL fallback test below explicitly constructs its own
  // master via `deriveTenantMasterKey` instead of going through
  // EntityDB.getMaster). Default the env to the dev opt-in so the
  // read-path wiring contracts these tests pin remain observable
  // through the plaintext branch — mirrors the #803 (commit
  // 0f47be81) drive-by fix in the round-trip suite. Callers that want
  // the sealed-cell EntityDB path bind `TENANT_MASTER_SEED`
  // explicitly to override.
  const mergedEnv: Record<string, unknown> = { AREST_ALLOW_PLAINTEXT: '1', ...env }
  ;(db as unknown as { env: Record<string, unknown> }).env = mergedEnv
  return db
}

// Compiling the bundled metamodel under vitest takes ~1-2 s on Node,
// matching the lifecycle / apply suites. #885 bumps the ceiling
// because the engine `apply` path now panics inside `merge_delta`
// on vitest's wasm32 SystemTime gap, and the wasm-bindgen
// panic_hook's backtrace serialisation through
// `console_error_panic_hook` adds 30-90 s of unwinding on top of
// the compile when the apply throws.
const COMPILE_TIMEOUT_MS = 240_000

// ── Tests ───────────────────────────────────────────────────────────

describe('EntityDB engine-routed cell reads (#765)', () => {
  let ctx: MockCtx
  let db: EntityDB

  beforeEach(() => {
    ctx = createMockCtx('cust-' + Math.random().toString(36).slice(2))
    db = makeEntityDB(ctx)
  })

  // ── Engine-path round-trip ─────────────────────────────────────────
  //
  // Write a cell via the engine apply path (#766's `EntityDB.put`),
  // then read via the engine path. Today's engine stores facts under
  // fact-type cells (Order_has_total) not entity-id cells, so the
  // engine-fetch returns null and the SQL fallback fires — the
  // round-trip works through the fallback. The brief allows this:
  //   "the SQL fallback... MUST stay live for as long as legacy cells
  //    exist in production storage."
  // The wiring is what we assert; the data-path resolution may shift
  // when the engine grows entity-cell semantics.

  it('round-trips a cell written via EntityDB.put through the engine-routed read', async () => {
    await db.put({ id: 'ord-1', type: 'Order', data: { total: '99', status: 'open' } })
    const cell = await db.get()
    expect(cell).not.toBeNull()
    expect(cell!.id).toBe('ord-1')
    expect(cell!.type).toBe('Order')
    expect(cell!.data.total).toBe('99')
    expect(cell!.data.status).toBe('open')
  }, COMPILE_TIMEOUT_MS)

  it('engine payload (CellContents shape) is returned directly when callFetchCell yields one', async () => {
    // Mock callFetchCell to return a {id, type, data} envelope. The
    // engine prereq guarantees the verb returns parseable JSON for
    // cells that ARE chain-resident under #770's RMAP-augmented
    // surface. We pin the worker's wiring contract that the engine
    // value, when present, is surfaced directly.
    const engineMod = await import('./api/engine')
    const ENGINE_PAYLOAD = { id: 'cust-1', type: 'Customer', data: { name: 'Engine' } }
    const spy = vi.spyOn(engineMod, 'callFetchCell').mockReturnValue(ENGINE_PAYLOAD)
    try {
      // Drive through fetchCellViaEngine directly — bypasses the
      // EntityDB wrapper so the helper's contract is the unit under
      // test (the wrapper is exercised by the round-trip above).
      const result = fetchCellViaEngine(1 /* fakeHandle */, 'cust-1')
      expect(result).not.toBeNull()
      expect(result!.id).toBe('cust-1')
      expect(result!.type).toBe('Customer')
      expect(result!.data.name).toBe('Engine')
      expect(spy).toHaveBeenCalledWith(1, 'cust-1')
    } finally {
      spy.mockRestore()
    }
  })

  // ── Engine-only contract (#885 / #777) ─────────────────────────────
  //
  // As of #885 there is NO SQL fallback. `fetchCellViaEngine` returns
  // `null` when the engine doesn't know about the cell (the worker's
  // in-memory cell graph at `EntityDB.get` provides the read-after-
  // write closure within an isolate). The pre-#885 "class (b) legacy
  // direct-SQL cells" path is gone — every cell write extends the
  // chain through `apply` (#797 Map carrier → CommitDelta).

  it('null engine payload returns null (no SQL fallback as of #885)', async () => {
    // Even if a stray SQL row exists (e.g. from a pre-#885 deploy),
    // `fetchCellViaEngine` ignores it. The engine is the only read
    // source.
    storeCell(ctx.storage.sql, 'legacy-1', 'LegacyEntity', { fromSql: 'yes', count: 7 })
    const engineMod = await import('./api/engine')
    const spy = vi.spyOn(engineMod, 'callFetchCell').mockReturnValue(null)
    try {
      const result = fetchCellViaEngine(1 /* fakeHandle */, 'legacy-1')
      expect(result).toBeNull()
      expect(spy).toHaveBeenCalledWith(1, 'legacy-1')
    } finally {
      spy.mockRestore()
    }
  })

  it('no engine handle bound (engineHandle = -1) returns null without consulting SQL', () => {
    // Pre-#885 callers fell through to a SQL SELECT when no engine
    // handle was bound. Post-#885 the function is engine-only: a
    // missing handle yields null.
    storeCell(ctx.storage.sql, 'no-engine-1', 'Order', { total: '42' })
    const result = fetchCellViaEngine(-1, '')
    expect(result).toBeNull()
  })

  it('sealed-cell engine-only path: returns null when engine returns null (no SQL fallback)', async () => {
    // The sealed variant `fetchCellSealedViaEngine` mirrors
    // `fetchCellViaEngine` post-#885 — engine-only, no SQL fallback.
    // The `master` argument is retained for signature parity with the
    // rotation path's AEAD helpers but is unused on the engine read.
    const master = await deriveTenantMasterKey('test-seed-#765', 'tenant-aad')
    const engineMod = await import('./api/engine')
    const spy = vi.spyOn(engineMod, 'callFetchCell').mockReturnValue(null)
    try {
      const result = await fetchCellSealedViaEngine(
        master, 1 /* fakeHandle */, 'sealed-1',
      )
      expect(result).toBeNull()
      expect(spy).toHaveBeenCalledWith(1, 'sealed-1')
    } finally {
      spy.mockRestore()
    }
  })

  // ── Missing cells return null without throwing ─────────────────────

  it('missing cell: engine-path returns null when nothing was ever written', () => {
    // No engine handle, no engine chain entry. The DO has just been
    // spun up. Result: null, no throw, no crash.
    const result = fetchCellViaEngine(-1, '')
    expect(result).toBeNull()
  })

  it('missing cell: engine-bound path returns null when engine has no chain entry', async () => {
    const engineMod = await import('./api/engine')
    const spy = vi.spyOn(engineMod, 'callFetchCell').mockReturnValue(null)
    try {
      const result = fetchCellViaEngine(1 /* fakeHandle */, 'missing-1')
      expect(result).toBeNull()
      expect(spy).toHaveBeenCalledWith(1, 'missing-1')
    } finally {
      spy.mockRestore()
    }
  })

  it('missing cell via EntityDB.get: returns null gracefully on a fresh DO', async () => {
    // Black-box from the DO surface: no put has ever happened, so
    // the engine has no chain entry and the in-memory cell graph is
    // empty. Result: null, no throw. This is the contract the
    // worker dispatcher depends on (404 vs 500).
    const cell = await db.get()
    expect(cell).toBeNull()
  }, COMPILE_TIMEOUT_MS)

  // ── Engine envelope shape coercion ──────────────────────────────────
  //
  // The engine's `to_json_string` produces JSON for the cell's stored
  // contents shape. For an entity cell materialised under #770's
  // RMAP-augmented surface, JSON.parse yields exactly the
  // `CellContents` envelope. For ANYTHING else (a Seq of facts, an
  // atom string, a Map missing required fields), the worker returns
  // null — `EntityDB.get` then falls through to the in-memory cell
  // graph cache.

  it('coercion rejects non-envelope engine payloads (atom string)', async () => {
    const engineMod = await import('./api/engine')
    const spy = vi.spyOn(engineMod, 'callFetchCell').mockReturnValue('plain-atom-string')
    try {
      // The engine payload is a string, not an envelope —
      // `adaptEngineCellPayload` returns null.
      const result = fetchCellViaEngine(1, 'fallback-1')
      expect(result).toBeNull()
    } finally {
      spy.mockRestore()
    }
  })

  it('coercion rejects engine payloads missing the id field', async () => {
    const engineMod = await import('./api/engine')
    // Map shape but missing the `id` field — coercion rejects.
    const spy = vi.spyOn(engineMod, 'callFetchCell').mockReturnValue({ type: 'Order', data: { x: 1 } })
    try {
      const result = fetchCellViaEngine(1, 'fallback-2')
      expect(result).toBeNull()
    } finally {
      spy.mockRestore()
    }
  })

  // ── Direct fetchCell still works (sync legacy contract) ─────────────
  //
  // The pre-#765 free `fetchCell` stays sync for backward-compat with
  // free callers (`getFacts`, `toPopulation`) and unit tests that
  // drive it directly. Pin the contract so a future change doesn't
  // accidentally async-ify it.

  it('legacy fetchCell remains sync and SQL-only (back-compat for free callers)', () => {
    storeCell(ctx.storage.sql, 'sync-1', 'Order', { v: 'sync' })
    const result = fetchCell(ctx.storage.sql)
    // Type-level: result is `CellContents | null`, not `Promise`.
    // If this ever changes, the line below would need `await`.
    expect(result).not.toBeNull()
    expect(result!.id).toBe('sync-1')
    expect(result!.data.v).toBe('sync')
  })
})

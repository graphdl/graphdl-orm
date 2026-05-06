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
  ;(db as unknown as { env: Record<string, unknown> }).env = env
  return db
}

// Compiling the bundled metamodel under vitest takes ~1-2 s on Node,
// matching the lifecycle / apply suites.
const COMPILE_TIMEOUT_MS = 90_000

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

  it('engine-path return value is preferred over SQL when callFetchCell yields a CellContents shape', async () => {
    // Mock callFetchCell to return a {id, type, data} envelope. The
    // engine prereq guarantees the verb returns parseable JSON for
    // cells that ARE chain-resident in a future engine surface; we
    // pin the worker's wiring contract that the engine value, when
    // present, wins over the SQL row. Without this contract, a
    // chain-resident cell would silently fall through to its stale
    // SQL sidecar — exactly the divergence #765 is meant to close.
    const engineMod = await import('./api/engine')
    const ENGINE_PAYLOAD = { id: 'cust-1', type: 'Customer', data: { name: 'Engine' } }
    // First seed the SQL row with a DIFFERENT value so the test fails
    // if the SQL path wins. Then mock callFetchCell to return the
    // engine payload — engine value should be returned.
    storeCell(ctx.storage.sql, 'cust-1', 'Customer', { name: 'SQL' })
    const spy = vi.spyOn(engineMod, 'callFetchCell').mockReturnValue(ENGINE_PAYLOAD)
    try {
      // Drive through fetchCellViaEngine directly — bypasses the
      // EntityDB wrapper so the helper's contract is the unit under
      // test (the wrapper is exercised by the round-trip above).
      const result = fetchCellViaEngine(ctx.storage.sql, 1 /* fakeHandle */, 'cust-1')
      expect(result).not.toBeNull()
      expect(result!.id).toBe('cust-1')
      expect(result!.type).toBe('Customer')
      expect(result!.data.name).toBe('Engine')
      expect(spy).toHaveBeenCalledWith(1, 'cust-1')
    } finally {
      spy.mockRestore()
    }
  })

  // ── Backward-compat: SQL fallback for class (b)/(c) cells ──────────
  //
  // The brief calls out three cell classes: (a) engine-apply-written,
  // (b) pre-#766 legacy direct-SQL, (c) rotateMaster-rewritten
  // direct-SQL. Classes (b) + (c) MUST stay readable through the SQL
  // fallback for the migration window before #768 drops the
  // `cell.version` column and a future task migrates the chain.

  it('SQL fallback returns the legacy value when callFetchCell returns null', async () => {
    // Simulate a class (b) legacy cell: the SQL row exists (written
    // via direct SQL pre-#766) but the engine has no chain entry.
    // `callFetchCell` returns null → `fetchCellViaEngine` falls
    // through to `fetchCell(sql)` → the legacy row surfaces.
    storeCell(ctx.storage.sql, 'legacy-1', 'LegacyEntity', { fromSql: 'yes', count: 7 })
    const engineMod = await import('./api/engine')
    const spy = vi.spyOn(engineMod, 'callFetchCell').mockReturnValue(null)
    try {
      const result = fetchCellViaEngine(ctx.storage.sql, 1 /* fakeHandle */, 'legacy-1')
      expect(result).not.toBeNull()
      expect(result!.id).toBe('legacy-1')
      expect(result!.type).toBe('LegacyEntity')
      expect(result!.data.fromSql).toBe('yes')
      expect(result!.data.count).toBe(7)
      expect(spy).toHaveBeenCalledWith(1, 'legacy-1')
    } finally {
      spy.mockRestore()
    }
  })

  it('SQL fallback fires when no engine handle is bound (engineHandle = -1)', async () => {
    // The default no-engine path: legacy callers (un-encrypted DOs
    // built before the per-DO engine landed) keep working without
    // surgery. The engine call is short-circuited by the
    // `engineHandle >= 0` guard.
    storeCell(ctx.storage.sql, 'no-engine-1', 'Order', { total: '42' })
    const result = fetchCellViaEngine(ctx.storage.sql, -1, '')
    expect(result).not.toBeNull()
    expect(result!.id).toBe('no-engine-1')
    expect(result!.data.total).toBe('42')
  })

  it('sealed-cell SQL fallback round-trips when engine returns null', async () => {
    // Class (b) for the encrypted path: SQL row carries the sealed
    // envelope, engine has no chain entry, `fetchCellSealedViaEngine`
    // falls through to `fetchCellSealed` which decrypts the SQL row.
    // Mirrors the rotateMaster (class c) path — same code path.
    const master = await deriveTenantMasterKey('test-seed-#765', 'tenant-aad')
    // Seed via a real seal/store so the SQL row has a valid sealed
    // envelope — we can't hand-craft one because the AAD includes
    // the per-cell HKDF derivation.
    const { storeCellSealed } = await import('./entity-do')
    await storeCellSealed(
      ctx.storage.sql, master, 'sealed-1', 'Order',
      { lane: 'sealed-fallback' }, -1,
    )
    const engineMod = await import('./api/engine')
    const spy = vi.spyOn(engineMod, 'callFetchCell').mockReturnValue(null)
    try {
      const result = await fetchCellSealedViaEngine(
        ctx.storage.sql, master, 1 /* fakeHandle */, 'sealed-1',
      )
      expect(result).not.toBeNull()
      expect(result!.id).toBe('sealed-1')
      expect(result!.type).toBe('Order')
      expect(result!.data.lane).toBe('sealed-fallback')
      expect(spy).toHaveBeenCalledWith(1, 'sealed-1')
    } finally {
      spy.mockRestore()
    }
  })

  // ── Missing cells return null without throwing ─────────────────────

  it('missing cell: engine-path returns null when nothing was ever written', () => {
    // No SQL row, no engine chain entry. The DO has just been spun
    // up. Both paths return null cleanly — no throw, no crash.
    const result = fetchCellViaEngine(ctx.storage.sql, -1, '')
    expect(result).toBeNull()
  })

  it('missing cell: engine-bound path returns null when engine and SQL both have nothing', async () => {
    const engineMod = await import('./api/engine')
    const spy = vi.spyOn(engineMod, 'callFetchCell').mockReturnValue(null)
    try {
      const result = fetchCellViaEngine(ctx.storage.sql, 1 /* fakeHandle */, 'missing-1')
      expect(result).toBeNull()
      expect(spy).toHaveBeenCalledWith(1, 'missing-1')
    } finally {
      spy.mockRestore()
    }
  })

  it('missing cell via EntityDB.get: returns null gracefully on a fresh DO', async () => {
    // Black-box from the DO surface: no put has ever happened, so
    // SQL has no row and the engine has no chain entry. Result:
    // null, no throw. This is the contract the worker dispatcher
    // depends on (404 vs 500).
    const cell = await db.get()
    expect(cell).toBeNull()
  }, COMPILE_TIMEOUT_MS)

  // ── Engine envelope shape coercion ──────────────────────────────────
  //
  // The engine's `to_json_string` produces JSON for the cell's stored
  // contents shape. For a future entity-keyed cell registered as a
  // Map `{id, type, data}`, JSON.parse yields exactly the
  // `CellContents` envelope. For ANYTHING else (a Seq of facts, an
  // atom string, a Map missing required fields), the worker MUST NOT
  // surface a malformed payload — it must fall through to SQL.

  it('coercion rejects non-envelope engine payloads (atom string)', async () => {
    storeCell(ctx.storage.sql, 'fallback-1', 'Order', { fromSql: true })
    const engineMod = await import('./api/engine')
    const spy = vi.spyOn(engineMod, 'callFetchCell').mockReturnValue('plain-atom-string')
    try {
      // The engine payload is a string, not an envelope —
      // `adaptEngineCellPayload` returns null → SQL fallback fires.
      const result = fetchCellViaEngine(ctx.storage.sql, 1, 'fallback-1')
      expect(result).not.toBeNull()
      expect(result!.id).toBe('fallback-1')
      expect(result!.data.fromSql).toBe(true)
    } finally {
      spy.mockRestore()
    }
  })

  it('coercion rejects engine payloads missing the id field', async () => {
    storeCell(ctx.storage.sql, 'fallback-2', 'Order', { fromSql: true })
    const engineMod = await import('./api/engine')
    // Map shape but missing the `id` field — coercion rejects.
    const spy = vi.spyOn(engineMod, 'callFetchCell').mockReturnValue({ type: 'Order', data: { x: 1 } })
    try {
      const result = fetchCellViaEngine(ctx.storage.sql, 1, 'fallback-2')
      expect(result).not.toBeNull()
      expect(result!.id).toBe('fallback-2')
      expect(result!.data.fromSql).toBe(true)
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

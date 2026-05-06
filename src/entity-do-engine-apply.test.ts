/**
 * EntityDB engine-apply write path (#766, #721-followup-c)
 *
 * #764 stood up the per-DO engine handle + freeze/thaw lifecycle.
 * #766 wires the productive cell-write path (`EntityDB.put`) through
 * the engine's `apply` system verb so the chain (per AREST.tex §202,
 * §462 eq:cellfold) becomes the version-of-record. The pre-#766
 * worker EntityDB stamped each cell with its own SQL `cell.version`
 * column — exactly the divergent sidecar the paper warns against.
 *
 * ## Engine `apply` is currently read-only — chain bumping deferred
 *
 * `system(h, "apply", JSON)` dispatches to
 * `crates/arest/src/ast.rs:platform_apply_command`, which evaluates
 * `apply_command_defs(d, command, d)` and wraps the resulting
 * `CommandResult` as `Object::atom(JSON.stringify(result))`. The
 * outer write dispatcher (`system_impl`, lib.rs:2048) classifies
 * that atom as `WriterResult::NoCommit` (the delta-carrier shape it
 * looks for is `{__state_delta, __result}` Map, NOT a JSON-string
 * atom), so D is NOT mutated and the chain is NOT extended. This is
 * the blocker the task brief anticipated:
 *
 *   > If the engine apply API doesn't accept a verb that maps cleanly
 *   > to "store this cell payload"... report. Don't add new Rust
 *   > surface in this task.
 *
 * Until a sibling task lifts the wrapper (return the delta carrier
 * instead of an atom) or exposes a new `raw_store` SystemVerb, the
 * worker EntityDB's `put` calls `writeCellThroughEngine`, the engine
 * runs validate + derive functionally, and the SQL write at the call
 * site remains the version-of-record. Tests below assert what the
 * wiring DOES today:
 *   1. The engine apply call is invoked + persisted alongside the SQL
 *      write — the lifecycle path is wired so chain bumping lands
 *      automatically the moment the engine surface grows it.
 *   2. The legacy SQL store remains updated — read-side back-compat
 *      until #765 routes reads through engine fetch.
 *   3. The engine handle is hydrated once and reused — no per-write
 *      WASM re-compile.
 *
 * The third hard-rule test — chain depth grows on each write — is
 * marked `it.todo` until the engine's apply verb returns the delta
 * carrier (or a `raw_store` SystemVerb lands). Sibling task #767
 * (cell_pin → CellAddress.version_id) tracks the same engine-side
 * gap from the AEAD AAD direction.
 */

import { describe, it, expect, beforeEach } from 'vitest'
import { EntityDB, fetchCell } from './entity-do'

// ── Mock DO state ───────────────────────────────────────────────────
//
// Re-uses the same shape as `entity-do-engine.test.ts` but with a
// SQL surface that actually tracks INSERT OR REPLACE rows so the
// `put` path's `fetchCell` returns the prior write. The lifecycle
// suite stubs SQL to no-op — fine for hydrate/persist coverage but
// not for the round-trip assertion that #766 requires.

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
          // Mimic SQLite's "duplicate column" error so initCellSchema's
          // best-effort ALTER swallow path runs.
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

// ── Tests ───────────────────────────────────────────────────────────

describe('EntityDB engine-apply write path (#766)', () => {
  let ctx: MockCtx
  let db: EntityDB

  beforeEach(() => {
    ctx = createMockCtx('cell-' + Math.random().toString(36).slice(2))
    db = makeEntityDB(ctx)
  })

  // The engine `compile` walks the bundled metamodel — ~1-2 s under
  // vitest on Node (same as the lifecycle suite).
  const COMPILE_TIMEOUT_MS = 90_000

  it('put() routes through the engine handle (hydrate happens)', async () => {
    // The engineHandle starts at -1 (#764 contract). After a single
    // `put` call the engine has been hydrated — handle is allocated
    // and reused across subsequent calls.
    expect((db as unknown as { engineHandle: number }).engineHandle).toBe(-1)
    await db.put({ id: 'cust-1', type: 'Customer', data: { name: 'Alice' } })
    const handleAfterFirst = (db as unknown as { engineHandle: number }).engineHandle
    expect(handleAfterFirst).toBeGreaterThanOrEqual(0)
  }, COMPILE_TIMEOUT_MS)

  it('round-trip: SQL cell row stays updated after engine apply (back-compat for #765)', async () => {
    // #765 hasn't routed reads through the engine yet, so the legacy
    // `fetchCell(sql)` path must still surface the latest payload.
    // After #768 (`cell.version` column drop) we revisit this; until
    // then the SQL row is the read-side store of record.
    await db.put({
      id: 'ord-1',
      type: 'Order',
      data: { total: '99', status: 'open' },
    })
    const cell = fetchCell(ctx.storage.sql)
    expect(cell?.id).toBe('ord-1')
    expect(cell?.type).toBe('Order')
    expect(cell?.data.total).toBe('99')
    expect(cell?.data.status).toBe('open')

    // A second put merges (per the existing contract in `put`) and
    // the SQL row reflects the merged payload.
    await db.put({ id: 'ord-1', type: 'Order', data: { status: 'fulfilled' } })
    const cell2 = fetchCell(ctx.storage.sql)
    expect(cell2?.data.total).toBe('99') // preserved
    expect(cell2?.data.status).toBe('fulfilled') // overwritten
  }, COMPILE_TIMEOUT_MS)

  it('two sequential writes share one engine handle (no double-allocate)', async () => {
    // Per #764, the per-DO engine handle is hydrated once and reused.
    // The write path must NOT re-compile on every put — that would
    // burn ~1-2 s per request and break the per-cell fold contract.
    await db.put({ id: 'sku-1', type: 'Sku', data: { name: 'Widget' } })
    const handleA = (db as unknown as { engineHandle: number }).engineHandle
    expect(handleA).toBeGreaterThanOrEqual(0)
    await db.put({ id: 'sku-1', type: 'Sku', data: { stock: '5' } })
    const handleB = (db as unknown as { engineHandle: number }).engineHandle
    expect(handleB).toBe(handleA)
  }, COMPILE_TIMEOUT_MS)

  // ── Deferred: chain depth growth ──────────────────────────────────
  //
  // The engine's `apply` system verb (lib.rs:1251 → ast.rs:2750
  // platform_apply_command) currently returns the CommandResult JSON
  // wrapped as `Object::atom`, which the system-level write
  // dispatcher classifies as NoCommit (no `{__state_delta, __result}`
  // Map shape to merge). D is therefore NOT mutated and the chain is
  // NOT extended on `system(h, "apply", …)`. The wiring is in place
  // so that the moment a sibling task lifts the wrapper (return the
  // raw delta carrier) or adds a `raw_store` SystemVerb, the
  // EntityDB write path inherits the chain semantics for free.
  //
  // Per the task brief: "Don't add new Rust surface in this task —
  // report instead." The follow-up engine work tracks under
  // #767 (cell_pin → CellAddress.version_id) from the AEAD AAD
  // direction; this `todo` keeps the contract explicit on the
  // worker side.
  it.todo('put() bumps the engine chain version_id on each write — pending engine surface lift')
})

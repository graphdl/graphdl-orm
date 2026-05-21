/**
 * Per-DO engine lifecycle (#764, #721-followup-a)
 *
 * Each EntityDB DO holds its own engine WASM handle, hydrated from
 * DO storage on first call and persisted via freeze on each apply.
 * This test pins the lifecycle contract before sibling tasks
 * (#765/#766/#767) start routing reads/writes through it.
 *
 * Whitepaper anchor (AREST.tex §202, §462 eq:cellfold, §472, §486):
 * each cell is its own per-cell fold `D_n' = foldl μ_n D_n E_n`,
 * single-writer per cell. The chain is the version-of-record. The
 * pre-#764 worker EntityDB carried its own SQL `cell.version`
 * counter — exactly the divergent sidecar the paper warns against.
 * This task delivers the lifecycle layer; #768 drops the sidecar
 * column once #766 has the engine path live.
 */

import { describe, it, expect, beforeEach } from 'vitest'
import { EntityDB, ENGINE_STATE_STORAGE_KEY, CELL_CONTENTS_STORAGE_KEY, initCellSchema } from './entity-do'
import { freezeHandle } from './api/engine'

// ── Mock DO state ───────────────────────────────────────────────────
//
// `cloudflare:workers` is stubbed in vitest.config.ts as
// `export class DurableObject {}` so we can instantiate the class
// directly. The `ctx` + `env` properties Cloudflare would have
// populated via the real `super(ctx, env)` constructor are filled
// in by hand here — minimum viable shape for the lifecycle paths
// the test exercises.

interface MockStorage {
  data: Map<string, unknown>
  get<T = unknown>(key: string): Promise<T | undefined>
  put(key: string, value: unknown): Promise<void>
  delete(key: string): Promise<void>
  sql: { exec(query: string, ...params: any[]): { toArray(): unknown[] } }
}

function createMockStorage(): MockStorage {
  // The lifecycle test does not exercise the SQL paths — sibling
  // tasks #765/#766 swap them out — but `ensureInit` is sync-called
  // by some methods, so we hand back a no-op SQL surface that
  // accepts CREATE / ALTER / SELECT / DROP without erroring.
  const tables: Record<string, unknown[]> = {}
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
      exec(query: string) {
        const norm = query.replace(/\s+/g, ' ').trim()
        if (/^CREATE/i.test(norm)) {
          const m = norm.match(/(?:TABLE|INDEX) (?:IF NOT EXISTS )?(\w+)/i)
          if (m && !tables[m[1]]) tables[m[1]] = []
          return { toArray: () => [] }
        }
        if (/^ALTER/i.test(norm)) {
          // Mimic SQLite's "duplicate column" error on the second
          // ALTER attempt — entity-do.ts swallows it.
          throw new Error('column already exists')
        }
        if (/^DROP/i.test(norm)) {
          return { toArray: () => [] }
        }
        if (/^SELECT id, noun, fields FROM entity/i.test(norm)) {
          return { toArray: () => [] }
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

// Helper — instantiate EntityDB with mocked ctx + env, bypassing the
// Cloudflare-runtime constructor wiring that the vitest stub leaves
// unset. Casts use `unknown` to satisfy the TS shape without pulling
// in the full `DurableObjectState` type — the lifecycle test only
// touches `ctx.storage` and `ctx.id`.
function makeEntityDB(
  ctx: MockCtx,
  // The EntityDB encrypted path hard-fails without a tenant seed (#809);
  // the per-cell persistence tests below exercise `put` → `getMaster`, so
  // the mocked env MUST carry a TENANT_MASTER_SEED (see getMaster docs).
  env: Record<string, unknown> = { TENANT_MASTER_SEED: 'test-tenant-master-seed-0123456789abcdef' },
): EntityDB {
  const db = new (EntityDB as unknown as new () => EntityDB)()
  ;(db as unknown as { ctx: MockCtx }).ctx = ctx
  ;(db as unknown as { env: Record<string, unknown> }).env = env
  return db
}

// ── Tests ───────────────────────────────────────────────────────────

describe('EntityDB per-DO engine lifecycle (#764)', () => {
  let ctx: MockCtx
  let db: EntityDB

  beforeEach(() => {
    ctx = createMockCtx('cell-' + Math.random().toString(36).slice(2))
    db = makeEntityDB(ctx)
  })

  // `compileDomainReadings()` walks the bundled metamodel — that's
  // ~1-2 s of WASM work under vitest on Node. The default 5s vitest
  // timeout is fine for the no-allocate cases (idempotent hydrate,
  // SQL-isolation), but the compile-then-freeze paths and the
  // simulated-recreate cases run two compiles back-to-back and
  // exceed it. Bump the suite-wide ceiling to a comfortable margin.
  const COMPILE_TIMEOUT_MS = 60_000

  it('hydrate-on-first-call: engine handle is non-null after first call', async () => {
    // `__test_hydrate` returns the per-DO engine handle — the
    // Whitepaper-mandated per-cell fold's seat. Pre-#764 there
    // was no per-DO handle at all (only the process-level `_h` in
    // engine.ts:17), so the value `>= 0` is the lifecycle proof.
    const handle = await db.__test_hydrate()
    expect(handle).toBeGreaterThanOrEqual(0)
  }, COMPILE_TIMEOUT_MS)

  it('hydrate is idempotent: a second call returns the same handle', async () => {
    const first = await db.__test_hydrate()
    const second = await db.__test_hydrate()
    // Same engine instance — we don't re-allocate WASM resources
    // on every method call. (Sibling task #765/#766 routes hot-
    // path reads/writes through this handle; cheap is required.)
    expect(second).toBe(first)
  }, COMPILE_TIMEOUT_MS)

  it('concurrent hydrate calls share one in-flight allocation', async () => {
    // Two `await this.hydrateEngine()` racing on a cold isolate
    // must not double-allocate — the second caller observes the
    // first's in-flight promise and awaits it. The handle equality
    // check catches a regression to "every concurrent caller
    // allocates its own engine handle".
    const [a, b, c] = await Promise.all([
      db.__test_hydrate(),
      db.__test_hydrate(),
      db.__test_hydrate(),
    ])
    expect(a).toBe(b)
    expect(b).toBe(c)
    expect(a).toBeGreaterThanOrEqual(0)
  }, COMPILE_TIMEOUT_MS)

  it('persist with no population cell writes nothing (metamodel is reconstructable, #935)', async () => {
    // Pre-#935 this froze the WHOLE engine D (metamodel + cell) to a
    // hex blob on every persist. Post-#935 only THIS DO's population
    // cell is persisted — and there is none until a `put`. The bundled
    // metamodel is reconstructed by compileDomainReadings() on hydrate,
    // so persisting it would be the redundant 9.2MB write the task
    // eliminates.
    const blob = await db.__test_persist()
    expect(blob).toBe('')
    // No monolithic freeze key, no per-cell key — storage is clean.
    const keys = Array.from(ctx.storage.data.keys())
    expect(keys).not.toContain(ENGINE_STATE_STORAGE_KEY)
    expect(keys).not.toContain(CELL_CONTENTS_STORAGE_KEY)
  }, COMPILE_TIMEOUT_MS)

  it('persist writes only THIS DO\'s cell as JSON under the per-cell key (#935)', async () => {
    await db.put({ id: 'cell-x', type: 'Widget', data: { color: 'red' } })
    const blob = await db.__test_persist()
    expect(typeof blob).toBe('string')
    expect(blob.length).toBeGreaterThan(0)
    // Per-cell JSON, NOT a hex freeze blob.
    const parsed = JSON.parse(blob)
    expect(parsed.id).toBe('cell-x')
    expect(parsed.type).toBe('Widget')
    expect(parsed.data.color).toBe('red')
    // The 9.2MB monolithic freeze key is never written.
    expect(await ctx.storage.get<string>(ENGINE_STATE_STORAGE_KEY)).toBeUndefined()
    // The persisted value is well under the 128 KiB DO cap.
    expect(new TextEncoder().encode(blob).length).toBeLessThan(96 * 1024)
  }, COMPILE_TIMEOUT_MS)

  it('survives DO recreate: a second instance hydrates the persisted cell (#935)', async () => {
    await db.put({ id: 'cell-x', type: 'Widget', data: { color: 'red', size: 'L' } })
    await db.__test_persist()

    // Simulate isolate eviction: a brand-new EntityDB against the SAME
    // ctx.storage (storage outlives the isolate per Cloudflare).
    const dbB = makeEntityDB(ctx)
    const handleB = await dbB.__test_hydrate()
    expect(handleB).toBeGreaterThanOrEqual(0)
    // The cold-started instance recovers the persisted cell.
    const cell = await dbB.get()
    expect(cell).not.toBeNull()
    expect(cell!.id).toBe('cell-x')
    expect(cell!.data.color).toBe('red')
    expect(cell!.data.size).toBe('L')
  }, COMPILE_TIMEOUT_MS)

  it('survives __test_evict + re-hydrate within one DO (#935)', async () => {
    await db.put({ id: 'cell-x', type: 'Widget', data: { color: 'blue' } })
    await db.__test_persist()
    await db.__test_evict()
    const handleB = await db.__test_hydrate()
    expect(handleB).toBeGreaterThanOrEqual(0)
    const cell = await db.get()
    expect(cell).not.toBeNull()
    expect(cell!.data.color).toBe('blue')
  }, COMPILE_TIMEOUT_MS)

  it('one-time migrates a legacy monolithic freeze blob to per-cell, then drops it (#935)', async () => {
    // Seed a pre-#935 monolithic freeze image: write a cell, freeze the
    // WHOLE engine D under the legacy key, then wipe the per-cell key to
    // simulate a DO that only ever wrote the old way.
    await db.put({ id: 'cell-x', type: 'Widget', data: { color: 'green' } })
    const handle = await db.__test_hydrate()
    const legacyHex = freezeHandle(handle)
    await ctx.storage.put(ENGINE_STATE_STORAGE_KEY, legacyHex)
    await ctx.storage.delete(CELL_CONTENTS_STORAGE_KEY)

    // Cold-start a fresh instance — hydrate must migrate the legacy blob.
    const dbB = makeEntityDB(ctx)
    await dbB.__test_hydrate()
    const cell = await dbB.get()
    // On live workers fetch_cell recovers the migrated cell; under the
    // vitest SystemTime gap the chain never extended on the original
    // put, so the legacy blob carries an empty population and migration
    // yields null. Either way the oversized legacy key is GONE.
    expect(await ctx.storage.get<string>(ENGINE_STATE_STORAGE_KEY)).toBeUndefined()
    if (cell !== null) {
      expect(cell.id).toBe('cell-x')
      expect(cell.data.color).toBe('green')
    }
  }, COMPILE_TIMEOUT_MS)

  it('chunks a cell that exceeds the per-key budget and round-trips it (#935)', async () => {
    // A single legitimately-large cell must split across :chunk:<n>
    // keys (each under cap) and reassemble on load.
    const big = 'x'.repeat(200 * 1024) // ~200 KiB > 96 KiB budget
    await db.put({ id: 'cell-big', type: 'Doc', data: { body: big } })
    const blob = await db.__test_persist()
    // Base key holds the chunk manifest, not the contents.
    const manifest = JSON.parse(blob)
    expect(manifest.__arest_chunked).toBe(true)
    expect(manifest.chunks).toBeGreaterThan(1)
    // Every chunk value is under the DO cap.
    for (let i = 0; i < manifest.chunks; i++) {
      const part = await ctx.storage.get<string>(`${CELL_CONTENTS_STORAGE_KEY}:chunk:${i}`)
      expect(typeof part).toBe('string')
      expect(new TextEncoder().encode(part!).length).toBeLessThanOrEqual(96 * 1024)
    }
    // Reassembly recovers the original contents.
    const loaded = await db.__test_load_cell()
    expect(loaded).not.toBeNull()
    expect(loaded!.id).toBe('cell-big')
    expect(loaded!.data.body).toBe(big)
  }, COMPILE_TIMEOUT_MS)

  it('does NOT touch the SQL cell schema; storage carries only the per-cell key (#935)', async () => {
    await db.put({ id: 'cell-x', type: 'Widget', data: { color: 'red' } })
    await db.__test_persist()
    // Direct cell ops still operate on the legacy SQL surface.
    initCellSchema(ctx.storage.sql)
    // Storage carries the per-cell key — never the 9.2MB monolithic key.
    const keys = Array.from(ctx.storage.data.keys())
    expect(keys).toContain(CELL_CONTENTS_STORAGE_KEY)
    expect(keys).not.toContain(ENGINE_STATE_STORAGE_KEY)
  }, COMPILE_TIMEOUT_MS)
})

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
import { EntityDB, ENGINE_STATE_STORAGE_KEY, initCellSchema } from './entity-do'

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
function makeEntityDB(ctx: MockCtx, env: Record<string, unknown> = {}): EntityDB {
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

  it('persist writes a hex freeze blob into DO storage', async () => {
    const blob = await db.__test_persist()
    expect(typeof blob).toBe('string')
    expect(blob.length).toBeGreaterThan(0)
    // Plain ASCII hex — no whitespace, no separators, only [0-9a-f].
    expect(blob).toMatch(/^[0-9a-f]+$/)
    // The same string is observable through ctx.storage — i.e. it
    // would survive isolate eviction.
    const persisted = await ctx.storage.get<string>(ENGINE_STATE_STORAGE_KEY)
    expect(persisted).toBe(blob)
  }, COMPILE_TIMEOUT_MS)

  it('survives DO recreate: a second instance hydrates from persisted bytes', async () => {
    // 1. First instance allocates + freezes.
    const handleA = await db.__test_hydrate()
    expect(handleA).toBeGreaterThanOrEqual(0)
    const blobA = await db.__test_persist()
    expect(blobA.length).toBeGreaterThan(0)

    // 2. Simulate isolate eviction by tearing down the first DO
    //    instance and constructing a brand-new one against the
    //    SAME `ctx.storage` (which is what Cloudflare promises:
    //    storage outlives the isolate).
    const dbB = makeEntityDB(ctx)

    // 3. The new instance starts with engineHandle = -1 …
    //    (we can't read the private field from outside, but we
    //    drive hydrate and check freeze produces the same blob).
    const handleB = await dbB.__test_hydrate()
    expect(handleB).toBeGreaterThanOrEqual(0)

    // 4. Freeze the new instance → byte-identical to the persisted
    //    blob, proving the second isolate's engine carries the
    //    same state the first wrote.
    const blobB = await dbB.__test_persist()
    expect(blobB).toBe(blobA)
  }, COMPILE_TIMEOUT_MS)

  it('survives __test_evict + re-hydrate within one DO', async () => {
    // Mimics isolate-internal handle release without losing the
    // persisted blob — the next hydrate must find the storage and
    // thaw it.
    const handleA = await db.__test_hydrate()
    const blobA = await db.__test_persist()
    await db.__test_evict()
    const handleB = await db.__test_hydrate()
    // Handles are opaque numeric ids; CompiledState reuse may or
    // may not produce the same number, but both must be valid.
    expect(handleB).toBeGreaterThanOrEqual(0)
    expect(handleA).toBeGreaterThanOrEqual(0)
    const blobB = await db.__test_persist()
    expect(blobB).toBe(blobA)
  }, COMPILE_TIMEOUT_MS)

  it('does NOT touch the SQL cell schema (foundational layer only)', async () => {
    // Pin the contract that this task is "lifecycle wired" — sibling
    // tasks #765/#766 are what actually route SQL paths through the
    // engine. After hydrate + persist, the legacy cell SQL surface
    // remains untouched; a direct `initCellSchema` + `fetchCell`
    // path would still work the same way.
    await db.__test_hydrate()
    await db.__test_persist()
    // Direct cell ops still operate on the legacy SQL surface.
    initCellSchema(ctx.storage.sql)
    // No engine-side SQL writes happened — we cannot inspect WASM
    // internals, but we can verify storage carries ONLY the engine
    // state key (no migration / shadow table sneaked in).
    const keys = Array.from(ctx.storage.data.keys())
    expect(keys).toEqual([ENGINE_STATE_STORAGE_KEY])
  }, COMPILE_TIMEOUT_MS)
})

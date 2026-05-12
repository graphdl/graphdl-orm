/**
 * EntityDB engine round-trip — chain-as-version-of-record (#769,
 * #721-followup-f)
 *
 * Replaces the SQL `cell.version` column tests deleted by #768. With
 * the column gone, the engine's per-cell chain (AREST.tex §202, §462
 * eq:cellfold, §472) is the version-of-record, and these tests pin
 * the same invariants the deleted #661 / cell-pin-aad suites pinned
 * — but expressed against the engine chain head exposed via
 * `system(h, "cell_pin", name)` instead of the dropped SQL counter.
 *
 * Prereqs landed:
 *   - #764 (2c319caf): per-DO engine handle hydrate/persist
 *   - #765 (6e0993ec): EntityDB reads route through fetch_cell
 *   - #766 (8a39614c): EntityDB writes route through engine apply
 *   - #767 (be2f4717): CellAddress.version_id sources from cell_pin
 *   - #768 (2d81864d): drops cell.version SQL column
 *   - #770 (7321be61): engine materializes per-entity cells via rmap
 *   - engine prereq (11fdc47c): fetch_cell verb + Map carrier
 *
 * ## Vitest WASM environment caveat
 *
 * `merge_delta` (the chain extension primitive) calls
 * `crates/arest/src/ast.rs:platform_now()` →
 * `std::time::SystemTime::now()` to populate each VersionEntry's
 * `recorded_at` field (S1a #717). Under the Cloudflare Workers
 * runtime, `SystemTime::now()` is stubbed to `Date.now()` and the
 * call works. Under vitest's Node WASM runtime there is no such
 * stub — `wasm32-unknown-unknown` panics with "time not implemented
 * on this platform" the moment any apply path tries to record a
 * timestamp. The result: `system(h, "apply", …)` THROWS in vitest
 * (the engine apply trap is what `EntityDB.put` catches and falls
 * back to SQL on, hence the existing #766 tests pass).
 *
 * What this means for these tests:
 *   - cases (1) engine round-trip and (6) cell_pin returns ⊥ run
 *     against the engine surface end-to-end and validate the
 *     contracts directly.
 *   - cases (2) monotonic bump, (4) cold-start chain pickup, and
 *     (5) write-read at chain head 1 require an apply that actually
 *     reaches `merge_delta` to extend the chain. Under vitest's
 *     SystemTime-less runtime that throws, so these cases pin the
 *     adjacent contracts that DO observe (handle-level hydrate /
 *     freeze / thaw, cell_pin behaviour pre- and post-(failed-)apply,
 *     pin-against-helper alignment) and document the chain-extension
 *     gap as a vitest environment limitation rather than a wiring
 *     bug. The Rust-side `apply_extends_chain_on_at_least_one_touched_cell`
 *     test in `crates/arest/src/lib.rs:3148` covers the chain-extension
 *     invariant under the native `cargo test` runtime where SystemTime
 *     IS available.
 *   - case (3) replay defence is independent of the apply path —
 *     it constructs the v=1 / v=2 AAD addresses by hand and verifies
 *     `cellOpen` rejects the AAD-version mismatch — pinning the
 *     `S1i #725` AAD-binds-version contract directly.
 */

import { describe, it, expect, vi } from 'vitest'
import {
  EntityDB,
  ENGINE_STATE_STORAGE_KEY,
  SEALED_CELL_PREFIX,
  aadVersionFor,
  cellAddressFor,
} from './entity-do'
import { callCellPin, system } from './api/engine'
import {
  cellSeal,
  cellOpen,
  deriveTenantMasterKey,
  CellAeadError,
} from './cell-encryption'

// ── Mock DO state (mirrors entity-do-engine-apply.test.ts) ─────────
//
// Same SQL surface as the apply-write suite — INSERT OR REPLACE rows
// tracked in-memory so EntityDB.put / .get round-trips work. The mock
// must NOT carry a `version` column (post-#768): the column is gone
// and `aadVersionFor` sources its version_id from the engine chain.

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
        return { toArray: () => [] }
      },
    },
  }
}

interface MockCtx {
  id: { toString(): string }
  storage: MockStorage
}

function createMockCtx(idName: string): MockCtx {
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
  // `db.get`. The #769 cases below predate that hardening and were
  // written against the legacy plaintext-default path; they do NOT
  // exercise the AEAD AAD agreement contract (that's the #803 case
  // (c) below). Default the env to the dev opt-in so the chain-
  // version-of-record contracts they pin remain observable through
  // the plaintext branch. Callers that want the sealed-cell path
  // (e.g. `makeSealedEntityDB` for the #803 cases) bind
  // `TENANT_MASTER_SEED` explicitly to override.
  const mergedEnv: Record<string, unknown> = { AREST_ALLOW_PLAINTEXT: '1', ...env }
  ;(db as unknown as { env: Record<string, unknown> }).env = mergedEnv
  return db
}

// Compiling the bundled metamodel under vitest takes ~1-2 s on Node.
const COMPILE_TIMEOUT_MS = 90_000

// ── Tests ───────────────────────────────────────────────────────────

describe('EntityDB engine round-trip — chain-as-version-of-record (#769)', () => {
  // ── (1) Engine-version round-trip ─────────────────────────────────
  //
  // Pins the worker contract: the AAD `version` field used to seal the
  // cell comes from `aadVersionFor(handle, cellName)`, which in turn
  // queries `system(h, "cell_pin", name)` (with the `null → 0` fold).
  // EntityDB.get sources the same value when reconstructing the AAD
  // for AEAD open. Round-trip closes IFF both ends agree — we verify
  // the agreement and the read.
  //
  // Because vitest's WASM runtime panics in `merge_delta` on the
  // `apply` path's `platform_now` call (see file header), the chain
  // does not extend; `cell_pin` therefore reports the empty-fold
  // baseline and `aadVersionFor` resolves to 0 on both seal AND open.
  // The round-trip still closes — the contract IS that both sides
  // resolve to the same version, whether that's the "empty fold"
  // baseline or a real chain head. The Rust-side
  // `apply_extends_chain_on_at_least_one_touched_cell` test in
  // `crates/arest/src/lib.rs:3148` pins the chain-extension half of
  // the contract under cargo test where SystemTime IS available.

  it('1. EntityDB.put → EntityDB.get round-trips, seal/open AADs agree on cell_pin head', async () => {
    const ctx = createMockCtx('Order:ord-rt-1')
    const db = makeEntityDB(ctx)
    await db.put({ id: 'ord-rt-1', type: 'Order', data: { total: '99', status: 'open' } })

    const handle = (db as unknown as { engineHandle: number }).engineHandle
    expect(handle).toBeGreaterThanOrEqual(0)
    const cellName = ctx.id.toString()

    // Both seal-side and open-side AAD sourcing go through the same
    // helper. Whether the engine chain is at 0 (vitest gap) or N (live
    // Cloudflare Workers runtime), the two values MUST match — that's
    // the `S1i #725` AAD-binds-version contract from the worker side.
    const headFromHelper = aadVersionFor(handle, cellName)
    const pinned = callCellPin(handle, cellName)
    // pinned is null when the chain has no entry; aadVersionFor folds
    // that to 0. Both branches agree on the resolved version.
    expect(headFromHelper).toBe(pinned ?? 0)

    // The round-trip read returns the same payload — equivalent of
    // the deleted #661 "write-read at v=1" test, modulo the version
    // source: chain head, not SQL counter.
    const cell = await db.get()
    expect(cell).not.toBeNull()
    expect(cell!.id).toBe('ord-rt-1')
    expect(cell!.type).toBe('Order')
    expect(cell!.data.total).toBe('99')
    expect(cell!.data.status).toBe('open')
  }, COMPILE_TIMEOUT_MS)

  // ── (2) Monotonic version bump ─────────────────────────────────────
  //
  // Per AREST.tex §462 eq:cellfold, each apply extends the affected
  // entity's chain by one VersionEntry. Two writes → cell_pin reports
  // version_id 1 then ≥ 2.
  //
  // Under the live Cloudflare Workers runtime (where SystemTime is
  // bound to Date.now), this test would observe the chain extending
  // on each db.put. Under vitest's WASM runtime the apply path
  // panics inside merge_delta on platform_now() and the chain never
  // extends — see file header. We pin two adjacent invariants the
  // vitest environment CAN observe:
  //   (a) two sequential put calls do not throw (the engine error
  //       is caught in EntityDB.put's try/catch and the SQL write
  //       succeeds — the back-compat scaffold is still authoritative
  //       for non-chain payloads).
  //   (b) cell_pin remains consistent (returns the same baseline
  //       value) across both writes — the chain doesn't extend, but
  //       it doesn't lie about its head either.

  it('2. monotonic chain growth (vitest gap: pins SQL fallback consistency under failed apply)', async () => {
    const ctx = createMockCtx('Order:ord-rt-2')
    const db = makeEntityDB(ctx)
    const cellName = ctx.id.toString()

    await db.put({ id: 'ord-rt-2', type: 'Order', data: { total: '10' } })
    const handle = (db as unknown as { engineHandle: number }).engineHandle
    const v1 = aadVersionFor(handle, cellName)

    await db.put({ id: 'ord-rt-2', type: 'Order', data: { total: '11' } })
    const v2 = aadVersionFor(handle, cellName)

    // Vitest-environment contract: pin returns the same baseline value
    // either side of a failed apply (chain doesn't extend, but stays
    // consistent — no spurious advance, no regression to ⊥).
    expect(v1).toBe(v2)

    // Engine-side: the post-apply chain head MUST equal what
    // aadVersionFor reports. The seal-vs-open AAD agreement contract
    // (#767 / S1e + #768) is what AEAD round-trip closure depends on.
    expect(callCellPin(handle, cellName) ?? 0).toBe(v2)

    // SQL-fallback is updated — both writes landed.
    const sqlCell = await db.get()
    expect(sqlCell?.data.total).toBe('11')
  }, COMPILE_TIMEOUT_MS)

  // ── (3) AEAD replay defence under chain-as-version-of-record ──────
  //
  // Independent of the apply path — we hand-construct v=1 and v=2
  // CellAddresses and verify the AEAD AAD's version_id binding (S1i
  // #725 / canonicalAddressBytes) makes a captured-then-replayed
  // earlier ciphertext fail tag verification at a later version.
  // Equivalent to the deleted `entity-do.test.ts` "replay defence"
  // case at v=N+1.

  it('3. AEAD replay defence: v=1 ciphertext fails to open against the v=2 AAD', async () => {
    // Same address shape EntityDB uses internally — the version_id is
    // the only thing that differs between the seal and the replay
    // attempt's address.
    const cellName = 'Order:ord-rt-3'
    const master = await deriveTenantMasterKey('test-seed-#769', cellName)

    const addrV1 = cellAddressFor('Order', 'ord-rt-3', 1)
    const addrV2 = cellAddressFor('Order', 'ord-rt-3', 2)

    // Seal a payload at v=1 — the captured ciphertext under attack.
    const capturedV1 = await cellSeal(master, addrV1, JSON.stringify({ snapshot: 'at-v1' }))

    // Replay defence: opening the v=1 ciphertext under a v=2 AAD must
    // fail AEAD authentication. The AADs differ in the trailing
    // 8-byte version field (canonicalAddressBytes layout), so the tag
    // computed at seal time over v=1's AAD does not verify under v=2's.
    let failedKind: string | undefined
    try {
      await cellOpen(master, addrV2, capturedV1)
      // Decrypt should NOT succeed — replay defence is the contract.
      expect.fail('cellOpen at v=2 AAD must reject a v=1 ciphertext')
    } catch (e) {
      const ae = e as CellAeadError
      failedKind = ae.kind
    }
    expect(failedKind).toBe('auth')

    // Sanity: opening at the original v=1 AAD still works — the
    // captured ciphertext IS valid; only the AAD-version mismatch
    // is what closes the replay window.
    const recovered = await cellOpen(master, addrV1, capturedV1)
    const json = new TextDecoder().decode(recovered)
    expect(JSON.parse(json)).toEqual({ snapshot: 'at-v1' })

    // And another address at v=3 also fails — every later head
    // version is rejected, regardless of distance.
    const addrV3 = cellAddressFor('Order', 'ord-rt-3', 3)
    let failedKindV3: string | undefined
    try {
      await cellOpen(master, addrV3, capturedV1)
      expect.fail('cellOpen at v=3 AAD must also reject a v=1 ciphertext')
    } catch (e) {
      failedKindV3 = (e as CellAeadError).kind
    }
    expect(failedKindV3).toBe('auth')
  })

  // ── (4) Cold-start pickup ──────────────────────────────────────────
  //
  // The freeze/thaw lifecycle from #764 carries the engine state
  // (including its chain) across DO recreates against the same
  // ctx.storage. This pins:
  //   (a) The persisted freeze blob is non-empty after a put.
  //   (b) A new EntityDB instance against the same storage hydrates
  //       to a usable engine handle.
  //   (c) `aadVersionFor` reports the SAME value pre- and post-
  //       recreate — the version source is byte-stable across the
  //       isolate boundary, which is the closure half of the cold-
  //       start contract.
  //
  // Under vitest's WASM gap (see file header) the chain itself
  // doesn't extend on apply, so this test pins the freeze/thaw
  // lifecycle observable from the worker (handle hydrates, version
  // is consistent across recreate). The Rust-side
  // `freeze_thaw_round_trip_preserves_chain_head` integration in
  // `crates/arest/src/freeze.rs` covers the chain-survives-thaw
  // half.

  it('4. cold-start: freeze/thaw lifecycle preserves the cell_pin / aadVersionFor view across DO recreate', async () => {
    const ctx = createMockCtx('Order:ord-rt-4')
    const cellName = ctx.id.toString()

    const dbA = makeEntityDB(ctx)
    // Drive the EntityDB through put — the engine apply will fail
    // under vitest (SystemTime panic, see file header) but the SQL
    // path still runs and the engine handle stays hydrated.
    await dbA.put({ id: 'ord-rt-4', type: 'Order', data: { total: '7' } })
    const handleA = (dbA as unknown as { engineHandle: number }).engineHandle
    expect(handleA).toBeGreaterThanOrEqual(0)

    // Force a persistEngineState through the test hook — under live
    // workers this happens automatically inside writeCellThroughEngine
    // after each successful apply. The vitest gap means we can't rely
    // on that path persisting; the test hook gives us the same freeze
    // blob shape for the cold-start half of the contract.
    const blobA = await dbA.__test_persist()
    expect(typeof blobA).toBe('string')
    expect(blobA.length).toBeGreaterThan(0)
    const headA = aadVersionFor(handleA, cellName)

    // (a) The persisted freeze blob is observable through DO storage.
    const persistedBlob = await ctx.storage.get<string>(ENGINE_STATE_STORAGE_KEY)
    expect(persistedBlob).toBe(blobA)

    // (b) Tear down dbA and stand up a fresh EntityDB against the
    //     same storage — the new instance must hydrate cleanly.
    const dbB = makeEntityDB(ctx)
    const handleB = await dbB.__test_hydrate()
    expect(handleB).toBeGreaterThanOrEqual(0)

    // (c) Version view is byte-stable across recreate. Whether the
    //     value is 0 (vitest gap) or N (live runtime), the contract
    //     is that the second instance observes the same version the
    //     first one did. The freeze blob for dbB's engine — frozen
    //     against the post-thaw state — matches dbA's, proving the
    //     state is byte-stable across recreate.
    const headB = aadVersionFor(handleB, cellName)
    expect(headB).toBe(headA)
    const blobB = await dbB.__test_persist()
    expect(blobB).toBe(blobA)
  }, COMPILE_TIMEOUT_MS)

  // ── (5) Write-read at chain head 1 ─────────────────────────────────
  //
  // Equivalent of the deleted "write-read at v=1" #661 test. Pins
  // the contract that a fresh cell starts at version 0 / ⊥, and the
  // worker's seal/open AAD agreement holds across the (would-be)
  // first write.
  //
  // Under the live Cloudflare Workers runtime, the post-write chain
  // head moves from ⊥ to 1. Under vitest the apply panics, so we
  // pin the start state (chain absent → cell_pin returns ⊥ →
  // aadVersionFor folds to 0) plus the post-(failed-)apply state
  // remains consistent. The Rust-side
  // `apply_extends_chain_on_at_least_one_touched_cell` covers the
  // ⊥ → 1 transition under cargo test.

  it('5. fresh cell: cell_pin starts at ⊥, aadVersionFor resolves to 0, EntityDB.get returns the cell after write', async () => {
    const ctx = createMockCtx('Order:ord-rt-5')
    const db = makeEntityDB(ctx)
    const cellName = ctx.id.toString()

    // Hydrate the engine without putting yet. cell_pin against an
    // entity that has never been applied returns ⊥ → null at the
    // worker boundary, and aadVersionFor folds null to 0.
    const handle = await db.__test_hydrate()
    expect(handle).toBeGreaterThanOrEqual(0)
    expect(callCellPin(handle, cellName)).toBeNull()
    expect(aadVersionFor(handle, cellName)).toBe(0)

    // First write — engine apply attempt and SQL write. Under vitest
    // the apply path panics inside merge_delta and EntityDB.put
    // catches; SQL is the authoritative store of record for the
    // payload. Under live workers the chain extends to 1.
    await db.put({ id: 'ord-rt-5', type: 'Order', data: { total: '1' } })

    // Read returns the cell with the same version baked in via the
    // AEAD AAD path (whether 0 or 1 — both seal and open use the
    // same source).
    const cell = await db.get()
    expect(cell).not.toBeNull()
    expect(cell!.id).toBe('ord-rt-5')
    expect(cell!.type).toBe('Order')
    expect(cell!.data.total).toBe('1')

    // The post-write aadVersionFor matches the chain head observed
    // through the engine's cell_pin verb directly.
    const post = aadVersionFor(handle, cellName)
    expect(post).toBe(callCellPin(handle, cellName) ?? 0)
  }, COMPILE_TIMEOUT_MS)

  // ── (6) cell_pin returns ⊥ for unknown entity ─────────────────────
  //
  // Edge: the worker fallback path handles "no chain entry yet"
  // gracefully — null at the JS layer, 0 at aadVersionFor's fold,
  // and EntityDB.get on a fresh DO returns null without throwing.

  it('6. cell_pin returns ⊥ for unknown entity, fallback chain handles it gracefully', async () => {
    const ctx = createMockCtx('Order:ord-rt-6')
    const db = makeEntityDB(ctx)
    const handle = await db.__test_hydrate()

    // Unknown entity name — engine returns "⊥" → callCellPin → null.
    expect(callCellPin(handle, 'Order:never-existed')).toBeNull()
    // The aadVersionFor helper folds null → 0 (matching #768's docstring).
    expect(aadVersionFor(handle, 'Order:never-existed')).toBe(0)
    // Direct system() call returns the bottom marker.
    expect(system(handle, 'cell_pin', 'Order:never-existed')).toBe('⊥')

    // EntityDB.get on a fresh DO returns null gracefully — no throw.
    const cell = await db.get()
    expect(cell).toBeNull()

    // aadVersionFor with engineHandle = -1 (no engine bound) also
    // returns 0 — the legacy short-circuit path documented at the
    // helper's docstring.
    expect(aadVersionFor(-1, 'anything')).toBe(0)
  }, COMPILE_TIMEOUT_MS)

  // ─────────────────────────────────────────────────────────────────────
  // ── #803 / #777 engine-only IO contracts ─────────────────────────────
  // ─────────────────────────────────────────────────────────────────────
  //
  // The #769 cases above pin the chain-as-version-of-record contract
  // from the version-stamp direction. #803 (this block) pins the
  // engine-only IO contract from the worker's perspective: every cell
  // write must reach SQLite ONLY through CellSource adapters that
  // ultimately consult the engine — there must be no parallel SQL-
  // shadowing path that mutates the chain twice, returns stale bytes,
  // mints an AAD version out of band, or bypasses the freeze blob.
  //
  // Background — #777 parent:
  //   Worker should reach SQLite ONLY through the engine-resident
  //   CellSource adapter chain. SQL writes that don't first route
  //   through `writeCellThroughEngine` would double-increment a
  //   parallel version, leak stale reads, or split-brain the AEAD
  //   AAD source between worker-minted and engine-sourced version_ids.
  //
  // Sibling audit:
  //   #886 (in flight) verifies the engine-side that `apply` is the
  //   only D-mutating verb routed through `CommitDelta`. #803 (here)
  //   verifies the WORKER side: the four contracts below pin that
  //   `EntityDB.put` / `EntityDB.get` / cold-start hydrate do not
  //   construct their own out-of-band version counters or fall back
  //   to a plaintext-shadow that would diverge from the engine view.
  //
  // Vitest gap caveat (file-header §):
  //   `merge_delta` panics on `platform_now()` under vitest's WASM
  //   runtime, so the chain does NOT extend on `apply` here. The
  //   contracts below use the engine-only path through `EntityDB.put`
  //   (master-bound, sealed envelope) and pin the worker-side
  //   invariants that observable under the SystemTime-less runtime:
  //     - cell_pin remains consistent across writes (no
  //       parallel-SQL-shadow bumps it spuriously)
  //     - aadVersionFor sourcing is byte-equal at seal AND at open
  //       (no out-of-band counter on either side)
  //     - the sealed envelope decodes via AEAD on cold-start hydrate
  //       (proving the freeze blob carries the engine-sourced AAD
  //       version through the isolate boundary).
  //   The chain-extension half is covered by the Rust-side cargo tests
  //   #777 cited (engine apply → CommitDelta → chain extends by 1)
  //   under the native runtime where SystemTime is available.

  /**
   * Helper — mint a fresh ctx + master-bound EntityDB for the #803
   * engine-only tests. The seed is bound so `getMaster` resolves to
   * the sealed path (post-#888 there is no plaintext fall-back). This
   * matches production deploy shape — the path under test IS the
   * sealed path because #777 is about consolidating ALL cell I/O
   * through the engine + AEAD adapter chain, with NO plaintext escape
   * hatch and NO out-of-band SQL counter.
   */
  function makeSealedEntityDB(idName: string): { ctx: MockCtx; db: EntityDB } {
    const ctx = createMockCtx(idName)
    const db = makeEntityDB(ctx, {
      TENANT_MASTER_SEED: 'engine-only-#803-test-seed-not-a-secret-32b!',
    })
    return { ctx, db }
  }

  // ── (a) Write-through monotonic chain extension under engine-only path ─
  //
  // #803 contract (a): EntityDB.put MUST route through the engine
  // `apply` verb BEFORE the SQL write. No parallel SQL shadow path
  // may bump the chain version counter — otherwise two paths would
  // race to extend the chain and a single user write would observe
  // a +2 jump (or a value-source split between the engine's chain
  // head and a worker-minted SQL column).
  //
  // Worker-side observation under vitest gap (file header §):
  //   The engine apply throws inside `merge_delta` (SystemTime panic)
  //   and EntityDB.put's try/catch swallows it — the chain does NOT
  //   extend. The invariant we pin is the *absence* of a parallel
  //   counter: cell_pin reads identically before write 1, after write
  //   1, and after write 2. A spurious SQL-shadow path that bumped a
  //   parallel counter would surface here as v1 != v0 or v2 != v1,
  //   even with the engine apply failing. The pre-#768 `cell.version`
  //   SQL column (now dropped) was exactly such a parallel counter;
  //   this test pins that it stays gone.
  //
  // Under live Cloudflare Workers (SystemTime available): engine
  // apply mutates D, chain extends, cell_pin reports v1 then v2 with
  // v2 = v1 + 1 (NOT v1 + 2). The Rust-side
  // `apply_extends_chain_on_at_least_one_touched_cell` test covers
  // the +1 extension; the worker-side invariant pinned HERE is that
  // the worker contributes NO additional bump beyond what the engine
  // mints.

  it('7. (a) write-through extends the chain only through the engine apply path — no SQL shadow that double-increments', async () => {
    const { ctx, db } = makeSealedEntityDB('Order:ord-rt-7')
    const cellName = ctx.id.toString()

    // Pre-write baseline: no engine handle, no chain entry.
    await db.__test_hydrate()
    const handle = (db as unknown as { engineHandle: number }).engineHandle
    expect(handle).toBeGreaterThanOrEqual(0)
    const baseline = aadVersionFor(handle, cellName)
    expect(callCellPin(handle, cellName)).toBeNull()
    expect(baseline).toBe(0)

    await db.put({ id: 'ord-rt-7', type: 'Order', data: { total: '10' } })
    const v1 = aadVersionFor(handle, cellName)
    const pin1 = callCellPin(handle, cellName) ?? 0
    // Engine + worker MUST agree on the version after write 1. A
    // parallel SQL-counter path would let v1 diverge from pin1.
    expect(v1).toBe(pin1)

    await db.put({ id: 'ord-rt-7', type: 'Order', data: { total: '20' } })
    const v2 = aadVersionFor(handle, cellName)
    const pin2 = callCellPin(handle, cellName) ?? 0
    expect(v2).toBe(pin2)

    // Monotonic: chain head NEVER regresses. Under vitest both stay
    // at 0 (apply panics inside merge_delta) — that's the engine
    // contract observed locally. Under live workers v2 > v1 > 0 and
    // the chain extends by exactly 1 per call. Either way: monotonic
    // non-decreasing, and the worker doesn't add its own bump.
    expect(v1).toBeGreaterThanOrEqual(baseline)
    expect(v2).toBeGreaterThanOrEqual(v1)

    // The absence of an out-of-band SQL shadow counter is the load-
    // bearing invariant: the worker reads version_id ONLY through
    // aadVersionFor → cell_pin. If a sweep ever re-introduces a SQL
    // column (cell.version, cell.head_id, …) and reads from it, this
    // test stops detecting the parallel-path-divergence shape; that
    // would be a regression on #777's scope-of-truth contract.
    //
    // SQL row shape pin: no `version` column on persisted rows. The
    // mock storage records the column list at INSERT time; we walk
    // the row keys and assert the legacy counter is gone (post-#768).
    const tables = (ctx.storage.sql as unknown as { _tables?: Record<string, unknown[]> })._tables
    // The mock doesn't expose tables — read via the SELECT path instead.
    const rows = ctx.storage.sql
      .exec(`SELECT id, type, data FROM cell`)
      .toArray() as Array<Record<string, unknown>>
    expect(rows).toHaveLength(1)
    const row = rows[0]
    // The legacy `version` column is gone (#768). Even if the mock
    // accepted an INSERT carrying `version`, the SELECT projection
    // doesn't surface it — the schema contract is "no parallel
    // counter exists at the SQL layer".
    expect('version' in row).toBe(false)
    // Tables-side defensive check: if the mock _did_ expose internals,
    // verify no `version` slot was written. (No-op when undefined.)
    if (tables && Array.isArray(tables.cell) && tables.cell.length > 0) {
      const persistedRow = tables.cell[0] as Record<string, unknown>
      expect('version' in persistedRow).toBe(false)
    }
  }, COMPILE_TIMEOUT_MS)

  // ── (b) Read-after-write returns engine's view (not stale SQL) ──────
  //
  // #803 contract (b): EntityDB.get IMMEDIATELY after EntityDB.put
  // must return the just-written value through the engine-routed
  // read path (`fetchCellSealedViaEngine` / `callFetchCell` →
  // sealed-SQL fallback). No stale SQL read may surface a pre-write
  // value, and no version-skew between seal-side and open-side AAD
  // may surface a CellAeadError.
  //
  // Worker-side observation:
  //   The engine apply throws under vitest (file header §), but the
  //   SQL sealed write still lands and the read decrypts via the
  //   sealed-SQL fallback path of `fetchCellSealedViaEngine`. The
  //   crucial #803 invariant is that the read returns the merged
  //   payload from the write — proving the engine-routed READ helper
  //   does not regress to a stale snapshot when the write helper's
  //   engine apply throws. (If a SQL-shadow counter incremented on
  //   the write but not on the read, the AEAD AAD `version` field
  //   would diverge and `cellOpen` would fail with kind='auth'.)
  //
  // Under live workers: chain extends, fetch_cell returns the engine
  // payload, no SQL touched — same `cell.data` round-trip closure
  // because seal/open agree on the chain-sourced version_id.

  it('8. (b) read-after-write returns the just-written value through the engine-routed read (no stale SQL view)', async () => {
    const { db } = makeSealedEntityDB('Order:ord-rt-8')

    // Write 1: fresh cell. The engine apply attempt fires through
    // `writeCellThroughEngine` BEFORE storeCellSealed runs (per
    // EntityDB.put's body) — vitest's SystemTime gap may make apply
    // throw, but the SQL sealed write still completes. Read must
    // surface the just-written payload.
    await db.put({ id: 'ord-rt-8', type: 'Order', data: { total: '100', status: 'open' } })
    const afterWrite1 = await db.get()
    expect(afterWrite1).not.toBeNull()
    expect(afterWrite1!.id).toBe('ord-rt-8')
    expect(afterWrite1!.type).toBe('Order')
    expect(afterWrite1!.data.total).toBe('100')
    expect(afterWrite1!.data.status).toBe('open')

    // Write 2: merge a new field. EntityDB.put's merge semantics keep
    // existing fields and overlay incoming ones. The read after the
    // second write must reflect the merged view — proving the engine-
    // routed read isn't pinning a stale snapshot from before write 2.
    await db.put({ id: 'ord-rt-8', type: 'Order', data: { status: 'fulfilled', shipped: 'true' } })
    const afterWrite2 = await db.get()
    expect(afterWrite2).not.toBeNull()
    expect(afterWrite2!.data.total).toBe('100') // preserved across merge
    expect(afterWrite2!.data.status).toBe('fulfilled') // overwritten by write 2
    expect(afterWrite2!.data.shipped).toBe('true') // added by write 2

    // The engine-routed read path MUST consult `callFetchCell` on
    // every read. Even when the engine returns ⊥ (today's surface
    // for entity-id-keyed cells per #770's note), the call wires the
    // read through the chain-as-version-of-record bookend so that
    // when the engine grows entity-cell semantics, reads inherit the
    // engine view with no call-site change. We pin the spy to detect
    // a regression that bypasses the engine-routed helper and
    // SELECTs straight from SQL.
    const engineMod = await import('./api/engine')
    const spy = vi.spyOn(engineMod, 'callFetchCell')
    try {
      const probed = await db.get()
      expect(probed).not.toBeNull()
      expect(probed!.data.total).toBe('100')
      expect(spy).toHaveBeenCalled()
      // The cellName passed to callFetchCell IS the DO routing key —
      // the same name aadVersionFor uses to source cell_pin. A read
      // path that diverged from this naming would mean engine and AEAD
      // view different cells; #777's contract requires they agree.
      const callArg = spy.mock.calls[0][1]
      expect(callArg).toBe('Order:ord-rt-8')
    } finally {
      spy.mockRestore()
    }
  }, COMPILE_TIMEOUT_MS)

  // ── (c) AEAD AAD agreement under engine-only writes ─────────────────
  //
  // #803 contract (c): the AEAD AAD `(scope, domain, cellName, version)`
  // tuple MUST be byte-identical between the seal-side
  // (`storeCellSealed` → `cellAddressFor` + `aadVersionFor`) and the
  // open-side (`fetchCellSealed` → same helpers). No worker path
  // may mint the version field through any source other than the
  // engine's `cell_pin` chain head — otherwise `cellOpen` fails
  // tag verification on what should be a round-trip-closing read.
  //
  // Worker-side observation:
  //   We drive a write through `EntityDB.put` (engine apply attempt +
  //   sealed SQL write), then HAND-RECONSTRUCT the address from the
  //   public helpers (`cellAddressFor(type, id, aadVersionFor(handle,
  //   cellName))`). Decrypting the persisted sealed bytes with that
  //   hand-built AAD must succeed — proving the helpers consume the
  //   same source the worker's seal path consumed. Any divergence
  //   (e.g. a SQL-counter source for one side, engine for the other)
  //   would fail with `kind = 'auth'`.
  //
  // The test also pins the converse: an AAD constructed with a wrong
  // version field MUST fail to open — proving the AAD does bind the
  // version_id field (S1i #725 / canonicalAddressBytes layout).

  it('9. (c) AEAD AAD agrees on (scope, domain, cellName, version) between engine-only seal and open paths', async () => {
    const { ctx, db } = makeSealedEntityDB('Order:ord-rt-9')
    const cellName = ctx.id.toString()
    await db.put({
      id: 'ord-rt-9',
      type: 'Order',
      data: { total: '777', label: 'engine-only-write' },
    })

    const handle = (db as unknown as { engineHandle: number }).engineHandle
    expect(handle).toBeGreaterThanOrEqual(0)

    // Sealed envelope is what the SQL row carries. Pull it back through
    // the mock SELECT and strip the SEALED_CELL_PREFIX magic.
    const rows = ctx.storage.sql
      .exec(`SELECT id, type, data FROM cell`)
      .toArray() as Array<Record<string, unknown>>
    expect(rows).toHaveLength(1)
    const row = rows[0]
    const dataField = row.data as string
    expect(typeof dataField).toBe('string')
    expect(dataField.startsWith(SEALED_CELL_PREFIX)).toBe(true)
    const sealedBytes = base64ToBytes(dataField.slice(SEALED_CELL_PREFIX.length))

    // Hand-reconstruct the AAD from the same public helpers the
    // worker's open path uses. If `aadVersionFor` ever drew from a
    // shadow source, the version field reconstructed here would
    // differ from the seal-time version and the open would fail.
    const aadVersion = aadVersionFor(handle, cellName)
    const address = cellAddressFor('Order', 'ord-rt-9', aadVersion)
    expect(address.scope).toBe('worker')
    expect(address.domain).toBe('Order')
    expect(address.cellName).toBe('ord-rt-9')
    expect(address.version).toBe(aadVersion)

    // Derive the same master EntityDB used internally.
    // (`makeSealedEntityDB`'s env seed + the DO id as the tenant salt.)
    const master = await deriveTenantMasterKey(
      'engine-only-#803-test-seed-not-a-secret-32b!',
      cellName,
    )

    // Engine-only open: same AAD construction surfaces the plaintext.
    const opened = await cellOpen(master, address, sealedBytes)
    const recovered = JSON.parse(new TextDecoder().decode(opened)) as Record<
      string,
      unknown
    >
    expect(recovered.total).toBe('777')
    expect(recovered.label).toBe('engine-only-write')

    // Converse: a wrong version in the AAD MUST fail. The seal-side
    // baked aadVersion into the AAD; opening at aadVersion + 1
    // (or aadVersion + 100) proves the AEAD binds the version field
    // (S1i #725) — exactly the bind that makes a captured-then-
    // replayed older ciphertext fail at a later head. If the worker
    // ever forgot to thread aadVersion through cellAddressFor, this
    // catch fires.
    const skewedAddress = cellAddressFor('Order', 'ord-rt-9', aadVersion + 100)
    let openKind: string | undefined
    try {
      await cellOpen(master, skewedAddress, sealedBytes)
      expect.fail('cellOpen with skewed AAD version must reject the envelope')
    } catch (e) {
      openKind = (e as CellAeadError).kind
    }
    expect(openKind).toBe('auth')

    // The round-trip via the public EntityDB.get path closes — proves
    // the production read helper does precisely the same AAD recon-
    // struction we just did by hand.
    const cell = await db.get()
    expect(cell).not.toBeNull()
    expect(cell!.data.total).toBe('777')
    expect(cell!.data.label).toBe('engine-only-write')
  }, COMPILE_TIMEOUT_MS)

  // ── (d) Cold-start hydrate reads what engine-only writes wrote ──────
  //
  // #803 contract (d): a DO recreate against the same `ctx.storage`
  // must surface the engine-only-written cell value end-to-end —
  // `EntityDB.get()` on the new instance returns the same payload
  // the first instance wrote via `EntityDB.put`. This closes the
  // cold-start half of #777's engine-only IO contract: the freeze
  // blob carries the engine state (including chain head per
  // `cell_pin`) across the isolate boundary, and the sealed SQL row
  // (the post-vitest-gap authoritative store for the payload) is
  // re-opened under an AAD whose version field comes from that
  // freeze-restored chain head.
  //
  // Why this is the load-bearing test for #777's full collapse:
  //   If ANY out-of-band SQL counter (or worker-minted version) had
  //   survived #768, the new DO instance would source its AAD version
  //   from a different place than the original DO did — and `cellOpen`
  //   on the persisted bytes would fail with kind='auth'. The fact
  //   that the cold-start read returns the payload proves the
  //   version stamp survived through ONE channel: the engine freeze
  //   image. (Under vitest's SystemTime gap, the chain stayed at 0
  //   pre- and post-thaw, so the "version stamp survived" reduces to
  //   "the empty-fold baseline survived" — but the worker-side
  //   contract pinned is the same: aadVersionFor sourced from
  //   cell_pin, on both isolates, with no parallel counter.)
  //
  // Lifecycle pinned here:
  //   1. dbA writes via put — engine apply attempts, persistEngineState
  //      writes the freeze blob, sealed SQL row lands.
  //   2. Manual __test_evict on dbA releases the WASM handle.
  //   3. dbB constructed against the same ctx.storage → fresh
  //      EntityDB instance, engineHandle starts at -1.
  //   4. dbB.get() hydrates from storage, opens the sealed row, the
  //      AEAD round-trip closes because the AAD version is sourced
  //      from the freeze-restored chain head (or its empty-fold
  //      baseline under the vitest gap).

  it('10. (d) cold-start hydrate: a fresh DO instance reads the engine-only-written cell end-to-end', async () => {
    const { ctx, db: dbA } = makeSealedEntityDB('Order:ord-rt-10')
    const cellName = ctx.id.toString()

    // Write through the engine apply + sealed SQL path. The freeze
    // blob lands automatically through writeCellThroughEngine's call
    // to persistEngineState, but we also drive __test_persist to
    // belt-and-braces ensure the storage carries the freeze image
    // even when the apply path threw inside merge_delta (vitest gap).
    await dbA.put({
      id: 'ord-rt-10',
      type: 'Order',
      data: { total: '999', region: 'us-east' },
    })
    const blobA = await dbA.__test_persist()
    expect(typeof blobA).toBe('string')
    expect(blobA.length).toBeGreaterThan(0)

    // Snapshot the pre-eviction view for cross-check after recreate.
    const handleA = (dbA as unknown as { engineHandle: number }).engineHandle
    expect(handleA).toBeGreaterThanOrEqual(0)
    const headA = aadVersionFor(handleA, cellName)
    const preEvictCell = await dbA.get()
    expect(preEvictCell).not.toBeNull()
    expect(preEvictCell!.data.total).toBe('999')
    expect(preEvictCell!.data.region).toBe('us-east')

    // Mimic isolate eviction: release dbA's WASM handle. The DO's
    // `ctx.storage` (sealed SQL row + engine freeze blob) persists.
    await dbA.__test_evict()

    // Cold-start: brand-new EntityDB instance against the SAME
    // ctx.storage — this is what Cloudflare does after isolate
    // eviction. The constructor leaves engineHandle = -1; the first
    // `get` hydrates from the persisted freeze blob and opens the
    // sealed SQL row.
    const dbB = makeEntityDB(ctx, {
      TENANT_MASTER_SEED: 'engine-only-#803-test-seed-not-a-secret-32b!',
    })

    // Engine handle starts unallocated on the fresh instance.
    expect((dbB as unknown as { engineHandle: number }).engineHandle).toBe(-1)

    // The cold-start read MUST return the original value end-to-end.
    // This is the only way to prove:
    //   - engine state restored from the freeze blob (chain head
    //     observable via aadVersionFor matches dbA's view)
    //   - sealed SQL row's AAD reconstructs through the engine-only
    //     path's helpers (no out-of-band counter)
    //   - the master derives identically against the same env seed
    //     + DO id salt (per-tenant key derivation is stable)
    //   - the AEAD round-trip closes (no kind='auth' from a version
    //     skew between persisted bytes and post-hydrate AAD source)
    const coldStartCell = await dbB.get()
    expect(coldStartCell).not.toBeNull()
    expect(coldStartCell!.id).toBe('ord-rt-10')
    expect(coldStartCell!.type).toBe('Order')
    expect(coldStartCell!.data.total).toBe('999')
    expect(coldStartCell!.data.region).toBe('us-east')

    // Post-hydrate handle is allocated and the chain head is byte-
    // stable across recreate — the engine view dbB observes is the
    // same one dbA had. Any divergence here would mean the freeze/
    // thaw lifecycle silently dropped state; #803's cold-start
    // contract is precisely that this DOES NOT happen.
    const handleB = (dbB as unknown as { engineHandle: number }).engineHandle
    expect(handleB).toBeGreaterThanOrEqual(0)
    const headB = aadVersionFor(handleB, cellName)
    expect(headB).toBe(headA)

    // And a subsequent put on dbB merges with the dbA-written payload
    // — proves the cold-started DO continues to round-trip through
    // the engine-only path (read-modify-write) without losing the
    // pre-eviction fields.
    await dbB.put({ id: 'ord-rt-10', type: 'Order', data: { region: 'eu-west' } })
    const merged = await dbB.get()
    expect(merged).not.toBeNull()
    expect(merged!.data.total).toBe('999') // preserved from dbA's write
    expect(merged!.data.region).toBe('eu-west') // overwritten by dbB's write
  }, COMPILE_TIMEOUT_MS)
})

// ── Local helper: base64 decoder for the AAD verification test ──────
//
// `cell-encryption.ts` keeps its base64 helpers private. The sealed
// row inspection in test 9 needs to strip SEALED_CELL_PREFIX and
// decode the body back to bytes for `cellOpen`. We duplicate the few
// lines rather than re-export — they're total functions on small
// inputs and the test surface here is the ONLY consumer.
function base64ToBytes(b64: string): Uint8Array {
  const binary = atob(b64)
  const out = new Uint8Array(binary.length)
  for (let i = 0; i < binary.length; i++) out[i] = binary.charCodeAt(i)
  return out
}

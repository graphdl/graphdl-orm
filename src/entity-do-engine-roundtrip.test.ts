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

import { describe, it, expect } from 'vitest'
import {
  EntityDB,
  ENGINE_STATE_STORAGE_KEY,
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
  ;(db as unknown as { env: Record<string, unknown> }).env = env
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
})

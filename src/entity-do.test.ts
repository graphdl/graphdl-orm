import { describe, it, expect } from 'vitest'
import type { SqlLike } from './entity-do'
import {
  initCellSchema, fetchCell, storeCell, removeCell,
  getFacts, getFactsBySchema, toPopulation,
  initSecretSchema, storeSecret, resolveSecret, deleteSecret, listConnectedSystems,
  SEALED_CELL_PREFIX,
  EntityDB,
} from './entity-do'

function createMockSql(): SqlLike & { tables: Record<string, any[]> } {
  const tables: Record<string, any[]> = {}
  return {
    tables,
    exec(query: string, ...params: any[]) {
      const n = query.replace(/\s+/g, ' ').trim()

      if (/^CREATE/i.test(n)) {
        const m = n.match(/(?:TABLE|INDEX)\s+(?:IF\s+NOT\s+EXISTS\s+)?(?:idx_\w+\s+ON\s+)?(\w+)/i)
        if (m && !tables[m[1]]) tables[m[1]] = []
        return { toArray: () => [] }
      }

      // DROP TABLE — just remove if exists
      if (/^DROP TABLE/i.test(n)) {
        const m = n.match(/DROP TABLE (\w+)/i)
        if (m && tables[m[1]]) delete tables[m[1]]
        return { toArray: () => [] }
      }

      // INSERT OR REPLACE
      if (/INSERT OR REPLACE/i.test(n)) {
        const m = n.match(/INSERT OR REPLACE INTO (\w+)\s*\(([^)]+)\)\s*VALUES/i)
        if (m) {
          const t = m[1], cols = m[2].split(',').map(c => c.trim())
          if (!tables[t]) tables[t] = []
          const row: any = {}; cols.forEach((c, i) => row[c] = params[i])
          const idx = tables[t].findIndex((r: any) => r[cols[0]] === row[cols[0]])
          if (idx >= 0) tables[t][idx] = row; else tables[t].push(row)
        }
        return { toArray: () => [] }
      }

      // INSERT
      if (/^INSERT INTO/i.test(n)) {
        const m = n.match(/INSERT INTO (\w+)\s*\(([^)]+)\)\s*VALUES/i)
        if (m) {
          const t = m[1], cols = m[2].split(',').map(c => c.trim())
          if (!tables[t]) tables[t] = []
          const row: any = {}; cols.forEach((c, i) => row[c] = params[i])
          tables[t].push(row)
        }
        return { toArray: () => [] }
      }

      // DELETE
      if (/^DELETE FROM (\w+)$/i.test(n)) {
        const m = n.match(/DELETE FROM (\w+)/i)
        if (m && tables[m[1]]) tables[m[1]] = []
        return { toArray: () => [] }
      }
      if (/^DELETE FROM/i.test(n)) {
        const m = n.match(/DELETE FROM (\w+) WHERE (\w+)\s*=\s*\?/i)
        if (m) {
          const t = m[1], col = m[2]
          if (tables[t]) tables[t] = tables[t].filter((r: any) => r[col] !== params[0])
        }
        return { toArray: () => [] }
      }

      // SELECT id, type, data FROM cell  (legacy plaintext path)
      if (/^SELECT id, type, data FROM cell/i.test(n)) {
        return { toArray: () => [...(tables['cell'] || [])] }
      }

      // SELECT id, type, data, version FROM cell  (versioned sealed path #661)
      if (/^SELECT id, type, data, version FROM cell/i.test(n)) {
        return { toArray: () => [...(tables['cell'] || [])] }
      }

      // SELECT version FROM cell WHERE id = ?  (sealed write read-modify-write)
      if (/^SELECT version FROM cell WHERE id = \?/i.test(n)) {
        return {
          toArray: () => (tables['cell'] || [])
            .filter((r: any) => r.id === params[0])
            .map((r: any) => ({ version: r.version ?? 0 })),
        }
      }

      // SELECT value FROM secrets WHERE system = ?
      if (/SELECT value FROM secrets/i.test(n)) {
        return { toArray: () => (tables['secrets'] || []).filter((r: any) => r.system === params[0]).map((r: any) => ({ value: r.value })) }
      }

      // SELECT system FROM secrets ORDER BY
      if (/SELECT system FROM secrets/i.test(n)) {
        return { toArray: () => [...(tables['secrets'] || [])].sort((a: any, b: any) => a.system > b.system ? 1 : -1).map(r => ({ system: r.system })) }
      }

      // ALTER TABLE — throw to simulate "already exists"
      if (/^ALTER/i.test(n)) {
        throw new Error('column already exists')
      }

      // SELECT * FROM <table>
      const sel = n.match(/SELECT \* FROM (\w+)/i)
      if (sel) return { toArray: () => [...(tables[sel[1]] || [])] }

      return { toArray: () => [] }
    },
  }
}

describe('entity-do (cell model)', () => {
  describe('cell operations (↑n / ↓n)', () => {
    it('storeCell creates a cell, fetchCell retrieves it', () => {
      const sql = createMockSql()
      initCellSchema(sql)

      const cell = storeCell(sql, 'alice@example.com', 'Customer', {
        name: 'Alice', plan: 'Growth',
      })

      expect(cell.id).toBe('alice@example.com')
      expect(cell.type).toBe('Customer')
      expect(cell.data.name).toBe('Alice')
      expect(cell.data.plan).toBe('Growth')

      const fetched = fetchCell(sql)
      expect(fetched?.id).toBe('alice@example.com')
      expect(fetched?.type).toBe('Customer')
      expect(fetched?.data.name).toBe('Alice')
    })

    it('fetchCell returns null for empty DO', () => {
      const sql = createMockSql()
      initCellSchema(sql)

      expect(fetchCell(sql)).toBeNull()
    })

    it('storeCell replaces existing cell contents', () => {
      const sql = createMockSql()
      initCellSchema(sql)

      storeCell(sql, 'cust-1', 'Customer', { name: 'Alice' })
      storeCell(sql, 'cust-1', 'Customer', { name: 'Alice', plan: 'Growth' })

      const cell = fetchCell(sql)
      expect(cell?.data.name).toBe('Alice')
      expect(cell?.data.plan).toBe('Growth')
    })

    it('removeCell hard-deletes the cell', () => {
      const sql = createMockSql()
      initCellSchema(sql)

      storeCell(sql, 'cust-1', 'Customer', { name: 'Alice' })
      const result = removeCell(sql)

      expect(result?.id).toBe('cust-1')
      expect(fetchCell(sql)).toBeNull()
    })

    it('removeCell returns null when no cell exists', () => {
      const sql = createMockSql()
      initCellSchema(sql)

      expect(removeCell(sql)).toBeNull()
    })
  })

  describe('fact projection', () => {
    it('projects cell data as facts', () => {
      const sql = createMockSql()
      initCellSchema(sql)
      storeCell(sql, 'alice@example.com', 'Customer', {
        name: 'Alice', plan: 'Growth', email: 'alice@example.com',
      })

      const facts = getFacts(sql)
      expect(facts.length).toBe(3)

      const nameFact = facts.find(f => f.graphSchemaId === 'Customer has name')
      expect(nameFact).toBeDefined()
      expect(nameFact!.bindings[0]).toEqual(['Customer', 'alice@example.com'])
      expect(nameFact!.bindings[1]).toEqual(['name', 'Alice'])
    })

    it('projects facts by schema', () => {
      const sql = createMockSql()
      initCellSchema(sql)
      storeCell(sql, 'cust-1', 'Customer', {
        name: 'Alice', plan: 'Growth', email: 'a@b.com',
      })

      const planFacts = getFactsBySchema(sql, 'Customer has plan')
      expect(planFacts).toHaveLength(1)
      expect(planFacts[0].bindings[1]).toEqual(['plan', 'Growth'])
    })

    it('converts to population format', () => {
      const sql = createMockSql()
      initCellSchema(sql)
      storeCell(sql, 'cust-1', 'Customer', { name: 'Alice', plan: 'Growth' })

      const pop = toPopulation(sql)
      expect(Object.keys(pop)).toHaveLength(2)
      expect(pop['Customer has name']).toHaveLength(1)
      expect(pop['Customer has plan']).toHaveLength(1)
    })

    it('empty data produces no facts', () => {
      const sql = createMockSql()
      initCellSchema(sql)
      storeCell(sql, 'cust-1', 'Customer', {})

      expect(getFacts(sql)).toHaveLength(0)
    })

    it('bindings ordered: entity first, field second', () => {
      const sql = createMockSql()
      initCellSchema(sql)
      storeCell(sql, 'ord-1', 'Order', { customer: 'alice' })

      const facts = getFacts(sql)
      expect(facts[0].bindings[0][0]).toBe('Order')
      expect(facts[0].bindings[1][0]).toBe('customer')
    })
  })

  describe('secret storage (infrastructure)', () => {
    it('stores and resolves', () => {
      const sql = createMockSql()
      initSecretSchema(sql)
      storeSecret(sql, 'acme-api', 'key_123')
      expect(resolveSecret(sql, 'acme-api')).toBe('key_123')
    })

    it('returns null for unknown', () => {
      const sql = createMockSql()
      initSecretSchema(sql)
      expect(resolveSecret(sql, 'nope')).toBeNull()
    })

    it('upserts', () => {
      const sql = createMockSql()
      initSecretSchema(sql)
      storeSecret(sql, 'acme-api', 'old')
      storeSecret(sql, 'acme-api', 'new')
      expect(resolveSecret(sql, 'acme-api')).toBe('new')
    })

    it('deletes', () => {
      const sql = createMockSql()
      initSecretSchema(sql)
      storeSecret(sql, 'acme-api', 'key')
      deleteSecret(sql, 'acme-api')
      expect(resolveSecret(sql, 'acme-api')).toBeNull()
    })

    it('lists systems', () => {
      const sql = createMockSql()
      initSecretSchema(sql)
      storeSecret(sql, 'acme-api', 'k1')
      storeSecret(sql, 'email-svc', 'k2')
      storeSecret(sql, 'analytics-db', 'k3')
      expect(listConnectedSystems(sql)).toEqual(['acme-api', 'analytics-db', 'email-svc'])
    })

    it('isolated from cell data', () => {
      const sql = createMockSql()
      initCellSchema(sql)
      initSecretSchema(sql)
      storeCell(sql, 'org-1', 'Organization', { name: 'Acme' })
      storeSecret(sql, 'acme-api', 'secret_value')

      const facts = getFacts(sql)
      expect(JSON.stringify(facts)).not.toContain('secret_value')
    })
  })

  // ── Per-cell version stamp tests removed (#768) ────────────────────
  //
  // The four tests that pinned the worker-minted SQL `cell.version`
  // counter (#661) were deleted as part of #768 (#721-followup-e):
  // the column is gone, the chain head from `system(h, "cell_pin", id)`
  // is now the canonical version source per AREST.tex §462 eq:cellfold
  // and §472. Sibling task #769 (#721-followup-f) lands fresh
  // engine-round-trip coverage that pins the new contract end-to-end.

  // ── getMaster: hard-fail on missing TENANT_MASTER_SEED (Sweep-8a / #809) ──
  //
  // Silently degrading the "encrypted" cell store to plaintext on a
  // missing master seed was a security foot-gun: an operator who
  // forgot the `wrangler secret put TENANT_MASTER_SEED` step would
  // ship a Worker that persists tenant data in the clear without
  // any visible signal. The DO now refuses to fall back unless BOTH
  // `ENVIRONMENT !== 'production'` AND `AREST_ALLOW_PLAINTEXT === '1'`
  // are set — production cannot run plaintext, even with a stray
  // opt-in flag.
  describe('getMaster missing-seed policy (#809)', () => {
    function makeEntityDB(env: Record<string, string | undefined>): EntityDB {
      // hydrateEngine + persistEngineState use ctx.storage.get/put for the
      // engine freeze blob. Stub them to mimic empty-storage behaviour.
      const kv: Record<string, unknown> = {}
      const ctx = {
        id: { toString: () => 'tenant-test' },
        storage: {
          sql: createMockSql(),
          get: async <T>(key: string): Promise<T | undefined> => kv[key] as T | undefined,
          put: async (key: string, value: unknown): Promise<void> => { kv[key] = value },
          delete: async (key: string): Promise<boolean> => {
            const had = key in kv
            delete kv[key]
            return had
          },
        },
      }
      // The vitest cloudflare stub makes `DurableObject` an empty class,
      // so it never wires `ctx`/`env` for us — assign them directly.
      const db = Object.create(EntityDB.prototype) as EntityDB
      ;(db as any).ctx = ctx
      ;(db as any).env = env
      ;(db as any).initialized = false
      ;(db as any).master = null
      // Pretend the engine is already hydrated so put() / get() don't
      // trigger compileDomainReadings() — these tests exercise the
      // master-key gate, not the engine round-trip (which is covered
      // by entity-do-engine-roundtrip.test.ts).
      ;(db as any).engineHandle = 0
      return db
    }

    it('throws with an actionable message when TENANT_MASTER_SEED is unset', async () => {
      const db = makeEntityDB({})
      let threw: unknown = null
      try {
        await db.get()
      } catch (e) {
        threw = e
      }
      expect(threw).toBeInstanceOf(Error)
      const msg = (threw as Error).message
      // Names the missing env var so the operator knows what to bind.
      expect(msg).toContain('TENANT_MASTER_SEED')
      // Names the dev opt-in so the dev path is discoverable.
      expect(msg).toContain('AREST_ALLOW_PLAINTEXT')
      // Tells the operator what to do (the wrangler command).
      expect(msg).toContain('wrangler secret put TENANT_MASTER_SEED')
    })

    it('still throws when AREST_ALLOW_PLAINTEXT is set to a non-"1" value', async () => {
      // Belt-and-braces: only the literal string "1" opts in. Stray
      // truthy values (`"true"`, `"yes"`, `"0"`, etc.) still trip the
      // fail-closed branch so a typo in dev config can't accidentally
      // leak into production.
      for (const v of ['true', 'yes', '0', '', 'on']) {
        const db = makeEntityDB({ AREST_ALLOW_PLAINTEXT: v })
        let threw: unknown = null
        try {
          await db.put({ id: 'c1', type: 'Customer', data: { name: 'A' } })
        } catch (e) {
          threw = e
        }
        expect(threw, `value=${JSON.stringify(v)}`).toBeInstanceOf(Error)
        expect((threw as Error).message).toContain('TENANT_MASTER_SEED')
      }
    })

    it('still throws in production even when AREST_ALLOW_PLAINTEXT="1"', async () => {
      // The dual-gate's whole point: even with the dev opt-in
      // mistakenly set in production, the missing seed still fails
      // closed. A stray AREST_ALLOW_PLAINTEXT=1 in a prod Worker
      // env must not silently downgrade to plaintext storage.
      const db = makeEntityDB({
        AREST_ALLOW_PLAINTEXT: '1',
        ENVIRONMENT: 'production',
      })
      let threw: unknown = null
      try {
        await db.put({ id: 'c1', type: 'Customer', data: { name: 'A' } })
      } catch (e) {
        threw = e
      }
      expect(threw).toBeInstanceOf(Error)
      expect((threw as Error).message).toContain('TENANT_MASTER_SEED')
    })

    it('allows the legacy plaintext branch when AREST_ALLOW_PLAINTEXT="1" and not production', async () => {
      // The dev-only escape hatch. With both gates open (no
      // ENVIRONMENT=production, AREST_ALLOW_PLAINTEXT='1'),
      // getMaster() returns null and EntityDB.put runs without
      // throwing. Read-back round-trips through the engine-only
      // path (in-memory cell graph cache under vitest's apply-panic
      // gap, engine `fetch_cell` on live workers).
      const db = makeEntityDB({ AREST_ALLOW_PLAINTEXT: '1' })
      const stored = await db.put({
        id: 'cust-1',
        type: 'Customer',
        data: { name: 'Alice' },
      })
      expect(stored.id).toBe('cust-1')
      const fetched = await db.get()
      expect(fetched?.data.name).toBe('Alice')
    })

    it('proceeds through the master-bound branch when TENANT_MASTER_SEED is bound (engine-only IO, #885)', async () => {
      // The production happy path. With the seed bound, the DO
      // derives a per-tenant master and routes the put through
      // `writeCellThroughEngine`. Post-#885 the worker no longer
      // writes the sealed envelope to the SQL `cell` table — the
      // engine's chain is the version-of-record and the worker's
      // in-memory cell graph mirrors the engine view. The seal
      // prefix verification this test used to perform against the
      // SQL row is now covered by `entity-do-engine-roundtrip.test.ts`
      // test 9 against the address-level AAD helpers (engine-only).
      const db = makeEntityDB({
        TENANT_MASTER_SEED: 'this-is-a-test-seed-not-a-real-secret-32b!',
      })
      const stored = await db.put({
        id: 'cust-2',
        type: 'Customer',
        data: { name: 'Bob' },
      })
      expect(stored.data.name).toBe('Bob')
      // SQL cell table stays empty — the engine-only IO contract.
      const sql = (db as any).ctx.storage.sql as ReturnType<typeof createMockSql>
      expect(sql.tables['cell'] ?? []).toHaveLength(0)
      // Read-back round-trips through the engine-only path.
      const fetched = await db.get()
      expect(fetched?.data.name).toBe('Bob')
    })
  })

  // ── A-9 closure: getMaster hard-fails in production (#902) ──────────
  //
  // Closes A-9 from the bridge/legacy security sweep (#779's other half;
  // A-17 / #903 landed in 8720fdf0). #809 introduced the dual-gate
  // policy and #888 closed the read-path SEALED_CELL_PREFIX fallback.
  // This block pins the precise contracts called out in the A-9 ticket:
  //
  //   (1) Production with no seed and no dev flag → hard-fail at the
  //       key-derivation gate, before any plaintext write can land.
  //   (2) Dev with the explicit `AREST_ALLOW_PLAINTEXT=1` opt-in →
  //       plaintext mode still works (the dev escape hatch survives).
  //   (3) `TENANT_MASTER_SEED` bound → returns a master normally.
  //
  // There is intentional overlap with #809 / #888 here — A-9 closes a
  // security finding, so it gets its own permanent regression pin
  // outside the sibling-task blocks. If a future refactor merges the
  // gates, deleting these tests should require explicit sign-off.
  describe('getMaster hard-fail in production (#902 / A-9)', () => {
    function makeEntityDB(env: Record<string, string | undefined>): EntityDB {
      const kv: Record<string, unknown> = {}
      const ctx = {
        id: { toString: () => 'tenant-902' },
        storage: {
          sql: createMockSql(),
          get: async <T>(key: string): Promise<T | undefined> => kv[key] as T | undefined,
          put: async (key: string, value: unknown): Promise<void> => { kv[key] = value },
          delete: async (key: string): Promise<boolean> => {
            const had = key in kv
            delete kv[key]
            return had
          },
        },
      }
      const db = Object.create(EntityDB.prototype) as EntityDB
      ;(db as any).ctx = ctx
      ;(db as any).env = env
      ;(db as any).initialized = false
      ;(db as any).master = null
      ;(db as any).engineHandle = 0
      return db
    }

    it('production without TENANT_MASTER_SEED throws at getMaster startup', async () => {
      // The canonical A-9 scenario: operator deployed to production
      // and forgot `wrangler secret put TENANT_MASTER_SEED`. No dev
      // opt-in is set. getMaster() must throw on the first key-
      // derivation attempt rather than silently writing plaintext.
      const db = makeEntityDB({ ENVIRONMENT: 'production' })
      let threw: unknown = null
      try {
        await db.put({ id: 'c-902-prod', type: 'Customer', data: { name: 'Eve' } })
      } catch (e) {
        threw = e
      }
      expect(threw).toBeInstanceOf(Error)
      const msg = (threw as Error).message
      // The error must name the missing env var so a deploy-time
      // grep against the worker logs surfaces it immediately.
      expect(msg).toContain('TENANT_MASTER_SEED')
      // And it must NOT have persisted anything — a fail-closed
      // gate means zero plaintext writes hit the cell table.
      const sql = (db as any).ctx.storage.sql as ReturnType<typeof createMockSql>
      expect(sql.tables['cell'] ?? []).toHaveLength(0)
    })

    it('dev with AREST_ALLOW_PLAINTEXT=1 permits the plaintext branch (engine-only IO, #885)', async () => {
      // The dev escape hatch is intentionally preserved so a local
      // worker without secret plumbing remains runnable. With the
      // dual gate open (non-prod ENVIRONMENT + AREST_ALLOW_PLAINTEXT
      // == '1'), getMaster() returns null and the write proceeds
      // through the engine-only path. Post-#885 there is no SQL
      // `cell` row written — the engine's chain is the version-of-
      // record and the worker's in-memory cell graph carries the
      // payload within the isolate.
      const db = makeEntityDB({ AREST_ALLOW_PLAINTEXT: '1' })
      const stored = await db.put({
        id: 'c-902-dev',
        type: 'Customer',
        data: { name: 'Mallory' },
      })
      expect(stored.id).toBe('c-902-dev')
      const fetched = await db.get()
      expect(fetched?.data.name).toBe('Mallory')
      // No SQL `cell` row — the engine-only IO contract (#885).
      const sql = (db as any).ctx.storage.sql as ReturnType<typeof createMockSql>
      expect(sql.tables['cell'] ?? []).toHaveLength(0)
    })

    it('getMaster with TENANT_MASTER_SEED set proceeds normally (engine-only IO, #885)', async () => {
      // Happy-path regression: a bound seed must continue to derive
      // a per-tenant master and the put must proceed through the
      // engine-only path without throwing. Post-#885 the SQL `cell`
      // row no longer carries a sealed envelope (engine chain is the
      // version-of-record), so the seal-prefix verification this
      // test used to perform against the SQL row has moved to the
      // address-level AAD helpers in entity-do-engine-roundtrip.test.ts.
      const db = makeEntityDB({
        TENANT_MASTER_SEED: 'a-9-closure-seed-#902-not-a-real-secret-32b!',
        ENVIRONMENT: 'production',
      })
      const stored = await db.put({
        id: 'c-902-prod-ok',
        type: 'Customer',
        data: { name: 'Carol' },
      })
      expect(stored.data.name).toBe('Carol')
      // No SQL `cell` row — the engine-only IO contract (#885).
      const sql = (db as any).ctx.storage.sql as ReturnType<typeof createMockSql>
      expect(sql.tables['cell'] ?? []).toHaveLength(0)
      const fetched = await db.get()
      expect(fetched?.data.name).toBe('Carol')
    })
  })

  // ── Sealed-cell prefix fail-loud contract (#888) ────────────────────
  //
  // Closes #777 (worker engine-only collapse) and a slice of #779
  // (security findings, A-9). Pre-#888 the worker would silently parse
  // a row missing the `SEALED_CELL_PREFIX` as legacy plaintext JSON —
  // a foot-gun for a supposedly-encrypted DB. After #888 a master-bound
  // read of a non-prefixed row throws naming the broken cell; rotation
  // refuses to no-op silently when the bytes are not sealed.
  describe('sealed-cell prefix is mandatory (#888)', () => {
    function makeEntityDB(env: Record<string, string | undefined>): EntityDB {
      const kv: Record<string, unknown> = {}
      const ctx = {
        id: { toString: () => 'tenant-888' },
        storage: {
          sql: createMockSql(),
          get: async <T>(key: string): Promise<T | undefined> => kv[key] as T | undefined,
          put: async (key: string, value: unknown): Promise<void> => { kv[key] = value },
          delete: async (key: string): Promise<boolean> => {
            const had = key in kv
            delete kv[key]
            return had
          },
        },
      }
      const db = Object.create(EntityDB.prototype) as EntityDB
      ;(db as any).ctx = ctx
      ;(db as any).env = env
      ;(db as any).initialized = false
      ;(db as any).master = null
      // Engine pre-hydrated so put()/get() don't trigger the WASM
      // compile path; this suite's unit-under-test is the seal-prefix
      // policy.
      ;(db as any).engineHandle = 0
      return db
    }

    it('round-trips a cell through the master-bound put/get path (engine-only IO, #885)', async () => {
      // Pin the happy path: write a cell with master bound, read it
      // back, confirm the round-trip closes. Post-#885 the SQL `cell`
      // table stays empty — the engine's chain is the version-of-
      // record and the worker's in-memory cell graph mirrors the
      // engine view for read-after-write.
      const db = makeEntityDB({
        TENANT_MASTER_SEED: 'sealed-cell-round-trip-seed-#888',
      })
      await db.put({ id: 'ord-888', type: 'Order', data: { status: 'open', total: '42' } })
      const sql = (db as any).ctx.storage.sql as ReturnType<typeof createMockSql>
      expect(sql.tables['cell'] ?? []).toHaveLength(0)
      const fetched = await db.get()
      expect(fetched).not.toBeNull()
      expect(fetched!.data.status).toBe('open')
      expect(fetched!.data.total).toBe('42')
    })

    it('fetchCellSealed throws when a hand-seeded SQL row lacks SEALED_CELL_PREFIX (legacy helper contract preserved, #888)', async () => {
      // The fail-loud contract on `fetchCellSealed` is preserved for
      // direct callers (rotation, AEAD-aware tooling) even though the
      // productive EntityDB.put/get path no longer writes/reads SQL
      // (#885). We drive the helper directly here so the regression
      // pin survives the engine-only collapse — a future caller that
      // reaches `fetchCellSealed` on a non-sealed row must still
      // throw naming the cell, not silently surface plaintext.
      const sql = createMockSql()
      // Hand-seed a plaintext row: bypasses storeCellSealed entirely.
      sql.exec(
        `INSERT OR REPLACE INTO cell (id, type, data) VALUES (?, ?, ?)`,
        'corrupt-1', 'Customer', JSON.stringify({ name: 'PlainAlice' }),
      )
      const { fetchCellSealed } = await import('./entity-do')
      const { deriveTenantMasterKey } = await import('./cell-encryption')
      const master = await deriveTenantMasterKey('sealed-cell-fail-loud-seed-#888', 'tenant-888')
      let threw: unknown = null
      try {
        await fetchCellSealed(sql, master, -1)
      } catch (e) {
        threw = e
      }
      expect(threw, 'fetchCellSealed of a plaintext row must throw').toBeInstanceOf(Error)
      const msg = (threw as Error).message
      // Must name the affected cell so the operator can locate it.
      expect(msg).toContain('corrupt-1')
      // Must reference the missing seal so the failure mode is obvious.
      expect(msg.toLowerCase()).toMatch(/seal|aead|encrypt/)
    })

    it('rotateMaster refuses to silently no-op on a non-sealed row', async () => {
      // Pre-#888 rotateMaster returned `{ ok: true, rotated: false }`
      // for any row missing the prefix — effectively the same plaintext
      // fallback foot-gun. After #888 it surfaces an explicit auth /
      // truncated kind so the rotation orchestrator can collect it
      // into its rotation report instead of pretending the row was
      // legitimately empty.
      const db = makeEntityDB({
        TENANT_MASTER_SEED: 'sealed-cell-rotation-fail-loud-seed-#888',
      })
      const sql = (db as any).ctx.storage.sql as ReturnType<typeof createMockSql>
      // Hand-seed a plaintext row, then drive rotation.
      sql.exec(
        `INSERT OR REPLACE INTO cell (id, type, data) VALUES (?, ?, ?)`,
        'corrupt-2', 'Order', JSON.stringify({ status: 'shipped' }),
      )
      let result: unknown = null
      let threw: unknown = null
      try {
        result = await db.rotateMaster({
          oldSeed: 'old-seed-#888',
          oldSalt: 'old-salt-#888',
          newSeed: 'new-seed-#888',
          newSalt: 'new-salt-#888',
        })
      } catch (e) {
        threw = e
      }
      // The non-sealed row is no longer treated as a clean no-op.
      // Acceptable failure shapes: either a thrown Error naming the
      // cell, or `{ ok: false, kind: 'auth' | 'truncated' }`. Both
      // surface the broken-row condition; both block silent success.
      const okFalse = result && typeof result === 'object'
        && (result as { ok?: boolean }).ok === false
      const threwError = threw instanceof Error
      expect(okFalse || threwError, JSON.stringify({ result, threw })).toBe(true)
      if (threwError) {
        expect((threw as Error).message).toMatch(/corrupt-2|seal|aead|sealed/i)
      }
    })
  })
})

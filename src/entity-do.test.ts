import { describe, it, expect } from 'vitest'
import type { SqlLike, Fact } from './entity-do'
import {
  initCellSchema, fetchCell, storeCell, removeCell,
  getFacts, getFactsBySchema, toPopulation,
  initSecretSchema, storeSecret, resolveSecret, deleteSecret, listConnectedSystems,
  storeCellSealed, fetchCellSealed,
  cellAddressFor, SEALED_CELL_PREFIX,
} from './entity-do'
import {
  cellOpen,
  CellAeadError,
  CELL_KEY_LEN,
  type TenantMasterKey,
} from './cell-encryption'

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

  // ── Per-cell version stamp (#661) ──────────────────────────────────
  //
  // Replay defence: every successful sealed write bumps the row's
  // `version` column, and that column is folded into the AEAD AAD via
  // `CellAddress.version`. A captured-then-replayed older sealed
  // envelope at the same `(scope, domain, cell_name)` no longer
  // decrypts because the persisted version (now N+1) doesn't match
  // the captured ciphertext's AAD (still N). These tests pin that
  // contract end-to-end through the EntityDB row schema.

  describe('per-cell version stamp (#661)', () => {
    function masterFromByte(byte: number): TenantMasterKey {
      const bytes = new Uint8Array(CELL_KEY_LEN)
      bytes.fill(byte)
      return { _bytes: bytes }
    }

    function base64ToBytes(b64: string): Uint8Array {
      const binary = atob(b64)
      const out = new Uint8Array(binary.length)
      for (let i = 0; i < binary.length; i++) out[i] = binary.charCodeAt(i)
      return out
    }

    /** Test 2: write/read round-trip at version 1 succeeds.
     *
     *  The first sealed write through `storeCellSealed` bumps the row
     *  from version 0 (default) to version 1. `fetchCellSealed` reads
     *  the persisted version and reconstructs the matching CellAddress,
     *  so the AEAD opener finds the right key + AAD and the round-trip
     *  recovers the original plaintext. */
    it('write/read round-trip at version 1 succeeds', async () => {
      const sql = createMockSql()
      initCellSchema(sql)
      const master = masterFromByte(0xa1)

      const stored = await storeCellSealed(
        sql, master, 'ord-1', 'Order',
        { item: 'widget', qty: 3 },
      )
      expect(stored.id).toBe('ord-1')

      // Inspect the persisted row — version must have been bumped to 1.
      const rows = (sql.tables['cell'] || []) as any[]
      expect(rows).toHaveLength(1)
      expect(rows[0].version).toBe(1)
      expect(typeof rows[0].data).toBe('string')
      expect(rows[0].data.startsWith(SEALED_CELL_PREFIX)).toBe(true)

      const fetched = await fetchCellSealed(sql, master)
      expect(fetched).not.toBeNull()
      expect(fetched!.id).toBe('ord-1')
      expect(fetched!.type).toBe('Order')
      expect(fetched!.data).toEqual({ item: 'widget', qty: 3 })
    })

    /** Test 3: write 5 times in succession — version monotonically
     *  increases (1 → 2 → 3 → 4 → 5); each read at the current
     *  version succeeds against the current sealed bytes. */
    it('successive writes monotonically bump the version', async () => {
      const sql = createMockSql()
      initCellSchema(sql)
      const master = masterFromByte(0xb2)

      for (let i = 1; i <= 5; i++) {
        await storeCellSealed(
          sql, master, 'ord-counter', 'Order',
          { tick: i },
        )
        const rows = (sql.tables['cell'] || []) as any[]
        expect(rows).toHaveLength(1)
        // Version is the count of successful writes so far.
        expect(rows[0].version).toBe(i)

        // The current row decrypts cleanly under the current address.
        const got = await fetchCellSealed(sql, master)
        expect(got).not.toBeNull()
        expect(got!.data).toEqual({ tick: i })
      }
    })

    /** Test 1: replay defence. Write a cell at version N, capture the
     *  sealed bytes + the matching CellAddress(N). Mutate the cell
     *  (write at N+1). Try to open the captured-N sealed bytes against
     *  CellAddress(N+1) — fails because the AAD now mismatches. This
     *  is the core invariant the version field defends. */
    it('replayed older sealed bytes fail under the bumped CellAddress (replay defence)', async () => {
      const sql = createMockSql()
      initCellSchema(sql)
      const master = masterFromByte(0xc3)

      // Round 1: write at version 1; capture sealed bytes + address(1).
      await storeCellSealed(
        sql, master, 'ord-replay', 'Order',
        { secret: 'first-write' },
      )
      const rowsAfterFirst = (sql.tables['cell'] || []) as any[]
      const capturedSealed = base64ToBytes(
        (rowsAfterFirst[0].data as string).slice(SEALED_CELL_PREFIX.length),
      )
      const capturedAddress = cellAddressFor('Order', 'ord-replay', 1)
      expect(capturedAddress.version).toBe(1)

      // The captured bytes still open under their captured (N=1)
      // address — sanity check that we have a valid envelope.
      const sanity = await cellOpen(master, capturedAddress, capturedSealed)
      expect(JSON.parse(new TextDecoder().decode(sanity))).toEqual({ secret: 'first-write' })

      // Round 2: mutate the cell (this bumps to version 2).
      await storeCellSealed(
        sql, master, 'ord-replay', 'Order',
        { secret: 'second-write' },
      )
      const rowsAfterSecond = (sql.tables['cell'] || []) as any[]
      expect(rowsAfterSecond[0].version).toBe(2)

      // The captured-at-N=1 sealed bytes opened against the NEW
      // address (version=2) MUST fail — the AAD is different, the
      // HKDF salt is different, and Poly1305 surfaces the mismatch
      // as auth failure.
      const newAddress = cellAddressFor('Order', 'ord-replay', 2)
      let threw: unknown = null
      try {
        await cellOpen(master, newAddress, capturedSealed)
      } catch (e) {
        threw = e
      }
      expect(threw).toBeInstanceOf(CellAeadError)
      expect((threw as CellAeadError).kind).toBe('auth')

      // And `fetchCellSealed` (which reads the persisted version)
      // sees the LATEST cell, not the captured one.
      const live = await fetchCellSealed(sql, master)
      expect(live!.data).toEqual({ secret: 'second-write' })
    })

    /** Test 4: cold-start replay path — a fresh isolate (simulated by
     *  a brand-new fetch against the same persisted SQL state) must
     *  pick up the persisted version, NOT default to 0. Without this,
     *  every read after the first write would surface as
     *  `CellAeadError(auth)` because the opener would derive the key
     *  under version 0 while the bytes were sealed under version N>0. */
    it('cold-start: a fresh fetch picks up the persisted version, not 0', async () => {
      const sql = createMockSql()
      initCellSchema(sql)
      const master = masterFromByte(0xd4)

      // Drive the version up to 3 by writing three times.
      await storeCellSealed(sql, master, 'ord-cs', 'Order', { v: 1 })
      await storeCellSealed(sql, master, 'ord-cs', 'Order', { v: 2 })
      await storeCellSealed(sql, master, 'ord-cs', 'Order', { v: 3 })
      const rows = (sql.tables['cell'] || []) as any[]
      expect(rows[0].version).toBe(3)

      // Simulate a cold isolate: drop the (in-process) module-level
      // engine state and re-derive the master from the persisted row
      // alone. The mockSql `tables` survives — that's the persisted
      // state the new isolate would see. There's no in-memory
      // version cache to warm; `fetchCellSealed` must reconstruct
      // the address from the row's `version` column alone.
      //
      // (We deliberately use a NEW master instance with the same
      // raw bytes to drive home that "fresh isolate" means no shared
      // in-memory state. Same bytes derive the same per-cell key, so
      // the open succeeds iff the version is read correctly.)
      const masterFresh: TenantMasterKey = { _bytes: new Uint8Array(master._bytes) }
      const fetched = await fetchCellSealed(sql, masterFresh)
      expect(fetched).not.toBeNull()
      expect(fetched!.data).toEqual({ v: 3 })

      // Also exercise the replay-defence post-cold-start: a stale
      // sealed envelope captured at version 1 must STILL fail to
      // open under the persisted version (3) even on a fresh isolate.
      // (We re-seal at v=1 explicitly to obtain a "captured" envelope.)
      const sql2 = createMockSql()
      initCellSchema(sql2)
      await storeCellSealed(sql2, master, 'ord-cs', 'Order', { v: 1 })
      const rows2 = (sql2.tables['cell'] || []) as any[]
      const capturedAtV1 = base64ToBytes(
        (rows2[0].data as string).slice(SEALED_CELL_PREFIX.length),
      )
      const liveAddress = cellAddressFor('Order', 'ord-cs', 3)
      let threw: unknown = null
      try {
        await cellOpen(masterFresh, liveAddress, capturedAtV1)
      } catch (e) {
        threw = e
      }
      expect(threw).toBeInstanceOf(CellAeadError)
    })
  })
})

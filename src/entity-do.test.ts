import { describe, it, expect } from 'vitest'
import type { SqlLike, Fact } from './entity-do'
import {
  initCellSchema, fetchCell, storeCell, removeCell,
  getFacts, getFactsBySchema, toPopulation,
  initSecretSchema, storeSecret, resolveSecret, deleteSecret, listConnectedSystems,
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

      // SELECT id, type, data FROM cell
      if (/SELECT id, type, data FROM cell/i.test(n)) {
        return { toArray: () => [...(tables['cell'] || [])] }
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
})

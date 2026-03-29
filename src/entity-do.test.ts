import { describe, it, expect, vi } from 'vitest'
import type { SqlLike, Fact } from './entity-do'
import {
  initEntitySchema, createEntity, getEntity, updateEntity, deleteEntity, getEvents,
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

      // INSERT ... ON CONFLICT
      if (/ON\s+CONFLICT/i.test(n)) {
        const m = n.match(/INSERT INTO (\w+)\s*\(([^)]+)\)\s*VALUES.*ON\s+CONFLICT\s*\(\s*(\w+)\s*\)/i)
        if (m) {
          const t = m[1], cols = m[2].split(',').map((c: string) => c.trim()), pk = m[3]
          if (!tables[t]) tables[t] = []
          const row: any = {}; cols.forEach((c, i) => row[c] = params[i])
          const ex = tables[t].find((r: any) => r[pk] === row[pk])
          if (ex) Object.assign(ex, row); else tables[t].push(row)
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

      // UPDATE
      if (/^UPDATE/i.test(n)) {
        const m = n.match(/UPDATE (\w+) SET (.+?) WHERE/i)
        if (m) {
          const t = m[1]
          const sets = m[2].split(',').map(s => s.trim().split(/\s*=\s*/))
          for (const row of (tables[t] || [])) {
            if (row.id === params[params.length - 1]) {
              let pi = 0
              for (const [col, expr] of sets) {
                if (expr === 'version + 1') row.version = (row.version || 0) + 1
                else row[col] = params[pi++]
              }
            }
          }
        }
        return { toArray: () => [] }
      }

      // DELETE
      if (/^DELETE FROM/i.test(n)) {
        const m = n.match(/DELETE FROM (\w+) WHERE (\w+)\s*=\s*\?/i)
        if (m) {
          const t = m[1], col = m[2]
          if (tables[t]) tables[t] = tables[t].filter((r: any) => r[col] !== params[0])
        }
        return { toArray: () => [] }
      }

      // SELECT value FROM secrets WHERE system = ?
      if (/SELECT value FROM secrets/i.test(n)) {
        return { toArray: () => (tables['secrets'] || []).filter((r: any) => r.system === params[0]).map((r: any) => ({ value: r.value })) }
      }

      // SELECT system FROM secrets ORDER BY
      if (/SELECT system FROM secrets/i.test(n)) {
        return { toArray: () => [...(tables['secrets'] || [])].sort((a: any, b: any) => a.system > b.system ? 1 : -1).map(r => ({ system: r.system })) }
      }

      // SELECT * FROM events WHERE
      if (/FROM events WHERE/i.test(n)) {
        return { toArray: () => (tables['events'] || []).filter((r: any) => r.timestamp > params[0]).sort((a: any, b: any) => b.timestamp > a.timestamp ? 1 : -1) }
      }
      if (/FROM events ORDER/i.test(n)) {
        return { toArray: () => [...(tables['events'] || [])].sort((a: any, b: any) => b.timestamp > a.timestamp ? 1 : -1) }
      }

      // SELECT * FROM <table>
      const sel = n.match(/SELECT \* FROM (\w+)/i)
      if (sel) return { toArray: () => [...(tables[sel[1]] || [])] }

      return { toArray: () => [] }
    },
  }
}

describe('entity-do', () => {
  describe('3NF row storage', () => {
    it('creates entity with fields', () => {
      const sql = createMockSql()
      initEntitySchema(sql)

      const row = createEntity(sql, 'alice@example.com', 'Customer', 'support', {
        name: 'Alice', plan: 'Growth',
      })

      expect(row.id).toBe('alice@example.com')
      expect(row.noun).toBe('Customer')
      expect(row.fields.name).toBe('Alice')
      expect(row.fields.plan).toBe('Growth')
    })

    it('retrieves entity row', () => {
      const sql = createMockSql()
      initEntitySchema(sql)
      createEntity(sql, 'cust-1', 'Customer', 'core', { name: 'Bob' })

      const row = getEntity(sql)
      expect(row?.id).toBe('cust-1')
      expect(row?.fields.name).toBe('Bob')
    })

    it('updates fields by merge', () => {
      const sql = createMockSql()
      initEntitySchema(sql)
      createEntity(sql, 'cust-1', 'Customer', 'core', { name: 'Alice' })

      const updated = updateEntity(sql, { plan: 'Growth' })
      expect(updated?.fields.name).toBe('Alice')
      expect(updated?.fields.plan).toBe('Growth')
    })
  })

  describe('fact projection', () => {
    it('projects row fields as facts', () => {
      const sql = createMockSql()
      initEntitySchema(sql)
      createEntity(sql, 'alice@example.com', 'Customer', 'core', {
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
      initEntitySchema(sql)
      createEntity(sql, 'cust-1', 'Customer', 'core', {
        name: 'Alice', plan: 'Growth', email: 'a@b.com',
      })

      const planFacts = getFactsBySchema(sql, 'Customer has plan')
      expect(planFacts).toHaveLength(1)
      expect(planFacts[0].bindings[1]).toEqual(['plan', 'Growth'])
    })

    it('converts to population format', () => {
      const sql = createMockSql()
      initEntitySchema(sql)
      createEntity(sql, 'cust-1', 'Customer', 'core', { name: 'Alice', plan: 'Growth' })

      const pop = toPopulation(sql)
      expect(Object.keys(pop)).toHaveLength(2)
      expect(pop['Customer has name']).toHaveLength(1)
      expect(pop['Customer has plan']).toHaveLength(1)
    })

    it('empty fields produce no facts', () => {
      const sql = createMockSql()
      initEntitySchema(sql)
      createEntity(sql, 'cust-1', 'Customer', 'core', {})

      expect(getFacts(sql)).toHaveLength(0)
    })

    it('bindings ordered by noun order: entity first, field second', () => {
      const sql = createMockSql()
      initEntitySchema(sql)
      createEntity(sql, 'ord-1', 'Order', 'orders', { customer: 'alice' })

      const facts = getFacts(sql)
      expect(facts[0].bindings[0][0]).toBe('Order')
      expect(facts[0].bindings[1][0]).toBe('customer')
    })
  })

  describe('secret storage', () => {
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

    it('isolated from entity data', () => {
      const sql = createMockSql()
      initEntitySchema(sql)
      initSecretSchema(sql)
      createEntity(sql, 'org-1', 'Organization', 'core', { name: 'Acme' })
      storeSecret(sql, 'acme-api', 'secret_value')

      const facts = getFacts(sql)
      expect(JSON.stringify(facts)).not.toContain('secret_value')
    })
  })
})

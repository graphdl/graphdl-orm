import { describe, it, expect, beforeEach } from 'vitest'
import type { SqlLike } from './registry-do'
import { initRegistrySchema, registerDomain, indexNoun, resolveNounInRegistry } from './registry-do'

/**
 * In-memory mock of SqlLike that stores rows per table and supports
 * CREATE TABLE, INSERT OR REPLACE, SELECT with JOIN, and basic WHERE.
 */
function createMockSql(): SqlLike & { tables: Record<string, any[]> } {
  const tables: Record<string, any[]> = {}

  return {
    tables,
    exec(query: string, ...params: any[]) {
      const trimmed = query.trim()

      // CREATE TABLE IF NOT EXISTS <name>
      const createMatch = trimmed.match(/CREATE\s+TABLE\s+IF\s+NOT\s+EXISTS\s+(\w+)/i)
      if (createMatch) {
        const tableName = createMatch[1]
        if (!tables[tableName]) {
          tables[tableName] = []
        }
        return { toArray: () => [] }
      }

      // INSERT OR REPLACE INTO <table> (col1, col2, ...) VALUES (?, ?, ...)
      const upsertMatch = trimmed.match(/INSERT\s+OR\s+REPLACE\s+INTO\s+(\w+)\s*\(([^)]+)\)\s*VALUES\s*\(([^)]+)\)/i)
      if (upsertMatch) {
        const tableName = upsertMatch[1]
        const columns = upsertMatch[2].split(',').map(c => c.trim())
        if (!tables[tableName]) {
          tables[tableName] = []
        }
        const row: Record<string, any> = {}
        for (let i = 0; i < columns.length; i++) {
          row[columns[i]] = params[i] !== undefined ? params[i] : null
        }
        // Find existing row by primary key columns (first column for domains, composite for noun_index)
        // For simplicity, remove any row where all PK columns match, then add the new one
        if (tableName === 'domains') {
          tables[tableName] = tables[tableName].filter(r => r.domain_slug !== row.domain_slug)
        } else if (tableName === 'noun_index') {
          tables[tableName] = tables[tableName].filter(r => !(r.noun_name === row.noun_name && r.domain_slug === row.domain_slug))
        } else if (tableName === 'entity_index') {
          tables[tableName] = tables[tableName].filter(r => !(r.noun_type === row.noun_type && r.entity_id === row.entity_id))
        }
        tables[tableName].push(row)
        return { toArray: () => [] }
      }

      // SELECT ... FROM noun_index JOIN domains ... WHERE noun_name = ?
      // (for resolveNounInRegistry JOIN query)
      const joinMatch = trimmed.match(/SELECT[\s\S]+FROM\s+noun_index[\s\S]+JOIN\s+domains[\s\S]+WHERE[\s\S]+noun_name\s*=\s*\?/i)
      if (joinMatch) {
        const nounName = params[0]
        const nounRows = (tables['noun_index'] || []).filter(r => r.noun_name === nounName)
        const results: any[] = []
        for (const nr of nounRows) {
          const domainRow = (tables['domains'] || []).find(r => r.domain_slug === nr.domain_slug)
          if (domainRow) {
            results.push({
              domain_slug: nr.domain_slug,
              domain_do_id: domainRow.domain_do_id,
            })
          }
        }
        return { toArray: () => results }
      }

      // SELECT * FROM <table> WHERE col = ?
      const selectWhereMatch = trimmed.match(/SELECT\s+\*\s+FROM\s+(\w+)\s+WHERE\s+(\w+)\s*=\s*\?/i)
      if (selectWhereMatch) {
        const tableName = selectWhereMatch[1]
        const col = selectWhereMatch[2]
        const val = params[0]
        const rows = (tables[tableName] || []).filter(r => r[col] === val)
        return { toArray: () => rows }
      }

      // SELECT * FROM <table>
      const selectMatch = trimmed.match(/SELECT\s+\*\s+FROM\s+(\w+)/i)
      if (selectMatch) {
        const tableName = selectMatch[1]
        return { toArray: () => tables[tableName] ? [...tables[tableName]] : [] }
      }

      return { toArray: () => [] }
    },
  }
}

describe('registry-do', () => {
  let sql: ReturnType<typeof createMockSql>

  beforeEach(() => {
    sql = createMockSql()
  })

  describe('initRegistrySchema', () => {
    it('creates all 3 tables', () => {
      initRegistrySchema(sql)

      expect(sql.tables).toHaveProperty('domains')
      expect(sql.tables).toHaveProperty('noun_index')
      expect(sql.tables).toHaveProperty('entity_index')
    })
  })

  describe('registerDomain', () => {
    it('adds a domain', () => {
      initRegistrySchema(sql)

      registerDomain(sql, 'acme-crm', 'do-id-123')

      expect(sql.tables['domains']).toHaveLength(1)
      expect(sql.tables['domains'][0].domain_slug).toBe('acme-crm')
      expect(sql.tables['domains'][0].domain_do_id).toBe('do-id-123')
      expect(sql.tables['domains'][0].visibility).toBe('private')
    })

    it('is idempotent — same slug with different doId updates it', () => {
      initRegistrySchema(sql)

      registerDomain(sql, 'acme-crm', 'do-id-123')
      registerDomain(sql, 'acme-crm', 'do-id-456')

      expect(sql.tables['domains']).toHaveLength(1)
      expect(sql.tables['domains'][0].domain_do_id).toBe('do-id-456')
    })
  })

  describe('indexNoun', () => {
    it('adds a noun-to-domain mapping', () => {
      initRegistrySchema(sql)

      indexNoun(sql, 'Person', 'acme-crm')

      expect(sql.tables['noun_index']).toHaveLength(1)
      expect(sql.tables['noun_index'][0].noun_name).toBe('Person')
      expect(sql.tables['noun_index'][0].domain_slug).toBe('acme-crm')
    })

    it('is idempotent', () => {
      initRegistrySchema(sql)

      indexNoun(sql, 'Person', 'acme-crm')
      indexNoun(sql, 'Person', 'acme-crm')

      expect(sql.tables['noun_index']).toHaveLength(1)
    })
  })

  describe('resolveNounInRegistry', () => {
    it('finds noun and returns domain info', () => {
      initRegistrySchema(sql)
      registerDomain(sql, 'acme-crm', 'do-id-123')
      indexNoun(sql, 'Person', 'acme-crm')

      const result = resolveNounInRegistry(sql, 'Person')

      expect(result).not.toBeNull()
      expect(result!.domainSlug).toBe('acme-crm')
      expect(result!.domainDoId).toBe('do-id-123')
    })

    it('returns null for unknown noun', () => {
      initRegistrySchema(sql)

      const result = resolveNounInRegistry(sql, 'UnknownNoun')

      expect(result).toBeNull()
    })
  })
})

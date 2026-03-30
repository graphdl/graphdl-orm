import { describe, it, expect, beforeEach } from 'vitest'
import type { SqlLike } from './registry-do'
import { initRegistrySchema, registerDomain, indexNoun, resolveNounInRegistry, indexEntity, deindexEntity, getEntityIds, listDomains, resolveSlugByUUID } from './registry-do'

/**
 * In-memory mock of SqlLike for the registry's schema (no deleted column).
 */
function createMockSql(): SqlLike & { tables: Record<string, any[]> } {
  const tables: Record<string, any[]> = {}

  return {
    tables,
    exec(query: string, ...params: any[]) {
      const trimmed = query.trim()

      // ALTER TABLE — ignore in mock
      if (trimmed.match(/^ALTER\s+TABLE/i)) {
        return { toArray: () => [] }
      }

      // CREATE TABLE IF NOT EXISTS <name>
      const createMatch = trimmed.match(/CREATE\s+TABLE\s+IF\s+NOT\s+EXISTS\s+(\w+)/i)
      if (createMatch) {
        if (!tables[createMatch[1]]) tables[createMatch[1]] = []
        return { toArray: () => [] }
      }

      // INSERT OR REPLACE INTO <table> (cols) VALUES (?, ...)
      const upsertMatch = trimmed.match(/INSERT\s+OR\s+REPLACE\s+INTO\s+(\w+)\s*\(([^)]+)\)\s*VALUES\s*\(([^)]+)\)/i)
      if (upsertMatch) {
        const tableName = upsertMatch[1]
        const columns = upsertMatch[2].split(',').map(c => c.trim())
        if (!tables[tableName]) tables[tableName] = []
        const row: Record<string, any> = {}
        for (let i = 0; i < columns.length; i++) {
          row[columns[i]] = params[i] !== undefined ? params[i] : null
        }
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

      // DELETE FROM entity_index WHERE noun_type=? AND entity_id=?
      const deleteEntityMatch = trimmed.match(/DELETE\s+FROM\s+entity_index\s+WHERE\s+noun_type\s*=\s*\?\s+AND\s+entity_id\s*=\s*\?/i)
      if (deleteEntityMatch) {
        tables['entity_index'] = (tables['entity_index'] || []).filter(
          (r: any) => !(r.noun_type === params[0] && r.entity_id === params[1])
        )
        return { toArray: () => [] }
      }

      // DELETE FROM entity_index WHERE domain_slug=?
      const deleteByDomainMatch = trimmed.match(/DELETE\s+FROM\s+entity_index\s+WHERE\s+domain_slug\s*=\s*\?/i)
      if (deleteByDomainMatch) {
        tables['entity_index'] = (tables['entity_index'] || []).filter(
          (r: any) => r.domain_slug !== params[0]
        )
        return { toArray: () => [] }
      }

      // DELETE FROM noun_index WHERE domain_slug=?
      const deleteNounByDomainMatch = trimmed.match(/DELETE\s+FROM\s+noun_index\s+WHERE\s+domain_slug\s*=\s*\?/i)
      if (deleteNounByDomainMatch) {
        tables['noun_index'] = (tables['noun_index'] || []).filter(
          (r: any) => r.domain_slug !== params[0]
        )
        return { toArray: () => [] }
      }

      // DELETE FROM <table> (wipe all)
      const deleteAllMatch = trimmed.match(/^DELETE\s+FROM\s+(\w+)$/i)
      if (deleteAllMatch) {
        tables[deleteAllMatch[1]] = []
        return { toArray: () => [] }
      }

      // SELECT ... FROM noun_index JOIN domains ... WHERE noun_name = ?
      const joinMatch = trimmed.match(/SELECT[\s\S]+FROM\s+noun_index[\s\S]+JOIN\s+domains[\s\S]+WHERE[\s\S]+noun_name\s*=\s*\?/i)
      if (joinMatch) {
        const nounRows = (tables['noun_index'] || []).filter(r => r.noun_name === params[0])
        const results: any[] = []
        for (const nr of nounRows) {
          const domainRow = (tables['domains'] || []).find(r => r.domain_slug === nr.domain_slug)
          if (domainRow) {
            results.push({ domain_slug: nr.domain_slug, domain_do_id: domainRow.domain_do_id })
          }
        }
        return { toArray: () => results }
      }

      // SELECT entity_id FROM entity_index WHERE noun_type=? AND domain_slug=?
      const entityIdDomainSelect = trimmed.match(/SELECT\s+entity_id\s+FROM\s+entity_index\s+WHERE\s+noun_type\s*=\s*\?\s+AND\s+domain_slug\s*=\s*\?/i)
      if (entityIdDomainSelect) {
        const rows = (tables['entity_index'] || [])
          .filter((r: any) => r.noun_type === params[0] && r.domain_slug === params[1])
          .map((r: any) => ({ entity_id: r.entity_id }))
        return { toArray: () => rows }
      }

      // SELECT entity_id FROM entity_index WHERE noun_type=?
      const entityIdSelect = trimmed.match(/SELECT\s+entity_id\s+FROM\s+entity_index\s+WHERE\s+noun_type\s*=\s*\?/i)
      if (entityIdSelect) {
        const rows = (tables['entity_index'] || [])
          .filter((r: any) => r.noun_type === params[0])
          .map((r: any) => ({ entity_id: r.entity_id }))
        return { toArray: () => rows }
      }

      // SELECT count(*) as c FROM entity_index WHERE domain_slug=?
      const countByDomainMatch = trimmed.match(/SELECT\s+count\(\*\)\s+as\s+c\s+FROM\s+entity_index\s+WHERE\s+domain_slug\s*=\s*\?/i)
      if (countByDomainMatch) {
        const c = (tables['entity_index'] || []).filter((r: any) => r.domain_slug === params[0]).length
        return { toArray: () => [{ c }] }
      }

      // SELECT count(*) as c FROM noun_index WHERE domain_slug=?
      const countNounByDomainMatch = trimmed.match(/SELECT\s+count\(\*\)\s+as\s+c\s+FROM\s+noun_index\s+WHERE\s+domain_slug\s*=\s*\?/i)
      if (countNounByDomainMatch) {
        const c = (tables['noun_index'] || []).filter((r: any) => r.domain_slug === params[0]).length
        return { toArray: () => [{ c }] }
      }

      // SELECT domain_slug FROM domains WHERE domain_uuid = ?
      const uuidMatch = trimmed.match(/SELECT\s+domain_slug\s+FROM\s+domains\s+WHERE\s+domain_uuid\s*=\s*\?/i)
      if (uuidMatch) {
        const rows = (tables['domains'] || []).filter(r => r.domain_uuid === params[0]).map(r => ({ domain_slug: r.domain_slug }))
        return { toArray: () => rows }
      }

      // SELECT domain_slug FROM domains
      const listDomainsMatch = trimmed.match(/SELECT\s+domain_slug\s+FROM\s+domains$/i)
      if (listDomainsMatch) {
        return { toArray: () => (tables['domains'] || []).map(r => ({ domain_slug: r.domain_slug })) }
      }

      // SELECT DISTINCT noun_name FROM noun_index
      const distinctNounsMatch = trimmed.match(/SELECT\s+DISTINCT\s+noun_name\s+FROM\s+noun_index/i)
      if (distinctNounsMatch) {
        const names = [...new Set((tables['noun_index'] || []).map((r: any) => r.noun_name))].sort()
        return { toArray: () => names.map(n => ({ noun_name: n })) }
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
      expect(resolveNounInRegistry(sql, 'UnknownNoun')).toBeNull()
    })
  })

  describe('indexEntity', () => {
    it('adds an entity to the population index', () => {
      initRegistrySchema(sql)
      indexEntity(sql, 'Person', 'person-1')
      expect(sql.tables['entity_index']).toHaveLength(1)
      expect(sql.tables['entity_index'][0].noun_type).toBe('Person')
      expect(sql.tables['entity_index'][0].entity_id).toBe('person-1')
    })

    it('is idempotent', () => {
      initRegistrySchema(sql)
      indexEntity(sql, 'Person', 'person-1')
      indexEntity(sql, 'Person', 'person-1')
      expect(sql.tables['entity_index']).toHaveLength(1)
    })
  })

  describe('deindexEntity', () => {
    it('removes entity from population (hard delete)', () => {
      initRegistrySchema(sql)
      indexEntity(sql, 'Person', 'person-1')
      deindexEntity(sql, 'Person', 'person-1')
      expect(sql.tables['entity_index']).toHaveLength(0)
    })
  })

  describe('getEntityIds', () => {
    it('returns all entity IDs for a type', () => {
      initRegistrySchema(sql)
      indexEntity(sql, 'Person', 'person-1')
      indexEntity(sql, 'Person', 'person-2')
      indexEntity(sql, 'Person', 'person-3')
      expect(getEntityIds(sql, 'Person')).toEqual(['person-1', 'person-2', 'person-3'])
    })

    it('excludes removed entities (hard deleted from index)', () => {
      initRegistrySchema(sql)
      indexEntity(sql, 'Person', 'person-1')
      indexEntity(sql, 'Person', 'person-2')
      deindexEntity(sql, 'Person', 'person-2')
      expect(getEntityIds(sql, 'Person')).toEqual(['person-1'])
    })
  })

  describe('domain-scoped entity_index', () => {
    it('indexes entities with domain_slug', () => {
      initRegistrySchema(sql)
      indexEntity(sql, 'Noun', 'entity-1', 'tickets')
      indexEntity(sql, 'Noun', 'entity-2', 'billing')
      expect(getEntityIds(sql, 'Noun', 'tickets')).toEqual(['entity-1'])
    })

    it('returns all entities when no domain filter', () => {
      initRegistrySchema(sql)
      indexEntity(sql, 'Noun', 'entity-1', 'tickets')
      indexEntity(sql, 'Noun', 'entity-2', 'billing')
      expect(getEntityIds(sql, 'Noun')).toEqual(['entity-1', 'entity-2'])
    })
  })

  describe('resolveSlugByUUID', () => {
    it('finds domain by UUID', () => {
      initRegistrySchema(sql)
      registerDomain(sql, 'acme-crm', 'do-id-123', 'private', 'uuid-abc')
      expect(resolveSlugByUUID(sql, 'uuid-abc')).toBe('acme-crm')
    })

    it('returns null for unknown UUID', () => {
      initRegistrySchema(sql)
      expect(resolveSlugByUUID(sql, 'nope')).toBeNull()
    })
  })
})

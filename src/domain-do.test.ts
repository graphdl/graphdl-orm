import { describe, it, expect, beforeEach } from 'vitest'
import type { SqlLike } from './domain-do'
import {
  METAMODEL_TABLES,
  initDomainSchema,
} from './domain-do'

/**
 * In-memory mock of SqlLike for testing initDomainSchema.
 */
function createMockSql(): SqlLike & { tables: Record<string, any[]>; tableColumns: Record<string, string[]> } {
  const tables: Record<string, any[]> = {}
  const tableColumns: Record<string, string[]> = {}

  function parseColumns(ddl: string): string[] {
    const bodyMatch = ddl.match(/\(([^]*)\)$/s)
    if (!bodyMatch) return ['id']
    const body = bodyMatch[1]
    const cols: string[] = []
    for (const line of body.split(',')) {
      const trimmed = line.trim()
      if (/^(UNIQUE|CHECK|FOREIGN|PRIMARY|CONSTRAINT)\s*\(/i.test(trimmed)) continue
      const colMatch = trimmed.match(/^(\w+)\s+/i)
      if (colMatch) cols.push(colMatch[1])
    }
    return cols
  }

  return {
    tables,
    tableColumns,
    exec(query: string, ...params: any[]) {
      const trimmed = query.trim()

      const createMatch = trimmed.match(/CREATE\s+TABLE\s+IF\s+NOT\s+EXISTS\s+(\w+)/i)
      if (createMatch) {
        const tableName = createMatch[1]
        if (!tables[tableName]) {
          tables[tableName] = []
          tableColumns[tableName] = parseColumns(trimmed)
        }
        return { toArray: () => [] }
      }

      if (/^CREATE\s+(UNIQUE\s+)?INDEX/i.test(trimmed)) {
        return { toArray: () => [] }
      }

      return { toArray: () => [] }
    },
  }
}

describe('domain-do', () => {
  let sql: ReturnType<typeof createMockSql>

  beforeEach(() => {
    sql = createMockSql()
  })

  describe('METAMODEL_TABLES', () => {
    it('contains all expected metamodel table names', () => {
      expect(METAMODEL_TABLES).toContain('nouns')
      expect(METAMODEL_TABLES).toContain('graph_schemas')
      expect(METAMODEL_TABLES).toContain('readings')
      expect(METAMODEL_TABLES).toContain('roles')
      expect(METAMODEL_TABLES).toContain('constraints')
      expect(METAMODEL_TABLES).toContain('constraint_spans')
      expect(METAMODEL_TABLES).toContain('state_machine_definitions')
      expect(METAMODEL_TABLES).toContain('statuses')
      expect(METAMODEL_TABLES).toContain('transitions')
      expect(METAMODEL_TABLES).toContain('guards')
      expect(METAMODEL_TABLES).toContain('event_types')
      expect(METAMODEL_TABLES).toContain('verbs')
      expect(METAMODEL_TABLES).toContain('functions')
      expect(METAMODEL_TABLES).toContain('streams')
      expect(METAMODEL_TABLES).toContain('generators')
    })

    it('does NOT contain instance tables', () => {
      expect(METAMODEL_TABLES).not.toContain('resources')
      expect(METAMODEL_TABLES).not.toContain('graphs')
      expect(METAMODEL_TABLES).not.toContain('resource_roles')
      expect(METAMODEL_TABLES).not.toContain('state_machines')
      expect(METAMODEL_TABLES).not.toContain('events')
      expect(METAMODEL_TABLES).not.toContain('guard_runs')
      expect(METAMODEL_TABLES).not.toContain('agents')
      expect(METAMODEL_TABLES).not.toContain('completions')
    })
  })

  describe('initDomainSchema', () => {
    it('creates all metamodel tables', () => {
      initDomainSchema(sql)

      for (const table of METAMODEL_TABLES) {
        expect(sql.tables).toHaveProperty(table)
      }
    })

    it('also creates supporting tables (domains, organizations, apps)', () => {
      initDomainSchema(sql)

      expect(sql.tables).toHaveProperty('domains')
      expect(sql.tables).toHaveProperty('organizations')
      expect(sql.tables).toHaveProperty('apps')
    })

    it('does NOT create instance tables', () => {
      initDomainSchema(sql)

      expect(sql.tables).not.toHaveProperty('resources')
      expect(sql.tables).not.toHaveProperty('resource_roles')
      expect(sql.tables).not.toHaveProperty('state_machines')
      expect(sql.tables).not.toHaveProperty('events')
      expect(sql.tables).not.toHaveProperty('guard_runs')
    })
  })
})

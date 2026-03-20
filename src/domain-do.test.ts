import { describe, it, expect, vi, beforeEach } from 'vitest'
import type { SqlLike } from './domain-do'
import {
  METAMODEL_TABLES,
  initDomainSchema,
  findInMetamodel,
  createInMetamodel,
  updateInMetamodel,
} from './domain-do'

/**
 * In-memory mock of SqlLike that stores rows per table and supports
 * basic SQL operations: CREATE TABLE, CREATE INDEX, INSERT, SELECT with
 * WHERE equality and IN, COUNT, UPDATE, and PRAGMA table_info.
 */
function createMockSql(): SqlLike & { tables: Record<string, any[]>; tableColumns: Record<string, string[]> } {
  const tables: Record<string, any[]> = {}
  const tableColumns: Record<string, string[]> = {}

  function parseColumns(ddl: string): string[] {
    // Extract column definitions from CREATE TABLE DDL
    const bodyMatch = ddl.match(/\(([^]*)\)$/s)
    if (!bodyMatch) return ['id']
    const body = bodyMatch[1]
    const cols: string[] = []
    for (const line of body.split(',')) {
      const trimmed = line.trim()
      // Skip constraints like UNIQUE(...), CHECK(...)
      if (/^(UNIQUE|CHECK|FOREIGN|PRIMARY|CONSTRAINT)\s*\(/i.test(trimmed)) continue
      const colMatch = trimmed.match(/^(\w+)\s+/i)
      if (colMatch) cols.push(colMatch[1])
    }
    return cols
  }

  function matchesWhere(row: Record<string, any>, whereStr: string, params: any[], paramOffset: number): { matches: boolean; paramsConsumed: number } {
    // Very basic WHERE parser — supports "col = ?" and "col IN (?, ?, ...)" with AND
    let consumed = 0
    const conditions = whereStr.split(/\s+AND\s+/i)
    for (const cond of conditions) {
      const trimmed = cond.trim()
      const eqMatch = trimmed.match(/^(\w+)\s*=\s*\?$/i)
      if (eqMatch) {
        const col = eqMatch[1]
        const val = params[paramOffset + consumed]
        consumed++
        if (row[col] !== val) return { matches: false, paramsConsumed: consumed }
        continue
      }
      const neqMatch = trimmed.match(/^(\w+)\s*!=\s*\?$/i)
      if (neqMatch) {
        const col = neqMatch[1]
        const val = params[paramOffset + consumed]
        consumed++
        if (row[col] === val) return { matches: false, paramsConsumed: consumed }
        continue
      }
      const inMatch = trimmed.match(/^(\w+)\s+IN\s+\(([^)]+)\)/i)
      if (inMatch) {
        const col = inMatch[1]
        const placeholderCount = (inMatch[2].match(/\?/g) || []).length
        const vals = params.slice(paramOffset + consumed, paramOffset + consumed + placeholderCount)
        consumed += placeholderCount
        if (!vals.includes(row[col])) return { matches: false, paramsConsumed: consumed }
        continue
      }
      const likeMatch = trimmed.match(/^(\w+)\s+LIKE\s+\?$/i)
      if (likeMatch) {
        const col = likeMatch[1]
        const pattern = params[paramOffset + consumed] as string
        consumed++
        const regex = new RegExp('^' + pattern.replace(/%/g, '.*').replace(/_/g, '.') + '$', 'i')
        if (!regex.test(String(row[col] ?? ''))) return { matches: false, paramsConsumed: consumed }
        continue
      }
      const isNotNullMatch = trimmed.match(/^(\w+)\s+IS\s+NOT\s+NULL$/i)
      if (isNotNullMatch) {
        const col = isNotNullMatch[1]
        if (row[col] == null) return { matches: false, paramsConsumed: consumed }
        continue
      }
      const isNullMatch = trimmed.match(/^(\w+)\s+IS\s+NULL$/i)
      if (isNullMatch) {
        const col = isNullMatch[1]
        if (row[col] != null) return { matches: false, paramsConsumed: consumed }
        continue
      }
      // Sub-select: col IN (SELECT id FROM table WHERE ...)
      const subSelectMatch = trimmed.match(/^(\w+)\s+IN\s+\(SELECT\s+id\s+FROM\s+(\w+)\s+WHERE\s+(.+)\)$/i)
      if (subSelectMatch) {
        const col = subSelectMatch[1]
        const targetTable = subSelectMatch[2]
        const subWhere = subSelectMatch[3]
        const targetRows = tables[targetTable] || []
        const subResult = matchesWhereSet(targetRows, subWhere, params, paramOffset + consumed)
        consumed += subResult.paramsConsumed
        const matchingIds = subResult.matchingRows.map((r: any) => r.id)
        if (!matchingIds.includes(row[col])) return { matches: false, paramsConsumed: consumed }
        continue
      }
    }
    return { matches: true, paramsConsumed: consumed }
  }

  function matchesWhereSet(rows: any[], whereStr: string, params: any[], paramOffset: number): { matchingRows: any[]; paramsConsumed: number } {
    const matching: any[] = []
    let totalConsumed = 0
    // For sub-selects, all rows share the same param set
    for (const row of rows) {
      const result = matchesWhere(row, whereStr, params, paramOffset)
      totalConsumed = result.paramsConsumed // same params for all rows
      if (result.matches) matching.push(row)
    }
    return { matchingRows: matching, paramsConsumed: totalConsumed }
  }

  return {
    tables,
    tableColumns,
    exec(query: string, ...params: any[]) {
      const trimmed = query.trim()

      // CREATE TABLE IF NOT EXISTS <name>
      const createMatch = trimmed.match(/CREATE\s+TABLE\s+IF\s+NOT\s+EXISTS\s+(\w+)/i)
      if (createMatch) {
        const tableName = createMatch[1]
        if (!tables[tableName]) {
          tables[tableName] = []
          tableColumns[tableName] = parseColumns(trimmed)
        }
        return { toArray: () => [] }
      }

      // CREATE INDEX — no-op
      if (/^CREATE\s+(UNIQUE\s+)?INDEX/i.test(trimmed)) {
        return { toArray: () => [] }
      }

      // PRAGMA table_info(<table>)
      const pragmaMatch = trimmed.match(/PRAGMA\s+table_info\((\w+)\)/i)
      if (pragmaMatch) {
        const tableName = pragmaMatch[1]
        const cols = tableColumns[tableName] || []
        return { toArray: () => cols.map(name => ({ name, type: 'TEXT', notnull: 0, dflt_value: null, pk: name === 'id' ? 1 : 0 })) }
      }

      // INSERT INTO <table> (col1, col2, ...) VALUES (?, ?, ...)
      const insertMatch = trimmed.match(/INSERT\s+INTO\s+(\w+)\s*\(([^)]+)\)\s*VALUES\s*\(([^)]+)\)/i)
      if (insertMatch) {
        const tableName = insertMatch[1]
        const columns = insertMatch[2].split(',').map(c => c.trim())
        if (!tables[tableName]) {
          tables[tableName] = []
        }
        const row: Record<string, any> = {}
        for (let i = 0; i < columns.length; i++) {
          row[columns[i]] = params[i] !== undefined ? params[i] : null
        }
        tables[tableName].push(row)
        return { toArray: () => [] }
      }

      // UPDATE <table> SET col1=?, col2=? WHERE id = ?
      const updateMatch = trimmed.match(/UPDATE\s+(\w+)\s+SET\s+(.+?)\s+WHERE\s+id\s*=\s*\?/i)
      if (updateMatch) {
        const tableName = updateMatch[1]
        const setClauses = updateMatch[2].split(',').map(c => c.trim().replace(/\s*=\s*\?/, ''))
        const idValue = params[setClauses.length] // id param is after SET params
        if (tables[tableName]) {
          const row = tables[tableName].find((r: any) => r.id === idValue)
          if (row) {
            for (let i = 0; i < setClauses.length; i++) {
              row[setClauses[i]] = params[i]
            }
          }
        }
        return { toArray: () => [] }
      }

      // SELECT COUNT(*) as cnt FROM <table> WHERE ...
      const countWhereMatch = trimmed.match(/SELECT\s+COUNT\(\*\)\s+as\s+cnt\s+FROM\s+(\w+)\s+WHERE\s+(.+)/i)
      if (countWhereMatch) {
        const tableName = countWhereMatch[1]
        const whereStr = countWhereMatch[2]
        const rows = tables[tableName] || []
        const matching = rows.filter(row => {
          const result = matchesWhere(row, whereStr, params, 0)
          return result.matches
        })
        return { toArray: () => [{ cnt: matching.length }] }
      }

      // SELECT COUNT(*) as cnt FROM <table>
      const countMatch = trimmed.match(/SELECT\s+COUNT\(\*\)\s+as\s+cnt\s+FROM\s+(\w+)/i)
      if (countMatch) {
        const tableName = countMatch[1]
        const rows = tables[tableName] || []
        return { toArray: () => [{ cnt: rows.length }] }
      }

      // SELECT * FROM <table> WHERE ... ORDER BY ... LIMIT ? OFFSET ?
      const selectFullMatch = trimmed.match(/SELECT\s+\*\s+FROM\s+(\w+)\s+WHERE\s+(.+?)\s+ORDER\s+BY\s+(\w+)\s+(ASC|DESC)\s+LIMIT\s+\?\s+OFFSET\s+\?/i)
      if (selectFullMatch) {
        const tableName = selectFullMatch[1]
        const whereStr = selectFullMatch[2]
        const orderCol = selectFullMatch[3]
        const dir = selectFullMatch[4].toUpperCase()
        const rows = tables[tableName] || []
        const matching = rows.filter(row => matchesWhere(row, whereStr, params, 0).matches)
        matching.sort((a: any, b: any) => {
          if (dir === 'DESC') return a[orderCol] > b[orderCol] ? -1 : a[orderCol] < b[orderCol] ? 1 : 0
          return a[orderCol] > b[orderCol] ? 1 : a[orderCol] < b[orderCol] ? -1 : 0
        })
        // Last two params are limit and offset
        const limit = params[params.length - 2]
        const offset = params[params.length - 1]
        return { toArray: () => matching.slice(offset, offset + limit) }
      }

      // SELECT * FROM <table> ORDER BY ... LIMIT ? OFFSET ?
      const selectOrderLimitMatch = trimmed.match(/SELECT\s+\*\s+FROM\s+(\w+)\s+ORDER\s+BY\s+(\w+)\s+(ASC|DESC)\s+LIMIT\s+\?\s+OFFSET\s+\?/i)
      if (selectOrderLimitMatch) {
        const tableName = selectOrderLimitMatch[1]
        const orderCol = selectOrderLimitMatch[2]
        const dir = selectOrderLimitMatch[3].toUpperCase()
        const rows = tables[tableName] ? [...tables[tableName]] : []
        rows.sort((a: any, b: any) => {
          if (dir === 'DESC') return a[orderCol] > b[orderCol] ? -1 : a[orderCol] < b[orderCol] ? 1 : 0
          return a[orderCol] > b[orderCol] ? 1 : a[orderCol] < b[orderCol] ? -1 : 0
        })
        const limit = params[params.length - 2]
        const offset = params[params.length - 1]
        return { toArray: () => rows.slice(offset, offset + limit) }
      }

      // SELECT * FROM <table> WHERE id = ?
      const selectByIdMatch = trimmed.match(/SELECT\s+\*\s+FROM\s+(\w+)\s+WHERE\s+id\s*=\s*\?/i)
      if (selectByIdMatch) {
        const tableName = selectByIdMatch[1]
        const rows = tables[tableName] || []
        const matching = rows.filter(r => r.id === params[0])
        return { toArray: () => matching }
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

describe('domain-do', () => {
  let sql: ReturnType<typeof createMockSql>

  beforeEach(() => {
    sql = createMockSql()
    vi.stubGlobal('crypto', { randomUUID: () => `uuid-${++uuidCounter}` })
    uuidCounter = 0
  })

  let uuidCounter = 0

  afterEach(() => {
    vi.unstubAllGlobals()
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

      // These are needed for FK references
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

  describe('createInMetamodel', () => {
    it('inserts a noun and returns it with an ID', () => {
      initDomainSchema(sql)

      const result = createInMetamodel(sql, 'nouns', {
        name: 'Person',
        objectType: 'entity',
        domain: 'domain-1',
      })

      expect(result).toBeDefined()
      expect(result.id).toBeDefined()
      expect(result.name).toBe('Person')
      expect(result.objectType).toBe('entity')
      expect(result.domain).toBe('domain-1')
    })

    it('uses provided ID when given', () => {
      initDomainSchema(sql)

      const result = createInMetamodel(sql, 'nouns', {
        id: 'custom-id',
        name: 'Color',
        objectType: 'value',
      })

      expect(result.id).toBe('custom-id')
    })

    it('creates a reading with graphSchema FK', () => {
      initDomainSchema(sql)

      // First create a graph schema
      const schema = createInMetamodel(sql, 'graph-schemas', {
        id: 'gs-1',
        name: 'PersonHasName',
        domain: 'domain-1',
      })

      // Then create a reading referencing it
      const reading = createInMetamodel(sql, 'readings', {
        text: 'Person has Name',
        graphSchema: 'gs-1',
        domain: 'domain-1',
      })

      expect(reading.text).toBe('Person has Name')
      expect(reading.graphSchema).toBe('gs-1')
      expect(reading.domain).toBe('domain-1')
    })

    it('creates a constraint with spans', () => {
      initDomainSchema(sql)

      // Create the constraint
      const constraint = createInMetamodel(sql, 'constraints', {
        id: 'c-1',
        kind: 'UC',
        modality: 'Alethic',
        domain: 'domain-1',
      })

      expect(constraint.id).toBe('c-1')
      expect(constraint.kind).toBe('UC')

      // Create a constraint span referencing the constraint
      const span = createInMetamodel(sql, 'constraint-spans', {
        constraint: 'c-1',
        role: 'role-1',
      })

      expect(span.constraint).toBe('c-1')
      expect(span.role).toBe('role-1')
    })
  })

  describe('findInMetamodel', () => {
    it('finds a noun by name', () => {
      initDomainSchema(sql)
      createInMetamodel(sql, 'nouns', { id: 'n-1', name: 'Person', objectType: 'entity', domain: 'domain-1' })
      createInMetamodel(sql, 'nouns', { id: 'n-2', name: 'Color', objectType: 'value', domain: 'domain-1' })

      const result = findInMetamodel(sql, 'nouns', { name: { equals: 'Person' } })

      expect(result.totalDocs).toBe(1)
      expect(result.docs).toHaveLength(1)
      expect(result.docs[0].name).toBe('Person')
      expect(result.docs[0].id).toBe('n-1')
    })

    it('finds nouns filtered by domain', () => {
      initDomainSchema(sql)
      createInMetamodel(sql, 'nouns', { id: 'n-1', name: 'Person', objectType: 'entity', domain: 'domain-1' })
      createInMetamodel(sql, 'nouns', { id: 'n-2', name: 'Car', objectType: 'entity', domain: 'domain-2' })

      const result = findInMetamodel(sql, 'nouns', { domain_id: { equals: 'domain-1' } })

      expect(result.totalDocs).toBe(1)
      expect(result.docs).toHaveLength(1)
      expect(result.docs[0].name).toBe('Person')
    })

    it('returns all docs when no where clause', () => {
      initDomainSchema(sql)
      createInMetamodel(sql, 'nouns', { id: 'n-1', name: 'Person', objectType: 'entity', domain: 'domain-1' })
      createInMetamodel(sql, 'nouns', { id: 'n-2', name: 'Color', objectType: 'value', domain: 'domain-1' })

      const result = findInMetamodel(sql, 'nouns', {})

      expect(result.totalDocs).toBe(2)
      expect(result.docs).toHaveLength(2)
    })

    it('supports pagination options', () => {
      initDomainSchema(sql)
      for (let i = 0; i < 5; i++) {
        createInMetamodel(sql, 'nouns', { id: `n-${i}`, name: `Noun${i}`, objectType: 'entity', domain: 'domain-1' })
      }

      const result = findInMetamodel(sql, 'nouns', {}, { limit: 2, page: 1 })

      expect(result.docs).toHaveLength(2)
      expect(result.totalDocs).toBe(5)
      expect(result.hasNextPage).toBe(true)
    })
  })

  describe('updateInMetamodel', () => {
    it('updates a noun\'s fields', () => {
      initDomainSchema(sql)
      createInMetamodel(sql, 'nouns', { id: 'n-1', name: 'Persn', objectType: 'entity', domain: 'domain-1' })

      const result = updateInMetamodel(sql, 'nouns', 'n-1', { name: 'Person' })

      expect(result).not.toBeNull()
      expect(result!.name).toBe('Person')
      // version should be incremented
      expect(result!.version).toBe(2)
    })

    it('returns null for non-existent id', () => {
      initDomainSchema(sql)

      const result = updateInMetamodel(sql, 'nouns', 'does-not-exist', { name: 'Person' })

      expect(result).toBeNull()
    })

    it('preserves fields not in the update', () => {
      initDomainSchema(sql)
      createInMetamodel(sql, 'nouns', { id: 'n-1', name: 'Person', objectType: 'entity', domain: 'domain-1', plural: 'People' })

      const result = updateInMetamodel(sql, 'nouns', 'n-1', { name: 'Human' })

      expect(result!.name).toBe('Human')
      expect(result!.objectType).toBe('entity')
      expect(result!.domain).toBe('domain-1')
      expect(result!.plural).toBe('People')
    })
  })
})

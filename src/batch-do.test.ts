import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import type { SqlLike, BatchEntity } from './batch-wal'
import { initBatchSchema, createBatch, getBatch, markCommitted, markFailed, getPendingBatches } from './batch-wal'

/**
 * In-memory mock of SqlLike that supports the SQL operations needed by
 * the batch WAL (initBatchSchema).
 *
 * Since DomainDB is a Durable Object and can't be instantiated in vitest,
 * we test the integration by calling the pure functions directly — the same
 * pattern used in batch-wal.test.ts.
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

      // UPDATE <table> SET col1=?, col2=?, ... WHERE id=?
      const updateMatch = trimmed.match(/UPDATE\s+(\w+)\s+SET\s+(.+?)\s+WHERE\s+id\s*=\s*\?/i)
      if (updateMatch) {
        const tableName = updateMatch[1]
        const setClauses = updateMatch[2].split(',').map(c => c.trim().replace(/\s*=\s*\?/, ''))
        const idValue = params[setClauses.length]
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

      // SELECT * FROM <table> WHERE <col> = ? ORDER BY <col> ASC|DESC
      const selectWhereOrderMatch = trimmed.match(/SELECT\s+\*\s+FROM\s+(\w+)\s+WHERE\s+(\w+)\s*=\s*\?\s+ORDER\s+BY\s+(\w+)\s+(ASC|DESC)/i)
      if (selectWhereOrderMatch) {
        const tableName = selectWhereOrderMatch[1]
        const whereCol = selectWhereOrderMatch[2]
        const orderCol = selectWhereOrderMatch[3]
        const dir = selectWhereOrderMatch[4].toUpperCase()
        const whereVal = params[0]
        if (!tables[tableName]) return { toArray: () => [] }
        let rows = tables[tableName].filter((r: any) => r[whereCol] === whereVal)
        rows = [...rows].sort((a: any, b: any) => {
          if (dir === 'DESC') return a[orderCol] > b[orderCol] ? -1 : a[orderCol] < b[orderCol] ? 1 : 0
          return a[orderCol] > b[orderCol] ? 1 : a[orderCol] < b[orderCol] ? -1 : 0
        })
        return { toArray: () => rows }
      }

      // SELECT * FROM <table> WHERE <col> = ?
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

describe('DomainDB.commitBatch (integration via pure functions)', () => {
  let sql: ReturnType<typeof createMockSql>

  beforeEach(() => {
    sql = createMockSql()
    vi.stubGlobal('crypto', { randomUUID: () => 'test-batch-uuid' })
  })

  afterEach(() => {
    vi.unstubAllGlobals()
  })

  it('initBatchSchema creates the batches table', () => {
    initBatchSchema(sql)

    // Batch WAL table exists
    expect(sql.tables).toHaveProperty('batches')
  })

  it('creates a batch with pending status after both schemas are initialized', () => {
    initBatchSchema(sql)

    const entities: BatchEntity[] = [
      { id: 'e1', type: 'Person', domain: 'acme', data: { name: 'Alice' } },
      { id: 'e2', type: 'Person', domain: 'acme', data: { name: 'Bob' } },
    ]

    const batch = createBatch(sql, 'acme', entities)

    expect(batch.id).toBe('test-batch-uuid')
    expect(batch.domain).toBe('acme')
    expect(batch.status).toBe('pending')
    expect(batch.entities).toEqual(entities)
    expect(batch.entityCount).toBe(2)
    expect(batch.createdAt).toBeDefined()
  })

  it('retrieves a batch by ID', () => {
    initBatchSchema(sql)

    const entities: BatchEntity[] = [
      { id: 'e1', type: 'Person', domain: 'acme', data: { name: 'Alice' } },
    ]

    createBatch(sql, 'acme', entities)
    const batch = getBatch(sql, 'test-batch-uuid')

    expect(batch).not.toBeNull()
    expect(batch!.id).toBe('test-batch-uuid')
    expect(batch!.status).toBe('pending')
    expect(batch!.entities).toEqual(entities)
  })

  it('marks a batch as committed', () => {
    initBatchSchema(sql)

    const entities: BatchEntity[] = [
      { id: 'e1', type: 'Person', domain: 'acme', data: { name: 'Alice' } },
    ]

    createBatch(sql, 'acme', entities)
    markCommitted(sql, 'test-batch-uuid')

    const row = sql.tables['batches'][0]
    expect(row.status).toBe('committed')
    expect(row.committed_at).toBeDefined()
  })

  it('marks a batch as failed with error message', () => {
    initBatchSchema(sql)

    const entities: BatchEntity[] = [
      { id: 'e1', type: 'Person', domain: 'acme', data: { name: 'Alice' } },
    ]

    createBatch(sql, 'acme', entities)
    markFailed(sql, 'test-batch-uuid', 'Constraint violation')

    const row = sql.tables['batches'][0]
    expect(row.status).toBe('failed')
    expect(row.error).toBe('Constraint violation')
  })

  it('returns pending batches only', () => {
    let uuidCounter = 0
    vi.stubGlobal('crypto', { randomUUID: () => `batch-${++uuidCounter}` })

    initBatchSchema(sql)

    const entities: BatchEntity[] = [{ id: 'e1', type: 'A', domain: 'd', data: {} }]

    createBatch(sql, 'acme', entities)
    createBatch(sql, 'acme', entities)
    createBatch(sql, 'acme', entities)

    // Set deterministic timestamps
    sql.tables['batches'][0].created_at = '2025-01-01T00:00:00.000Z'
    sql.tables['batches'][1].created_at = '2025-01-02T00:00:00.000Z'
    sql.tables['batches'][2].created_at = '2025-01-03T00:00:00.000Z'

    // Commit first, fail third
    markCommitted(sql, 'batch-1')
    markFailed(sql, 'batch-3', 'error')

    const pending = getPendingBatches(sql)
    expect(pending).toHaveLength(1)
    expect(pending[0].id).toBe('batch-2')
    expect(pending[0].status).toBe('pending')
  })

  it('uses domain slug from setDomainId pattern for batch creation', () => {
    initBatchSchema(sql)

    // Simulates the DO pattern: setDomainId stores slug, then commitBatch uses it
    const domainSlug = 'my-domain'
    const entities: BatchEntity[] = [
      { id: 'e1', type: 'Order', domain: domainSlug, data: { total: 42 } },
    ]

    const batch = createBatch(sql, domainSlug, entities)

    expect(batch.domain).toBe('my-domain')
    expect(batch.entityCount).toBe(1)
  })
})

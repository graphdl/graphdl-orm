import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import type { SqlLike } from './batch-wal'
import { initBatchSchema, createBatch, getBatch, markCommitted, markFailed, getPendingBatches } from './batch-wal'
import type { BatchEntity } from './batch-wal'

/**
 * In-memory mock of SqlLike that tracks SQL operations.
 *
 * Supports CREATE TABLE, INSERT INTO, SELECT with WHERE/ORDER BY, and UPDATE.
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
        const idValue = params[setClauses.length] // id param is after the SET params
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

      // SELECT * FROM <table> ORDER BY <col> ASC|DESC
      const selectOrderMatch = trimmed.match(/SELECT\s+\*\s+FROM\s+(\w+)\s+ORDER\s+BY\s+(\w+)\s+(ASC|DESC)/i)
      if (selectOrderMatch) {
        const tableName = selectOrderMatch[1]
        const orderCol = selectOrderMatch[2]
        const dir = selectOrderMatch[3].toUpperCase()
        if (!tables[tableName]) return { toArray: () => [] }
        const rows = [...tables[tableName]]
        rows.sort((a: any, b: any) => {
          if (dir === 'DESC') return a[orderCol] > b[orderCol] ? -1 : a[orderCol] < b[orderCol] ? 1 : 0
          return a[orderCol] > b[orderCol] ? 1 : a[orderCol] < b[orderCol] ? -1 : 0
        })
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

describe('batch-wal', () => {
  let sql: ReturnType<typeof createMockSql>

  beforeEach(() => {
    sql = createMockSql()
    vi.stubGlobal('crypto', { randomUUID: () => 'test-batch-uuid' })
  })

  afterEach(() => {
    vi.unstubAllGlobals()
  })

  describe('initBatchSchema', () => {
    it('creates the batches table', () => {
      initBatchSchema(sql)

      expect(sql.tables).toHaveProperty('batches')
    })
  })

  describe('createBatch', () => {
    it('creates a batch with pending status', () => {
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

      // Verify stored in table
      expect(sql.tables['batches']).toHaveLength(1)
      expect(sql.tables['batches'][0].status).toBe('pending')
      expect(sql.tables['batches'][0].entity_count).toBe(2)
    })
  })

  describe('getBatch', () => {
    it('retrieves a batch by ID', () => {
      initBatchSchema(sql)

      const entities: BatchEntity[] = [
        { id: 'e1', type: 'Person', domain: 'acme', data: { name: 'Alice' } },
      ]

      createBatch(sql, 'acme', entities)

      const batch = getBatch(sql, 'test-batch-uuid')

      expect(batch).not.toBeNull()
      expect(batch!.id).toBe('test-batch-uuid')
      expect(batch!.domain).toBe('acme')
      expect(batch!.status).toBe('pending')
      expect(batch!.entities).toEqual(entities)
      expect(batch!.entityCount).toBe(1)
    })

    it('returns null for unknown batch ID', () => {
      initBatchSchema(sql)

      const batch = getBatch(sql, 'nonexistent')

      expect(batch).toBeNull()
    })
  })

  describe('markCommitted', () => {
    it('marks a batch as committed with committed_at timestamp', () => {
      initBatchSchema(sql)

      const entities: BatchEntity[] = [
        { id: 'e1', type: 'Person', domain: 'acme', data: { name: 'Alice' } },
      ]

      createBatch(sql, 'acme', entities)
      markCommitted(sql, 'test-batch-uuid')

      const row = sql.tables['batches'][0]
      expect(row.status).toBe('committed')
      expect(row.committed_at).toBeDefined()
      expect(row.committed_at).not.toBeNull()
    })
  })

  describe('markFailed', () => {
    it('marks a batch as failed with error message', () => {
      initBatchSchema(sql)

      const entities: BatchEntity[] = [
        { id: 'e1', type: 'Person', domain: 'acme', data: { name: 'Alice' } },
      ]

      createBatch(sql, 'acme', entities)
      markFailed(sql, 'test-batch-uuid', 'Constraint violation on Person.name')

      const row = sql.tables['batches'][0]
      expect(row.status).toBe('failed')
      expect(row.error).toBe('Constraint violation on Person.name')
    })
  })

  describe('getPendingBatches', () => {
    it('returns pending batches ordered by created_at', () => {
      let uuidCounter = 0
      vi.stubGlobal('crypto', { randomUUID: () => `batch-${++uuidCounter}` })

      initBatchSchema(sql)

      // Insert batches with explicit created_at for deterministic ordering
      const entities1: BatchEntity[] = [{ id: 'e1', type: 'A', domain: 'd', data: {} }]
      const entities2: BatchEntity[] = [{ id: 'e2', type: 'B', domain: 'd', data: {} }]

      createBatch(sql, 'domain-a', entities1)
      createBatch(sql, 'domain-b', entities2)

      // Manually set created_at for deterministic ordering
      sql.tables['batches'][0].created_at = '2025-01-01T00:00:00.000Z'
      sql.tables['batches'][1].created_at = '2025-01-02T00:00:00.000Z'

      const pending = getPendingBatches(sql)

      expect(pending).toHaveLength(2)
      expect(pending[0].id).toBe('batch-1')
      expect(pending[1].id).toBe('batch-2')
    })

    it('does not return committed batches', () => {
      let uuidCounter = 0
      vi.stubGlobal('crypto', { randomUUID: () => `batch-${++uuidCounter}` })

      initBatchSchema(sql)

      const entities: BatchEntity[] = [{ id: 'e1', type: 'A', domain: 'd', data: {} }]

      createBatch(sql, 'domain-a', entities)
      createBatch(sql, 'domain-b', entities)

      markCommitted(sql, 'batch-1')

      const pending = getPendingBatches(sql)

      expect(pending).toHaveLength(1)
      expect(pending[0].id).toBe('batch-2')
    })

    it('does not return failed batches', () => {
      let uuidCounter = 0
      vi.stubGlobal('crypto', { randomUUID: () => `batch-${++uuidCounter}` })

      initBatchSchema(sql)

      const entities: BatchEntity[] = [{ id: 'e1', type: 'A', domain: 'd', data: {} }]

      createBatch(sql, 'domain-a', entities)
      createBatch(sql, 'domain-b', entities)

      markFailed(sql, 'batch-1', 'some error')

      const pending = getPendingBatches(sql)

      expect(pending).toHaveLength(1)
      expect(pending[0].id).toBe('batch-2')
    })
  })
})

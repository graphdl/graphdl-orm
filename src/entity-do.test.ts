import { describe, it, expect, vi, beforeEach } from 'vitest'
import type { SqlLike, EntityData } from './entity-do'
import { initEntitySchema, createEntity, getEntity } from './entity-do'

/**
 * In-memory mock of SqlLike that tracks SQL operations.
 *
 * Stores rows keyed by table name. Supports CREATE TABLE, INSERT INTO, and
 * SELECT FROM queries by parsing the SQL string.
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
          row[columns[i]] = params[i]
        }
        tables[tableName].push(row)
        return { toArray: () => [] }
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

describe('entity-do', () => {
  let sql: ReturnType<typeof createMockSql>

  beforeEach(() => {
    sql = createMockSql()
  })

  describe('initEntitySchema', () => {
    it('creates both entity and events tables', () => {
      initEntitySchema(sql)

      expect(sql.tables).toHaveProperty('entity')
      expect(sql.tables).toHaveProperty('events')
    })
  })

  describe('createEntity', () => {
    it('inserts entity row and logs CDC event', () => {
      // Mock crypto.randomUUID
      vi.stubGlobal('crypto', { randomUUID: () => 'test-event-uuid' })

      initEntitySchema(sql)

      const input: EntityData = {
        id: 'entity-1',
        type: 'User',
        data: { name: 'Alice', age: 30 },
      }

      const result = createEntity(sql, input)

      expect(result).toEqual({ id: 'entity-1', version: 1 })

      // Verify entity was inserted
      const entities = sql.tables.entity
      expect(entities).toHaveLength(1)
      expect(entities[0].id).toBe('entity-1')
      expect(entities[0].type).toBe('User')
      expect(entities[0].data).toBe(JSON.stringify({ name: 'Alice', age: 30 }))
      expect(entities[0].version).toBe(1)
      expect(entities[0].created_at).toBeDefined()
      expect(entities[0].updated_at).toBeDefined()

      // Verify CDC event was logged
      const events = sql.tables.events
      expect(events).toHaveLength(1)
      expect(events[0].id).toBe('test-event-uuid')
      expect(events[0].operation).toBe('create')
      expect(events[0].data).toBe(JSON.stringify({ name: 'Alice', age: 30 }))
      expect(events[0].prev).toBeNull()

      vi.unstubAllGlobals()
    })
  })

  describe('getEntity', () => {
    it('returns entity data with parsed JSON', () => {
      vi.stubGlobal('crypto', { randomUUID: () => 'evt-1' })

      initEntitySchema(sql)

      const input: EntityData = {
        id: 'entity-2',
        type: 'Product',
        data: { title: 'Widget', price: 9.99 },
      }

      createEntity(sql, input)
      const entity = getEntity(sql)

      expect(entity).not.toBeNull()
      expect(entity!.id).toBe('entity-2')
      expect(entity!.type).toBe('Product')
      expect(entity!.data).toEqual({ title: 'Widget', price: 9.99 })
      expect(entity!.version).toBe(1)
      expect(entity!.createdAt).toBeDefined()
      expect(entity!.updatedAt).toBeDefined()
      expect(entity!.deletedAt).toBeNull()

      vi.unstubAllGlobals()
    })

    it('returns null for empty DO', () => {
      initEntitySchema(sql)
      const entity = getEntity(sql)
      expect(entity).toBeNull()
    })
  })
})

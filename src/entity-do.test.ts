import { describe, it, expect, vi, beforeEach } from 'vitest'
import type { SqlLike, EntityData, EventRecord } from './entity-do'
import { initEntitySchema, createEntity, getEntity, updateEntity, deleteEntity, getEvents } from './entity-do'

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

      // SELECT * FROM <table> WHERE <condition> ORDER BY ...
      const selectWhereMatch = trimmed.match(/SELECT\s+\*\s+FROM\s+(\w+)\s+WHERE\s+(.+?)\s+ORDER\s+BY\s+(\w+)\s+(ASC|DESC)/i)
      if (selectWhereMatch) {
        const tableName = selectWhereMatch[1]
        const dir = selectWhereMatch[4].toUpperCase()
        const orderCol = selectWhereMatch[3]
        if (!tables[tableName]) return { toArray: () => [] }
        let rows = [...tables[tableName]]
        // Apply WHERE timestamp > ? filter
        if (selectWhereMatch[2].match(/timestamp\s*>\s*\?/i) && params.length > 0) {
          const since = params[0]
          rows = rows.filter((r: any) => r.timestamp > since)
        }
        rows.sort((a: any, b: any) => {
          if (dir === 'DESC') return a[orderCol] > b[orderCol] ? -1 : a[orderCol] < b[orderCol] ? 1 : 0
          return a[orderCol] > b[orderCol] ? 1 : a[orderCol] < b[orderCol] ? -1 : 0
        })
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

  describe('updateEntity', () => {
    it('merges fields, increments version, logs CDC with prev state', () => {
      let eventCounter = 0
      vi.stubGlobal('crypto', { randomUUID: () => `evt-${++eventCounter}` })

      initEntitySchema(sql)
      createEntity(sql, { id: 'e1', type: 'User', data: { name: 'Alice', age: 30 } })

      const result = updateEntity(sql, { name: 'Bob' })

      expect(result).not.toBeNull()
      expect(result!.id).toBe('e1')
      expect(result!.version).toBe(2)

      // Verify entity was updated in store
      const entity = getEntity(sql)
      expect(entity!.data).toEqual({ name: 'Bob', age: 30 })
      expect(entity!.version).toBe(2)

      // Verify CDC event was logged
      const events = sql.tables.events
      expect(events).toHaveLength(2) // create + update
      const updateEvent = events[1]
      expect(updateEvent.operation).toBe('update')
      expect(JSON.parse(updateEvent.data)).toEqual({ name: 'Bob', age: 30 })
      expect(JSON.parse(updateEvent.prev)).toEqual({ name: 'Alice', age: 30 })

      vi.unstubAllGlobals()
    })

    it('returns null when no entity exists', () => {
      initEntitySchema(sql)

      const result = updateEntity(sql, { name: 'Bob' })

      expect(result).toBeNull()
    })

    it('preserves existing fields not in the update (patch semantics)', () => {
      let eventCounter = 0
      vi.stubGlobal('crypto', { randomUUID: () => `evt-${++eventCounter}` })

      initEntitySchema(sql)
      createEntity(sql, { id: 'e1', type: 'User', data: { name: 'Alice', age: 30, email: 'alice@test.com' } })

      updateEntity(sql, { age: 31 })

      const entity = getEntity(sql)
      expect(entity!.data).toEqual({ name: 'Alice', age: 31, email: 'alice@test.com' })

      vi.unstubAllGlobals()
    })
  })

  describe('deleteEntity', () => {
    it('sets deleted_at and logs CDC event', () => {
      let eventCounter = 0
      vi.stubGlobal('crypto', { randomUUID: () => `evt-${++eventCounter}` })

      initEntitySchema(sql)
      createEntity(sql, { id: 'e1', type: 'User', data: { name: 'Alice' } })

      const result = deleteEntity(sql)

      expect(result).not.toBeNull()
      expect(result!.id).toBe('e1')
      expect(result!.deleted).toBe(true)

      // Verify entity has deleted_at set
      const entity = getEntity(sql)
      expect(entity!.deletedAt).toBeDefined()
      expect(entity!.deletedAt).not.toBeNull()

      // Verify CDC event
      const events = sql.tables.events
      expect(events).toHaveLength(2) // create + delete
      const deleteEvent = events[1]
      expect(deleteEvent.operation).toBe('delete')
      expect(deleteEvent.data).toBeNull()
      expect(JSON.parse(deleteEvent.prev)).toEqual({ name: 'Alice' })

      vi.unstubAllGlobals()
    })

    it('returns null when no entity exists', () => {
      initEntitySchema(sql)

      const result = deleteEntity(sql)

      expect(result).toBeNull()
    })
  })

  describe('getEvents', () => {
    it('returns events in reverse chronological order', () => {
      let eventCounter = 0
      vi.stubGlobal('crypto', { randomUUID: () => `evt-${++eventCounter}` })

      initEntitySchema(sql)

      // Manually insert events with known timestamps to ensure deterministic ordering
      sql.tables.events.push(
        { id: 'evt-a', timestamp: '2025-01-01T00:00:00.000Z', operation: 'create', data: JSON.stringify({ name: 'Alice' }), prev: null },
        { id: 'evt-b', timestamp: '2025-02-01T00:00:00.000Z', operation: 'update', data: JSON.stringify({ name: 'Bob' }), prev: JSON.stringify({ name: 'Alice' }) },
        { id: 'evt-c', timestamp: '2025-03-01T00:00:00.000Z', operation: 'update', data: JSON.stringify({ name: 'Charlie' }), prev: JSON.stringify({ name: 'Bob' }) },
      )

      const events = getEvents(sql)

      expect(events).toHaveLength(3)
      // Newest first
      expect(events[0].operation).toBe('update')
      expect(JSON.parse(events[0].data!)).toEqual({ name: 'Charlie' })
      expect(events[1].operation).toBe('update')
      expect(events[2].operation).toBe('create')

      vi.unstubAllGlobals()
    })

    it('filters events with since parameter', () => {
      let eventCounter = 0
      vi.stubGlobal('crypto', { randomUUID: () => `evt-${++eventCounter}` })

      initEntitySchema(sql)

      // Manually insert events with known timestamps for reliable filtering
      sql.tables.events.push(
        { id: 'old-1', timestamp: '2025-01-01T00:00:00.000Z', operation: 'create', data: '{}', prev: null },
        { id: 'old-2', timestamp: '2025-06-01T00:00:00.000Z', operation: 'update', data: '{}', prev: '{}' },
        { id: 'new-1', timestamp: '2026-01-01T00:00:00.000Z', operation: 'update', data: '{}', prev: '{}' },
      )

      const events = getEvents(sql, '2025-06-01T00:00:00.000Z')

      expect(events).toHaveLength(1)
      expect(events[0].id).toBe('new-1')

      vi.unstubAllGlobals()
    })
  })
})

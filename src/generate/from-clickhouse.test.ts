import { describe, it, expect } from 'vitest'
import { parseClickHouseSQL, fromClickHouse } from './from-clickhouse'
import * as fs from 'fs'
import * as path from 'path'

describe('parseClickHouseSQL', () => {
  it('parses a simple table', () => {
    const sql = `CREATE TABLE vindex.make (
      id String,
      make String,
      edmundsId String DEFAULT '',
      createdAt UInt32 DEFAULT 1
    ) ENGINE = ReplacingMergeTree(createdAt)
    ORDER BY id;`

    const tables = parseClickHouseSQL(sql)
    expect(tables).toHaveLength(1)
    expect(tables[0].name).toBe('make')
    expect(tables[0].database).toBe('vindex')
    expect(tables[0].columns.length).toBeGreaterThan(0)
    expect(tables[0].orderBy).toEqual(['id'])
  })

  it('parses multiple tables', () => {
    const sql = `
    CREATE TABLE t1 (id String, name String) ENGINE = MergeTree ORDER BY id;
    CREATE TABLE t2 (id String, value UInt32) ENGINE = MergeTree ORDER BY id;`

    const tables = parseClickHouseSQL(sql)
    expect(tables).toHaveLength(2)
    expect(tables[0].name).toBe('t1')
    expect(tables[1].name).toBe('t2')
  })

  it('parses PARTITION BY', () => {
    const sql = `CREATE TABLE specs (
      id String, year UInt16
    ) ENGINE = ReplacingMergeTree(createdAt) PARTITION BY year ORDER BY id;`

    const tables = parseClickHouseSQL(sql)
    expect(tables[0].partitionBy).toBe('year')
  })

  it('extracts FK from comments', () => {
    const sql = `CREATE TABLE option (
      id String,
      specs String, -- FK ? specs.id
      type String
    ) ENGINE = MergeTree ORDER BY id;`

    const tables = parseClickHouseSQL(sql)
    const specsCol = tables[0].columns.find(c => c.name === 'specs')
    expect(specsCol?.fk).toBe('specs.id')
  })
})

describe('fromClickHouse', () => {
  it('generates readings from parsed tables', () => {
    const sql = `CREATE TABLE vindex.make (
      id String,
      make String,
      edmundsId String DEFAULT '',
      kbbId String DEFAULT '',
      meta String,
      createdAt UInt32 DEFAULT 1
    ) ENGINE = ReplacingMergeTree(createdAt) ORDER BY id;`

    const tables = parseClickHouseSQL(sql)
    const readings = fromClickHouse(tables, 'vindex')

    expect(readings).toContain('Make(.id) is an entity type.')
    expect(readings).toContain('Make has Make.')
    expect(readings).toContain('Make has Edmunds Id.')
    expect(readings).toContain('Make has Kbb Id.')
    // Infrastructure columns should be skipped
    expect(readings).not.toContain('Created At')
    expect(readings).not.toContain('Meta')
    expect(readings).not.toContain('Label')
  })

  it('generates readings from a full ClickHouse schema file', () => {
    // Skip if no external schema file is available
    const schemaPath = process.env.CLICKHOUSE_SCHEMA_PATH
    if (!schemaPath) return
    let sql: string
    try {
      sql = fs.readFileSync(schemaPath, 'utf-8')
    } catch {
      return
    }

    const tables = parseClickHouseSQL(sql)
    expect(tables.length).toBeGreaterThan(5)

    const readings = fromClickHouse(tables, 'vindex')

    // Should have key entity types
    expect(readings).toContain('Make(.id) is an entity type.')
    expect(readings).toContain('Specs(.id) is an entity type.')
    expect(readings).toContain('Color(.id) is an entity type.')
    expect(readings).toContain('Option(.id) is an entity type.')

    // Should have fact types
    expect(readings).toContain('Specs has Year.')
    expect(readings).toContain('Specs has Trim.')

    // Domain visibility
    expect(readings).toContain("Domain 'vindex' has Visibility 'public'.")
  })

  it('handles FK references as relationship fact types', () => {
    const sql = `
    CREATE TABLE specs (id String, make String) ENGINE = MergeTree ORDER BY id;
    CREATE TABLE option (
      id String,
      specs String, -- FK ? specs.id
      type String
    ) ENGINE = MergeTree ORDER BY id;`

    const tables = parseClickHouseSQL(sql)
    const readings = fromClickHouse(tables, 'test')

    expect(readings).toContain('Option has Specs.')
    expect(readings).toContain('Each Option has at most one Specs.')
  })
})

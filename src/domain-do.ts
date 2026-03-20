/**
 * DomainDB — pure functions for metamodel schema initialization and
 * collection CRUD. Holds only type-level data (nouns, readings,
 * constraints, etc.), not instance data.
 *
 * The DO class wrapping these functions will be added in a subsequent task.
 */

import { BOOTSTRAP_DDL } from './schema/bootstrap'
import { COLLECTION_TABLE_MAP, FIELD_MAP, FK_TARGET_TABLE } from './collections'

// =========================================================================
// Types
// =========================================================================

export interface SqlLike {
  exec(query: string, ...params: any[]): { toArray(): any[] }
}

// =========================================================================
// Constants
// =========================================================================

/**
 * The metamodel table names — type-level tables that define the schema,
 * NOT instance/runtime data.
 */
export const METAMODEL_TABLES: string[] = [
  'nouns',
  'graph_schemas',
  'readings',
  'roles',
  'constraints',
  'constraint_spans',
  'state_machine_definitions',
  'statuses',
  'transitions',
  'guards',
  'event_types',
  'verbs',
  'functions',
  'streams',
  'generators',
]

/**
 * Tables that must also be created because metamodel tables have FK
 * references to them (e.g. nouns.domain_id → domains.id).
 */
const SUPPORTING_TABLES: string[] = [
  'organizations',
  'org_memberships',
  'apps',
  'domains',
  'models',
  'agent_definitions',
]

/** All tables whose DDL we need to run. */
const ALL_REQUIRED_TABLES = new Set([...METAMODEL_TABLES, ...SUPPORTING_TABLES])

/** Fields common to every table — Payload camelCase → SQL snake_case. */
const COMMON_FIELDS: Record<string, string> = {
  createdAt: 'created_at',
  updatedAt: 'updated_at',
}

// =========================================================================
// Helpers (ported from do.ts)
// =========================================================================

/** Convert a snake_case string to camelCase. */
function snakeToCamel(s: string): string {
  return s.replace(/_([a-z])/g, (_, c) => c.toUpperCase())
}

/**
 * Resolve a where-clause key to a SQL column name.
 */
function resolveColumn(key: string, fieldMap: Record<string, string>): string {
  if (key in fieldMap) return fieldMap[key]
  const camelKey = snakeToCamel(key)
  if (camelKey !== key) {
    if (camelKey in fieldMap) return fieldMap[camelKey]
    if (camelKey in COMMON_FIELDS) return COMMON_FIELDS[camelKey]
  }
  if (key in COMMON_FIELDS) return COMMON_FIELDS[key]
  return key
}

/** Build a reverse map: SQL column name → Payload field name. */
function reverseFieldMap(fieldMap: Record<string, string>): Record<string, string> {
  const reversed: Record<string, string> = {}
  for (const [payloadName, sqlCol] of Object.entries(fieldMap)) {
    reversed[sqlCol] = payloadName
  }
  return reversed
}

/**
 * Translate a Payload-style where object to a SQL WHERE clause + params.
 *
 * Supports:
 * - and / or logical combinators
 * - equals, not_equals, in, like, exists operators
 * - Direct value shorthand (field: value)
 * - Dot-notation FK traversal (e.g. domain.domainSlug)
 */
function buildWhereClause(
  where: Record<string, any>,
  fieldMap: Record<string, string>,
): { clause: string; params: any[] } {
  const conditions: string[] = []
  const params: any[] = []

  if (where.and) {
    const subs = (where.and as any[]).map(sub => buildWhereClause(sub, fieldMap))
    const clauses = subs.filter(s => s.clause).map(s => `(${s.clause})`)
    if (clauses.length) conditions.push(clauses.join(' AND '))
    for (const sub of subs) params.push(...sub.params)
  }

  if (where.or) {
    const subs = (where.or as any[]).map(sub => buildWhereClause(sub, fieldMap))
    const clauses = subs.filter(s => s.clause).map(s => `(${s.clause})`)
    if (clauses.length) conditions.push(`(${clauses.join(' OR ')})`)
    for (const sub of subs) params.push(...sub.params)
  }

  for (const [key, condition] of Object.entries(where)) {
    if (key === 'and' || key === 'or') continue

    // Dot-notation FK traversal
    if (key.includes('.')) {
      const [relationName, fieldName] = key.split('.', 2)
      const fkCol = resolveColumn(relationName, fieldMap)
      const fkColResolved = fkCol.endsWith('_id') ? fkCol : `${fkCol}_id`
      const targetTable = FK_TARGET_TABLE[fkColResolved]
      if (!targetTable) continue

      const targetFieldMap = FIELD_MAP[targetTable] || {}
      const targetCol = resolveColumn(fieldName, targetFieldMap)

      if (typeof condition === 'object' && condition !== null) {
        if ('equals' in condition) {
          conditions.push(`${fkCol} IN (SELECT id FROM ${targetTable} WHERE ${targetCol} = ?)`)
          params.push(condition.equals)
        } else if ('not_equals' in condition) {
          conditions.push(`${fkCol} NOT IN (SELECT id FROM ${targetTable} WHERE ${targetCol} = ?)`)
          params.push(condition.not_equals)
        } else if ('in' in condition && Array.isArray(condition.in)) {
          const placeholders = condition.in.map(() => '?').join(', ')
          conditions.push(`${fkCol} IN (SELECT id FROM ${targetTable} WHERE ${targetCol} IN (${placeholders}))`)
          params.push(...condition.in)
        } else if ('like' in condition) {
          conditions.push(`${fkCol} IN (SELECT id FROM ${targetTable} WHERE ${targetCol} LIKE ?)`)
          params.push(condition.like)
        } else if ('exists' in condition) {
          conditions.push(condition.exists ? `${fkCol} IS NOT NULL` : `${fkCol} IS NULL`)
        }
      } else {
        conditions.push(`${fkCol} IN (SELECT id FROM ${targetTable} WHERE ${targetCol} = ?)`)
        params.push(condition)
      }
      continue
    }

    const col = resolveColumn(key, fieldMap)

    if (typeof condition === 'object' && condition !== null) {
      if ('equals' in condition) { conditions.push(`${col} = ?`); params.push(condition.equals) }
      else if ('not_equals' in condition) { conditions.push(`${col} != ?`); params.push(condition.not_equals) }
      else if ('in' in condition && Array.isArray(condition.in)) {
        const placeholders = condition.in.map(() => '?').join(', ')
        conditions.push(`${col} IN (${placeholders})`); params.push(...condition.in)
      }
      else if ('like' in condition) { conditions.push(`${col} LIKE ?`); params.push(condition.like) }
      else if ('exists' in condition) { conditions.push(condition.exists ? `${col} IS NOT NULL` : `${col} IS NULL`) }
      if ('value' in condition && Object.keys(condition).length === 1) {
        conditions.push(`${col} = ?`); params.push(condition.value)
      }
    } else {
      conditions.push(`${col} = ?`); params.push(condition)
    }
  }

  return { clause: conditions.join(' AND '), params }
}

/** Map a SQL row to a Payload-style object using the reverse field map. */
function rowToPayload(row: Record<string, unknown>, reverseMap: Record<string, string>): Record<string, unknown> {
  const result: Record<string, unknown> = {}
  for (const [col, value] of Object.entries(row)) {
    const payloadName = reverseMap[col] || col
    result[payloadName] = value
  }
  return result
}

/** Map a Payload-style data object to SQL column names. */
function payloadToRow(data: Record<string, unknown>, fieldMap: Record<string, string>): Record<string, unknown> {
  const result: Record<string, unknown> = {}
  for (const [key, value] of Object.entries(data)) {
    if (key === 'createdAt' || key === 'updatedAt' || key === 'version') continue
    const col = resolveColumn(key, fieldMap)
    result[col] = value
  }
  return result
}

/** Get table column names by querying PRAGMA. */
function getTableColumns(sql: SqlLike, table: string): string[] {
  const rows = sql.exec(`PRAGMA table_info(${table})`).toArray()
  return rows.map(r => (r as any).name as string)
}

// =========================================================================
// Resolve table from collection slug
// =========================================================================

function resolveTable(collectionSlug: string): string {
  const table = COLLECTION_TABLE_MAP[collectionSlug]
  if (!table) {
    throw new Error(`Unknown collection: ${collectionSlug}`)
  }
  return table
}

function getFieldMap(table: string): Record<string, string> {
  return FIELD_MAP[table] || {}
}

// =========================================================================
// Public API
// =========================================================================

/**
 * Runs only the BOOTSTRAP_DDL statements for metamodel tables and their
 * supporting FK targets (organizations, apps, domains, etc.).
 */
export function initDomainSchema(sql: SqlLike): void {
  for (const ddl of BOOTSTRAP_DDL) {
    // Extract table name from CREATE TABLE or CREATE INDEX
    const createTableMatch = ddl.match(/CREATE\s+TABLE\s+IF\s+NOT\s+EXISTS\s+(\w+)/i)
    if (createTableMatch) {
      const tableName = createTableMatch[1]
      if (ALL_REQUIRED_TABLES.has(tableName)) {
        sql.exec(ddl)
      }
      continue
    }

    const createIndexMatch = ddl.match(/CREATE\s+(?:UNIQUE\s+)?INDEX\s+IF\s+NOT\s+EXISTS\s+\w+\s+ON\s+(\w+)/i)
    if (createIndexMatch) {
      const tableName = createIndexMatch[1]
      if (ALL_REQUIRED_TABLES.has(tableName)) {
        sql.exec(ddl)
      }
      continue
    }
  }
}

/**
 * Find records in a metamodel collection.
 *
 * Port of GraphDLDB.findInCollection for metamodel tables.
 */
export function findInMetamodel(
  sql: SqlLike,
  collection: string,
  where?: Record<string, any>,
  opts?: { limit?: number; page?: number; sort?: string },
): { docs: Record<string, unknown>[]; totalDocs: number; hasNextPage: boolean; page: number; limit: number } {
  const table = resolveTable(collection)
  const fieldMap = getFieldMap(table)
  const reverseMap = reverseFieldMap(fieldMap)
  const limit = opts?.limit ?? 100
  const page = opts?.page ?? 1
  const offset = (page - 1) * limit

  let query = `SELECT * FROM ${table}`
  let countQuery = `SELECT COUNT(*) as cnt FROM ${table}`
  const queryParams: any[] = []
  const countParams: any[] = []

  if (where && Object.keys(where).length > 0) {
    const { clause, params } = buildWhereClause(where, fieldMap)
    if (clause) {
      query += ` WHERE ${clause}`
      countQuery += ` WHERE ${clause}`
      queryParams.push(...params)
      countParams.push(...params)
    }
  }

  // Sort
  if (opts?.sort) {
    const sortField = opts.sort.startsWith('-') ? opts.sort.slice(1) : opts.sort
    const sortDir = opts.sort.startsWith('-') ? 'DESC' : 'ASC'
    const sortCol = resolveColumn(sortField, fieldMap)
    query += ` ORDER BY ${sortCol} ${sortDir}`
  } else {
    query += ' ORDER BY created_at DESC'
  }

  query += ` LIMIT ? OFFSET ?`
  queryParams.push(limit, offset)

  const rows = sql.exec(query, ...queryParams).toArray()
  const countRow = sql.exec(countQuery, ...countParams).toArray()
  const totalDocs = ((countRow[0] as any)?.cnt as number) ?? 0

  const docs = rows.map(row => rowToPayload(row as Record<string, unknown>, reverseMap))
  const hasNextPage = offset + limit < totalDocs

  return { docs, totalDocs, hasNextPage, page, limit }
}

/**
 * Create a new record in a metamodel collection.
 *
 * Port of GraphDLDB.createInCollection for metamodel tables.
 */
export function createInMetamodel(
  sql: SqlLike,
  collection: string,
  data: Record<string, unknown>,
): Record<string, unknown> {
  const table = resolveTable(collection)
  const fieldMap = getFieldMap(table)
  const reverseMap = reverseFieldMap(fieldMap)

  const id = (data.id as string) ?? crypto.randomUUID()
  const now = new Date().toISOString().replace('T', ' ').replace('Z', '')

  // Translate Payload field names to SQL column names
  const { id: _id, createdAt: _ca, updatedAt: _ua, version: _v, ...rest } = data
  const sqlData = payloadToRow(rest, fieldMap)

  // Only include columns that exist in the table
  const tableColumns = new Set(getTableColumns(sql, table))
  const filteredData: Record<string, unknown> = {}
  for (const [col, value] of Object.entries(sqlData)) {
    if (tableColumns.has(col)) filteredData[col] = value
  }

  // Build columns and values
  const columns = ['id', 'created_at', 'updated_at', 'version', ...Object.keys(filteredData)]
  const placeholders = columns.map(() => '?').join(', ')
  const values = [id, now, now, 1, ...Object.values(filteredData)]

  sql.exec(
    `INSERT INTO ${table} (${columns.join(', ')}) VALUES (${placeholders})`,
    ...values,
  )

  // Return the created record
  const rows = sql.exec(`SELECT * FROM ${table} WHERE id = ?`, id).toArray()
  return rowToPayload(rows[0] as Record<string, unknown>, reverseMap)
}

/**
 * Update an existing record in a metamodel collection.
 *
 * Port of GraphDLDB.updateInCollection for metamodel tables.
 */
export function updateInMetamodel(
  sql: SqlLike,
  collection: string,
  id: string,
  updates: Record<string, unknown>,
): Record<string, unknown> | null {
  const table = resolveTable(collection)
  const fieldMap = getFieldMap(table)
  const reverseMap = reverseFieldMap(fieldMap)

  // Check existence
  const existing = sql.exec(`SELECT * FROM ${table} WHERE id = ?`, id).toArray()
  if (existing.length === 0) return null

  const currentVersion = ((existing[0] as any).version as number) ?? 1
  const now = new Date().toISOString().replace('T', ' ').replace('Z', '')

  // Translate and build SET clause
  const { id: _id, createdAt: _ca, updatedAt: _ua, version: _v, ...rest } = updates
  const sqlData = payloadToRow(rest, fieldMap)

  const setClauses = ['updated_at = ?', 'version = ?']
  const setValues: any[] = [now, currentVersion + 1]

  // Only set columns that exist in the table
  const tableColumns = getTableColumns(sql, table)
  const tableColumnSet = new Set(tableColumns)

  for (const [col, value] of Object.entries(sqlData)) {
    if (tableColumnSet.has(col)) {
      setClauses.push(`${col} = ?`)
      setValues.push(value)
    }
  }

  setValues.push(id)
  sql.exec(
    `UPDATE ${table} SET ${setClauses.join(', ')} WHERE id = ?`,
    ...setValues,
  )

  // Return updated record
  const rows = sql.exec(`SELECT * FROM ${table} WHERE id = ?`, id).toArray()
  return rowToPayload(rows[0] as Record<string, unknown>, reverseMap)
}

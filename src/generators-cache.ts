/**
 * Generators cache — pure functions for CRUD on the `generators` SQL table.
 *
 * The generators table caches compiled outputs (OpenAPI, SQLite DDL, schema
 * rules, etc.) keyed by (domain_id, output_format). It lives in DomainDB's
 * SQLite storage, not in entity-per-DO like the rest of the metamodel.
 *
 * This module replaces the legacy Payload field-map query engine that was
 * previously inlined in domain-do.ts. The queries are simple enough that
 * the generic buildWhereClause/FIELD_MAP machinery is no longer needed.
 */

import type { SqlLike } from './sql-like'

// =========================================================================
// Field mapping — generators-specific
// =========================================================================

/** Payload camelCase → SQL snake_case for the generators table. */
const FIELD_MAP: Record<string, string> = {
  domain: 'domain_id',
  outputFormat: 'output_format',
  versionNum: 'version_num',
}

/** SQL snake_case → Payload camelCase (reverse of FIELD_MAP). */
const REVERSE_MAP: Record<string, string> = Object.fromEntries(
  Object.entries(FIELD_MAP).map(([k, v]) => [v, k]),
)

/** Common timestamp fields. */
const COMMON_FIELDS: Record<string, string> = {
  createdAt: 'created_at',
  updatedAt: 'updated_at',
}

/** Resolve a key to a SQL column name. */
function toColumn(key: string): string {
  return FIELD_MAP[key] || COMMON_FIELDS[key] || key
}

/** Map a SQL row to a Payload-style object. */
function rowToPayload(row: Record<string, unknown>): Record<string, unknown> {
  const result: Record<string, unknown> = {}
  for (const [col, value] of Object.entries(row)) {
    result[REVERSE_MAP[col] || col] = value
  }
  return result
}

// =========================================================================
// Where clause builder (generators-only, simplified)
// =========================================================================

/**
 * Build a WHERE clause from a Payload-style where object.
 * Supports: equals, not_equals, in, like, exists, direct value shorthand,
 * and/or combinators. No FK traversal (generators has no joins).
 */
function buildWhere(
  where: Record<string, any>,
): { clause: string; params: any[] } {
  const conditions: string[] = []
  const params: any[] = []

  if (where.and) {
    const subs = (where.and as any[]).map(sub => buildWhere(sub))
    const clauses = subs.filter(s => s.clause).map(s => `(${s.clause})`)
    if (clauses.length) conditions.push(clauses.join(' AND '))
    for (const sub of subs) params.push(...sub.params)
  }

  if (where.or) {
    const subs = (where.or as any[]).map(sub => buildWhere(sub))
    const clauses = subs.filter(s => s.clause).map(s => `(${s.clause})`)
    if (clauses.length) conditions.push(`(${clauses.join(' OR ')})`)
    for (const sub of subs) params.push(...sub.params)
  }

  for (const [key, condition] of Object.entries(where)) {
    if (key === 'and' || key === 'or') continue
    const col = toColumn(key)

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

// =========================================================================
// CRUD operations
// =========================================================================

/**
 * Find generator records matching a where clause, with pagination and sort.
 */
export function findGenerators(
  sql: SqlLike,
  where?: Record<string, any>,
  opts?: { limit?: number; page?: number; sort?: string },
): { docs: Record<string, unknown>[]; totalDocs: number; hasNextPage: boolean; page: number; limit: number } {
  const limit = opts?.limit ?? 100
  const page = opts?.page ?? 1
  const offset = (page - 1) * limit

  let query = 'SELECT * FROM generators'
  let countQuery = 'SELECT COUNT(*) as cnt FROM generators'
  const queryParams: any[] = []
  const countParams: any[] = []

  if (where && Object.keys(where).length > 0) {
    const { clause, params } = buildWhere(where)
    if (clause) {
      query += ` WHERE ${clause}`
      countQuery += ` WHERE ${clause}`
      queryParams.push(...params)
      countParams.push(...params)
    }
  }

  if (opts?.sort) {
    const sortField = opts.sort.startsWith('-') ? opts.sort.slice(1) : opts.sort
    const sortDir = opts.sort.startsWith('-') ? 'DESC' : 'ASC'
    query += ` ORDER BY ${toColumn(sortField)} ${sortDir}`
  } else {
    query += ' ORDER BY created_at DESC'
  }

  query += ' LIMIT ? OFFSET ?'
  queryParams.push(limit, offset)

  const rows = sql.exec(query, ...queryParams).toArray()
  const countRow = sql.exec(countQuery, ...countParams).toArray()
  const totalDocs = ((countRow[0] as any)?.cnt as number) ?? 0

  const docs = rows.map(row => rowToPayload(row as Record<string, unknown>))
  const hasNextPage = offset + limit < totalDocs

  return { docs, totalDocs, hasNextPage, page, limit }
}

/**
 * Get a single generator record by ID.
 */
export function getGenerator(
  sql: SqlLike,
  id: string,
): Record<string, unknown> | null {
  const rows = sql.exec('SELECT * FROM generators WHERE id = ?', id).toArray()
  if (rows.length === 0) return null
  return rowToPayload(rows[0] as Record<string, unknown>)
}

/**
 * Create a new generator record.
 */
export function createGenerator(
  sql: SqlLike,
  data: Record<string, unknown>,
): Record<string, unknown> {
  const id = (data.id as string) ?? crypto.randomUUID()
  const now = new Date().toISOString().replace('T', ' ').replace('Z', '')

  // Get existing table columns
  const tableColumns = new Set(
    sql.exec('PRAGMA table_info(generators)').toArray().map((r: any) => r.name as string),
  )

  // Translate Payload field names to SQL column names, filtering to existing columns
  const filteredData: Record<string, unknown> = {}
  for (const [key, value] of Object.entries(data)) {
    if (key === 'id' || key === 'createdAt' || key === 'updatedAt' || key === 'version') continue
    const col = toColumn(key)
    if (tableColumns.has(col)) filteredData[col] = value
  }

  const columns = ['id', 'created_at', 'updated_at', 'version', ...Object.keys(filteredData)]
  const placeholders = columns.map(() => '?').join(', ')
  const values = [id, now, now, 1, ...Object.values(filteredData)]

  sql.exec(
    `INSERT INTO generators (${columns.join(', ')}) VALUES (${placeholders})`,
    ...values,
  )

  const rows = sql.exec('SELECT * FROM generators WHERE id = ?', id).toArray()
  return rowToPayload(rows[0] as Record<string, unknown>)
}

/**
 * Update an existing generator record by ID.
 */
export function updateGenerator(
  sql: SqlLike,
  id: string,
  updates: Record<string, unknown>,
): Record<string, unknown> | null {
  const existing = sql.exec('SELECT * FROM generators WHERE id = ?', id).toArray()
  if (existing.length === 0) return null

  const currentVersion = ((existing[0] as any).version as number) ?? 1
  const now = new Date().toISOString().replace('T', ' ').replace('Z', '')

  const tableColumns = new Set(
    sql.exec('PRAGMA table_info(generators)').toArray().map((r: any) => r.name as string),
  )

  const setClauses = ['updated_at = ?', 'version = ?']
  const setValues: any[] = [now, currentVersion + 1]

  for (const [key, value] of Object.entries(updates)) {
    if (key === 'id' || key === 'createdAt' || key === 'updatedAt' || key === 'version') continue
    const col = toColumn(key)
    if (tableColumns.has(col)) {
      setClauses.push(`${col} = ?`)
      setValues.push(value)
    }
  }

  setValues.push(id)
  sql.exec(
    `UPDATE generators SET ${setClauses.join(', ')} WHERE id = ?`,
    ...setValues,
  )

  const rows = sql.exec('SELECT * FROM generators WHERE id = ?', id).toArray()
  return rowToPayload(rows[0] as Record<string, unknown>)
}

/**
 * Delete a generator record by ID. No cascade needed — generators has no children.
 */
export function deleteGenerator(
  sql: SqlLike,
  id: string,
): { deleted: boolean } {
  const existing = sql.exec('SELECT id FROM generators WHERE id = ?', id).toArray()
  if (existing.length === 0) return { deleted: false }
  sql.exec('DELETE FROM generators WHERE id = ?', id)
  return { deleted: true }
}

/**
 * Delete all generator records for a domain.
 */
export function deleteGeneratorsForDomain(sql: SqlLike, domainId: string): void {
  try { sql.exec('DELETE FROM generators WHERE domain_id = ?', domainId) } catch { /* */ }
}

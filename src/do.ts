/**
 * GraphDLDB — Durable Object for the GraphDL ORM metamodel.
 *
 * Extends DurableObject directly (not @dotdo/db's DB) because we need
 * proper 3NF tables instead of the generic entities+JSON blob pattern.
 *
 * Provides:
 * - 3NF table initialization via initTables()
 * - Collection CRUD methods that translate Payload CMS field names to SQL columns
 * - Write mutex for serialized mutations
 * - Payload-compatible where clause builder
 */

import { DurableObject } from 'cloudflare:workers'
import { ALL_DDL, ALL_BOOTSTRAP } from './schema'
import { COLLECTION_TABLE_MAP, FIELD_MAP, FK_TARGET_TABLE } from './collections'

/** Fields common to every table — Payload camelCase → SQL snake_case. */
const COMMON_FIELDS: Record<string, string> = {
  createdAt: 'created_at',
  updatedAt: 'updated_at',
}
import type { Env } from './types'

// =========================================================================
// Helpers
// =========================================================================

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

    // Dot-notation: where[domain.domainSlug][equals]=joey
    // → domain_id IN (SELECT id FROM domains WHERE domain_slug = ?)
    if (key.includes('.')) {
      const [relationName, fieldName] = key.split('.', 2)
      const fkCol = fieldMap[relationName] || `${relationName}_id`
      const targetTable = FK_TARGET_TABLE[fkCol]
      if (!targetTable) continue // unknown relationship, skip

      const targetFieldMap = FIELD_MAP[targetTable] || {}
      const targetCol = targetFieldMap[fieldName] || fieldName

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

    const col = fieldMap[key] || COMMON_FIELDS[key] || key

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
    // Skip meta fields
    if (key === 'createdAt' || key === 'updatedAt' || key === 'version') continue
    const col = fieldMap[key] || COMMON_FIELDS[key] || key
    result[col] = value
  }
  return result
}

/** Get table column names by querying PRAGMA. */
function getTableColumns(sql: SqlStorage, table: string): string[] {
  const rows = sql.exec(`PRAGMA table_info(${table})`).toArray()
  return rows.map(r => r.name as string)
}

// =========================================================================
// GraphDLDB Durable Object
// =========================================================================

export class GraphDLDB extends DurableObject {
  protected sql: SqlStorage
  protected _writeTail: Promise<void> = Promise.resolve()

  constructor(ctx: DurableObjectState, env: Env) {
    super(ctx, env)
    this.sql = ctx.storage.sql
    this.initTables()
  }

  // =========================================================================
  // Table Initialization
  // =========================================================================

  protected initTables(): void {
    for (const ddl of ALL_DDL) {
      this.sql.exec(ddl)
    }

    // Migrations: add columns that didn't exist in earlier DDL versions
    const migrations = [
      'ALTER TABLE functions ADD COLUMN headers TEXT',
      'ALTER TABLE domains ADD COLUMN app_id TEXT',
    ]
    for (const migration of migrations) {
      try { this.sql.exec(migration) } catch { /* column already exists */ }
    }

    // Migration: widen org_memberships role CHECK to include 'admin'
    // SQLite can't ALTER CHECK constraints, so recreate the table
    try {
      const hasAdmin = this.sql.exec(
        `SELECT sql FROM sqlite_master WHERE name = 'org_memberships'`
      ).toArray()
      const ddl = hasAdmin[0]?.sql as string || ''
      if (ddl && !ddl.includes("'admin'")) {
        this.sql.exec(`CREATE TABLE IF NOT EXISTS org_memberships_new (
          id TEXT PRIMARY KEY,
          user_email TEXT NOT NULL,
          organization_id TEXT NOT NULL REFERENCES organizations(id),
          role TEXT NOT NULL DEFAULT 'member' CHECK (role IN ('owner', 'admin', 'member')),
          created_at TEXT NOT NULL DEFAULT (datetime('now')),
          updated_at TEXT NOT NULL DEFAULT (datetime('now')),
          version INTEGER NOT NULL DEFAULT 1,
          UNIQUE(user_email, organization_id)
        )`)
        this.sql.exec(`INSERT INTO org_memberships_new SELECT * FROM org_memberships`)
        this.sql.exec(`DROP TABLE org_memberships`)
        this.sql.exec(`ALTER TABLE org_memberships_new RENAME TO org_memberships`)
        this.sql.exec(`CREATE INDEX IF NOT EXISTS idx_org_memberships_email ON org_memberships(user_email)`)
        this.sql.exec(`CREATE INDEX IF NOT EXISTS idx_org_memberships_org ON org_memberships(organization_id)`)
      }
    } catch { /* migration already applied or table doesn't exist yet */ }

    // Migration: unique partial index — at most one owner per org
    try {
      this.sql.exec(`CREATE UNIQUE INDEX IF NOT EXISTS idx_org_one_owner ON org_memberships(organization_id) WHERE role = 'owner'`)
    } catch { /* already exists */ }

    // CDC events table — tracks mutations for sync/forwarding
    this.sql.exec(`
      CREATE TABLE IF NOT EXISTS cdc_events (
        id TEXT PRIMARY KEY,
        timestamp TEXT NOT NULL DEFAULT (datetime('now')),
        operation TEXT NOT NULL,
        table_name TEXT NOT NULL,
        entity_id TEXT NOT NULL,
        data TEXT
      )
    `)
    this.sql.exec('CREATE INDEX IF NOT EXISTS idx_cdc_events_timestamp ON cdc_events(timestamp)')
    this.sql.exec('CREATE INDEX IF NOT EXISTS idx_cdc_events_table ON cdc_events(table_name, entity_id)')

    // Bootstrap metamodel data (idempotent)
    for (const dml of ALL_BOOTSTRAP) {
      this.sql.exec(dml)
    }
  }

  // =========================================================================
  // Write Lock
  // =========================================================================

  protected withWriteLock<T>(fn: () => Promise<T>): Promise<T> {
    const result = this._writeTail.then(fn, fn)
    this._writeTail = result.then(
      () => {},
      () => {},
    )
    return result
  }

  // =========================================================================
  // CDC Event Logging
  // =========================================================================

  protected logCdcEvent(operation: string, tableName: string, entityId: string, data?: Record<string, unknown>): void {
    const id = crypto.randomUUID()
    const jsonData = data ? JSON.stringify(data) : null
    this.sql.exec(
      'INSERT INTO cdc_events (id, timestamp, operation, table_name, entity_id, data) VALUES (?, datetime(\'now\'), ?, ?, ?, ?)',
      id, operation, tableName, entityId, jsonData,
    )
  }

  // =========================================================================
  // Collection Methods
  // =========================================================================

  /**
   * Resolve a Payload collection slug to a SQL table name.
   * Throws if the slug is unknown.
   */
  protected resolveTable(collectionSlug: string): string {
    const table = COLLECTION_TABLE_MAP[collectionSlug]
    if (!table) {
      throw new Error(`Unknown collection: ${collectionSlug}`)
    }
    return table
  }

  /**
   * Get the field map for a table (Payload field name → SQL column name).
   */
  protected getFieldMap(table: string): Record<string, string> {
    return FIELD_MAP[table] || {}
  }

  /**
   * Find records in a collection.
   *
   * @param collectionSlug — Payload collection slug (e.g. 'graph-schemas')
   * @param where — Payload-style where object
   * @param options — { limit, page, sort }
   * @returns { docs, totalDocs, hasNextPage, page, limit }
   */
  async findInCollection(
    collectionSlug: string,
    where?: Record<string, any>,
    options?: { limit?: number; page?: number; sort?: string },
  ): Promise<{ docs: Record<string, unknown>[]; totalDocs: number; hasNextPage: boolean; page: number; limit: number }> {
    const table = this.resolveTable(collectionSlug)
    const fieldMap = this.getFieldMap(table)
    const reverseMap = reverseFieldMap(fieldMap)
    const limit = options?.limit ?? 100
    const page = options?.page ?? 1
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
    if (options?.sort) {
      const sortField = options.sort.startsWith('-') ? options.sort.slice(1) : options.sort
      const sortDir = options.sort.startsWith('-') ? 'DESC' : 'ASC'
      const sortCol = fieldMap[sortField] || COMMON_FIELDS[sortField] || sortField
      query += ` ORDER BY ${sortCol} ${sortDir}`
    } else {
      query += ' ORDER BY created_at DESC'
    }

    query += ` LIMIT ? OFFSET ?`
    queryParams.push(limit, offset)

    const rows = this.sql.exec(query, ...queryParams).toArray()
    const countRow = this.sql.exec(countQuery, ...countParams).toArray()
    const totalDocs = (countRow[0]?.cnt as number) ?? 0

    const docs = rows.map(row => rowToPayload(row as Record<string, unknown>, reverseMap))
    const hasNextPage = offset + limit < totalDocs

    return { docs, totalDocs, hasNextPage, page, limit }
  }

  /**
   * Get a single record by ID.
   */
  async getFromCollection(collectionSlug: string, id: string): Promise<Record<string, unknown> | null> {
    const table = this.resolveTable(collectionSlug)
    const fieldMap = this.getFieldMap(table)
    const reverseMap = reverseFieldMap(fieldMap)

    const rows = this.sql.exec(`SELECT * FROM ${table} WHERE id = ?`, id).toArray()
    if (rows.length === 0) return null

    return rowToPayload(rows[0] as Record<string, unknown>, reverseMap)
  }

  /**
   * Create a new record in a collection.
   */
  async createInCollection(collectionSlug: string, data: Record<string, unknown>): Promise<Record<string, unknown>> {
    return this.withWriteLock(async () => {
      const table = this.resolveTable(collectionSlug)
      const fieldMap = this.getFieldMap(table)
      const reverseMap = reverseFieldMap(fieldMap)

      const id = (data.id as string) ?? crypto.randomUUID()
      const now = new Date().toISOString().replace('T', ' ').replace('Z', '')

      // Translate Payload field names to SQL column names
      const { id: _id, createdAt: _ca, updatedAt: _ua, version: _v, ...rest } = data
      const sqlData = payloadToRow(rest, fieldMap)

      // Only include columns that exist in the table
      const tableColumns = new Set(getTableColumns(this.sql, table))
      const filteredData: Record<string, unknown> = {}
      for (const [col, value] of Object.entries(sqlData)) {
        if (tableColumns.has(col)) filteredData[col] = value
      }

      // Build columns and values
      const columns = ['id', 'created_at', 'updated_at', 'version', ...Object.keys(filteredData)]
      const placeholders = columns.map(() => '?').join(', ')
      const values = [id, now, now, 1, ...Object.values(filteredData)]

      this.sql.exec(
        `INSERT INTO ${table} (${columns.join(', ')}) VALUES (${placeholders})`,
        ...values,
      )

      this.logCdcEvent('create', table, id, { ...filteredData, id })

      // Return the created record
      const rows = this.sql.exec(`SELECT * FROM ${table} WHERE id = ?`, id).toArray()
      return rowToPayload(rows[0] as Record<string, unknown>, reverseMap)
    })
  }

  /**
   * Update an existing record.
   */
  async updateInCollection(collectionSlug: string, id: string, data: Record<string, unknown>): Promise<Record<string, unknown> | null> {
    return this.withWriteLock(async () => {
      const table = this.resolveTable(collectionSlug)
      const fieldMap = this.getFieldMap(table)
      const reverseMap = reverseFieldMap(fieldMap)

      // Check existence
      const existing = this.sql.exec(`SELECT * FROM ${table} WHERE id = ?`, id).toArray()
      if (existing.length === 0) return null

      const currentVersion = (existing[0].version as number) ?? 1
      const now = new Date().toISOString().replace('T', ' ').replace('Z', '')

      // Translate and build SET clause
      const { id: _id, createdAt: _ca, updatedAt: _ua, version: _v, ...rest } = data
      const sqlData = payloadToRow(rest, fieldMap)

      const setClauses = ['updated_at = ?', 'version = ?']
      const setValues: any[] = [now, currentVersion + 1]

      // Only set columns that exist in the table
      const tableColumns = getTableColumns(this.sql, table)
      const tableColumnSet = new Set(tableColumns)

      for (const [col, value] of Object.entries(sqlData)) {
        if (tableColumnSet.has(col)) {
          setClauses.push(`${col} = ?`)
          setValues.push(value)
        }
      }

      setValues.push(id)
      this.sql.exec(
        `UPDATE ${table} SET ${setClauses.join(', ')} WHERE id = ?`,
        ...setValues,
      )

      this.logCdcEvent('update', table, id, sqlData)

      // Return updated record
      const rows = this.sql.exec(`SELECT * FROM ${table} WHERE id = ?`, id).toArray()
      return rowToPayload(rows[0] as Record<string, unknown>, reverseMap)
    })
  }

  /**
   * Delete a record by ID.
   */
  async deleteFromCollection(collectionSlug: string, id: string): Promise<{ deleted: boolean }> {
    return this.withWriteLock(async () => {
      const table = this.resolveTable(collectionSlug)

      // Check existence before delete
      const existing = this.sql.exec(`SELECT id FROM ${table} WHERE id = ?`, id).toArray()
      if (existing.length === 0) return { deleted: false }

      this.sql.exec(`DELETE FROM ${table} WHERE id = ?`, id)
      this.logCdcEvent('delete', table, id)

      return { deleted: true }
    })
  }

  // =========================================================================
  // Bulk / Utility
  // =========================================================================

  /**
   * Wipe all data from all tables. For testing/reset only.
   */
  async wipeAllData(): Promise<void> {
    return this.withWriteLock(async () => {
      // Delete in reverse dependency order (instances first, then definitions, then core)
      const tables = [
        'cdc_events',
        'guard_runs', 'events', 'state_machines', 'resource_roles', 'resources', 'graphs',
        'functions', 'streams', 'verbs', 'guards', 'transitions', 'statuses', 'event_types', 'state_machine_definitions',
        'constraint_spans', 'constraints', 'roles', 'readings', 'graph_schemas', 'nouns',
        'domains', 'org_memberships', 'organizations',
      ]
      for (const table of tables) {
        this.sql.exec(`DELETE FROM ${table}`)
      }
    })
  }

  // =========================================================================
  // Request Routing (stub — Task 5 adds the full REST router)
  // =========================================================================

  async fetch(request: Request): Promise<Response> {
    return new Response(JSON.stringify({ status: 'ok', version: '0.1.0' }), {
      headers: { 'Content-Type': 'application/json' },
    })
  }
}

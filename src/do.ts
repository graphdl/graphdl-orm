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
import { DomainModel, SqlDataLoader } from './model/domain-model'

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
  private models: Map<string, DomainModel> = new Map()

  getModel(domainId: string): DomainModel {
    let model = this.models.get(domainId)
    if (!model) {
      model = new DomainModel(new SqlDataLoader(this.sql), domainId)
      this.models.set(domainId, model)
    }
    return model
  }

  constructor(ctx: DurableObjectState, env: Env) {
    super(ctx, env)
    this.sql = ctx.storage.sql
    // Ensure clean FK state on startup — defer_foreign_keys persists on reused connections
    this.sql.exec('PRAGMA defer_foreign_keys = OFF')
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
      'CREATE INDEX IF NOT EXISTS idx_domains_app ON domains(app_id)',
      // events table: add domain_id and updated_at
      'ALTER TABLE events ADD COLUMN domain_id TEXT',
      'ALTER TABLE events ADD COLUMN updated_at TEXT',
      'CREATE INDEX IF NOT EXISTS idx_events_domain ON events(domain_id)',
      // statuses + transitions: add domain_id for direct domain scoping
      'ALTER TABLE statuses ADD COLUMN domain_id TEXT',
      'CREATE INDEX IF NOT EXISTS idx_statuses_domain ON statuses(domain_id)',
      'ALTER TABLE transitions ADD COLUMN domain_id TEXT',
      'ALTER TABLE transitions ADD COLUMN state_machine_definition_id TEXT',
      'CREATE INDEX IF NOT EXISTS idx_transitions_domain ON transitions(domain_id)',
      // resources: add created_by for per-user scoping
      'ALTER TABLE resources ADD COLUMN created_by TEXT',
      'CREATE INDEX IF NOT EXISTS idx_resources_created_by ON resources(created_by)',
      // constraints: add text column for source text round-tripping
      'ALTER TABLE constraints ADD COLUMN text TEXT',
      // constraints: add set_comparison_argument_length for SS/XC/EQ
      'ALTER TABLE constraints ADD COLUMN set_comparison_argument_length INTEGER',
      // constraint_spans: add subset_autofill for SS constraints
      'ALTER TABLE constraint_spans ADD COLUMN subset_autofill INTEGER DEFAULT 0',
      // apps: add config columns (from readings/organizations.md)
      'ALTER TABLE apps ADD COLUMN app_type TEXT',
      'ALTER TABLE apps ADD COLUMN chat_endpoint TEXT',
      // messages: add sender_identity for user/assistant role distinction
      'ALTER TABLE messages ADD COLUMN sender_identity TEXT',
    ]

    // Entity tables (messages, support_requests, etc.) are created by applySchema on first use.
    for (const migration of migrations) {
      try { this.sql.exec(migration) } catch { /* column/index already exists */ }
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

    // Migration: widen constraints kind CHECK to include 'RC' for ring constraints
    try {
      const constraintsDdl = this.sql.exec(
        `SELECT sql FROM sqlite_master WHERE name = 'constraints'`
      ).toArray()
      const sql = constraintsDdl[0]?.sql as string || ''
      if (sql && !sql.includes("'RC'")) {
        this.sql.exec(`CREATE TABLE IF NOT EXISTS constraints_new (
          id TEXT PRIMARY KEY,
          kind TEXT NOT NULL CHECK (kind IN ('UC', 'MC', 'SS', 'XC', 'EQ', 'OR', 'XO', 'RC')),
          modality TEXT NOT NULL DEFAULT 'Alethic' CHECK (modality IN ('Alethic', 'Deontic')),
          text TEXT,
          domain_id TEXT REFERENCES domains(id),
          created_at TEXT NOT NULL DEFAULT (datetime('now')),
          updated_at TEXT NOT NULL DEFAULT (datetime('now')),
          version INTEGER NOT NULL DEFAULT 1
        )`)
        this.sql.exec(`INSERT INTO constraints_new SELECT id, kind, modality, text, domain_id, created_at, updated_at, version FROM constraints`)
        this.sql.exec(`DROP TABLE constraints`)
        this.sql.exec(`ALTER TABLE constraints_new RENAME TO constraints`)
        this.sql.exec(`CREATE INDEX IF NOT EXISTS idx_constraints_domain ON constraints(domain_id)`)
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

  /** Wipe all metamodel data for a domain (nouns, readings, constraints, roles, etc.) */
  wipeDomainMetamodel(domainId: string): { deleted: boolean; domainId: string; counts: Record<string, any> } {
    const counts: Record<string, any> = {}
    this.sql.exec('PRAGMA foreign_keys = OFF')

    // Delete child records via parent join (roles/constraint_spans don't have domain_id)
    const countAndDelete = (table: string, where: string, ...params: any[]) => {
      try {
        const before = [...this.sql.exec(`SELECT count(*) as c FROM ${table} WHERE ${where}`, ...params)][0] as any
        this.sql.exec(`DELETE FROM ${table} WHERE ${where}`, ...params)
        counts[table] = { before: before?.c || 0, after: 0 }
      } catch (e: any) { counts[table] = { error: e.message } }
    }

    // constraint_spans → via constraints
    countAndDelete('constraint_spans',
      'constraint_id IN (SELECT id FROM constraints WHERE domain_id = ?)', domainId)
    // roles → via graph_schemas
    countAndDelete('roles',
      'graph_schema_id IN (SELECT id FROM graph_schemas WHERE domain_id = ?)', domainId)
    // Direct domain_id tables
    for (const table of ['constraints', 'readings', 'graph_schemas', 'nouns']) {
      countAndDelete(table, 'domain_id = ?', domainId)
    }

    this.sql.exec('PRAGMA foreign_keys = ON')
    try { this.sql.exec(`DELETE FROM generators WHERE domain_id = ?`, domainId) } catch { /* */ }
    this.getModel(domainId).invalidate()
    return { deleted: true, domainId, counts }
  }

  /**
   * Resolve derivation rules for an entity type.
   * Derivation rules are readings with `:=` predicates that compute values from related entities.
   *
   * Example: "SupportRequest was submitted on Date := min of SentAt where SupportRequest has Message and Message has SentAt."
   * → For each SupportRequest, SELECT MIN(sent_at) FROM messages WHERE support_request_id = ?
   */
  private resolveDerivations(
    domainId: string,
    nounName: string,
    docs: Record<string, unknown>[],
    toTableName: (s: string) => string,
    toColumnName: (s: string) => string,
  ): Array<{ field: string; resolve: (entityId: string) => unknown }> {
    if (docs.length === 0) return []

    // Find derivation readings for this noun — search by domain_id on the reading itself
    const likePattern = `${nounName}%:=%`
    const derivationReadings = [...this.sql.exec(
      `SELECT text FROM readings WHERE domain_id = ? AND text LIKE ?`,
      domainId, likePattern,
    )]

    if (derivationReadings.length === 0) return []

    const results: Array<{ field: string; resolve: (entityId: string) => unknown }> = []

    for (const row of derivationReadings) {
      const text = row.text as string
      const match = text.match(/^(\w+)\s+(.+?)\s*:=\s*(.+?)\.?$/)
      if (!match) continue

      const [, subject, predicate, expression] = match

      // Parse "min of SentAt where SupportRequest has Message and Message has SentAt"
      const minMatch = expression.trim().match(/^min of (\w+) where (\w+) has (\w+) and (\w+) has (\w+)$/i)
      if (minMatch) {
        const [, aggregateField, parentNoun, childNoun, childNoun2, valueField] = minMatch
        const childTable = toTableName(childNoun)
        const parentFkCol = toColumnName(parentNoun) + '_id'
        const valueCol = toColumnName(valueField)

        // Derive the camelCase field name from the predicate
        // "was submitted on Date" → "date" or "submittedOn"
        const predicateWords = predicate.trim().split(/\s+/)
        const lastWord = predicateWords[predicateWords.length - 1]
        const fieldName = toColumnName(lastWord)

        try {
          // Verify table and columns exist
          this.sql.exec(`SELECT 1 FROM ${childTable} LIMIT 0`)
          const cols = new Set(
            this.sql.exec(`PRAGMA table_info(${childTable})`).toArray().map((c: any) => c.name as string)
          )
          if (!cols.has(parentFkCol) || !cols.has(valueCol)) continue

          results.push({
            field: fieldName,
            resolve: (entityId: string) => {
              try {
                const result = this.sql.exec(
                  `SELECT MIN(${valueCol}) as val FROM ${childTable} WHERE ${parentFkCol} = ? AND domain_id = ?`,
                  entityId, domainId,
                ).toArray()
                return result[0]?.val ?? null
              } catch { return null }
            },
          })
        } catch { /* table doesn't exist */ }
      }
    }

    return results
  }

  /** Read entity table info from cached schema-map. No applySchema, no write lock. */
  private async getEntityTableFromCache(
    domainId: string,
    nounName: string,
    toTableName: (s: string) => string,
  ): Promise<{ tableName: string; fieldMap: Record<string, string> } | null> {
    const rows = this.sql.exec(
      "SELECT output FROM generators WHERE domain_id = ? AND output_format = 'schema-map' LIMIT 1", domainId,
    ).toArray()
    if (!rows.length || !rows[0].output) return null
    const cached = JSON.parse(rows[0].output as string)
    const tableName = cached.tableMap?.[nounName] || toTableName(nounName)
    try {
      this.sql.exec(`SELECT 1 FROM ${tableName} LIMIT 0`)
    } catch {
      return null
    }
    return { tableName, fieldMap: cached.fieldMap?.[tableName] || {} }
  }

  // =========================================================================
  // Debug
  // =========================================================================

  inspectTable(table: string): Record<string, any> {
    try {
      const columns = [...this.sql.exec(`PRAGMA table_info(${table})`)].map((r: any) => ({
        name: r.name, type: r.type, notnull: r.notnull, dflt_value: r.dflt_value, pk: r.pk,
      }))
      const fks = [...this.sql.exec(`PRAGMA foreign_key_list(${table})`)].map((r: any) => ({
        id: r.id, seq: r.seq, table: r.table, from: r.from, to: r.to,
      }))
      const foreignKeysOn = [...this.sql.exec('PRAGMA foreign_keys')][0]
      const ddl = [...this.sql.exec(`SELECT sql FROM sqlite_master WHERE type='table' AND name='${table}'`)]
      return { table, columns, foreignKeys: fks, foreignKeysPragma: foreignKeysOn, ddl: ddl[0]?.sql || null }
    } catch (e: any) {
      return { table, error: e.message }
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

    // Broadcast to connected WebSocket clients
    const event = JSON.stringify({ type: 'cdc', operation, table: tableName, id: entityId, ...(data?.domain_id && { domain: data.domain_id }) })
    for (const ws of this.ctx.getWebSockets()) {
      try {
        const tags = this.ctx.getTags(ws)
        // Send to all clients, or domain-filtered clients
        const domainId = data?.domain_id as string | undefined
        if (!domainId || tags.includes('all') || tags.includes(domainId)) {
          ws.send(event)
        }
      } catch { /* client disconnected */ }
    }
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

      // Auto-invalidate DomainModel cache
      const domainId = (sqlData.domain_id ?? filteredData.domain_id) as string
      if (domainId) this.getModel(domainId).invalidate(collectionSlug)

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

      // Auto-invalidate DomainModel cache
      const domainId = (sqlData.domain_id ?? (existing[0] as any).domain_id) as string
      if (domainId) this.getModel(domainId).invalidate(collectionSlug)

      // Return updated record
      const rows = this.sql.exec(`SELECT * FROM ${table} WHERE id = ?`, id).toArray()
      return rowToPayload(rows[0] as Record<string, unknown>, reverseMap)
    })
  }

  /**
   * Cascade-delete all children of a domain (no lock — caller must hold write lock).
   */
  private cascadeDeleteDomain(domainId: string): number {
    let cascaded = 0
    // Disable FK enforcement during cascade — cross-domain references (e.g. roles
    // from other domains referencing nouns in this domain) would otherwise block deletion
    try { this.sql.exec('PRAGMA foreign_keys = OFF') } catch { /* best effort */ }

    // Delete domain-scoped children in leaf-to-root order
    const domainScopedTables = [
      'guard_runs', 'events', 'state_machines', 'resource_roles', 'resources', 'graphs',
      'completions', 'agents', 'agent_definitions',
      'functions', 'streams', 'verbs', 'guards', 'transitions', 'statuses', 'event_types', 'state_machine_definitions',
      'constraint_spans', 'constraints', 'roles', 'readings', 'graph_schemas', 'nouns',
    ]
    for (const child of domainScopedTables) {
      try {
        // Tables without direct domain_id — delete via parent FK chain
        if (child === 'constraint_spans') {
          this.sql.exec(
            `DELETE FROM constraint_spans WHERE constraint_id IN (SELECT id FROM constraints WHERE domain_id = ?)`, domainId
          )
        } else if (child === 'roles') {
          // Delete roles by reading OR by noun — covers cross-domain references
          this.sql.exec(
            `DELETE FROM roles WHERE reading_id IN (SELECT id FROM readings WHERE domain_id = ?) OR noun_id IN (SELECT id FROM nouns WHERE domain_id = ?)`, domainId, domainId
          )
        } else if (child === 'transitions') {
          this.sql.exec(
            `DELETE FROM transitions WHERE status_from_id IN (SELECT id FROM statuses WHERE state_machine_definition_id IN (SELECT id FROM state_machine_definitions WHERE domain_id = ?))`, domainId
          )
        } else if (child === 'statuses') {
          this.sql.exec(
            `DELETE FROM statuses WHERE state_machine_definition_id IN (SELECT id FROM state_machine_definitions WHERE domain_id = ?)`, domainId
          )
        } else {
          this.sql.exec(`DELETE FROM ${child} WHERE domain_id = ?`, domainId)
        }
        cascaded++
      } catch { /* table may not exist yet */ }
    }

    try { this.sql.exec('PRAGMA foreign_keys = ON') } catch { /* best effort */ }
    return cascaded
  }

  /**
   * FK dependency map: parent_table → [(child_table, fk_column)].
   * Used by deleteFromCollection to cascade-delete children before the parent.
   */
  private static readonly CASCADE_MAP: Record<string, Array<[string, string]>> = {
    readings:                    [['roles', 'reading_id']],
    graph_schemas:               [['roles', 'graph_schema_id'], ['readings', 'graph_schema_id']],
    nouns:                       [['roles', 'noun_id']],
    constraints:                 [['constraint_spans', 'constraint_id']],
    roles:                       [['constraint_spans', 'role_id']],
    state_machine_definitions:   [['statuses', 'state_machine_definition_id'], ['transitions', 'state_machine_definition_id']],
    statuses:                    [['transitions', 'status_from_id'], ['transitions', 'status_to_id']],
    event_types:                 [['transitions', 'event_type_id']],
    transitions:                 [['guards', 'transition_id']],
  }

  /**
   * Cascade-delete children of a record, recursively.
   * Returns count of deleted child rows.
   */
  private cascadeDeleteChildren(table: string, id: string): number {
    let cascaded = 0
    const children = GraphDLDB.CASCADE_MAP[table]
    if (!children) return cascaded

    for (const [childTable, fkCol] of children) {
      // Find child IDs first, then recurse before deleting
      const childRows = this.sql.exec(`SELECT id FROM ${childTable} WHERE ${fkCol} = ?`, id).toArray()
      for (const row of childRows) {
        cascaded += this.cascadeDeleteChildren(childTable, row.id as string)
        this.sql.exec(`DELETE FROM ${childTable} WHERE id = ?`, row.id as string)
        this.logCdcEvent('delete', childTable, row.id as string)
        cascaded++
      }
    }
    return cascaded
  }

  /**
   * Delete a record by ID. Cascade-deletes all children via FK dependency map.
   */
  async deleteFromCollection(collectionSlug: string, id: string): Promise<{ deleted: boolean; cascaded?: number }> {
    return this.withWriteLock(async () => {
      const table = this.resolveTable(collectionSlug)

      // Check existence before delete (SELECT * so we can read domain_id for cache invalidation)
      const existing = this.sql.exec(`SELECT * FROM ${table} WHERE id = ?`, id).toArray()
      if (existing.length === 0) return { deleted: false }

      let cascaded = 0

      if (table === 'domains') {
        cascaded = this.cascadeDeleteDomain(id)
      } else if (table === 'apps') {
        // Cascade-delete all domains belonging to this app
        const appDomains = this.sql.exec(`SELECT id FROM domains WHERE app_id = ?`, id).toArray()
        for (const domain of appDomains) {
          cascaded += this.cascadeDeleteDomain(domain.id as string)
          this.sql.exec(`DELETE FROM domains WHERE id = ?`, domain.id as string)
          this.logCdcEvent('delete', 'domains', domain.id as string)
          cascaded++
        }
      } else {
        // General cascade: delete children via FK dependency map
        cascaded = this.cascadeDeleteChildren(table, id)
      }

      this.sql.exec(`DELETE FROM ${table} WHERE id = ?`, id)
      this.logCdcEvent('delete', table, id)

      // Auto-invalidate DomainModel cache
      const domainId = (existing[0] as any).domain_id as string
      if (domainId) this.getModel(domainId).invalidate(collectionSlug)

      return { deleted: true, ...(cascaded > 0 && { cascaded }) }
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

  /**
   * Create a fact instance: a Graph linking Resources through Roles.
   *
   * This is the instance-level write operation. "Customer has ContactName 'John'"
   * becomes: Graph(schema=CustomerHasContactName) + ResourceRole(Customer) + ResourceRole(ContactName).
   *
   * @param domainId - Domain ID
   * @param graphSchemaId - Graph Schema ID (the fact type)
   * @param bindings - Array of { nounId, value?, resourceId? } for each role
   *                   If resourceId is provided, uses existing resource.
   *                   If value is provided, finds or creates a resource for the noun with that value.
   * @returns The created Graph with its resource roles
   */
  async createFact(
    domainId: string,
    graphSchemaId: string,
    bindings: Array<{ nounId: string; value?: string; resourceId?: string }>,
  ): Promise<Record<string, unknown>> {
    return this.withWriteLock(async () => {
      const now = new Date().toISOString().replace('T', ' ').replace('Z', '')

      // Resolve or create resources for each binding
      const resolvedResources: string[] = []
      for (const binding of bindings) {
        if (binding.resourceId) {
          resolvedResources.push(binding.resourceId)
          continue
        }

        // Find existing resource by noun + value, or create one
        if (binding.value !== undefined) {
          const existing = this.sql.exec(
            'SELECT id FROM resources WHERE noun_id = ? AND domain_id = ? AND value = ? LIMIT 1',
            binding.nounId, domainId, binding.value,
          ).toArray()

          if (existing.length > 0) {
            resolvedResources.push(existing[0].id as string)
          } else {
            const resourceId = crypto.randomUUID()
            this.sql.exec(
              'INSERT INTO resources (id, noun_id, domain_id, value, created_at, updated_at, version) VALUES (?, ?, ?, ?, ?, ?, 1)',
              resourceId, binding.nounId, domainId, binding.value, now, now,
            )
            this.logCdcEvent('create', 'resources', resourceId)
            resolvedResources.push(resourceId)
          }
        }
      }

      // Create the Graph
      const graphId = crypto.randomUUID()
      this.sql.exec(
        'INSERT INTO graphs (id, graph_schema_id, domain_id, created_at, updated_at, version) VALUES (?, ?, ?, ?, ?, 1)',
        graphId, graphSchemaId, domainId, now, now,
      )
      this.logCdcEvent('create', 'graphs', graphId)

      // Create ResourceRoles linking Graph → Resources
      // Get roles for this graph schema to map bindings to role indices
      const roles = this.sql.exec(
        'SELECT id, role_index FROM roles WHERE graph_schema_id = ? ORDER BY role_index',
        graphSchemaId,
      ).toArray()

      for (let i = 0; i < Math.min(resolvedResources.length, roles.length); i++) {
        const rrId = crypto.randomUUID()
        this.sql.exec(
          'INSERT INTO resource_roles (id, graph_id, resource_id, role_id, domain_id, created_at, updated_at, version) VALUES (?, ?, ?, ?, ?, ?, ?, 1)',
          rrId, graphId, resolvedResources[i], roles[i].id, domainId, now, now,
        )
        this.logCdcEvent('create', 'resource_roles', rrId)
      }

      // Invalidate cache
      this.getModel(domainId).invalidate('graphs')

      return {
        id: graphId,
        graphSchema: graphSchemaId,
        domain: domainId,
        resourceRoles: resolvedResources.map((rid, i) => ({
          resource: rid,
          role: roles[i]?.id,
          roleIndex: i,
        })),
      }
    })
  }

  /**
   * Create an entity instance with fields — the high-level write operation.
   *
   * "Create a SupportRequest with issueType='billing', customer='john@example.com'"
   * becomes: Resource + Graph facts for each field, resolved from readings.
   *
   * The caller doesn't need to know noun IDs, graph schema IDs, or reading text.
   * The readings are the source — the system resolves everything.
   */
  async createEntity(
    domainId: string,
    nounName: string,
    fields: Record<string, string | string[] | Record<string, string | string[]>>,
    reference?: string,
    createdBy?: string,
  ): Promise<Record<string, unknown>> {
    // Pre-import and pre-warm child entity tables BEFORE entering the write lock
    // to avoid await-induced transaction breaks and getEntityTable deadlocks
    const { toColumnName, toTableName } = await import('./generate/sqlite')

    // Pre-warm: ensure entity tables exist for any array-of-objects child nouns
    for (const [fieldName, fieldValue] of Object.entries(fields)) {
      if (Array.isArray(fieldValue) && fieldValue.length > 0 && typeof fieldValue[0] === 'object') {
        const singular = fieldName.replace(/s$/, '')
        const childNoun = singular.charAt(0).toUpperCase() + singular.slice(1)
        await this.getEntityTable(domainId, childNoun) // triggers applySchema if needed
      }
    }
    // Also pre-warm the parent table
    await this.getEntityTable(domainId, nounName)

    return this.withWriteLock(async () => {
      // Reset deferred FK state (may persist from prior requests on reused connection)
      this.sql.exec('PRAGMA defer_foreign_keys = OFF')
      const now = new Date().toISOString().replace('T', ' ').replace('Z', '')

      // Read table info from cached schema-map (pre-warmed above, no await needed)
      const entityTable = await this.getEntityTableFromCache(domainId, nounName, toTableName)

      if (entityTable) {
        // ── 3NF path: insert one row into the entity's table ──
        const { tableName, fieldMap } = entityTable
        const id = crypto.randomUUID()
        const cols = ['id', 'domain_id', 'created_at', 'updated_at', 'version']
        const vals: any[] = [id, domainId, now, now, 1]

        // Get existing columns in the table
        const tableColumns = new Set<string>()
        try {
          const pragma = this.sql.exec(`PRAGMA table_info(${tableName})`).toArray()
          for (const row of pragma) tableColumns.add(row.name as string)
        } catch { /* table doesn't exist */ }

        // Map fields to columns
        const nestedEntities: Array<{ nounName: string; fields: Record<string, any>; fkCol: string }> = []
        const childCreationResults: any[] = []

        for (const [fieldName, fieldValue] of Object.entries(fields)) {
          if (fieldValue === undefined || fieldValue === null) continue

          // Nested object = entity type — handle after parent insert
          if (typeof fieldValue === 'object' && !Array.isArray(fieldValue)) {
            const colName = fieldMap[fieldName] || toColumnName(fieldName) + '_id'
            if (tableColumns.has(colName)) {
              // Will be filled after nested entity is created
              const nestedNounName = fieldName.charAt(0).toUpperCase() + fieldName.slice(1)
              // Inherit parent's string fields as back-relations
              const nestedFields = { ...fieldValue } as Record<string, any>
              for (const [fk, fv] of Object.entries(fields)) {
                if (typeof fv === 'string' && !(fk in nestedFields)) {
                  nestedFields[fk] = fv
                }
              }
              nestedEntities.push({ nounName: nestedNounName, fields: nestedFields, fkCol: colName })
            }
            continue
          }

          // Array of objects = child entities (e.g. messages: [{body, sentAt}, ...])
          // Create each as a separate entity with an FK back to this parent.
          if (Array.isArray(fieldValue) && fieldValue.length > 0 && typeof fieldValue[0] === 'object') {
            const singular = fieldName.replace(/s$/, '')
            const childNoun = singular.charAt(0).toUpperCase() + singular.slice(1)
            const parentFkField = nounName.charAt(0).toLowerCase() + nounName.slice(1)
            for (const childObj of fieldValue) {
              const childData = { ...(childObj as Record<string, any>), [parentFkField]: id }
              try {
                const childResult = await this.createEntityInner(domainId, childNoun, childData, undefined, createdBy, now, toColumnName, toTableName)
                childCreationResults.push({ noun: childNoun, ...childResult })
              } catch (err: any) {
                childCreationResults.push({ noun: childNoun, error: err.message })
              }
            }
            // Store child IDs as JSON array if the column exists
            const idsCol = fieldMap[fieldName] || toColumnName(fieldName)
            if (tableColumns.has(idsCol)) {
              const childIds = childCreationResults.filter((r: any) => r.id).map((r: any) => r.id)
              if (childIds.length) {
                this.sql.exec(`UPDATE ${tableName} SET ${idsCol} = ? WHERE id = ?`, JSON.stringify(childIds), id)
              }
            }
            continue
          }

          // Array of primitives = JSON column
          if (Array.isArray(fieldValue)) {
            const colName = fieldMap[fieldName] || toColumnName(fieldName)
            if (tableColumns.has(colName)) {
              cols.push(colName)
              vals.push(JSON.stringify(fieldValue))
            }
            continue
          }

          // String value — find the column
          // Try: fieldMap mapping, then snake_case, then _id suffix for FK
          const colName = fieldMap[fieldName] || toColumnName(fieldName)
          const fkColName = colName + '_id'
          if (tableColumns.has(colName)) {
            cols.push(colName)
            vals.push(fieldValue)
          } else if (tableColumns.has(fkColName)) {
            // FK reference — find or create the related entity
            cols.push(fkColName)
            vals.push(fieldValue) // For now, store the value directly (TODO: resolve to resource ID)
          }
        }

        // Insert the row
        this.sql.exec(
          `INSERT INTO ${tableName} (${cols.join(', ')}) VALUES (${cols.map(() => '?').join(', ')})`,
          ...vals,
        )
        this.logCdcEvent('create', tableName, id, { domain_id: domainId })

        // Create nested entities and update FK columns
        for (const nested of nestedEntities) {
          try {
            const nestedResult = await this.createEntityInner(domainId, nested.nounName, nested.fields, undefined, createdBy, now, toColumnName, toTableName)
            if (nestedResult.id) {
              this.sql.exec(`UPDATE ${tableName} SET ${nested.fkCol} = ? WHERE id = ?`, nestedResult.id, id)
            }
          } catch { /* nested entity creation is best-effort */ }
        }

        this.getModel(domainId).invalidate()

        // Auto-create state machine instance at initial state if the noun has a definition
        const smStatus = this.autoCreateStateMachine(domainId, nounName, id, now)

        return {
          id,
          noun: nounName,
          table: tableName,
          domain: domainId,
          reference,
          createdBy,
          ...(smStatus && { status: smStatus }),
          ...(childCreationResults.length > 0 && { children: childCreationResults }),
        }
      }

      // ── Fallback: generic resource path (no 3NF table yet) ──
      const nouns = this.sql.exec(
        'SELECT * FROM nouns WHERE name = ? AND domain_id = ? LIMIT 1', nounName, domainId,
      ).toArray()
      if (!nouns.length) throw new Error(`Noun "${nounName}" not found in domain`)

      const resourceId = crypto.randomUUID()
      this.sql.exec(
        'INSERT INTO resources (id, noun_id, domain_id, reference, value, created_by, created_at, updated_at, version) VALUES (?, ?, ?, ?, ?, ?, ?, ?, 1)',
        resourceId, nouns[0].id, domainId, reference || null, JSON.stringify(fields), createdBy || null, now, now,
      )
      this.logCdcEvent('create', 'resources', resourceId)

      // Auto-create state machine for fallback path too
      const smStatus = this.autoCreateStateMachine(domainId, nounName, resourceId, now)

      return {
        id: resourceId,
        noun: nounName,
        domain: domainId,
        reference,
        createdBy,
        ...(smStatus && { status: smStatus }),
      }
    })
  }

  /**
   * Auto-create a state machine instance at the initial state for an entity.
   * Called from within createEntity's write lock — uses direct SQL, no async.
   * Returns the initial status name or null if no state machine applies.
   */
  private autoCreateStateMachine(domainId: string, nounName: string, entityId: string, now: string): string | null {
    try {
      // Find state machine definition for this noun (by noun name match)
      const defs = this.sql.exec(
        `SELECT smd.id, smd.noun_id FROM state_machine_definitions smd
         JOIN nouns n ON smd.noun_id = n.id
         WHERE n.name = ? AND smd.domain_id = ?
         LIMIT 1`,
        nounName, domainId,
      ).toArray()
      if (!defs.length) return null

      const defId = defs[0].id as string

      // Find initial status (first status with no incoming transitions, or first created)
      const statuses = this.sql.exec(
        'SELECT id, name FROM statuses WHERE state_machine_definition_id = ? ORDER BY created_at ASC',
        defId,
      ).toArray()
      if (!statuses.length) return null

      let initialStatus = statuses[0]
      for (const s of statuses) {
        const incoming = this.sql.exec(
          'SELECT 1 FROM transitions WHERE to_status_id = ? LIMIT 1', s.id,
        ).toArray()
        if (!incoming.length) { initialStatus = s; break }
      }

      // Create state machine instance
      const smId = crypto.randomUUID()
      this.sql.exec(
        `INSERT INTO state_machines (id, name, state_machine_definition_id, current_status_id, domain_id, created_at, updated_at, version)
         VALUES (?, ?, ?, ?, ?, ?, ?, 1)`,
        smId, entityId, defId, initialStatus.id, domainId, now, now,
      )

      return initialStatus.name as string
    } catch {
      return null // best-effort — don't block entity creation
    }
  }

  /**
   * Inner createEntity — no write lock, no getEntityTable (avoids deadlock).
   * Derives table name directly and inserts. Used for child entity creation
   * from within createEntity's write lock.
   */
  private async createEntityInner(
    domainId: string,
    nounName: string,
    fields: Record<string, any>,
    reference?: string,
    createdBy?: string,
    now?: string,
    toColumnName: (s: string) => string,
    toTableName: (s: string) => string,
  ): Promise<{ id: string }> {
    const ts = now || new Date().toISOString().replace('T', ' ').replace('Z', '')

    // Read fieldMap from cached schema-map (written by applySchema, already called by parent createEntity)
    let fieldMap: Record<string, string> = {}
    try {
      const rows = this.sql.exec(
        "SELECT output FROM generators WHERE domain_id = ? AND output_format = 'schema-map' LIMIT 1", domainId,
      ).toArray()
      if (rows.length && rows[0].output) {
        const cached = JSON.parse(rows[0].output as string)
        fieldMap = cached.fieldMap?.[tableName] || {}
      }
    } catch { /* no cache yet */ }

    // FK checks already deferred by parent createEntity's PRAGMA defer_foreign_keys = ON

    // Check if table exists — if not, fall back to generic resources
    try {
      this.sql.exec(`SELECT 1 FROM ${tableName} LIMIT 0`)
    } catch {
      const id = crypto.randomUUID()
      const nouns = this.sql.exec('SELECT id FROM nouns WHERE name = ? AND domain_id = ? LIMIT 1', nounName, domainId).toArray()
      if (nouns.length) {
        this.sql.exec(
          'INSERT INTO resources (id, noun_id, domain_id, value, created_at, updated_at, version) VALUES (?, ?, ?, ?, ?, ?, 1)',
          id, nouns[0].id, domainId, JSON.stringify(fields), ts, ts,
        )
      }
      return { id }
    }

    const id = crypto.randomUUID()
    const cols = ['id', 'domain_id', 'created_at', 'updated_at', 'version']
    const vals: any[] = [id, domainId, ts, ts, 1]

    const tableColumns = new Set<string>()
    try {
      const pragma = this.sql.exec(`PRAGMA table_info(${tableName})`).toArray()
      for (const row of pragma) tableColumns.add(row.name as string)
    } catch { /* */ }

    for (const [fieldName, fieldValue] of Object.entries(fields)) {
      if (fieldValue === undefined || fieldValue === null || typeof fieldValue === 'object') continue
      const colName = fieldMap[fieldName] || toColumnName(fieldName)
      const fkColName = colName + '_id'
      if (tableColumns.has(colName)) { cols.push(colName); vals.push(fieldValue) }
      else if (tableColumns.has(fkColName)) { cols.push(fkColName); vals.push(fieldValue) }
    }

    try {
      this.sql.exec(
        `INSERT INTO ${tableName} (${cols.join(', ')}) VALUES (${cols.map(() => '?').join(', ')})`,
        ...vals,
      )
      // Verify the row actually persisted
      const verify = [...this.sql.exec(`SELECT id FROM ${tableName} WHERE id = ?`, id)]
      if (verify.length === 0) {
        throw new Error(`INSERT appeared to succeed but row not found | table=${tableName} id=${id}`)
      }
    } catch (e: any) {
      throw new Error(`${e.message} | table=${tableName} cols=[${cols.join(',')}] vals=[${vals.map(v => String(v).slice(0, 30)).join(',')}]`)
    }
    this.logCdcEvent('create', tableName, id, { domain_id: domainId })
    return { id }
  }

  /**
   * Apply generated schema to the DO's SQLite — materializes 3NF tables from readings.
   *
   * The loop: Readings → OpenAPI → SQLite DDL → CREATE TABLE IF NOT EXISTS.
   * Tables are created or extended (new columns added), never dropped.
   * Returns the table map and field map for use by createEntity.
   */
  async applySchema(domainId: string): Promise<{ tableMap: Record<string, string>; fieldMap: Record<string, Record<string, string>> }> {
    const model = this.getModel(domainId)
    model.invalidate() // Clear cache to ensure fresh data
    const openapi = await (await import('./generate/openapi')).generateOpenAPI(model)
    const { ddl, tableMap, fieldMap } = (await import('./generate/sqlite')).generateSQLite(openapi)

    // Apply DDL — CREATE TABLE IF NOT EXISTS, ALTER TABLE ADD COLUMN for new columns
    for (const statement of ddl) {
      if (statement.startsWith('CREATE TABLE')) {
        // Convert to IF NOT EXISTS
        const safe = statement.replace('CREATE TABLE', 'CREATE TABLE IF NOT EXISTS')
        try { this.sql.exec(safe) } catch { /* table exists with different schema */ }

        // Check for new columns and add them
        const tableMatch = statement.match(/CREATE TABLE (\w+)/)
        if (tableMatch) {
          const tableName = tableMatch[1]
          const existingCols = new Set<string>()
          try {
            const pragma = this.sql.exec(`PRAGMA table_info(${tableName})`).toArray()
            for (const row of pragma) existingCols.add(row.name as string)
          } catch { continue }

          // Parse columns from DDL
          const colSection = statement.match(/\(([\s\S]+)\)/)
          if (colSection) {
            for (const line of colSection[1].split(',')) {
              const colMatch = line.trim().match(/^(\w+)\s/)
              if (colMatch && !existingCols.has(colMatch[1])) {
                const colDef = line.trim()
                // Strip NOT NULL and DEFAULT for ALTER TABLE ADD COLUMN
                const safeCol = colDef.replace(/NOT NULL/g, '').replace(/DEFAULT\s+[^,)]+/g, '').trim()
                try { this.sql.exec(`ALTER TABLE ${tableName} ADD COLUMN ${safeCol}`) } catch { /* already exists */ }
              }
            }
          }
        }
      } else if (statement.startsWith('CREATE INDEX')) {
        try { this.sql.exec(statement) } catch { /* index exists */ }
      }
    }

    // Store the mapping so createEntity can use it
    // Cache in the domain model for fast access
    const cached = { tableMap, fieldMap, appliedAt: new Date().toISOString() }
    this.getModel(domainId).invalidate()

    // Persist the mapping in generators collection for external consumers
    const existing = this.sql.exec(
      "SELECT id FROM generators WHERE domain_id = ? AND output_format = 'schema-map'", domainId,
    ).toArray()
    const mapJson = JSON.stringify(cached)
    const now = new Date().toISOString().replace('T', ' ').replace('Z', '')
    if (existing.length) {
      this.sql.exec('UPDATE generators SET output = ?, updated_at = ? WHERE id = ?', mapJson, now, existing[0].id)
    } else {
      this.sql.exec(
        "INSERT INTO generators (id, domain_id, output_format, output, created_at, updated_at, version) VALUES (?, ?, 'schema-map', ?, ?, ?, 1)",
        crypto.randomUUID(), domainId, mapJson, now, now,
      )
    }

    // Auto-recompile business rules on schema change
    try {
      const rules = await (await import('./generate/business-rules')).generateBusinessRules(model)
      const rulesJson = JSON.stringify(rules)
      const rulesExisting = this.sql.exec(
        "SELECT id FROM generators WHERE domain_id = ? AND output_format = 'business-rules'", domainId,
      ).toArray()
      if (rulesExisting.length) {
        this.sql.exec('UPDATE generators SET output = ?, updated_at = ? WHERE id = ?', rulesJson, now, rulesExisting[0].id)
      } else {
        this.sql.exec(
          "INSERT INTO generators (id, domain_id, output_format, output, created_at, updated_at, version) VALUES (?, ?, 'business-rules', ?, ?, ?, 1)",
          crypto.randomUUID(), domainId, rulesJson, now, now,
        )
      }
    } catch { /* business rules compilation is best-effort */ }

    return { tableMap, fieldMap }
  }

  /**
   * Get the 3NF table name and field map for a noun in a domain.
   * Uses cached schema-map from applySchema, or runs it if missing.
   */
  private async getEntityTable(domainId: string, nounName: string): Promise<{
    tableName: string
    fieldMap: Record<string, string>
  } | null> {
    // Check for cached schema map
    const rows = this.sql.exec(
      "SELECT output FROM generators WHERE domain_id = ? AND output_format = 'schema-map' LIMIT 1", domainId,
    ).toArray()

    let schemaMap: { tableMap: Record<string, string>; fieldMap: Record<string, Record<string, string>> }

    if (rows.length && rows[0].output) {
      schemaMap = JSON.parse(rows[0].output as string)
    } else {
      // Generate and apply on first use
      schemaMap = await this.applySchema(domainId)
    }

    const { toTableName } = await import('./generate/sqlite')
    const tableName = schemaMap.tableMap[nounName] || toTableName(nounName)

    // Verify table exists
    try {
      this.sql.exec(`SELECT 1 FROM ${tableName} LIMIT 0`)
    } catch {
      // Table doesn't exist yet — apply schema and retry
      schemaMap = await this.applySchema(domainId)
      try {
        this.sql.exec(`SELECT 1 FROM ${tableName} LIMIT 0`)
      } catch {
        return null // noun doesn't produce a table
      }
    }

    return {
      tableName,
      fieldMap: schemaMap.fieldMap[tableName] || {},
    }
  }

  /**
   * Resolve array-of-ID fields in an entity doc to full objects.
   *
   * For each field that's an array of UUID strings, query the corresponding
   * entity table and replace IDs with full row objects (snake→camel converted).
   * Also resolves reverse FK relationships: finds child entity tables that have
   * a `<noun>_id` column pointing back to this entity and embeds matching rows.
   */
  async populateEntity(
    domainId: string,
    nounName: string,
    doc: Record<string, unknown>,
  ): Promise<Record<string, unknown>> {
    const { toTableName, toColumnName } = await import('./generate/sqlite')
    const populated = { ...doc }
    const uuidRe = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i

    const snakeToCamel = (row: Record<string, unknown>) => {
      const out: Record<string, unknown> = {}
      for (const [k, v] of Object.entries(row)) {
        const ck = k.replace(/_([a-z])/g, (_, c) => c.toUpperCase())
        if (typeof v === 'string' && (v.startsWith('[') || v.startsWith('{'))) {
          try { out[ck] = JSON.parse(v) } catch { out[ck] = v }
        } else {
          out[ck] = v
        }
      }
      return out
    }

    // Forward: resolve arrays of UUIDs to full entity rows
    for (const [key, val] of Object.entries(populated)) {
      if (!Array.isArray(val) || val.length === 0) continue
      if (typeof val[0] !== 'string' || !uuidRe.test(val[0])) continue

      // Derive noun from field name: "messages" → "Message"
      const singular = key.replace(/s$/, '')
      const nounGuess = singular.charAt(0).toUpperCase() + singular.slice(1)
      const tableName = toTableName(nounGuess)
      try {
        this.sql.exec(`SELECT 1 FROM ${tableName} LIMIT 0`)
        const resolved: Record<string, unknown>[] = []
        for (const id of val as string[]) {
          const rows = this.sql.exec(`SELECT * FROM ${tableName} WHERE id = ? AND domain_id = ?`, id, domainId).toArray()
          resolved.push(rows.length ? snakeToCamel(rows[0] as Record<string, unknown>) : { id })
        }
        populated[key] = resolved
      } catch { /* table doesn't exist, keep raw array */ }
    }

    // Reverse FK: find child tables with a column pointing back to this entity
    const fkCol = toColumnName(nounName) + '_id'
    const tables = this.sql.exec("SELECT name FROM sqlite_master WHERE type='table'").toArray()
    for (const { name } of tables) {
      const table = name as string
      if (table.startsWith('_') || table === toTableName(nounName)) continue
      try {
        const cols = this.sql.exec(`PRAGMA table_info(${table})`).toArray()
        if (!cols.some(c => c.name === fkCol)) continue
        const children = this.sql.exec(
          `SELECT * FROM ${table} WHERE ${fkCol} = ? AND domain_id = ? ORDER BY created_at ASC`,
          doc.id as string, domainId,
        ).toArray()
        if (children.length > 0) {
          const camelField = table.replace(/_([a-z])/g, (_, c) => c.toUpperCase())
          populated[camelField] = children.map(r => snakeToCamel(r as Record<string, unknown>))
        }
      } catch { /* skip */ }
    }

    return populated
  }

  /**
   * Update an entity in a 3NF table.
   */
  async updateEntity(domainId: string, nounName: string, id: string, fields: Record<string, any>): Promise<Record<string, unknown> | null> {
    const { toTableName, toColumnName } = await import('./generate/sqlite')
    const entityTable = await this.getEntityTable(domainId, nounName)
    const tableName = entityTable?.tableName || toTableName(nounName)
    const fieldMap = entityTable?.fieldMap || {}

    try {
      const existing = this.sql.exec(`SELECT id FROM ${tableName} WHERE id = ? AND domain_id = ?`, id, domainId).toArray()
      if (existing.length === 0) return null

      const tableColumns = new Set<string>()
      for (const row of this.sql.exec(`PRAGMA table_info(${tableName})`).toArray()) {
        tableColumns.add(row.name as string)
      }

      const sets: string[] = []
      const vals: any[] = []
      for (const [fieldName, fieldValue] of Object.entries(fields)) {
        if (fieldValue === undefined) continue
        const colName = fieldMap[fieldName] || toColumnName(fieldName)
        const fkColName = colName + '_id'
        if (tableColumns.has(colName)) { sets.push(`${colName} = ?`); vals.push(fieldValue) }
        else if (tableColumns.has(fkColName)) { sets.push(`${fkColName} = ?`); vals.push(fieldValue) }
      }

      if (sets.length === 0) return null

      sets.push("updated_at = datetime('now')")
      this.sql.exec(`UPDATE ${tableName} SET ${sets.join(', ')} WHERE id = ? AND domain_id = ?`, ...vals, id, domainId)
      this.logCdcEvent('update', tableName, id, { domain_id: domainId })

      const updated = this.sql.exec(`SELECT * FROM ${tableName} WHERE id = ?`, id).toArray()
      if (!updated.length) return null
      const doc: Record<string, unknown> = {}
      for (const [key, val] of Object.entries(updated[0] as Record<string, unknown>)) {
        doc[key.replace(/_([a-z])/g, (_, c) => c.toUpperCase())] = val
      }
      return doc
    } catch {
      return null
    }
  }

  /**
   * Delete an entity from a 3NF table.
   * Checks the entity's own table first, then walks up the supertype chain.
   * Also cascade-deletes child entities that have an FK referencing this entity.
   */
  async deleteEntity(domainId: string, nounName: string, id: string): Promise<{ deleted: boolean }> {
    const { toTableName, toColumnName } = await import('./generate/sqlite')

    // Build supertype chain: [SupportRequest, Request, ...]
    const tablesToTry = [toTableName(nounName)]
    const supertypes = this.sql.exec(
      'SELECT n2.name FROM nouns n1 JOIN nouns n2 ON n1.super_type_id = n2.id WHERE n1.name = ? AND n1.domain_id = ?',
      nounName, domainId,
    ).toArray()
    for (const st of supertypes) {
      tablesToTry.push(toTableName(st.name as string))
    }

    // Try deleting from each table in the chain
    for (const tableName of tablesToTry) {
      try {
        const existing = this.sql.exec(`SELECT id FROM ${tableName} WHERE id = ? AND domain_id = ?`, id, domainId).toArray()
        if (existing.length === 0) continue

        // Cascade: delete child entities that reference this entity via FK
        const fkCol = toColumnName(nounName) + '_id'
        const allTables = this.sql.exec("SELECT name FROM sqlite_master WHERE type='table'").toArray()
        for (const { name } of allTables) {
          const table = name as string
          if (table.startsWith('_') || table === tableName) continue
          try {
            const cols = this.sql.exec(`PRAGMA table_info(${table})`).toArray()
            if (cols.some(c => c.name === fkCol)) {
              this.sql.exec(`DELETE FROM ${table} WHERE ${fkCol} = ? AND domain_id = ?`, id, domainId)
            }
          } catch { /* skip */ }
        }

        this.sql.exec(`DELETE FROM ${tableName} WHERE id = ? AND domain_id = ?`, id, domainId)
        this.logCdcEvent('delete', tableName, id, { domain_id: domainId })
        return { deleted: true }
      } catch { /* table doesn't exist, try next */ }
    }

    return { deleted: false }
  }

  /**
   * Query a 3NF entity table. Returns rows with pagination.
   * The table name is derived from the noun name via RMAP.
   */
  async queryEntities(
    domainId: string,
    nounName: string,
    options?: { where?: Record<string, any>; sort?: string; limit?: number; page?: number },
  ): Promise<{ docs: Record<string, unknown>[]; totalDocs: number; page: number; limit: number; hasNextPage: boolean }> {
    const { toTableName, toColumnName } = await import('./generate/sqlite')
    const tableName = toTableName(nounName)
    const limit = options?.limit || 100
    const page = options?.page || 1
    const offset = (page - 1) * limit

    // Verify table exists
    try {
      this.sql.exec(`SELECT 1 FROM ${tableName} LIMIT 0`)
    } catch {
      return { docs: [], totalDocs: 0, page, limit, hasNextPage: false }
    }

    // Build WHERE clause
    let whereClause = 'WHERE domain_id = ?'
    const params: any[] = [domainId]
    const countParams: any[] = [domainId]

    if (options?.where) {
      for (const [field, condition] of Object.entries(options.where)) {
        const col = toColumnName(field)
        if (typeof condition === 'object' && condition !== null) {
          for (const [op, val] of Object.entries(condition as Record<string, any>)) {
            if (op === 'equals') { whereClause += ` AND ${col} = ?`; params.push(val); countParams.push(val) }
            else if (op === 'not_equals') { whereClause += ` AND ${col} != ?`; params.push(val); countParams.push(val) }
            else if (op === 'like') { whereClause += ` AND ${col} LIKE ?`; params.push(val); countParams.push(val) }
          }
        } else {
          whereClause += ` AND ${col} = ?`; params.push(condition); countParams.push(condition)
        }
      }
    }

    // Find subtype tables that share columns with this table (subtype UNION)
    // e.g., querying Message also returns SupportResponse (a subtype of Message)
    const subtypeNouns = this.sql.exec(
      'SELECT n.name FROM nouns n WHERE n.super_type_id IN (SELECT id FROM nouns WHERE name = ? AND domain_id = ?) AND n.domain_id = ?',
      nounName, domainId, domainId,
    ).toArray()

    const baseColumns = new Set<string>()
    try {
      for (const row of this.sql.exec(`PRAGMA table_info(${tableName})`).toArray()) {
        baseColumns.add(row.name as string)
      }
    } catch { /* table doesn't exist */ }

    const unionParts: string[] = [`SELECT *, '${nounName}' as _type FROM ${tableName} ${whereClause}`]

    for (const subNoun of subtypeNouns) {
      const subTable = toTableName(subNoun.name as string)
      try {
        this.sql.exec(`SELECT 1 FROM ${subTable} LIMIT 0`)
        // Build SELECT with matching columns — use NULL for columns only in the base table
        const subColumns = new Set<string>()
        for (const row of this.sql.exec(`PRAGMA table_info(${subTable})`).toArray()) {
          subColumns.add(row.name as string)
        }
        const selectCols = [...baseColumns].map(col => subColumns.has(col) ? col : `NULL as ${col}`)
        selectCols.push(`'${subNoun.name}' as _type`)

        // Build a WHERE clause for the subtype that only includes conditions on columns it actually has.
        // Without this, conditions like `support_request_id = ?` become `NULL = ?` and match nothing.
        let subWhereClause = 'WHERE domain_id = ?'
        const subParams: any[] = [domainId]
        if (options?.where) {
          for (const [field, condition] of Object.entries(options.where)) {
            const col = toColumnName(field)
            if (!subColumns.has(col)) continue // skip conditions on columns this subtype doesn't have
            if (typeof condition === 'object' && condition !== null) {
              for (const [op, val] of Object.entries(condition as Record<string, any>)) {
                if (op === 'equals') { subWhereClause += ` AND ${col} = ?`; subParams.push(val) }
                else if (op === 'not_equals') { subWhereClause += ` AND ${col} != ?`; subParams.push(val) }
                else if (op === 'like') { subWhereClause += ` AND ${col} LIKE ?`; subParams.push(val) }
              }
            } else {
              subWhereClause += ` AND ${col} = ?`; subParams.push(condition)
            }
          }
        }
        unionParts.push(`SELECT ${selectCols.join(', ')} FROM ${subTable} ${subWhereClause}`)
        // Track subtype params separately so they can be spliced in correctly
        ;(unionParts as any).__subParams = (unionParts as any).__subParams || []
        ;(unionParts as any).__subParams.push(subParams)
      } catch { /* subtype table doesn't exist yet */ }
    }

    // Sort
    const sortField = options?.sort?.startsWith('-') ? options.sort.slice(1) : (options?.sort || 'created_at')
    const sortDir = options?.sort?.startsWith('-') ? 'DESC' : 'ASC'

    let query: string
    let countQuery: string
    if (unionParts.length > 1) {
      const union = unionParts.join(' UNION ALL ')
      query = `SELECT * FROM (${union}) ORDER BY ${toColumnName(sortField)} ${sortDir} LIMIT ? OFFSET ?`
      countQuery = `SELECT count(*) as cnt FROM (${union})`
      // Build combined params: base table params + each subtype's own params
      const allParams: any[] = [...params]
      const allCountParams: any[] = [...countParams]
      const subParamsArr: any[][] = (unionParts as any).__subParams || []
      for (const sp of subParamsArr) {
        allParams.push(...sp)
        allCountParams.push(...sp)
      }
      allParams.push(limit, offset)
      params.length = 0; params.push(...allParams)
      countParams.length = 0; countParams.push(...allCountParams)
    } else {
      query = `SELECT * FROM ${tableName} ${whereClause} ORDER BY ${toColumnName(sortField)} ${sortDir} LIMIT ? OFFSET ?`
      countQuery = `SELECT count(*) as cnt FROM ${tableName} ${whereClause}`
      params.push(limit, offset)
    }

    const rows = this.sql.exec(query, ...params).toArray()
    const countRow = this.sql.exec(countQuery, ...countParams).toArray()
    const totalDocs = (countRow[0]?.cnt as number) ?? 0

    // Convert snake_case columns to camelCase for API consumers
    // Parse JSON-encoded arrays and objects stored in TEXT columns
    const docs = rows.map(row => {
      const doc: Record<string, unknown> = {}
      for (const [key, val] of Object.entries(row as Record<string, unknown>)) {
        const camelKey = key.replace(/_([a-z])/g, (_, c) => c.toUpperCase())
        if (typeof val === 'string' && (val.startsWith('[') || val.startsWith('{'))) {
          try { doc[camelKey] = JSON.parse(val) } catch { doc[camelKey] = val }
        } else {
          doc[camelKey] = val
        }
      }
      return doc
    })

    // Resolve derivation rules — computed properties from related entities
    // Derivations are dynamic: they query related tables at read time, no schema compilation needed
    // Resolve derivation rules — computed properties from related entities
    try {
      const derivations = this.resolveDerivations(domainId, nounName, docs, toTableName, toColumnName)
      for (const doc of docs) {
        for (const d of derivations) {
          if (doc.id) doc[d.field] = d.resolve(doc.id as string)
        }
      }
    } catch { /* derivation resolution is best-effort */ }

    // Normalize state machine status onto each entity
    try {
      for (const doc of docs) {
        if (!doc.id) continue
        const smRows = this.sql.exec(
          `SELECT s.name FROM state_machines sm
           JOIN statuses s ON sm.current_status_id = s.id
           WHERE sm.name = ? AND sm.domain_id = ?
           LIMIT 1`,
          doc.id as string, domainId,
        ).toArray()
        if (smRows.length) doc.status = smRows[0].name
      }
    } catch { /* state machine lookup is best-effort */ }

    return { docs, totalDocs, page, limit, hasNextPage: offset + limit < totalDocs }
  }

  async generate(domainId: string, format: string): Promise<any> {
    const model = this.getModel(domainId)
    model.invalidate() // Always use fresh data for generation
    switch (format) {
      case 'openapi':
        return (await import('./generate/openapi')).generateOpenAPI(model)
      case 'sqlite':
        return (await import('./generate/sqlite')).generateSQLite(
          await (await import('./generate/openapi')).generateOpenAPI(model))
      case 'xstate':
        return (await import('./generate/xstate')).generateXState(model)
      case 'ilayer':
        return (await import('./generate/ilayer')).generateILayer(model)
      case 'readings':
        return (await import('./generate/readings')).generateReadings(model)
      case 'business-rules':
        return (await import('./generate/business-rules')).generateBusinessRules(model)
      case 'mdxui':
        return (await import('./generate/mdxui')).generateMdxui(model)
      case 'readme':
        return (await import('./generate/readme')).generateReadme(model)
      case 'schema':
        return this.applySchema(domainId)
      default:
        throw new Error(`Unknown format: ${format}`)
    }
  }

  async fetch(request: Request): Promise<Response> {
    // WebSocket upgrade for live event streaming
    if (request.headers.get('Upgrade') === 'websocket') {
      const url = new URL(request.url)
      const domain = url.searchParams.get('domain') || 'all'
      const pair = new WebSocketPair()
      this.ctx.acceptWebSocket(pair[1], [domain])
      return new Response(null, { status: 101, webSocket: pair[0] })
    }

    return new Response(JSON.stringify({ status: 'ok', version: '0.1.0' }), {
      headers: { 'Content-Type': 'application/json' },
    })
  }

  async webSocketMessage(ws: WebSocket, message: string | ArrayBuffer): Promise<void> {
    // Clients can send { type: 'subscribe', domain: '...' } to change subscription
    try {
      const data = JSON.parse(typeof message === 'string' ? message : new TextDecoder().decode(message))
      if (data.type === 'ping') {
        ws.send(JSON.stringify({ type: 'pong' }))
      }
    } catch { /* ignore malformed messages */ }
  }

  async webSocketClose(ws: WebSocket): Promise<void> {
    // Cleanup handled by the runtime
  }
}

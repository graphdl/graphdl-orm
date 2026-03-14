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
    ]
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
    fields: Record<string, string | string[]>,
    reference?: string,
    createdBy?: string,
  ): Promise<Record<string, unknown>> {
    return this.withWriteLock(async () => {
      const now = new Date().toISOString().replace('T', ' ').replace('Z', '')

      // Find the noun
      const nouns = this.sql.exec(
        'SELECT * FROM nouns WHERE name = ? AND domain_id = ? LIMIT 1', nounName, domainId,
      ).toArray()
      if (!nouns.length) throw new Error(`Noun "${nounName}" not found in domain`)
      const noun = nouns[0]

      // Create the primary resource
      const resourceId = crypto.randomUUID()
      const cols = ['id', 'noun_id', 'domain_id', 'created_at', 'updated_at', 'version']
      const vals: any[] = [resourceId, noun.id, domainId, now, now, 1]
      if (reference) { cols.push('reference'); vals.push(reference) }
      if (createdBy) { cols.push('created_by'); vals.push(createdBy) }

      // If there's a field matching the noun name (self-reference), use it as value
      const lowerNoun = nounName.charAt(0).toLowerCase() + nounName.slice(1)
      const selfValue = fields[lowerNoun] || fields[nounName]
      if (selfValue) { cols.push('value'); vals.push(selfValue) }

      this.sql.exec(
        `INSERT INTO ${cols.map(() => '').join('')}resources (${cols.join(', ')}) VALUES (${cols.map(() => '?').join(', ')})`,
        ...vals,
      )
      this.logCdcEvent('create', 'resources', resourceId)

      // Find all binary fact types where this noun plays role 0
      const roles = this.sql.exec(
        `SELECT r.graph_schema_id, r.role_index, r.id as role_id, n.name as noun_name, n.id as noun_id, n.object_type
         FROM roles r
         JOIN nouns n ON r.noun_id = n.id
         JOIN graph_schemas gs ON r.graph_schema_id = gs.id
         WHERE gs.domain_id = ?
         ORDER BY r.graph_schema_id, r.role_index`,
        domainId,
      ).toArray()

      // Group roles by graph schema
      const schemaRoles = new Map<string, any[]>()
      for (const r of roles) {
        const list = schemaRoles.get(r.graph_schema_id as string) ?? []
        list.push(r)
        schemaRoles.set(r.graph_schema_id as string, list)
      }

      // For each binary fact type where this noun is role 0, check if we have a matching field
      // Singular field name = one fact. Plural field name = one fact per array element.
      const facts: Array<{ graphSchemaId: string; value: string; valueNounId: string }> = []
      for (const [schemaId, schemaRoleList] of schemaRoles) {
        if (schemaRoleList.length !== 2) continue  // only handle binary fact types
        const role0 = schemaRoleList.find((r: any) => r.role_index === 0)
        const role1 = schemaRoleList.find((r: any) => r.role_index === 1)
        if (!role0 || !role1) continue
        if (role0.noun_name !== nounName) continue  // this noun must be role 0

        // Match field name to role 1 noun name (camelCase singular or plural)
        const valueNounName = role1.noun_name as string
        const camelName = valueNounName.charAt(0).toLowerCase() + valueNounName.slice(1)
        const pluralName = camelName + 's'
        const rawValue = fields[camelName] || fields[valueNounName] || fields[pluralName] || fields[valueNounName + 's']
        if (!rawValue) continue

        // Normalize to array — plural field names expect arrays, singular expect one value
        const values = Array.isArray(rawValue) ? rawValue : [rawValue]
        for (const v of values) {
          facts.push({
            graphSchemaId: schemaId,
            value: v,
            valueNounId: role1.noun_id as string,
          })
        }
      }

      // Also check for fact types where this noun is role 1 (e.g. "Customer submits SupportRequest")
      for (const [schemaId, schemaRoleList] of schemaRoles) {
        if (schemaRoleList.length !== 2) continue
        const role0 = schemaRoleList.find((r: any) => r.role_index === 0)
        const role1 = schemaRoleList.find((r: any) => r.role_index === 1)
        if (!role0 || !role1) continue
        if (role1.noun_name !== nounName) continue  // this noun is role 1

        const subjectNounName = role0.noun_name as string
        const camelName = subjectNounName.charAt(0).toLowerCase() + subjectNounName.slice(1)
        const pluralName = camelName + 's'
        const rawValue = fields[camelName] || fields[subjectNounName] || fields[pluralName] || fields[subjectNounName + 's']
        if (!rawValue) continue

        const subjectValues = Array.isArray(rawValue) ? rawValue : [rawValue]
        for (const fieldValue of subjectValues) {
        // Create/find the subject resource and link it
        const existing = this.sql.exec(
          'SELECT id FROM resources WHERE noun_id = ? AND domain_id = ? AND value = ? LIMIT 1',
          role0.noun_id, domainId, fieldValue,
        ).toArray()

        const subjectResourceId = existing.length
          ? existing[0].id as string
          : (() => {
              const rid = crypto.randomUUID()
              this.sql.exec(
                'INSERT INTO resources (id, noun_id, domain_id, value, created_at, updated_at, version) VALUES (?, ?, ?, ?, ?, ?, 1)',
                rid, role0.noun_id, domainId, fieldValue, now, now,
              )
              this.logCdcEvent('create', 'resources', rid)
              return rid
            })()

        // Create the graph linking subject → this entity
        const graphId = crypto.randomUUID()
        this.sql.exec(
          'INSERT INTO graphs (id, graph_schema_id, domain_id, created_at, updated_at, version) VALUES (?, ?, ?, ?, ?, 1)',
          graphId, schemaId, domainId, now, now,
        )
        const rrRoles = this.sql.exec(
          'SELECT id, role_index FROM roles WHERE graph_schema_id = ? ORDER BY role_index', schemaId,
        ).toArray()
        for (const rr of rrRoles) {
          const rrId = crypto.randomUUID()
          const resId = (rr.role_index as number) === 0 ? subjectResourceId : resourceId
          this.sql.exec(
            'INSERT INTO resource_roles (id, graph_id, resource_id, role_id, domain_id, created_at, updated_at, version) VALUES (?, ?, ?, ?, ?, ?, ?, 1)',
            rrId, graphId, resId, rr.id, domainId, now, now,
          )
        }
        this.logCdcEvent('create', 'graphs', graphId)
        } // end for subjectValues
      }

      // Create graphs for the binary facts where this noun is role 0
      for (const fact of facts) {
        // Find or create the value resource
        const existing = this.sql.exec(
          'SELECT id FROM resources WHERE noun_id = ? AND domain_id = ? AND value = ? LIMIT 1',
          fact.valueNounId, domainId, fact.value,
        ).toArray()

        const valueResourceId = existing.length
          ? existing[0].id as string
          : (() => {
              const rid = crypto.randomUUID()
              this.sql.exec(
                'INSERT INTO resources (id, noun_id, domain_id, value, created_at, updated_at, version) VALUES (?, ?, ?, ?, ?, ?, 1)',
                rid, fact.valueNounId, domainId, fact.value, now, now,
              )
              this.logCdcEvent('create', 'resources', rid)
              return rid
            })()

        // Create graph
        const graphId = crypto.randomUUID()
        this.sql.exec(
          'INSERT INTO graphs (id, graph_schema_id, domain_id, created_at, updated_at, version) VALUES (?, ?, ?, ?, ?, 1)',
          graphId, fact.graphSchemaId, domainId, now, now,
        )

        // Create resource roles
        const graphRoles = this.sql.exec(
          'SELECT id, role_index FROM roles WHERE graph_schema_id = ? ORDER BY role_index', fact.graphSchemaId,
        ).toArray()
        for (const gr of graphRoles) {
          const rrId = crypto.randomUUID()
          const resId = (gr.role_index as number) === 0 ? resourceId : valueResourceId
          this.sql.exec(
            'INSERT INTO resource_roles (id, graph_id, resource_id, role_id, domain_id, created_at, updated_at, version) VALUES (?, ?, ?, ?, ?, ?, ?, 1)',
            rrId, graphId, resId, gr.id, domainId, now, now,
          )
        }
        this.logCdcEvent('create', 'graphs', graphId)
      }

      this.getModel(domainId).invalidate('graphs')

      return {
        id: resourceId,
        noun: nounName,
        domain: domainId,
        reference,
        facts: facts.length,
        createdBy,
      }
    })
  }

  async generate(domainId: string, format: string): Promise<any> {
    const model = this.getModel(domainId)
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
      case 'constraint-ir':
        return (await import('./generate/constraint-ir')).generateConstraintIR(model)
      case 'mdxui':
        return (await import('./generate/mdxui')).generateMdxui(model)
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

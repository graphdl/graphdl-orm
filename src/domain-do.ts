/**
 * DomainDB — Durable Object that holds a single domain's SQL storage.
 *
 * Actively used capabilities:
 * 1. Batch WAL (delegates to batch-wal.ts)
 * 2. Generators cache (delegates to generators-cache.ts)
 * 3. createFact / createEntity (instance fact creation, used by seed.ts)
 * 4. applySchema (generates + applies entity-instance DDL from metamodel)
 * 5. WebSocket CDC broadcast
 */

import { nounToTable } from './collections'
import { initBatchSchema, createBatch, getBatch, markCommitted, markFailed, getPendingBatches } from './batch-wal'
import type { BatchEntity, Batch } from './batch-wal'
import {
  findGenerators,
  getGenerator,
  createGenerator,
  updateGenerator,
  deleteGenerator,
  deleteGeneratorsForDomain,
} from './generators-cache'

// =========================================================================
// Types
// =========================================================================

import type { SqlLike } from './sql-like'
export type { SqlLike } from './sql-like'

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

/** Maps FK column names to their target table (used by createEntity for FK resolution). */
const FK_TARGET_TABLE: Record<string, string> = {
  app_id: 'apps',
  domain_id: 'domains',
  organization_id: 'organizations',
  super_type_id: 'nouns',
  graph_schema_id: 'graph_schemas',
  reading_id: 'readings',
  noun_id: 'nouns',
  constraint_id: 'constraints',
  role_id: 'roles',
  state_machine_definition_id: 'state_machine_definitions',
  from_status_id: 'statuses',
  to_status_id: 'statuses',
  event_type_id: 'event_types',
  verb_id: 'verbs',
  transition_id: 'transitions',
  guard_id: 'guards',
  citation_id: 'citations',
  graph_id: 'graphs',
  resource_id: 'resources',
  state_machine_id: 'state_machines',
  current_status_id: 'statuses',
  model_id: 'models',
  agent_definition_id: 'agent_definitions',
  agent_id: 'agents',
}

/** All tables to wipe during reset, in leaf-to-root dependency order. */
const WIPE_TABLES: readonly string[] = [
  'cdc_events',
  'generators', 'guard_runs', 'events', 'state_machines',
  'resource_roles', 'graph_citations', 'resources', 'graphs',
  'completions', 'agents', 'agent_definitions', 'models',
  'citations',
  'functions', 'streams', 'verbs',
  'guards', 'transitions', 'statuses', 'event_types', 'state_machine_definitions',
  'constraint_spans', 'constraints', 'roles', 'readings', 'graph_schemas',
  'nouns',
  'domains', 'apps', 'org_memberships', 'organizations',
]

// =========================================================================
// Wipe helpers
// =========================================================================

/**
 * Wipe all metamodel data for a domain (nouns, readings, constraints, etc.
 * plus generators cache).
 */
function wipeDomainMetamodelData(
  sql: SqlLike,
  domainId: string,
): { deleted: boolean; domainId: string; counts: Record<string, any> } {
  const counts: Record<string, any> = {}
  sql.exec('PRAGMA foreign_keys = OFF')

  const countAndDelete = (table: string, where: string, ...params: any[]) => {
    try {
      const before = sql.exec(`SELECT count(*) as c FROM ${table} WHERE ${where}`, ...params).toArray()[0] as any
      sql.exec(`DELETE FROM ${table} WHERE ${where}`, ...params)
      counts[table] = { before: before?.c || 0, after: 0 }
    } catch (e: any) { counts[table] = { error: e.message } }
  }

  countAndDelete('constraint_spans',
    'constraint_id IN (SELECT id FROM constraints WHERE domain_id = ?)', domainId)
  countAndDelete('roles',
    'graph_schema_id IN (SELECT id FROM graph_schemas WHERE domain_id = ?)', domainId)
  for (const table of ['constraints', 'readings', 'graph_schemas', 'nouns']) {
    countAndDelete(table, 'domain_id = ?', domainId)
  }

  sql.exec('PRAGMA foreign_keys = ON')
  deleteGeneratorsForDomain(sql, domainId)
  return { deleted: true, domainId, counts }
}

/**
 * Wipe all data from all tables (for testing/reset).
 */
function wipeAllMetamodelData(sql: SqlLike): void {
  try { sql.exec('PRAGMA foreign_keys = OFF') } catch { /* best effort */ }
  for (const table of WIPE_TABLES) {
    try { sql.exec(`DELETE FROM ${table}`) } catch { /* table may not exist yet */ }
  }
  try { sql.exec('PRAGMA foreign_keys = ON') } catch { /* best effort */ }
}

// =========================================================================
// Durable Object class
// =========================================================================

import { DurableObject } from 'cloudflare:workers'
import { ALL_DDL } from './schema'
import { DomainModel, SqlDataLoader } from './model/domain-model'

/**
 * DomainDB — Durable Object that holds a single domain's SQL storage.
 *
 * Stores generators cache (compiled outputs), entity-instance tables
 * (via applySchema), and delegates batch WAL to batch-wal.ts.
 */
export class DomainDB extends DurableObject {
  private initialized = false
  private domainId: string | null = null
  private sql!: SqlStorage
  private models: Map<string, DomainModel> = new Map()
  private _writeTail: Promise<void> = Promise.resolve()

  private ensureInit(): void {
    if (this.initialized) return
    this.sql = this.ctx.storage.sql
    this.sql.exec('PRAGMA defer_foreign_keys = OFF')
    // Run ALL bootstrap DDL so entity tables, CDC, etc. are available
    for (const ddl of ALL_DDL) {
      this.sql.exec(ddl)
    }
    // CDC events table
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
    initBatchSchema(this.ctx.storage.sql)
    this.initialized = true
  }

  getModel(domainId: string): DomainModel {
    let model = this.models.get(domainId)
    if (!model) {
      model = new DomainModel(new SqlDataLoader(this.sql as any), domainId)
      this.models.set(domainId, model)
    }
    return model
  }

  private withWriteLock<T>(fn: () => Promise<T>): Promise<T> {
    const result = this._writeTail.then(fn, fn)
    this._writeTail = result.then(() => {}, () => {})
    return result
  }

  private logCdcEvent(operation: string, tableName: string, entityId: string, data?: Record<string, unknown>): void {
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
        const domainId = data?.domain_id as string | undefined
        if (!domainId || tags.includes('all') || tags.includes(domainId)) {
          ws.send(event)
        }
      } catch { /* client disconnected */ }
    }
  }

  /** Set the domain ID for this DO. Called once after creation. */
  async setDomainId(id: string): Promise<void> {
    this.ensureInit()
    this.domainId = id
  }

  // -----------------------------------------------------------------------
  // Batch WAL methods (delegate to batch-wal.ts)
  // -----------------------------------------------------------------------

  async commitBatch(entities: BatchEntity[]): Promise<Batch> {
    this.ensureInit()
    const domain = this.domainId
    if (!domain) throw new Error('DomainDB: domainId required for commitBatch (call setDomainId first)')
    return createBatch(this.ctx.storage.sql, domain, entities)
  }

  async getBatch(id: string): Promise<Batch | null> {
    this.ensureInit()
    return getBatch(this.ctx.storage.sql, id)
  }

  async markBatchCommitted(id: string): Promise<void> {
    this.ensureInit()
    markCommitted(this.ctx.storage.sql, id)
  }

  async markBatchFailed(id: string, error: string): Promise<void> {
    this.ensureInit()
    markFailed(this.ctx.storage.sql, id, error)
  }

  async getPendingBatches(): Promise<Batch[]> {
    this.ensureInit()
    return getPendingBatches(this.ctx.storage.sql)
  }

  // -----------------------------------------------------------------------
  // Generators cache (delegate to generators-cache.ts)
  // -----------------------------------------------------------------------

  /** Find generator records. Only the generators collection uses this. */
  async findInCollection(
    _collection: string,
    where?: Record<string, any>,
    opts?: { limit?: number; page?: number; sort?: string },
  ): Promise<ReturnType<typeof findGenerators>> {
    this.ensureInit()
    return findGenerators(this.ctx.storage.sql, where, opts)
  }

  /** Get a single generator record by ID. */
  async getFromCollection(_collection: string, id: string): Promise<Record<string, unknown> | null> {
    this.ensureInit()
    return getGenerator(this.ctx.storage.sql, id)
  }

  /** Create a generator record. */
  async createInCollection(
    _collection: string,
    data: Record<string, unknown>,
  ): Promise<Record<string, unknown>> {
    this.ensureInit()
    return this.withWriteLock(async () => {
      const doc = createGenerator(this.ctx.storage.sql, data)
      this.logCdcEvent('create', 'generators', doc.id as string, doc)
      const domainId = doc.domain_id as string || doc.domain as string
      if (domainId) this.getModel(domainId).invalidate('generators')
      return doc
    })
  }

  /** Update a generator record. */
  async updateInCollection(
    _collection: string,
    id: string,
    updates: Record<string, unknown>,
  ): Promise<Record<string, unknown> | null> {
    this.ensureInit()
    return this.withWriteLock(async () => {
      const doc = updateGenerator(this.ctx.storage.sql, id, updates)
      if (doc) {
        this.logCdcEvent('update', 'generators', id, doc)
        const domainId = doc.domain_id as string || doc.domain as string
        if (domainId) this.getModel(domainId).invalidate('generators')
      }
      return doc
    })
  }

  /** Delete a generator record by ID. */
  async deleteFromCollection(_collection: string, id: string): Promise<{ deleted: boolean }> {
    this.ensureInit()
    return this.withWriteLock(async () => {
      const result = deleteGenerator(this.ctx.storage.sql, id)
      if (result.deleted) this.logCdcEvent('delete', 'generators', id)
      return result
    })
  }

  /** Wipe all metamodel data for a domain. */
  wipeDomainMetamodel(domainId: string): { deleted: boolean; domainId: string; counts: Record<string, any> } {
    this.ensureInit()
    const result = wipeDomainMetamodelData(this.ctx.storage.sql, domainId)
    this.getModel(domainId).invalidate()
    return result
  }

  /** Wipe all data from all tables. */
  async wipeAllData(): Promise<void> {
    this.ensureInit()
    return this.withWriteLock(async () => {
      wipeAllMetamodelData(this.ctx.storage.sql)
    })
  }

  // -----------------------------------------------------------------------
  // Schema application + code generation
  // -----------------------------------------------------------------------

  /** Generate and apply the entity-instance schema from this domain's metamodel. */
  async applySchema(domainId?: string): Promise<{ tableMap: Record<string, string>; fieldMap: Record<string, Record<string, string>> }> {
    this.ensureInit()
    const id = domainId || this.domainId
    if (!id) throw new Error('DomainDB: domainId required for applySchema')
    const model = this.getModel(id)
    model.invalidate()
    const openapi = await (await import('./generate/openapi')).generateOpenAPI(model)
    const { ddl, tableMap, fieldMap } = (await import('./generate/sqlite')).generateSQLite(openapi)

    // Apply DDL
    for (const statement of ddl) {
      if (statement.startsWith('CREATE TABLE')) {
        const safe = statement.replace('CREATE TABLE', 'CREATE TABLE IF NOT EXISTS')
        try { this.sql.exec(safe) } catch { /* table exists with different schema */ }
        const tableMatch = statement.match(/CREATE TABLE (\w+)/)
        if (tableMatch) {
          const tableName = tableMatch[1]
          const existingCols = new Set<string>()
          try {
            const pragma = this.sql.exec(`PRAGMA table_info(${tableName})`).toArray()
            for (const row of pragma) existingCols.add(row.name as string)
          } catch { continue }
          const colSection = statement.match(/\(([\s\S]+)\)/)
          if (colSection) {
            for (const line of colSection[1].split(',')) {
              const colMatch = line.trim().match(/^(\w+)\s/)
              if (colMatch && !existingCols.has(colMatch[1])) {
                const colDef = line.trim()
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

    // Cache the mapping in generators table
    const cached = { tableMap, fieldMap, appliedAt: new Date().toISOString() }
    model.invalidate()
    const existing = this.sql.exec(
      "SELECT id FROM generators WHERE domain_id = ? AND output_format = 'schema-map'", id,
    ).toArray()
    const mapJson = JSON.stringify(cached)
    const now = new Date().toISOString().replace('T', ' ').replace('Z', '')
    if (existing.length) {
      this.sql.exec('UPDATE generators SET output = ?, updated_at = ? WHERE id = ?', mapJson, now, existing[0].id)
    } else {
      this.sql.exec(
        "INSERT INTO generators (id, domain_id, output_format, output, created_at, updated_at, version) VALUES (?, ?, 'schema-map', ?, ?, ?, 1)",
        crypto.randomUUID(), id, mapJson, now, now,
      )
    }

    // Auto-recompile domain schema
    try {
      const rules = await (await import('./generate/schema')).generateSchema(model)
      const rulesJson = JSON.stringify(rules)
      const rulesExisting = this.sql.exec(
        "SELECT id FROM generators WHERE domain_id = ? AND output_format = 'schema'", id,
      ).toArray()
      if (rulesExisting.length) {
        this.sql.exec('UPDATE generators SET output = ?, updated_at = ? WHERE id = ?', rulesJson, now, rulesExisting[0].id)
      } else {
        this.sql.exec(
          "INSERT INTO generators (id, domain_id, output_format, output, created_at, updated_at, version) VALUES (?, ?, 'schema', ?, ?, ?, 1)",
          crypto.randomUUID(), id, rulesJson, now, now,
        )
      }
    } catch { /* domain schema compilation is best-effort */ }

    return { tableMap, fieldMap }
  }

  // -----------------------------------------------------------------------
  // Fact + entity instance creation (used by seed.ts and claims pipeline)
  // -----------------------------------------------------------------------

  /** Create a fact instance: a Graph linking Resources through Roles. */
  async createFact(
    domainId: string,
    graphSchemaId: string,
    bindings: Array<{ nounId: string; value?: string; resourceId?: string }>,
  ): Promise<Record<string, unknown>> {
    this.ensureInit()
    return this.withWriteLock(async () => {
      const now = new Date().toISOString().replace('T', ' ').replace('Z', '')

      const resolvedResources: string[] = []
      for (const binding of bindings) {
        if (binding.resourceId) {
          resolvedResources.push(binding.resourceId)
          continue
        }
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

      const graphId = crypto.randomUUID()
      this.sql.exec(
        'INSERT INTO graphs (id, graph_schema_id, domain_id, created_at, updated_at, version) VALUES (?, ?, ?, ?, ?, 1)',
        graphId, graphSchemaId, domainId, now, now,
      )
      this.logCdcEvent('create', 'graphs', graphId)

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

  /** Read entity table info from cached schema-map. */
  private getEntityTableFromCache(
    domainId: string,
    nounName: string,
    toTableName: (s: string) => string,
  ): { tableName: string; fieldMap: Record<string, string> } | null {
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

  /** Get the 3NF table name and field map for a noun in a domain. */
  private async getEntityTable(domainId: string, nounName: string): Promise<{
    tableName: string; fieldMap: Record<string, string>
  } | null> {
    const rows = this.sql.exec(
      "SELECT output FROM generators WHERE domain_id = ? AND output_format = 'schema-map' LIMIT 1", domainId,
    ).toArray()
    let schemaMap: { tableMap: Record<string, string>; fieldMap: Record<string, Record<string, string>> }
    if (rows.length && rows[0].output) {
      schemaMap = JSON.parse(rows[0].output as string)
    } else {
      schemaMap = await this.applySchema(domainId)
    }
    const { toTableName } = await import('./generate/sqlite')
    const tableName = schemaMap.tableMap[nounName] || toTableName(nounName)
    try {
      this.sql.exec(`SELECT 1 FROM ${tableName} LIMIT 0`)
    } catch {
      schemaMap = await this.applySchema(domainId)
      try { this.sql.exec(`SELECT 1 FROM ${tableName} LIMIT 0`) } catch { return null }
    }
    return { tableName, fieldMap: schemaMap.fieldMap[tableName] || {} }
  }

  /** Auto-create a state machine instance at the initial state for an entity. */
  private autoCreateStateMachine(domainId: string, nounName: string, entityId: string, now: string): string | null {
    try {
      const defs = this.sql.exec(
        `SELECT smd.id, smd.noun_id FROM state_machine_definitions smd
         JOIN nouns n ON smd.noun_id = n.id
         WHERE n.name = ? AND smd.domain_id = ?
         LIMIT 1`,
        nounName, domainId,
      ).toArray()
      if (!defs.length) return null
      const defId = defs[0].id as string
      const statuses = this.sql.exec(
        'SELECT id, name FROM statuses WHERE state_machine_definition_id = ? ORDER BY created_at ASC',
        defId,
      ).toArray()
      if (!statuses.length) return null
      let initialStatus = statuses[0]
      for (const s of statuses) {
        const incoming = this.sql.exec('SELECT 1 FROM transitions WHERE to_status_id = ? LIMIT 1', s.id).toArray()
        if (!incoming.length) { initialStatus = s; break }
      }
      const smId = crypto.randomUUID()
      this.sql.exec(
        `INSERT INTO state_machines (id, name, state_machine_definition_id, current_status_id, domain_id, created_at, updated_at, version)
         VALUES (?, ?, ?, ?, ?, ?, ?, 1)`,
        smId, entityId, defId, initialStatus.id, domainId, now, now,
      )
      return initialStatus.name as string
    } catch {
      return null
    }
  }

  /** Inner createEntity — no write lock, no getEntityTable. Used for child entities. */
  private async createEntityInner(
    domainId: string, nounName: string, fields: Record<string, any>,
    reference?: string, createdBy?: string, now?: string,
    toColumnName?: (s: string) => string, toTableName?: (s: string) => string,
  ): Promise<{ id: string }> {
    if (!toColumnName || !toTableName) {
      const sqlite = await import('./generate/sqlite')
      toColumnName = toColumnName || sqlite.toColumnName
      toTableName = toTableName || sqlite.toTableName
    }
    const tableName = toTableName(nounName)
    const ts = now || new Date().toISOString().replace('T', ' ').replace('Z', '')
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
    try { this.sql.exec(`SELECT 1 FROM ${tableName} LIMIT 0`) } catch {
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
      const verify = this.sql.exec(`SELECT id FROM ${tableName} WHERE id = ?`, id).toArray()
      if (verify.length === 0) throw new Error(`INSERT appeared to succeed but row not found`)
    } catch (e: any) {
      throw new Error(`${e.message} | table=${tableName}`)
    }
    this.logCdcEvent('create', tableName, id, { domain_id: domainId })
    return { id }
  }

  /** Create an entity instance with fields — the high-level write operation. */
  async createEntity(
    domainId: string,
    nounName: string,
    fields: Record<string, string | string[] | Record<string, string | string[]>>,
    reference?: string,
    createdBy?: string,
  ): Promise<Record<string, unknown>> {
    this.ensureInit()
    const { toColumnName, toTableName } = await import('./generate/sqlite')

    // Pre-warm child entity tables
    for (const [fieldName, fieldValue] of Object.entries(fields)) {
      if (Array.isArray(fieldValue) && fieldValue.length > 0 && typeof fieldValue[0] === 'object') {
        const singular = fieldName.replace(/s$/, '')
        const childNoun = singular.charAt(0).toUpperCase() + singular.slice(1)
        await this.getEntityTable(domainId, childNoun)
      }
    }
    await this.getEntityTable(domainId, nounName)

    return this.withWriteLock(async () => {
      this.sql.exec('PRAGMA defer_foreign_keys = OFF')
      const now = new Date().toISOString().replace('T', ' ').replace('Z', '')
      const entityTable = this.getEntityTableFromCache(domainId, nounName, toTableName)

      if (entityTable) {
        const { tableName, fieldMap } = entityTable
        const tableColumns = new Set<string>()
        try {
          const pragma = this.sql.exec(`PRAGMA table_info(${tableName})`).toArray()
          for (const row of pragma) tableColumns.add(row.name as string)
        } catch { /* table doesn't exist */ }

        const refCol = tableColumns.has('reference') ? 'reference'
          : tableColumns.has('slug') ? 'slug'
          : tableColumns.has('domain_slug') ? 'domain_slug'
          : tableColumns.has('name') ? 'name'
          : null

        if (reference && refCol) {
          let existing = this.sql.exec(
            `SELECT id FROM ${tableName} WHERE ${refCol} = ? AND domain_id = ? LIMIT 1`,
            reference, domainId,
          ).toArray()
          if (!existing.length) {
            existing = this.sql.exec(
              `SELECT id FROM ${tableName} WHERE ${refCol} = ? LIMIT 1`,
              reference,
            ).toArray()
          }
          if (existing.length) {
            const existingId = existing[0].id as string
            const updates: string[] = [`updated_at = ?`]
            const updateVals: any[] = [now]
            for (const [fieldName, fieldValue] of Object.entries(fields)) {
              if (fieldValue === undefined || fieldValue === null) continue
              if (typeof fieldValue !== 'string' && typeof fieldValue !== 'number' && typeof fieldValue !== 'boolean') continue
              const colName = fieldMap[fieldName] || toColumnName(fieldName)
              if (tableColumns.has(colName)) { updates.push(`${colName} = ?`); updateVals.push(fieldValue) }
            }
            if (updates.length > 1) {
              this.sql.exec(`UPDATE ${tableName} SET ${updates.join(', ')} WHERE id = ?`, ...updateVals, existingId)
              this.logCdcEvent('update', tableName, existingId, { domain_id: domainId })
            }
            this.getModel(domainId).invalidate()
            return { id: existingId, noun: nounName, table: tableName, domain: domainId, reference, updated: true }
          }
        }

        const id = crypto.randomUUID()
        const cols = ['id', 'domain_id', 'created_at', 'updated_at', 'version']
        const vals: any[] = [id, domainId, now, now, 1]
        if (reference && refCol) { cols.push(refCol); vals.push(reference) }
        if (createdBy && tableColumns.has('created_by')) { cols.push('created_by'); vals.push(createdBy) }

        const nestedEntities: Array<{ nounName: string; fields: Record<string, any>; fkCol: string }> = []
        const childCreationResults: any[] = []

        for (const [fieldName, fieldValue] of Object.entries(fields)) {
          if (fieldValue === undefined || fieldValue === null) continue
          if (typeof fieldValue === 'object' && !Array.isArray(fieldValue)) {
            const colName = fieldMap[fieldName] || toColumnName(fieldName) + '_id'
            if (tableColumns.has(colName)) {
              const nestedNounName = fieldName.charAt(0).toUpperCase() + fieldName.slice(1)
              const nestedFields = { ...fieldValue } as Record<string, any>
              for (const [fk, fv] of Object.entries(fields)) {
                if (typeof fv === 'string' && !(fk in nestedFields)) nestedFields[fk] = fv
              }
              nestedEntities.push({ nounName: nestedNounName, fields: nestedFields, fkCol: colName })
            }
            continue
          }
          if (Array.isArray(fieldValue) && fieldValue.length > 0 && typeof fieldValue[0] === 'object') {
            const singular = fieldName.replace(/s$/, '')
            const childNoun = singular.charAt(0).toUpperCase() + singular.slice(1)
            const parentFkField = nounName.charAt(0).toLowerCase() + nounName.slice(1)
            for (const childObj of fieldValue) {
              const childData = { ...(childObj as Record<string, any>), [parentFkField]: id }
              try {
                const childResult = await this.createEntityInner(domainId, childNoun, childData, undefined, createdBy, now, toColumnName, toTableName)
                childCreationResults.push({ noun: childNoun, ...childResult })
              } catch (err: any) { childCreationResults.push({ noun: childNoun, error: err.message }) }
            }
            const idsCol = fieldMap[fieldName] || toColumnName(fieldName)
            if (tableColumns.has(idsCol)) {
              const childIds = childCreationResults.filter((r: any) => r.id).map((r: any) => r.id)
              if (childIds.length) this.sql.exec(`UPDATE ${tableName} SET ${idsCol} = ? WHERE id = ?`, JSON.stringify(childIds), id)
            }
            continue
          }
          if (Array.isArray(fieldValue)) {
            const colName = fieldMap[fieldName] || toColumnName(fieldName)
            if (tableColumns.has(colName)) { cols.push(colName); vals.push(JSON.stringify(fieldValue)) }
            continue
          }
          const colName = fieldMap[fieldName] || toColumnName(fieldName)
          const fkColName = colName + '_id'
          if (tableColumns.has(colName)) { cols.push(colName); vals.push(fieldValue) }
          else if (tableColumns.has(fkColName)) {
            const targetTable = FK_TARGET_TABLE[fkColName]
            if (targetTable && typeof fieldValue === 'string') {
              const refCols = ['slug', 'domain_slug', 'name', 'reference']
              let resolved: string | null = null
              for (const rc of refCols) {
                try {
                  const rows = this.sql.exec(`SELECT id FROM ${targetTable} WHERE ${rc} = ? LIMIT 1`, fieldValue).toArray()
                  if (rows.length) { resolved = rows[0].id as string; break }
                } catch { /* column may not exist */ }
              }
              cols.push(fkColName); vals.push(resolved || fieldValue)
            } else { cols.push(fkColName); vals.push(fieldValue) }
          }
        }

        this.sql.exec(
          `INSERT INTO ${tableName} (${cols.join(', ')}) VALUES (${cols.map(() => '?').join(', ')})`,
          ...vals,
        )
        this.logCdcEvent('create', tableName, id, { domain_id: domainId })

        for (const nested of nestedEntities) {
          try {
            const nestedResult = await this.createEntityInner(domainId, nested.nounName, nested.fields, undefined, createdBy, now, toColumnName, toTableName)
            if (nestedResult.id) this.sql.exec(`UPDATE ${tableName} SET ${nested.fkCol} = ? WHERE id = ?`, nestedResult.id, id)
          } catch { /* nested entity creation is best-effort */ }
        }

        this.getModel(domainId).invalidate()
        const smStatus = this.autoCreateStateMachine(domainId, nounName, id, now)

        return {
          id, noun: nounName, table: tableName, domain: domainId, reference, createdBy,
          ...(smStatus && { status: smStatus }),
          ...(childCreationResults.length > 0 && { children: childCreationResults }),
        }
      }

      // Fallback: generic resource path
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
      const smStatus = this.autoCreateStateMachine(domainId, nounName, resourceId, now)

      return {
        id: resourceId, noun: nounName, domain: domainId, reference, createdBy,
        ...(smStatus && { status: smStatus }),
      }
    })
  }

  // -----------------------------------------------------------------------
  // WebSocket (CDC broadcast)
  // -----------------------------------------------------------------------

  async fetch(request: Request): Promise<Response> {
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
    try {
      const data = JSON.parse(typeof message === 'string' ? message : new TextDecoder().decode(message))
      if (data.type === 'ping') ws.send(JSON.stringify({ type: 'pong' }))
    } catch { /* ignore malformed messages */ }
  }

  async webSocketClose(_ws: WebSocket): Promise<void> {
    // Cleanup handled by the runtime
  }
}

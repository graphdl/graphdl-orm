/**
 * DomainDB — Durable Object that holds a single domain's metamodel
 * and collection CRUD. Type-level data (nouns, readings, constraints, etc.).
 *
 * The Payload CMS field-map layer (FIELD_MAP, FK_TARGET_TABLE, etc.) is
 * retained only for the generators collection and the generate/createFact
 * pipelines. All other collection CRUD has migrated to entity-type DOs.
 */

import { COLLECTION_TABLE_MAP, NOUN_TABLE_MAP } from './collections'
import { initBatchSchema, createBatch, getBatch, markCommitted, markFailed, getPendingBatches } from './batch-wal'
import type { BatchEntity, Batch } from './batch-wal'

// =========================================================================
// Legacy Payload field maps (private — inlined from deleted collections.ts)
// =========================================================================

/** Column mapping per table. Maps Payload field names to SQLite column names. */
const FIELD_MAP: Record<string, Record<string, string>> = {
  nouns: { domain: 'domain_id', superType: 'super_type_id', objectType: 'object_type', promptText: 'prompt_text', enumValues: 'enum_values', valueType: 'value_type', worldAssumption: 'world_assumption', referenceScheme: 'reference_scheme' },
  graph_schemas: { domain: 'domain_id' },
  readings: { domain: 'domain_id', graphSchema: 'graph_schema_id' },
  roles: { reading: 'reading_id', noun: 'noun_id', graphSchema: 'graph_schema_id', roleIndex: 'role_index' },
  constraints: { domain: 'domain_id', setComparisonArgumentLength: 'set_comparison_argument_length' },
  constraint_spans: { constraint: 'constraint_id', role: 'role_id', subsetAutofill: 'subset_autofill' },
  apps: { organization: 'organization_id', appType: 'app_type', chatEndpoint: 'chat_endpoint' },
  domains: { domainSlug: 'domain_slug', organization: 'organization_id', app: 'app_id' },
  organizations: {},
  org_memberships: { organization: 'organization_id', userEmail: 'user_email' },
  state_machine_definitions: { domain: 'domain_id', noun: 'noun_id' },
  statuses: { stateMachineDefinition: 'state_machine_definition_id', domain: 'domain_id' },
  transitions: { from: 'from_status_id', to: 'to_status_id', eventType: 'event_type_id', verb: 'verb_id', stateMachineDefinition: 'state_machine_definition_id', domain: 'domain_id' },
  guards: { transition: 'transition_id', graphSchema: 'graph_schema_id', domain: 'domain_id' },
  event_types: { domain: 'domain_id' },
  verbs: { status: 'status_id', transition: 'transition_id', graph: 'graph_id', agentDefinition: 'agent_definition_id', domain: 'domain_id' },
  functions: { callbackUrl: 'callback_url', httpMethod: 'http_method', headers: 'headers', verb: 'verb_id', domain: 'domain_id' },
  streams: { domain: 'domain_id' },
  models: {},
  agent_definitions: { model: 'model_id', domain: 'domain_id' },
  agents: { agentDefinition: 'agent_definition_id', resource: 'resource_id', domain: 'domain_id' },
  completions: { agent: 'agent_id', inputText: 'input_text', outputText: 'output_text', occurredAt: 'occurred_at', domain: 'domain_id' },
  citations: { domain: 'domain_id', retrievalDate: 'retrieval_date' },
  graph_citations: { graph: 'graph_id', citation: 'citation_id', domain: 'domain_id' },
  graphs: { graphSchema: 'graph_schema_id', domain: 'domain_id', isDone: 'is_done' },
  resources: { noun: 'noun_id', domain: 'domain_id', createdBy: 'created_by' },
  resource_roles: { graph: 'graph_id', resource: 'resource_id', role: 'role_id', domain: 'domain_id' },
  state_machines: { stateMachineDefinition: 'state_machine_definition_id', stateMachineType: 'state_machine_definition_id', currentStatus: 'current_status_id', stateMachineStatus: 'current_status_id', resource: 'resource_id', domain: 'domain_id' },
  events: { eventType: 'event_type_id', stateMachine: 'state_machine_id', graph: 'graph_id', occurredAt: 'occurred_at', domain: 'domain_id' },
  generators: { domain: 'domain_id', outputFormat: 'output_format', versionNum: 'version_num' },
  guard_runs: { guard: 'guard_id', graph: 'graph_id', domain: 'domain_id' },
}

/** Maps FK column names to their target table. */
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

/**
 * Find records in a metamodel collection.
 *
 * Port of GraphDLDB.findInCollection for metamodel tables.
 */
function findInMetamodel(
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
function createInMetamodel(
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
function updateInMetamodel(
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

/**
 * Get a single record by ID from a metamodel collection.
 */
function getFromMetamodel(
  sql: SqlLike,
  collection: string,
  id: string,
): Record<string, unknown> | null {
  const table = resolveTable(collection)
  const fieldMap = getFieldMap(table)
  const reverseMap = reverseFieldMap(fieldMap)

  const rows = sql.exec(`SELECT * FROM ${table} WHERE id = ?`, id).toArray()
  if (rows.length === 0) return null

  return rowToPayload(rows[0] as Record<string, unknown>, reverseMap)
}

/**
 * FK dependency map: parent_table → [(child_table, fk_column)].
 * Used by deleteFromMetamodel to cascade-delete children before the parent.
 */
const CASCADE_MAP: Record<string, Array<[string, string]>> = {
  readings:                    [['roles', 'reading_id']],
  graph_schemas:               [['roles', 'graph_schema_id'], ['readings', 'graph_schema_id']],
  nouns:                       [['roles', 'noun_id']],
  constraints:                 [['constraint_spans', 'constraint_id']],
  roles:                       [['constraint_spans', 'role_id']],
  state_machine_definitions:   [['statuses', 'state_machine_definition_id'], ['transitions', 'state_machine_definition_id'], ['state_machines', 'state_machine_definition_id']],
  statuses:                    [['transitions', 'from_status_id'], ['transitions', 'to_status_id'], ['state_machines', 'current_status_id']],
  event_types:                 [['transitions', 'event_type_id']],
  transitions:                 [['guards', 'transition_id']],
  verbs:                       [['functions', 'verb_id']],
}

/**
 * Cascade-delete children of a record, recursively.
 */
function cascadeDeleteChildren(sql: SqlLike, table: string, id: string): number {
  let cascaded = 0
  const children = CASCADE_MAP[table]
  if (!children) return cascaded

  for (const [childTable, fkCol] of children) {
    try {
      const childRows = sql.exec(`SELECT id FROM ${childTable} WHERE ${fkCol} = ?`, id).toArray()
      for (const row of childRows) {
        cascaded += cascadeDeleteChildren(sql, childTable, (row as any).id as string)
        sql.exec(`DELETE FROM ${childTable} WHERE id = ?`, (row as any).id as string)
        cascaded++
      }
    } catch { /* table may not exist */ }
  }
  return cascaded
}

/**
 * Cascade-delete all children of a domain.
 */
function cascadeDeleteDomain(sql: SqlLike, domainId: string): number {
  let cascaded = 0
  try { sql.exec('PRAGMA foreign_keys = OFF') } catch { /* best effort */ }

  const domainScopedTables = [
    'guard_runs', 'events', 'state_machines', 'resource_roles', 'resources', 'graphs',
    'completions', 'agents', 'agent_definitions',
    'functions', 'streams', 'verbs', 'guards', 'transitions', 'statuses', 'event_types', 'state_machine_definitions',
    'constraint_spans', 'constraints', 'roles', 'readings', 'graph_schemas', 'nouns',
  ]
  for (const child of domainScopedTables) {
    try {
      if (child === 'constraint_spans') {
        sql.exec(
          `DELETE FROM constraint_spans WHERE constraint_id IN (SELECT id FROM constraints WHERE domain_id = ?)`, domainId
        )
      } else if (child === 'roles') {
        sql.exec(
          `DELETE FROM roles WHERE reading_id IN (SELECT id FROM readings WHERE domain_id = ?) OR noun_id IN (SELECT id FROM nouns WHERE domain_id = ?)`, domainId, domainId
        )
      } else if (child === 'transitions') {
        sql.exec(
          `DELETE FROM transitions WHERE from_status_id IN (SELECT id FROM statuses WHERE state_machine_definition_id IN (SELECT id FROM state_machine_definitions WHERE domain_id = ?))`, domainId
        )
      } else if (child === 'statuses') {
        sql.exec(
          `DELETE FROM statuses WHERE state_machine_definition_id IN (SELECT id FROM state_machine_definitions WHERE domain_id = ?)`, domainId
        )
      } else {
        sql.exec(`DELETE FROM ${child} WHERE domain_id = ?`, domainId)
      }
      cascaded++
    } catch { /* table may not exist yet */ }
  }

  try { sql.exec('PRAGMA foreign_keys = ON') } catch { /* best effort */ }
  return cascaded
}

/**
 * Delete a record by ID from a metamodel collection, with cascade.
 */
function deleteFromMetamodel(
  sql: SqlLike,
  collection: string,
  id: string,
): { deleted: boolean; cascaded?: number } {
  const table = resolveTable(collection)

  const existing = sql.exec(`SELECT * FROM ${table} WHERE id = ?`, id).toArray()
  if (existing.length === 0) return { deleted: false }

  let cascaded = 0

  if (table === 'domains') {
    cascaded = cascadeDeleteDomain(sql, id)
  } else if (table === 'apps') {
    const appDomains = sql.exec(`SELECT id FROM domains WHERE app_id = ?`, id).toArray()
    for (const domain of appDomains) {
      cascaded += cascadeDeleteDomain(sql, (domain as any).id as string)
      sql.exec(`DELETE FROM domains WHERE id = ?`, (domain as any).id as string)
      cascaded++
    }
  } else {
    cascaded = cascadeDeleteChildren(sql, table, id)
  }

  sql.exec(`DELETE FROM ${table} WHERE id = ?`, id)
  return { deleted: true, ...(cascaded > 0 && { cascaded }) }
}

/**
 * Inspect a table's schema (debug helper).
 */
function inspectTableSchema(
  sql: SqlLike,
  table: string,
): Record<string, any> {
  try {
    const columns = sql.exec(`PRAGMA table_info(${table})`).toArray().map((r: any) => ({
      name: r.name, type: r.type, notnull: r.notnull, dflt_value: r.dflt_value, pk: r.pk,
    }))
    const fks = sql.exec(`PRAGMA foreign_key_list(${table})`).toArray().map((r: any) => ({
      id: r.id, seq: r.seq, table: r.table, from: r.from, to: r.to,
    }))
    const foreignKeysOn = sql.exec('PRAGMA foreign_keys').toArray()[0]
    const ddl = sql.exec(`SELECT sql FROM sqlite_master WHERE type='table' AND name='${table}'`).toArray()
    return { table, columns, foreignKeys: fks, foreignKeysPragma: foreignKeysOn, ddl: (ddl[0] as any)?.sql || null }
  } catch (e: any) {
    return { table, error: e.message }
  }
}

/**
 * Wipe all metamodel data for a domain.
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
  try { sql.exec(`DELETE FROM generators WHERE domain_id = ?`, domainId) } catch { /* */ }
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
 * DomainDB — Durable Object that holds a single domain's metamodel and
 * all collection + entity operations previously in GraphDLDB (do.ts).
 *
 * Each DO instance stores type-level data (nouns, readings, constraints,
 * etc.) for one domain, and can generate + apply the entity-instance
 * schema from that metamodel.
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
    // Run ALL bootstrap DDL (not just metamodel) so entity tables, CDC, etc. are available
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
    // Batch WAL table — coexists with metamodel tables, will replace them in Task 18
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
  // Batch WAL methods
  // -----------------------------------------------------------------------

  /** Create a new batch in the WAL with pending status. */
  async commitBatch(entities: BatchEntity[]): Promise<Batch> {
    this.ensureInit()
    const domain = this.domainId
    if (!domain) throw new Error('DomainDB: domainId required for commitBatch (call setDomainId first)')
    return createBatch(this.ctx.storage.sql, domain, entities)
  }

  /** Retrieve a batch by ID. */
  async getBatch(id: string): Promise<Batch | null> {
    this.ensureInit()
    return getBatch(this.ctx.storage.sql, id)
  }

  /** Mark a batch as committed. */
  async markBatchCommitted(id: string): Promise<void> {
    this.ensureInit()
    markCommitted(this.ctx.storage.sql, id)
  }

  /** Mark a batch as failed with an error message. */
  async markBatchFailed(id: string, error: string): Promise<void> {
    this.ensureInit()
    markFailed(this.ctx.storage.sql, id, error)
  }

  /** Return all pending batches ordered by creation time (oldest first). */
  async getPendingBatches(): Promise<Batch[]> {
    this.ensureInit()
    return getPendingBatches(this.ctx.storage.sql)
  }

  /** Find records in a metamodel collection. */
  async findInCollection(
    collection: string,
    where?: Record<string, any>,
    opts?: { limit?: number; page?: number; sort?: string },
  ): Promise<ReturnType<typeof findInMetamodel>> {
    this.ensureInit()
    return findInMetamodel(this.ctx.storage.sql, collection, where, opts)
  }

  /** Get a single record by ID. */
  async getFromCollection(collectionSlug: string, id: string): Promise<Record<string, unknown> | null> {
    this.ensureInit()
    return getFromMetamodel(this.ctx.storage.sql, collectionSlug, id)
  }

  /** Create a record in a metamodel collection. */
  async createInCollection(
    collection: string,
    data: Record<string, unknown>,
  ): Promise<Record<string, unknown>> {
    this.ensureInit()
    return this.withWriteLock(async () => {
      const doc = createInMetamodel(this.ctx.storage.sql, collection, data)
      const table = COLLECTION_TABLE_MAP[collection]
      if (table) {
        this.logCdcEvent('create', table, doc.id as string, doc)
        const domainId = doc.domain_id as string || doc.domain as string
        if (domainId) this.getModel(domainId).invalidate(collection)
      }
      return doc
    })
  }

  /** Update a record in a metamodel collection. */
  async updateInCollection(
    collection: string,
    id: string,
    updates: Record<string, unknown>,
  ): Promise<Record<string, unknown> | null> {
    this.ensureInit()
    return this.withWriteLock(async () => {
      const doc = updateInMetamodel(this.ctx.storage.sql, collection, id, updates)
      if (doc) {
        const table = COLLECTION_TABLE_MAP[collection]
        if (table) {
          this.logCdcEvent('update', table, id, doc)
          const domainId = doc.domain_id as string || doc.domain as string
          if (domainId) this.getModel(domainId).invalidate(collection)
        }
      }
      return doc
    })
  }

  /** Delete a record by ID with cascade. */
  async deleteFromCollection(collectionSlug: string, id: string): Promise<{ deleted: boolean; cascaded?: number }> {
    this.ensureInit()
    return this.withWriteLock(async () => {
      const result = deleteFromMetamodel(this.ctx.storage.sql, collectionSlug, id)
      if (result.deleted) {
        const table = COLLECTION_TABLE_MAP[collectionSlug]
        if (table) this.logCdcEvent('delete', table, id)
      }
      return result
    })
  }

  /** Debug: inspect table schema. */
  inspectTable(table: string): Record<string, any> {
    this.ensureInit()
    return inspectTableSchema(this.ctx.storage.sql, table)
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

    // Cache the mapping
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
    const tableName = NOUN_TABLE_MAP[nounName] || schemaMap.tableMap[nounName] || toTableName(nounName)
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

  /** Query a 3NF entity table. Returns rows with pagination. */
  async queryEntities(
    domainId: string,
    nounName: string,
    options?: { where?: Record<string, any>; sort?: string; limit?: number; page?: number },
  ): Promise<{ docs: Record<string, unknown>[]; totalDocs: number; page: number; limit: number; hasNextPage: boolean }> {
    this.ensureInit()
    const { toTableName, toColumnName } = await import('./generate/sqlite')
    const tableName = NOUN_TABLE_MAP[nounName] || toTableName(nounName)
    const limit = options?.limit || 100
    const page = options?.page || 1
    const offset = (page - 1) * limit

    try { this.sql.exec(`SELECT 1 FROM ${tableName} LIMIT 0`) } catch {
      return { docs: [], totalDocs: 0, page, limit, hasNextPage: false }
    }

    let whereClause = 'WHERE 1=1'
    const params: any[] = []
    const countParams: any[] = []

    const localCount = this.sql.exec(
      `SELECT count(*) as c FROM ${tableName} WHERE domain_id = ?`, domainId,
    ).toArray()
    if ((localCount[0]?.c as number) > 0) {
      whereClause = 'WHERE domain_id = ?'
      params.push(domainId)
      countParams.push(domainId)
    }

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

    const sortField = options?.sort?.startsWith('-') ? options.sort.slice(1) : (options?.sort || 'created_at')
    const sortDir = options?.sort?.startsWith('-') ? 'DESC' : 'ASC'

    const query = `SELECT * FROM ${tableName} ${whereClause} ORDER BY ${toColumnName(sortField)} ${sortDir} LIMIT ? OFFSET ?`
    const countQuery = `SELECT count(*) as cnt FROM ${tableName} ${whereClause}`
    params.push(limit, offset)

    const rows = this.sql.exec(query, ...params).toArray()
    const countRow = this.sql.exec(countQuery, ...countParams).toArray()
    const totalDocs = (countRow[0]?.cnt as number) ?? 0

    const docs = rows.map(row => {
      const doc: Record<string, unknown> = {}
      for (const [key, val] of Object.entries(row as Record<string, unknown>)) {
        const camelKey = key.replace(/_([a-z])/g, (_, c) => c.toUpperCase())
        if (typeof val === 'string' && (val.startsWith('[') || val.startsWith('{'))) {
          try { doc[camelKey] = JSON.parse(val) } catch { doc[camelKey] = val }
        } else { doc[camelKey] = val }
      }
      return doc
    })

    return { docs, totalDocs, page, limit, hasNextPage: offset + limit < totalDocs }
  }

  /** Generate output for a domain in the given format. */
  async generate(domainId: string, format: string): Promise<any> {
    this.ensureInit()
    const model = this.getModel(domainId)
    model.invalidate()
    switch (format) {
      case 'openapi': return (await import('./generate/openapi')).generateOpenAPI(model)
      case 'sqlite': return (await import('./generate/sqlite')).generateSQLite(
        await (await import('./generate/openapi')).generateOpenAPI(model))
      case 'xstate': return (await import('./generate/xstate')).generateXState(model)
      case 'ilayer': return (await import('./generate/ilayer')).generateILayer(model)
      case 'readings': return (await import('./generate/readings')).generateReadings(model)
      case 'schema': return (await import('./generate/schema')).generateSchema(model)
      case 'mdxui': return (await import('./generate/mdxui')).generateMdxui(model)
      case 'readme': return (await import('./generate/readme')).generateReadme(model)
      case 'json-schema': return this.applySchema(domainId)
      default: throw new Error(`Unknown format: ${format}`)
    }
  }

  /** Handle HTTP requests (WebSocket upgrade). */
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

/**
 * generateSQLite — Transforms the OpenAPI intermediate representation from
 * generateOpenAPI() into SQLite CREATE TABLE DDL.
 *
 * This is a NEW output format (not in the original Generator.ts). It replaces
 * hand-written schema files in `src/schema/*.ts` with auto-generated DDL.
 */

// ---------------------------------------------------------------------------
// Name conversion helpers
// ---------------------------------------------------------------------------

/** PascalCase → snake_case plural (table name). */
export function toTableName(name: string): string {
  return (
    name
      .replace(/([A-Z])/g, '_$1')
      .toLowerCase()
      .replace(/^_/, '') + 's'
  )
}

/** camelCase → snake_case (column name). */
export function toColumnName(name: string): string {
  return name
    .replace(/([A-Z])/g, '_$1')
    .toLowerCase()
    .replace(/^_/, '')
}

// ---------------------------------------------------------------------------
// Type mapping
// ---------------------------------------------------------------------------

const JSON_SCHEMA_TO_SQLITE: Record<string, string> = {
  string: 'TEXT',
  integer: 'INTEGER',
  number: 'REAL',
  boolean: 'INTEGER',
  array: 'TEXT', // stored as JSON
  object: 'TEXT', // stored as JSON
}

function sqliteType(jsonSchemaType: string | undefined): string {
  return JSON_SCHEMA_TO_SQLITE[jsonSchemaType || ''] || 'TEXT'
}

// ---------------------------------------------------------------------------
// FK detection
// ---------------------------------------------------------------------------

/**
 * If a property is an entity reference, return the referenced entity name.
 * Detects two patterns:
 *   1. `{ oneOf: [... { $ref: '#/components/schemas/Foo' } ...] }`
 *   2. `{ $ref: '#/components/schemas/Foo' }`
 */
function extractRef(prop: any): string | null {
  if (prop.$ref) {
    return prop.$ref.split('/').pop() || null
  }
  if (Array.isArray(prop.oneOf)) {
    for (const entry of prop.oneOf) {
      if (entry.$ref) {
        return entry.$ref.split('/').pop() || null
      }
    }
  }
  return null
}

// ---------------------------------------------------------------------------
// System columns present on every table
// ---------------------------------------------------------------------------

const SYSTEM_COLUMNS = [
  'id TEXT PRIMARY KEY',
  'domain_id TEXT REFERENCES domains(id)',
  "created_at TEXT NOT NULL DEFAULT (datetime('now'))",
  "updated_at TEXT NOT NULL DEFAULT (datetime('now'))",
  'version INTEGER NOT NULL DEFAULT 1',
]

// ---------------------------------------------------------------------------
// generateSQLite
// ---------------------------------------------------------------------------

export interface GenerateSQLiteResult {
  /** Array of DDL statements (CREATE TABLE, CREATE INDEX). */
  ddl: string[]
  /** Entity name → table name mapping. */
  tableMap: Record<string, string>
  /** Table name → { payloadFieldName → sqlColumnName } (only entries where they differ). */
  fieldMap: Record<string, Record<string, string>>
}

/**
 * Convert an OpenAPI-style schema object (from `generateOpenAPI()`) into
 * SQLite CREATE TABLE DDL.
 */
export function generateSQLite(openapi: any): GenerateSQLiteResult {
  const schemas: Record<string, any> = openapi?.components?.schemas || {}
  const ddl: string[] = []
  const tableMap: Record<string, string> = {}
  const fieldMap: Record<string, Record<string, string>> = {}

  // Step 1: Find entity schemas — keys starting with `Update` that have
  //         type: 'object', where the base name also exists as a schema.
  const entityNames: string[] = []
  for (const key of Object.keys(schemas)) {
    if (!key.startsWith('Update')) continue
    const schema = schemas[key]
    if (schema.type !== 'object') continue
    const baseName = key.slice('Update'.length)
    if (!schemas[baseName]) continue
    entityNames.push(baseName)
  }

  // Step 2: Generate DDL for each entity
  for (const entityName of entityNames) {
    const updateSchema = schemas[`Update${entityName}`]
    const tableName = toTableName(entityName)
    tableMap[entityName] = tableName

    const columns: string[] = [...SYSTEM_COLUMNS]
    const fkColumns: string[] = ['domain_id'] // always index domain_id
    const fieldDiffs: Record<string, string> = {}

    const properties: Record<string, any> = updateSchema.properties || {}

    for (const [propName, propDef] of Object.entries(properties) as [string, any][]) {
      const refEntity = extractRef(propDef)

      if (refEntity) {
        // FK column
        const colName = toColumnName(propName) + '_id'
        const targetTable = toTableName(refEntity)
        columns.push(`${colName} TEXT REFERENCES ${targetTable}(id)`)
        fkColumns.push(colName)
        // Track fieldMap diff
        if (propName !== colName) {
          fieldDiffs[propName] = colName
        }
      } else {
        // Value column
        const colName = toColumnName(propName)
        const colType = sqliteType(propDef.type)
        columns.push(`${colName} ${colType}`)
        // Track fieldMap diff only when names differ
        if (propName !== colName) {
          fieldDiffs[propName] = colName
        }
      }
    }

    // CREATE TABLE
    ddl.push(`CREATE TABLE ${tableName} (\n  ${columns.join(',\n  ')}\n)`)

    // CREATE INDEX for each FK column
    for (const fkCol of fkColumns) {
      ddl.push(
        `CREATE INDEX IF NOT EXISTS idx_${tableName}_${fkCol} ON ${tableName}(${fkCol})`,
      )
    }

    // Only record fieldMap if there are diffs
    if (Object.keys(fieldDiffs).length > 0) {
      fieldMap[tableName] = fieldDiffs
    }
  }

  return { ddl, tableMap, fieldMap }
}

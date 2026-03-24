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

import { NOUN_TABLE_MAP } from '../collections'

/** Noun name → SQL table name. Checks metamodel mapping first, then generates snake_case plural. */
export function toTableName(name: string): string {
  // Check metamodel table mapping (handles irregular plurals like Status → statuses)
  if (NOUN_TABLE_MAP[name]) return NOUN_TABLE_MAP[name]

  // Handle spaced names: "Support Request" → "support_requests"
  if (name.includes(' ')) {
    return name.toLowerCase().replace(/\s+/g, '_') + 's'
  }

  // PascalCase → snake_case plural
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

/**
 * If a property is an array with items referencing an entity, return the
 * referenced entity name. This indicates a M:N (many-to-many) relationship
 * that needs a junction table.
 *
 * Detects patterns like:
 *   `{ type: 'array', items: { $ref: '...' } }`
 *   `{ type: 'array', items: { oneOf: [... { $ref: '...' } ...] } }`
 */
function extractArrayRef(prop: any): string | null {
  if (prop.type !== 'array' || !prop.items) return null
  return extractRef(prop.items)
}

/**
 * Generate a junction table name from two entity names. Names are sorted
 * alphabetically to ensure deterministic naming regardless of which entity
 * the property was declared on.
 */
function toJunctionTableName(entityA: string, entityB: string): string {
  const [first, second] = [toTableName(entityA), toTableName(entityB)].sort()
  return `${first}_${second}`
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

  // Collect entity names into a set for FK validation
  const entityNameSet = new Set(entityNames)

  // Track junction tables to generate after entity tables
  // Each entry: { ownerEntity, propName, targetEntity }
  const junctionDefs: { ownerEntity: string; propName: string; targetEntity: string }[] = []

  // Step 2: Generate DDL for each entity
  for (const entityName of entityNames) {
    const updateSchema = schemas[`Update${entityName}`]
    const tableName = toTableName(entityName)
    tableMap[entityName] = tableName

    const columns: string[] = [...SYSTEM_COLUMNS]
    const fkColumns: string[] = ['domain_id'] // always index domain_id
    const fieldDiffs: Record<string, string> = {}

    const properties: Record<string, any> = updateSchema.properties || {}

    // Per Halpin Ch 10 / RMAP: MC constraints → NOT NULL columns.
    // The required array lives on the NewX schema (set by fact-processors via setTableProperty).
    const newSchema = schemas[`New${entityName}`]
    const requiredFields = new Set<string>(newSchema?.required || [])

    for (const [propName, propDef] of Object.entries(properties) as [string, any][]) {
      // Check for M:N array reference BEFORE checking for regular refs
      const arrayRefEntity = extractArrayRef(propDef)
      if (arrayRefEntity && entityNameSet.has(arrayRefEntity)) {
        // M:N relationship — defer junction table generation
        junctionDefs.push({ ownerEntity: entityName, propName, targetEntity: arrayRefEntity })
        continue // Do NOT add a column to this table
      }

      const refEntity = extractRef(propDef)

      const notNull = requiredFields.has(propName) ? ' NOT NULL' : ''

      if (refEntity && entityNameSet.has(refEntity)) {
        // FK column — only if the referenced entity has its own table
        const colName = toColumnName(propName) + '_id'
        const targetTable = toTableName(refEntity)
        columns.push(`${colName} TEXT${notNull} REFERENCES ${targetTable}(id)`)
        fkColumns.push(colName)
        // Track fieldMap diff
        if (propName !== colName) {
          fieldDiffs[propName] = colName
        }
      } else if (refEntity) {
        // Reference to a value type / non-table entity — store as plain TEXT
        const colName = toColumnName(propName)
        columns.push(`${colName} TEXT${notNull}`)
        if (propName !== colName) {
          fieldDiffs[propName] = colName
        }
      } else {
        // Value column
        const colName = toColumnName(propName)
        const colType = sqliteType(propDef.type)
        columns.push(`${colName} ${colType}${notNull}`)
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

  // Step 3: Generate junction tables for M:N relationships
  // De-duplicate by the sorted entity pair to avoid creating the same junction
  // table twice (e.g., Role→Reading and Reading→Role).
  const generatedJunctions = new Set<string>()

  for (const { ownerEntity, targetEntity } of junctionDefs) {
    const junctionTable = toJunctionTableName(ownerEntity, targetEntity)

    if (generatedJunctions.has(junctionTable)) continue
    generatedJunctions.add(junctionTable)

    const ownerTable = toTableName(ownerEntity)
    const targetTable = toTableName(targetEntity)

    // Build FK column names from the entity names (snake_case + _id)
    const ownerCol = toColumnName(ownerEntity) + '_id'
    const targetCol = toColumnName(targetEntity) + '_id'

    const junctionColumns = [
      'id TEXT PRIMARY KEY',
      `${ownerCol} TEXT NOT NULL REFERENCES ${ownerTable}(id)`,
      `${targetCol} TEXT NOT NULL REFERENCES ${targetTable}(id)`,
      'domain_id TEXT REFERENCES domains(id)',
      "created_at TEXT NOT NULL DEFAULT (datetime('now'))",
      "updated_at TEXT NOT NULL DEFAULT (datetime('now'))",
      'version INTEGER NOT NULL DEFAULT 1',
      `UNIQUE(${ownerCol}, ${targetCol})`,
    ]

    ddl.push(
      `CREATE TABLE ${junctionTable} (\n  ${junctionColumns.join(',\n  ')}\n)`,
    )

    // Indexes on each FK column and domain_id
    ddl.push(
      `CREATE INDEX IF NOT EXISTS idx_${junctionTable}_${ownerCol} ON ${junctionTable}(${ownerCol})`,
    )
    ddl.push(
      `CREATE INDEX IF NOT EXISTS idx_${junctionTable}_${targetCol} ON ${junctionTable}(${targetCol})`,
    )
    ddl.push(
      `CREATE INDEX IF NOT EXISTS idx_${junctionTable}_domain_id ON ${junctionTable}(domain_id)`,
    )

    // Register junction table in tableMap using a composite key
    const junctionKey = `${ownerEntity}_${targetEntity}`
    tableMap[junctionKey] = junctionTable
  }

  return { ddl, tableMap, fieldMap }
}

/**
 * Generate CREATE VIEW statements for derived fact types.
 *
 * Takes the DomainSchema (which has derivation rules) and the table map
 * from generateSQLite, and produces SQL views for derived fact types.
 *
 * Per Halpin p.426: derived fact types map to views or computed columns.
 * Simple comparison derivations (X has Y := X has Z 'value') produce
 * a view that filters the base table.
 */
export function generateDerivedViews(
  domainSchema: {
    derivationRules: Array<{
      id: string
      text: string
      kind: string
      antecedentFactTypeIds: string[]
      consequentFactTypeId: string
    }>
    factTypes: Record<string, { reading: string; roles: Array<{ nounName: string; roleIndex: number }> }>
  },
  tableMap: Record<string, string>,
): string[] {
  const views: string[] = []

  for (const rule of domainSchema.derivationRules) {
    if (rule.kind !== 'subtypeInheritance' && rule.kind !== 'closedWorldNegation') {
      // For modus ponens rules: create a view joining antecedent tables
      if (rule.kind === 'modusPonens' && rule.antecedentFactTypeIds.length > 0) {
        const antFt = domainSchema.factTypes[rule.antecedentFactTypeIds[0]]
        const consFt = rule.consequentFactTypeId ? domainSchema.factTypes[rule.consequentFactTypeId] : null
        if (antFt && consFt) {
          const antSubject = antFt.roles[0]?.nounName
          const consSubject = consFt.roles[0]?.nounName
          if (antSubject && consSubject) {
            const antTable = tableMap[antSubject]
            const consTable = tableMap[consSubject]
            if (antTable && consTable) {
              const viewName = `derived_${toColumnName(rule.id)}`
              views.push(
                `CREATE VIEW IF NOT EXISTS ${viewName} AS SELECT a.* FROM ${antTable} a WHERE EXISTS (SELECT 1 FROM ${consTable} c WHERE c.id = a.id)`
              )
            }
          }
        }
      }
    }
  }

  return views
}

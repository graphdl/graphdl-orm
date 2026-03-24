/**
 * RMAP — Relational Mapping Procedure (Halpin, Ch. 10)
 *
 * Pure function: takes a validated ORM2 conceptual schema IR and produces
 * an array of relational table definitions.
 *
 * Steps implemented:
 *   1. Compound UC → separate table (M:N, ternary+)
 *   2. Functional roles → grouped into entity table
 *   6. Constraint mapping (UC → keys, MC → NOT NULL)
 *
 * Steps 0 (preprocessing), 3 (1:1 absorption), 4 (independent entity),
 * and 5 (composite identifiers) are stubs for Task 11.
 */

// ---------------------------------------------------------------------------
// Schema IR types
// ---------------------------------------------------------------------------

export interface RmapSchemaIR {
  nouns: Array<{ name: string; objectType: string; refScheme?: string }>
  factTypes: Array<{
    id: string
    reading: string
    roles: Array<{ nounName: string; roleIndex: number }>
  }>
  constraints: Array<{
    kind: string
    factTypeId: string
    roles: number[]
    modality?: string
  }>
  subtypes?: Array<{ subtype: string; supertype: string }>
}

// ---------------------------------------------------------------------------
// Output types
// ---------------------------------------------------------------------------

export interface TableColumn {
  name: string
  type: string       // TEXT, INTEGER, REAL, etc.
  nullable: boolean
  references?: string // FK target table
}

export interface TableDef {
  name: string
  columns: TableColumn[]
  primaryKey: string[]
  checks?: string[]
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/** Convert "Person" → "person", "API Product" → "api_product" */
function toSnake(name: string): string {
  return name
    .replace(/([a-z])([A-Z])/g, '$1_$2')
    .replace(/[\s-]+/g, '_')
    .toLowerCase()
}

/** Column name for a noun used as a foreign-key reference */
function fkColumnName(nounName: string): string {
  return toSnake(nounName) + '_id'
}

/** Column name for a value-type noun (plain snake_case, no _id suffix) */
function valueColumnName(nounName: string): string {
  return toSnake(nounName)
}

function lookupNoun(schema: RmapSchemaIR, name: string) {
  return schema.nouns.find(n => n.name === name)
}

/**
 * Determine the column name for the "target" role of a functional fact type.
 * Entity targets get `_id` suffix; value targets use plain name.
 */
function columnNameForTarget(schema: RmapSchemaIR, nounName: string): string {
  const noun = lookupNoun(schema, nounName)
  if (noun && noun.objectType === 'value') {
    return valueColumnName(nounName)
  }
  return fkColumnName(nounName)
}

// ---------------------------------------------------------------------------
// RMAP core
// ---------------------------------------------------------------------------

export function rmap(schema: RmapSchemaIR): TableDef[] {
  const tables: TableDef[] = []

  // Index UCs by factTypeId for quick lookup
  const ucsByFt = new Map<string, Array<{ roles: number[] }>>()
  for (const c of schema.constraints) {
    if (c.kind === 'UC') {
      let list = ucsByFt.get(c.factTypeId)
      if (!list) { list = []; ucsByFt.set(c.factTypeId, list) }
      list.push({ roles: c.roles })
    }
  }

  // Index MCs by factTypeId + role for quick lookup
  const mcSet = new Set<string>()
  for (const c of schema.constraints) {
    if (c.kind === 'MC') {
      for (const r of c.roles) {
        mcSet.add(`${c.factTypeId}:${r}`)
      }
    }
  }

  // Classify each fact type by its UC pattern
  // "compound" = UC spans all roles → separate table (Step 1)
  // "functional" = UC on a single role → absorbed into source entity (Step 2)
  const compoundFacts: typeof schema.factTypes = []
  const functionalFacts: typeof schema.factTypes = []

  for (const ft of schema.factTypes) {
    const ucs = ucsByFt.get(ft.id) ?? []
    let isCompound = false
    let isFunctional = false
    for (const uc of ucs) {
      if (uc.roles.length >= 2) {
        isCompound = true
      } else if (uc.roles.length === 1) {
        isFunctional = true
      }
    }
    if (isCompound) compoundFacts.push(ft)
    if (isFunctional) functionalFacts.push(ft)
  }

  // -----------------------------------------------------------------------
  // Step 1: Compound UC → separate table
  // -----------------------------------------------------------------------
  for (const ft of compoundFacts) {
    const ucs = ucsByFt.get(ft.id)!
    // Find the spanning UC (the one covering most roles)
    const spanningUC = ucs.reduce((a, b) => a.roles.length >= b.roles.length ? a : b)

    const columns: TableColumn[] = []
    const pkCols: string[] = []
    for (const role of ft.roles) {
      const colName = columnNameForTarget(schema, role.nounName)
      const noun = lookupNoun(schema, role.nounName)
      const isEntity = noun?.objectType === 'entity'
      columns.push({
        name: colName,
        type: 'TEXT',
        nullable: false,
        ...(isEntity ? { references: toSnake(role.nounName) } : {}),
      })
      if (spanningUC.roles.includes(role.roleIndex)) {
        pkCols.push(colName)
      }
    }

    // Table name derived from reading verb or concatenated role names
    const tableName = toSnake(ft.roles.map(r => r.nounName).join('_'))
    tables.push({ name: tableName, columns, primaryKey: pkCols })
  }

  // -----------------------------------------------------------------------
  // Step 2: Functional roles → grouped into entity table
  // -----------------------------------------------------------------------
  // Collect all functional facts grouped by the source entity (the UC-bearing role)
  const entityColumns = new Map<string, { columns: TableColumn[]; mandatoryFacts: Set<string> }>()

  for (const ft of functionalFacts) {
    const ucs = ucsByFt.get(ft.id)!
    for (const uc of ucs) {
      if (uc.roles.length !== 1) continue
      const sourceRoleIdx = uc.roles[0]
      const sourceRole = ft.roles.find(r => r.roleIndex === sourceRoleIdx)
      if (!sourceRole) continue
      const sourceNoun = lookupNoun(schema, sourceRole.nounName)
      if (!sourceNoun || sourceNoun.objectType !== 'entity') continue

      const entityKey = sourceNoun.name
      let entry = entityColumns.get(entityKey)
      if (!entry) {
        entry = { columns: [], mandatoryFacts: new Set() }
        entityColumns.set(entityKey, entry)
      }

      // The target role(s) become columns
      for (const role of ft.roles) {
        if (role.roleIndex === sourceRoleIdx) continue
        const colName = columnNameForTarget(schema, role.nounName)
        // Check if the source role is mandatory for this fact type
        const isMandatory = mcSet.has(`${ft.id}:${sourceRoleIdx}`)
        const noun = lookupNoun(schema, role.nounName)
        const isEntity = noun?.objectType === 'entity'
        entry.columns.push({
          name: colName,
          type: 'TEXT',
          nullable: !isMandatory,
          ...(isEntity ? { references: toSnake(role.nounName) } : {}),
        })
        if (isMandatory) {
          entry.mandatoryFacts.add(ft.id)
        }
      }
    }
  }

  // Emit one table per entity that has functional columns
  for (const [entityName, entry] of entityColumns) {
    const tableName = toSnake(entityName)
    const noun = lookupNoun(schema, entityName)!
    const idCol: TableColumn = {
      name: 'id',
      type: 'TEXT',
      nullable: false,
    }
    tables.push({
      name: tableName,
      columns: [idCol, ...entry.columns],
      primaryKey: ['id'],
    })
  }

  // -----------------------------------------------------------------------
  // Step 4: Independent entity → single-column table (stub)
  // Any entity noun that has no functional or compound facts still needs a
  // table if referenced. For now, only emit if it appears as FK target but
  // has no own table yet.
  // -----------------------------------------------------------------------
  // (deferred to Task 11)

  return tables
}

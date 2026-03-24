/**
 * RMAP — Relational Mapping Procedure (Halpin, Ch. 10)
 *
 * Pure function: takes a validated ORM2 conceptual schema IR and produces
 * an array of relational table definitions.
 *
 * Steps implemented:
 *   0.1. Binarize exclusive unaries (XO → status column)
 *   0.3. Subtype absorption (default: absorb into root supertype)
 *   1.   Compound UC → separate table (M:N, ternary+)
 *   2.   Functional roles → grouped into entity table
 *   3.   1:1 absorption (absorb toward mandatory side)
 *   4.   Independent entity → single-column table
 *   6.   Constraint mapping (UC → keys, MC → NOT NULL, VC → CHECK, SS → FK)
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
    /** XO constraint: group of fact type IDs that are mutually exclusive unaries */
    xoGroup?: string[]
    /** VC constraint: allowed values */
    values?: string[]
    /** SS constraint: target fact type for subset */
    targetFactTypeId?: string
    /** SS constraint: target roles */
    targetRoles?: number[]
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

/**
 * Derive a table name for a compound (M:N / ternary) fact type.
 * Tries to extract a verb from the reading for disambiguation;
 * falls back to concatenated role names.
 */
function compoundTableName(ft: { reading: string; roles: Array<{ nounName: string }> }, nounNames: Set<string>): string {
  // Extract words from reading that are NOT noun names → the verb(s)
  const words = ft.reading.split(/\s+/)
  const verbs = words.filter(w => !nounNames.has(w))
  if (verbs.length > 0) {
    // Use role names joined with the first verb: "person_teaches_course"
    const parts: string[] = []
    for (const w of words) {
      parts.push(toSnake(w))
    }
    return parts.join('_')
  }
  // Fallback: concatenated role names
  return toSnake(ft.roles.map(r => r.nounName).join('_'))
}

/**
 * Derive a column name for an XO group of unary predicates.
 * "male"/"female" → "sex", "active"/"inactive" → "status"
 * Falls back to "status" if no heuristic matches.
 */
function deriveXoColumnName(values: string[]): string {
  const lower = values.map(v => v.toLowerCase())
  if (lower.includes('male') && lower.includes('female')) return 'sex'
  return 'status'
}

// ---------------------------------------------------------------------------
// RMAP core
// ---------------------------------------------------------------------------

export function rmap(schema: RmapSchemaIR): TableDef[] {
  const tables: TableDef[] = []

  // -----------------------------------------------------------------------
  // Step 0.1: Binarize exclusive unaries
  // XO constraint over unary fact types → synthetic status column
  // -----------------------------------------------------------------------
  const xoConstraints = schema.constraints.filter(c => c.kind === 'XO' && c.xoGroup)
  // Track which fact types have been binarized so we skip them later
  const binarizedFtIds = new Set<string>()
  // Map: entityName → Array<{ columnName, values, isMandatory }>
  const xoColumns = new Map<string, Array<{
    columnName: string
    values: string[]
    nullable: boolean
  }>>()

  for (const xo of xoConstraints) {
    const group = xo.xoGroup!
    // All fact types in the group must be unary on the same entity
    const unaryFts = group
      .map(ftId => schema.factTypes.find(ft => ft.id === ftId))
      .filter((ft): ft is NonNullable<typeof ft> => ft != null && ft.roles.length === 1)

    if (unaryFts.length < 2) continue

    const entityName = unaryFts[0].roles[0].nounName
    // Extract status values from readings: "Person is male" → "male"
    const values: string[] = []
    for (const ft of unaryFts) {
      const reading = ft.reading
      // Extract the predicate part after "is " or the last word
      const match = reading.match(/\bis\s+(\w+)$/i)
      if (match) {
        values.push(match[1])
      } else {
        // Fallback: last word of reading
        const words = reading.split(/\s+/)
        values.push(words[words.length - 1])
      }
      binarizedFtIds.add(ft.id)
    }

    // Determine column name: find a common semantic label
    // Use the XO group label if available, otherwise derive from values
    const columnName = deriveXoColumnName(values)

    // Check if mandatory: any MC on the entity role of any unary in the group
    const isMandatory = unaryFts.some(ft =>
      schema.constraints.some(c =>
        c.kind === 'MC' && c.factTypeId === ft.id && c.roles.includes(0)
      )
    )

    let list = xoColumns.get(entityName)
    if (!list) { list = []; xoColumns.set(entityName, list) }
    list.push({ columnName, values, nullable: !isMandatory })
  }

  // -----------------------------------------------------------------------
  // Step 0.3: Subtype absorption (default)
  // Build root-supertype map; redirect subtype entity → root supertype
  // -----------------------------------------------------------------------
  const subtypeToRoot = new Map<string, string>()
  if (schema.subtypes && schema.subtypes.length > 0) {
    // Build parent map
    const parentOf = new Map<string, string>()
    for (const st of schema.subtypes) {
      parentOf.set(st.subtype, st.supertype)
    }
    // Resolve each subtype to its root
    const resolveRoot = (name: string): string => {
      const visited = new Set<string>()
      let current = name
      while (parentOf.has(current) && !visited.has(current)) {
        visited.add(current)
        current = parentOf.get(current)!
      }
      return current
    }
    for (const st of schema.subtypes) {
      subtypeToRoot.set(st.subtype, resolveRoot(st.subtype))
    }
  }
  /** Resolve entity name to its root supertype (or itself if not a subtype) */
  function resolveEntity(name: string): string {
    return subtypeToRoot.get(name) ?? name
  }
  // Set of subtype names (these should not get their own tables)
  const subtypeNames = new Set(subtypeToRoot.keys())

  // -----------------------------------------------------------------------
  // Index constraints
  // -----------------------------------------------------------------------

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

  // Index VCs (value constraints) by factTypeId + role
  const vcsByFtRole = new Map<string, string[]>()
  for (const c of schema.constraints) {
    if (c.kind === 'VC' && c.values) {
      for (const r of c.roles) {
        vcsByFtRole.set(`${c.factTypeId}:${r}`, c.values)
      }
    }
  }

  // Index SS (subset) constraints
  const ssConstraints = schema.constraints.filter(c => c.kind === 'SS' && c.targetFactTypeId)

  // -----------------------------------------------------------------------
  // Classify fact types by UC pattern (skip binarized unaries)
  // -----------------------------------------------------------------------
  const compoundFacts: typeof schema.factTypes = []
  const functionalFacts: typeof schema.factTypes = []

  for (const ft of schema.factTypes) {
    if (binarizedFtIds.has(ft.id)) continue  // skip binarized unaries
    if (ft.roles.length < 2) continue         // skip remaining unaries

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
  // Step 3 prep: Detect 1:1 fact types
  // A 1:1 has two separate single-role UCs on different roles of the same
  // binary fact type.
  // -----------------------------------------------------------------------
  const oneToOneFacts: typeof schema.factTypes = []
  const oneToOneFtIds = new Set<string>()

  for (const ft of functionalFacts) {
    if (ft.roles.length !== 2) continue
    const ucs = ucsByFt.get(ft.id) ?? []
    const singleUcRoles = ucs.filter(uc => uc.roles.length === 1).map(uc => uc.roles[0])
    // Check if there are UCs on BOTH roles
    const role0UC = singleUcRoles.includes(ft.roles[0].roleIndex)
    const role1UC = singleUcRoles.includes(ft.roles[1].roleIndex)
    if (role0UC && role1UC) {
      oneToOneFacts.push(ft)
      oneToOneFtIds.add(ft.id)
    }
  }

  // -----------------------------------------------------------------------
  // Step 1: Compound UC → separate table
  // -----------------------------------------------------------------------
  // Track table names emitted so we avoid duplicates
  const emittedTableNames = new Set<string>()

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

    // Table name derived from reading (uses verb for disambiguation)
    const nounNameSet = new Set(schema.nouns.map(n => n.name))
    const tableName = compoundTableName(ft, nounNameSet)

    // Map SS constraints for this compound fact type → CHECK annotation
    const checks: string[] = []
    for (const ss of ssConstraints) {
      if (ss.factTypeId === ft.id && ss.targetFactTypeId) {
        const targetFt = schema.factTypes.find(f => f.id === ss.targetFactTypeId)
        if (targetFt) {
          const targetTableName = compoundTableName(targetFt, nounNameSet)
          checks.push(`FK (${pkCols.join(', ')}) REFERENCES ${targetTableName}`)
        }
      }
    }

    const tableDef: TableDef = { name: tableName, columns, primaryKey: pkCols }
    if (checks.length > 0) tableDef.checks = checks
    tables.push(tableDef)
    emittedTableNames.add(tableName)
  }

  // -----------------------------------------------------------------------
  // Step 2: Functional roles → grouped into entity table
  // (excludes 1:1 facts — handled in Step 3)
  // -----------------------------------------------------------------------
  // Collect all functional facts grouped by the source entity (the UC-bearing role)
  const entityColumns = new Map<string, {
    columns: TableColumn[]
    mandatoryFacts: Set<string>
    checks: string[]
  }>()

  for (const ft of functionalFacts) {
    if (oneToOneFtIds.has(ft.id)) continue  // handled in Step 3
    const ucs = ucsByFt.get(ft.id)!
    for (const uc of ucs) {
      if (uc.roles.length !== 1) continue
      const sourceRoleIdx = uc.roles[0]
      const sourceRole = ft.roles.find(r => r.roleIndex === sourceRoleIdx)
      if (!sourceRole) continue
      const sourceNoun = lookupNoun(schema, sourceRole.nounName)
      if (!sourceNoun || sourceNoun.objectType !== 'entity') continue

      // Resolve to root supertype for absorption
      const entityKey = resolveEntity(sourceNoun.name)
      let entry = entityColumns.get(entityKey)
      if (!entry) {
        entry = { columns: [], mandatoryFacts: new Set(), checks: [] }
        entityColumns.set(entityKey, entry)
      }

      // The target role(s) become columns
      for (const role of ft.roles) {
        if (role.roleIndex === sourceRoleIdx) continue
        const colName = columnNameForTarget(schema, role.nounName)
        // Check if the source role is mandatory for this fact type
        const isMandatory = mcSet.has(`${ft.id}:${sourceRoleIdx}`)
        // Subtype-absorbed columns are always nullable
        const isSubtypeAbsorbed = subtypeNames.has(sourceNoun.name)
        const noun = lookupNoun(schema, role.nounName)
        const isEntity = noun?.objectType === 'entity'
        entry.columns.push({
          name: colName,
          type: 'TEXT',
          nullable: isSubtypeAbsorbed ? true : !isMandatory,
          ...(isEntity ? { references: toSnake(role.nounName) } : {}),
        })
        if (isMandatory && !isSubtypeAbsorbed) {
          entry.mandatoryFacts.add(ft.id)
        }

        // Check for VC on the target role
        const vcKey = `${ft.id}:${role.roleIndex}`
        const vcValues = vcsByFtRole.get(vcKey)
        if (vcValues) {
          const quotedVals = vcValues.map(v => `'${v}'`).join(', ')
          entry.checks.push(`${colName} IN (${quotedVals})`)
        }
      }
    }
  }

  // -----------------------------------------------------------------------
  // Step 3: 1:1 absorption
  // Absorb toward the side with more mandatory constraints (fewer nulls).
  // -----------------------------------------------------------------------
  for (const ft of oneToOneFacts) {
    const role0 = ft.roles[0]
    const role1 = ft.roles[1]
    const mc0 = mcSet.has(`${ft.id}:${role0.roleIndex}`)  // role 0 (source entity) is mandatory
    const mc1 = mcSet.has(`${ft.id}:${role1.roleIndex}`)  // role 1 (target entity) is mandatory

    // Determine absorption direction: absorb the FK into the mandatory side's table
    // If role0 (source) is mandatory → absorb target column into source entity table
    // If role1 (target) is mandatory → absorb source column into target entity table
    // If both mandatory → absorb into role0 (arbitrary but consistent)
    // If neither mandatory → absorb into role0 (arbitrary)
    let absorbIntoEntity: string
    let fkTargetEntity: string
    let isMandatory: boolean

    if (mc0 && !mc1) {
      // Source is mandatory → absorb FK into source
      absorbIntoEntity = resolveEntity(role0.nounName)
      fkTargetEntity = role1.nounName
      isMandatory = true
    } else if (mc1 && !mc0) {
      // Target is mandatory → absorb FK into target
      absorbIntoEntity = resolveEntity(role1.nounName)
      fkTargetEntity = role0.nounName
      isMandatory = true
    } else {
      // Both or neither mandatory → absorb into source
      absorbIntoEntity = resolveEntity(role0.nounName)
      fkTargetEntity = role1.nounName
      isMandatory = mc0
    }

    let entry = entityColumns.get(absorbIntoEntity)
    if (!entry) {
      entry = { columns: [], mandatoryFacts: new Set(), checks: [] }
      entityColumns.set(absorbIntoEntity, entry)
    }

    const colName = fkColumnName(fkTargetEntity)
    entry.columns.push({
      name: colName,
      type: 'TEXT',
      nullable: !isMandatory,
      references: toSnake(fkTargetEntity),
    })
  }

  // -----------------------------------------------------------------------
  // Step 0.1 (continued): Inject XO binarized columns into entity entries
  // -----------------------------------------------------------------------
  for (const [entityName, xoCols] of xoColumns) {
    const resolved = resolveEntity(entityName)
    let entry = entityColumns.get(resolved)
    if (!entry) {
      entry = { columns: [], mandatoryFacts: new Set(), checks: [] }
      entityColumns.set(resolved, entry)
    }
    for (const xoCol of xoCols) {
      entry.columns.push({
        name: xoCol.columnName,
        type: 'TEXT',
        nullable: xoCol.nullable,
      })
      const quotedVals = xoCol.values.map(v => `'${v}'`).join(', ')
      entry.checks.push(`${xoCol.columnName} IN (${quotedVals})`)
    }
  }

  // -----------------------------------------------------------------------
  // Emit entity tables (Step 2 + Step 3 + Step 0.1 absorbed columns)
  // -----------------------------------------------------------------------
  for (const [entityName, entry] of entityColumns) {
    // Skip if this entity is a subtype (its columns were absorbed into root)
    if (subtypeNames.has(entityName)) continue

    const tableName = toSnake(entityName)
    const idCol: TableColumn = {
      name: 'id',
      type: 'TEXT',
      nullable: false,
    }
    const tableDef: TableDef = {
      name: tableName,
      columns: [idCol, ...entry.columns],
      primaryKey: ['id'],
    }
    if (entry.checks.length > 0) {
      tableDef.checks = entry.checks
    }
    tables.push(tableDef)
    emittedTableNames.add(tableName)
  }

  // -----------------------------------------------------------------------
  // Step 4: Independent entity → single-column table
  // Any entity noun that has no table yet but is referenced as FK target
  // (or simply exists in the schema) gets a single-column id table.
  // -----------------------------------------------------------------------
  const referencedEntities = new Set<string>()
  for (const t of tables) {
    for (const col of t.columns) {
      if (col.references) {
        referencedEntities.add(col.references)
      }
    }
  }

  for (const refTable of referencedEntities) {
    if (emittedTableNames.has(refTable)) continue
    // Find the corresponding noun
    const noun = schema.nouns.find(n => toSnake(n.name) === refTable && n.objectType === 'entity')
    if (!noun) continue
    // Skip subtypes
    if (subtypeNames.has(noun.name)) continue

    tables.push({
      name: refTable,
      columns: [{ name: 'id', type: 'TEXT', nullable: false }],
      primaryKey: ['id'],
    })
    emittedTableNames.add(refTable)
  }

  return tables
}

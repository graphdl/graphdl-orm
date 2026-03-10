/**
 * generateReadings — exports the domain model back to FORML2 readings text.
 *
 * This is the reverse of ingestion: it queries the DO for nouns, readings,
 * constraint-spans, constraints, roles, and state-machine-definitions, then
 * formats them as FORML2.
 *
 * Ported from Generator.ts.bak lines 189-295. Adapted for the DO
 * findInCollection API (no depth population — FKs are resolved manually).
 */

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/** Fetch all docs from a collection via findInCollection. */
async function fetchAll(db: any, slug: string, where?: any): Promise<any[]> {
  const result = await db.findInCollection(slug, where, { limit: 10000 })
  return result?.docs || []
}

/** Get a field from a doc, tolerating both Payload names and SQLite column names. */
function getField(doc: any, payloadName: string, sqlName: string): string | undefined {
  return doc[payloadName] ?? doc[sqlName]
}

// ---------------------------------------------------------------------------
// generateReadings
// ---------------------------------------------------------------------------

/**
 * Query the DO for domain model data and produce FORML2 readings text.
 *
 * @param db        - Durable Object stub with `findInCollection(slug, where?, options?)`
 * @param domainId  - The domain ID to scope queries
 * @returns `{ text, format: 'forml2' }`
 */
export async function generateReadings(db: any, domainId: string): Promise<{ text: string; format: string }> {
  const domainFilter = { domain: { equals: domainId } }

  // ------ 1. Fetch all domain model data ------
  const [nouns, readings, constraintSpanRows, constraints, roles, smDefs, eventTypes] = await Promise.all([
    fetchAll(db, 'nouns', domainFilter),
    fetchAll(db, 'readings', domainFilter),
    fetchAll(db, 'constraint-spans'),
    fetchAll(db, 'constraints'),
    fetchAll(db, 'roles'),
    fetchAll(db, 'state-machine-definitions', domainFilter),
    fetchAll(db, 'event-types'),
  ])

  // Build lookup maps for manual FK resolution
  const nounById = new Map(nouns.map((n: any) => [n.id, n]))
  const constraintById = new Map(constraints.map((c: any) => [c.id, c]))
  const roleById = new Map(roles.map((r: any) => [r.id, r]))
  const eventTypeById = new Map(eventTypes.map((e: any) => [e.id, e]))

  const lines: string[] = []

  // ------ 2. Entity types ------
  const entities = nouns.filter((n: any) => n.objectType === 'entity')
  const values = nouns.filter((n: any) => n.objectType === 'value')

  if (entities.length) {
    lines.push('# Entity Types')
    lines.push('')
    for (const e of entities) {
      // referenceScheme could be a comma-separated string or array (in SQLite it's stored as text)
      const refScheme = e.referenceScheme
        ? (Array.isArray(e.referenceScheme)
            ? e.referenceScheme.map((r: any) => (typeof r === 'object' ? r.name : r)).join(', ')
            : String(e.referenceScheme))
        : null

      // superType is an FK (ID) in the flat model — resolve to name
      const superTypeId = getField(e, 'superType', 'super_type_id')
      const superTypeNoun = superTypeId ? nounById.get(superTypeId) : null
      const superTypeName = superTypeNoun?.name ?? null

      let line = e.name
      if (refScheme) line += ` (${refScheme})`
      if (superTypeName) line += ` : ${superTypeName}`
      lines.push(line)
    }
    lines.push('')
  }

  // ------ 3. Value types ------
  if (values.length) {
    lines.push('# Value Types')
    lines.push('')
    for (const v of values) {
      let line = v.name
      const parts: string[] = []
      const vt = getField(v, 'valueType', 'value_type')
      if (vt) parts.push(vt)
      if (v.format) parts.push(`format: ${v.format}`)
      if (v.pattern) parts.push(`pattern: ${v.pattern}`)
      const enumVals = getField(v, 'enum', 'enumValues') ?? getField(v, 'enumValues', 'enum_values')
      if (enumVals) parts.push(`enum: ${enumVals}`)
      if (parts.length) line += ` (${parts.join(', ')})`
      lines.push(line)
    }
    lines.push('')
  }

  // ------ 4. Readings with constraint annotations ------
  if (readings.length) {
    lines.push('# Readings')
    lines.push('')
    for (const r of readings) {
      if (!r.text) continue

      const gsId = getField(r, 'graphSchema', 'graph_schema_id')

      // Find constraint-spans whose role belongs to this reading's graphSchema
      const roleConstraints = constraintSpanRows.filter((cs: any) => {
        const roleId = getField(cs, 'role', 'role_id')
        const role = roleId ? roleById.get(roleId) : null
        if (!role) return false
        const roleGs = getField(role, 'graphSchema', 'graph_schema_id')
        return roleGs === gsId
      })

      let constraintSuffix = ''
      for (const cs of roleConstraints) {
        const constraintId = getField(cs, 'constraint', 'constraint_id')
        const constraint = constraintId ? constraintById.get(constraintId) : null
        if (constraint) {
          const kind = constraint.kind || ''
          const modality = constraint.modality === 'Deontic' ? 'D' : ''
          constraintSuffix += ` [${modality}${kind}]`
        }
      }

      lines.push(r.text + constraintSuffix)
    }
    lines.push('')
  }

  // ------ 5. State machines ------
  for (const sm of smDefs) {
    const smName = sm.title || sm.id
    lines.push(`# State Machine: ${smName}`)
    lines.push('')

    // Fetch statuses for this state machine definition
    const smdId = sm.id
    const statuses = await fetchAll(db, 'statuses', { stateMachineDefinition: { equals: smdId } })
    if (!statuses.length) continue

    const statusById = new Map(statuses.map((s: any) => [s.id, s]))

    // Fetch transitions where from_status_id is one of these statuses
    // Since findInCollection doesn't support `or`, fetch all transitions and filter
    const allTransitions = await fetchAll(db, 'transitions')
    const statusIds = new Set(statuses.map((s: any) => s.id))
    const transitions = allTransitions.filter((t: any) => {
      const fromId = getField(t, 'from', 'from_status_id')
      return fromId && statusIds.has(fromId)
    })

    for (const t of transitions) {
      const fromId = getField(t, 'from', 'from_status_id')
      const toId = getField(t, 'to', 'to_status_id')
      const eventTypeId = getField(t, 'eventType', 'event_type_id')

      const fromStatus = fromId ? statusById.get(fromId) : null
      const toStatus = toId ? statusById.get(toId) : null
      const eventType = eventTypeId ? eventTypeById.get(eventTypeId) : null

      const from = fromStatus?.name
      const to = toStatus?.name
      const event = eventType?.name

      if (from && to && event) {
        lines.push(`${smName} transitions from ${from} to ${to} on ${event}`)
      }
    }
    lines.push('')
  }

  return { text: lines.join('\n'), format: 'forml2' }
}

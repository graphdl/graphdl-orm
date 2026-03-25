export interface SchemaIR {
  nouns: Array<{ name: string; objectType: string }>
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
    text?: string
  }>
  subtypes?: Array<{ subtype: string; supertype: string }>
}

export interface CsdpViolation {
  type: 'arity_violation' | 'missing_mandatory' | 'conflicting_constraints' |
        'undeclared_noun' | 'non_elementary_fact' | 'missing_subtype_constraint' |
        'missing_ring_constraint'
  message: string
  fix: string
  factTypeId?: string
  constraintId?: string
}

export interface CsdpResult {
  valid: boolean
  violations: CsdpViolation[]
}

const RING_CONSTRAINT_KINDS = new Set(['IR', 'AS', 'AT', 'SY', 'IT', 'TR', 'AC'])

export function validateCsdp(schema: SchemaIR): CsdpResult {
  const violations: CsdpViolation[] = []
  const declaredNouns = new Set(schema.nouns.map(n => n.name))

  // CSDP Step 1 quality: undeclared noun check
  for (const factType of schema.factTypes) {
    for (const role of factType.roles) {
      if (!declaredNouns.has(role.nounName)) {
        violations.push({
          type: 'undeclared_noun',
          message: `Role references noun '${role.nounName}' which is not declared in nouns array.`,
          fix: `Add { name: '${role.nounName}', objectType: '...' } to the nouns array.`,
          factTypeId: factType.id,
        })
      }
    }
  }

  // CSDP Step 1 elementarity: non-elementary fact check
  for (const factType of schema.factTypes) {
    // Strip out noun names from the reading to avoid false positives
    // (e.g. "Research and Development has Budget" should not be flagged)
    let stripped = factType.reading
    for (const role of factType.roles) {
      stripped = stripped.replace(role.nounName, '')
    }
    // Check if the remaining text contains " and " as a conjunction
    if (/\band\b/i.test(stripped)) {
      violations.push({
        type: 'non_elementary_fact',
        message: `Reading '${factType.reading}' may conjoin independent assertions (contains 'and' outside noun names).`,
        fix: `Consider splitting '${factType.reading}' into separate elementary fact types.`,
        factTypeId: factType.id,
      })
    }
  }

  // CSDP Step 4: Arity check
  // For each UC, verify it spans at least n-1 roles of its fact type
  for (const constraint of schema.constraints) {
    if (constraint.kind !== 'UC') continue
    const factType = schema.factTypes.find(ft => ft.id === constraint.factTypeId)
    if (!factType) continue
    const arity = factType.roles.length
    if (arity >= 3 && constraint.roles.length < arity - 1) {
      violations.push({
        type: 'arity_violation',
        message: `UC on '${factType.reading}' spans ${constraint.roles.length} of ${arity} roles. For arity ${arity}, UC must span at least ${arity - 1} roles.`,
        fix: `Split '${factType.reading}' into binary fact types, or extend the UC to span ${arity - 1} roles.`,
        factTypeId: factType.id,
      })
    }
  }

  // CSDP Step 6: Missing subtype constraint
  if (schema.subtypes && schema.subtypes.length > 0) {
    // Group subtypes by supertype
    const subtypesBySupertype = new Map<string, string[]>()
    for (const st of schema.subtypes) {
      const existing = subtypesBySupertype.get(st.supertype) ?? []
      existing.push(st.subtype)
      subtypesBySupertype.set(st.supertype, existing)
    }

    for (const [supertype, subs] of subtypesBySupertype) {
      // Check if there is a totality (TO) or exclusion (XO) constraint
      // that references the supertype and its subtypes
      const hasSubtypeConstraint = schema.constraints.some(c => {
        if (c.kind !== 'XO' && c.kind !== 'TO') return false
        // Check if the constraint text references the supertype and subtypes
        if (c.text) {
          const mentionsSupertype = c.text.includes(supertype)
          const mentionsSubtypes = subs.some(sub => c.text!.includes(sub))
          return mentionsSupertype && mentionsSubtypes
        }
        return false
      })

      if (!hasSubtypeConstraint) {
        violations.push({
          type: 'missing_subtype_constraint',
          message: `Subtypes [${subs.join(', ')}] of '${supertype}' have no totality or exclusion constraint.`,
          fix: `Add a totality (TO) and/or exclusion (XO) constraint for the ${supertype} subtype partition.`,
        })
      }
    }
  }

  // CSDP Step 7: Missing ring constraint
  for (const factType of schema.factTypes) {
    if (factType.roles.length !== 2) continue
    const [r0, r1] = factType.roles
    if (r0.nounName !== r1.nounName) continue

    // Check if any ring constraint exists for this fact type
    const hasRingConstraint = schema.constraints.some(c =>
      c.factTypeId === factType.id && RING_CONSTRAINT_KINDS.has(c.kind)
    )

    if (!hasRingConstraint) {
      violations.push({
        type: 'missing_ring_constraint',
        message: `Binary fact type '${factType.reading}' is self-referential (both roles played by '${r0.nounName}') but has no ring constraint.`,
        fix: `Add a ring constraint (IR, AS, AT, SY, IT, TR, or AC) to '${factType.reading}'.`,
        factTypeId: factType.id,
      })
    }
  }

  return { valid: violations.length === 0, violations }
}

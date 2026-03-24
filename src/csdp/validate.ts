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

export function validateCsdp(schema: SchemaIR): CsdpResult {
  const violations: CsdpViolation[] = []

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

  return { valid: violations.length === 0, violations }
}

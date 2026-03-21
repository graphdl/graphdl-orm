/**
 * Eager enrichment via subset constraint autofill.
 * On entity write, forward chain to derive cross-entity relationships.
 */

export interface AutofillConstraint {
  /** The fact type that triggers the lookup */
  sourceFactTypeId: string
  /** The field on the source entity to match */
  sourceField: string
  /** The noun type to look up in */
  targetNounType: string
  /** The field on the target entity to match against */
  targetField: string
  /** The field to set on the source entity with the target's reference */
  derivedField: string
}

export interface EnrichmentContext {
  /** Autofill constraints applicable to this noun type */
  constraints: AutofillConstraint[]
  /** Look up entities by field value — returns matching entity IDs */
  resolveEntities: (nounType: string, field: string, value: string) => Promise<string[]>
}

/**
 * Enrich entity data by resolving autofill constraints.
 * Returns a new data object with derived fields added.
 * Pure function over the input — does not mutate.
 */
export async function enrichEntity(
  data: Record<string, unknown>,
  ctx: EnrichmentContext,
): Promise<Record<string, unknown>> {
  const enriched = { ...data }

  for (const constraint of ctx.constraints) {
    const sourceValue = data[constraint.sourceField]
    if (typeof sourceValue !== 'string') continue

    const matches = await ctx.resolveEntities(
      constraint.targetNounType,
      constraint.targetField,
      sourceValue,
    )

    if (matches.length === 1) {
      enriched[constraint.derivedField] = matches[0]
    } else if (matches.length > 1) {
      enriched[constraint.derivedField] = matches // multiple matches = array
    }
  }

  return enriched
}

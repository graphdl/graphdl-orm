/**
 * Deontic constraint checking on entity write paths.
 *
 * Loads Constraint entities with modality='Deontic' for a domain,
 * resolves which constraints apply to a given entity type via
 * Constraint Span -> Role -> Noun, and checks the entity data
 * against each applicable constraint.
 *
 * Forbidden constraints produce severity='error' violations (block write).
 * Unmet obligatory constraints produce severity='error' violations (block write).
 * Permitted constraints never produce violations (they explicitly allow).
 */

import type { ViolationInput } from './outcomes'

// ---------------------------------------------------------------------------
// Stub interfaces (same shapes as EntityDB / RegistryDB DO RPCs)
// ---------------------------------------------------------------------------

export interface RegistryStub {
  getEntityIds(entityType: string, domainSlug?: string): Promise<string[]>
}

export interface EntityStub {
  get(): Promise<{ id: string; type: string; data: Record<string, unknown> } | null>
}

// ---------------------------------------------------------------------------
// Result type
// ---------------------------------------------------------------------------

export interface DeonticCheckResult {
  allowed: boolean
  violations: ViolationInput[]
}

// ---------------------------------------------------------------------------
// Deontic operator detection
// ---------------------------------------------------------------------------

export type DeonticOperator = 'forbidden' | 'obligatory' | 'permitted'

/**
 * Derive the deontic operator from the constraint's natural-language text.
 * Falls back to 'obligatory' when the text does not contain a recognizable keyword.
 */
export function parseDeonticOperator(text: string): DeonticOperator {
  const lower = text.toLowerCase()
  if (lower.includes('forbidden')) return 'forbidden'
  if (lower.includes('permitted')) return 'permitted'
  return 'obligatory'
}

// ---------------------------------------------------------------------------
// Core check
// ---------------------------------------------------------------------------

/**
 * Check deontic constraints applicable to an entity type before creation.
 *
 * 1. Load all Constraint entities for this domain with modality='Deontic'
 * 2. Load Constraint Spans, Roles, and Nouns to resolve which constraints
 *    apply to this entity type
 * 3. For each applicable constraint, check if the entity data violates it:
 *    - forbidden: the entity's existence or data pattern is itself a violation
 *    - obligatory (MC): required fields must be present and non-empty
 *    - permitted: no violation (explicitly allows)
 * 4. Return violations with severity based on operator
 */
export async function checkDeonticConstraints(
  entityType: string,
  entityData: Record<string, unknown>,
  domain: string,
  registry: RegistryStub,
  getStub: (id: string) => EntityStub,
): Promise<DeonticCheckResult> {
  // 1. Load all Constraint entities for this domain with modality='Deontic'
  const constraintIds = await registry.getEntityIds('Constraint', domain)
  const constraintSettled = await Promise.allSettled(
    constraintIds.map(async (id) => {
      const stub = getStub(id)
      const entity = await stub.get()
      return entity
    }),
  )

  const deonticConstraints: Array<{
    id: string
    kind: string
    modality: string
    text: string
    operator: DeonticOperator
  }> = []

  for (const result of constraintSettled) {
    if (result.status !== 'fulfilled' || !result.value) continue
    const { id, data } = result.value
    if (data.modality !== 'Deontic') continue
    const text = (data.text as string) || ''
    deonticConstraints.push({
      id,
      kind: (data.kind as string) || '',
      modality: 'Deontic',
      text,
      operator: (data.deontic_operator as DeonticOperator) || parseDeonticOperator(text),
    })
  }

  if (deonticConstraints.length === 0) {
    return { allowed: true, violations: [] }
  }

  // 2. Load Constraint Spans to find which constraints reference which roles
  const spanIds = await registry.getEntityIds('Constraint Span')
  const spanSettled = await Promise.allSettled(
    spanIds.map(async (id) => {
      const stub = getStub(id)
      return stub.get()
    }),
  )

  // Map: constraint id -> role ids
  const constraintToRoleIds = new Map<string, string[]>()
  for (const result of spanSettled) {
    if (result.status !== 'fulfilled' || !result.value) continue
    const { data } = result.value
    const constraintId = (data.constraint_id || data.constraint) as string
    const roleId = (data.role_id || data.role) as string
    if (!constraintId || !roleId) continue
    const existing = constraintToRoleIds.get(constraintId) || []
    existing.push(roleId)
    constraintToRoleIds.set(constraintId, existing)
  }

  // 3. Load Roles referenced by deontic constraint spans
  const roleIdsNeeded = new Set<string>()
  for (const c of deonticConstraints) {
    const roles = constraintToRoleIds.get(c.id) || []
    for (const rid of roles) roleIdsNeeded.add(rid)
  }

  // Map: role id -> noun id
  const roleToNounId = new Map<string, string>()
  if (roleIdsNeeded.size > 0) {
    const roleSettled = await Promise.allSettled(
      [...roleIdsNeeded].map(async (id) => {
        const stub = getStub(id)
        const entity = await stub.get()
        return entity ? { roleId: id, nounId: (entity.data.noun_id || entity.data.noun || entity.data.nounId) as string } : null
      }),
    )
    for (const result of roleSettled) {
      if (result.status !== 'fulfilled' || !result.value) continue
      if (result.value.nounId) {
        roleToNounId.set(result.value.roleId, result.value.nounId)
      }
    }
  }

  // 4. Load Noun entities to resolve noun id -> noun name
  const nounIdsNeeded = new Set<string>(roleToNounId.values())
  const nounIdToName = new Map<string, string>()
  if (nounIdsNeeded.size > 0) {
    const nounSettled = await Promise.allSettled(
      [...nounIdsNeeded].map(async (id) => {
        const stub = getStub(id)
        const entity = await stub.get()
        return entity ? { nounId: id, name: (entity.data.name as string) || '' } : null
      }),
    )
    for (const result of nounSettled) {
      if (result.status !== 'fulfilled' || !result.value) continue
      if (result.value.name) {
        nounIdToName.set(result.value.nounId, result.value.name)
      }
    }
  }

  // 5. Build map: constraint id -> set of noun names it applies to
  const constraintToNounNames = new Map<string, Set<string>>()
  for (const c of deonticConstraints) {
    const roleIds = constraintToRoleIds.get(c.id) || []
    const names = new Set<string>()
    for (const rid of roleIds) {
      const nounId = roleToNounId.get(rid)
      if (nounId) {
        const name = nounIdToName.get(nounId)
        if (name) names.add(name)
      }
    }
    constraintToNounNames.set(c.id, names)
  }

  // 6. Filter to constraints that apply to this entity type
  const applicable = deonticConstraints.filter((c) => {
    const names = constraintToNounNames.get(c.id)
    if (!names || names.size === 0) return false
    return names.has(entityType)
  })

  if (applicable.length === 0) {
    return { allowed: true, violations: [] }
  }

  // 7. Evaluate each applicable constraint
  const violations: ViolationInput[] = []

  for (const constraint of applicable) {
    const { operator, kind, text, id } = constraint

    if (operator === 'permitted') {
      // Permitted constraints never produce violations
      continue
    }

    if (operator === 'forbidden') {
      // The entity's existence in this fact type is forbidden
      violations.push({
        domain,
        constraintId: id,
        text: text || `Forbidden: creating ${entityType} violates deontic constraint`,
        severity: 'error',
      })
      continue
    }

    // operator === 'obligatory'
    if (kind === 'MC') {
      // Mandatory constraint: the entity type must participate in a fact type.
      // At creation time, check that the entity data includes the required
      // relationship field. The constraint text mentions the related noun —
      // extract it to determine the required field.
      const nounNames = constraintToNounNames.get(id)
      if (nounNames) {
        for (const relatedNoun of nounNames) {
          if (relatedNoun === entityType) continue
          // Convention: field name is camelCase of the related noun + optional 'Id' suffix
          const fieldName = relatedNoun.charAt(0).toLowerCase() + relatedNoun.slice(1)
          const fieldNameId = fieldName + 'Id'
          const hasField = entityData[fieldName] !== undefined && entityData[fieldName] !== null && entityData[fieldName] !== ''
          const hasFieldId = entityData[fieldNameId] !== undefined && entityData[fieldNameId] !== null && entityData[fieldNameId] !== ''

          // Also check snake_case variant
          const snakeField = relatedNoun.replace(/([A-Z])/g, '_$1').toLowerCase().replace(/^_/, '')
          const snakeFieldId = snakeField + '_id'
          const hasSnakeField = entityData[snakeField] !== undefined && entityData[snakeField] !== null && entityData[snakeField] !== ''
          const hasSnakeFieldId = entityData[snakeFieldId] !== undefined && entityData[snakeFieldId] !== null && entityData[snakeFieldId] !== ''

          if (!hasField && !hasFieldId && !hasSnakeField && !hasSnakeFieldId) {
            violations.push({
              domain,
              constraintId: id,
              text: text || `Obligatory: ${entityType} must have ${relatedNoun}`,
              severity: 'error',
            })
          }
        }
      }
    } else {
      // Generic obligatory constraint: flag as error
      // For UC, IR, and other constraint kinds, the constraint text
      // describes what is obligatory. Without population data to check
      // uniqueness etc., we flag as a warning rather than blocking.
      violations.push({
        domain,
        constraintId: id,
        text: text || `Obligatory constraint on ${entityType}`,
        severity: 'warning',
      })
    }
  }

  const hasErrors = violations.some((v) => v.severity === 'error')

  return {
    allowed: !hasErrors,
    violations,
  }
}

/**
 * Derive `isSemantic` on Constraint entities after materialization.
 *
 * A constraint is semantic iff:
 * - modality is 'Deontic'
 * - it spans a role played by a noun with no Resource instances
 *
 * Semantic constraints require LLM evaluation (judgment).
 * Non-semantic deontic constraints can be evaluated structurally (FOL engine).
 */

export interface DeriveSemantic {
  getEntityIds(type: string, domain?: string): Promise<string[]>
  getEntity(id: string): Promise<{ id: string; type: string; data: Record<string, unknown> } | null>
  patchEntity(id: string, fields: Record<string, unknown>): Promise<void>
}

export async function deriveSemanticFlags(
  domain: string,
  ctx: DeriveSemantic,
): Promise<{ total: number; semantic: number; deterministic: number }> {
  // Load all constraints for the domain
  const constraintIds = await ctx.getEntityIds('Constraint', domain)
  const constraints = (await Promise.all(constraintIds.map(id => ctx.getEntity(id)))).filter(Boolean) as Array<{ id: string; type: string; data: Record<string, unknown> }>

  // Load all constraint spans for the domain
  const spanIds = await ctx.getEntityIds('ConstraintSpan', domain)
  const spans = (await Promise.all(spanIds.map(id => ctx.getEntity(id)))).filter(Boolean) as Array<{ id: string; type: string; data: Record<string, unknown> }>

  // Load all roles for the domain
  const roleIds = await ctx.getEntityIds('Role', domain)
  const roles = (await Promise.all(roleIds.map(id => ctx.getEntity(id)))).filter(Boolean) as Array<{ id: string; type: string; data: Record<string, unknown> }>

  // Build lookup: constraintId → roleIds (via spans)
  const constraintRoles = new Map<string, string[]>()
  for (const span of spans) {
    const cid = span.data.constraint as string || span.data.constraintId as string
    const rid = span.data.role as string || span.data.roleId as string
    if (!cid || !rid) continue
    if (!constraintRoles.has(cid)) constraintRoles.set(cid, [])
    constraintRoles.get(cid)!.push(rid)
  }

  // Build lookup: roleId → nounId
  const roleNoun = new Map<string, string>()
  for (const role of roles) {
    const nounId = role.data.noun as string || role.data.nounId as string
    if (nounId) roleNoun.set(role.id, nounId)
  }

  // Load all nouns and check which have Resource instances
  const nounIds = await ctx.getEntityIds('Noun', domain)
  const nouns = (await Promise.all(nounIds.map(id => ctx.getEntity(id)))).filter(Boolean) as Array<{ id: string; type: string; data: Record<string, unknown> }>

  const nounHasInstances = new Map<string, boolean>()
  for (const noun of nouns) {
    const name = noun.data.name as string
    if (!name) continue
    const resourceIds = await ctx.getEntityIds('Resource', domain)
    // Check if any resource is instance of this noun
    // For efficiency, load resources and check nounId field
    // But resources could be in any domain — check globally too
    const allResourceIds = await ctx.getEntityIds('Resource')
    let hasInstances = false
    // Sample up to 50 resources to check
    const sample = allResourceIds.slice(0, 50)
    for (const rid of sample) {
      const resource = await ctx.getEntity(rid)
      if (resource && (resource.data.noun === noun.id || resource.data.nounId === noun.id)) {
        hasInstances = true
        break
      }
    }
    nounHasInstances.set(noun.id, hasInstances)
  }

  let semantic = 0
  let deterministic = 0

  for (const constraint of constraints) {
    const isDeontic = constraint.data.modality === 'Deontic'
    if (!isDeontic) continue

    const roleIdsForConstraint = constraintRoles.get(constraint.id) || []
    let isSemantic = false

    for (const rid of roleIdsForConstraint) {
      const nounId = roleNoun.get(rid)
      if (!nounId) continue
      if (!nounHasInstances.get(nounId)) {
        isSemantic = true
        break
      }
    }

    // If no spans found, treat as semantic (can't determine structurally)
    if (roleIdsForConstraint.length === 0) {
      isSemantic = true
    }

    const currentValue = constraint.data.isSemantic
    if (currentValue !== isSemantic) {
      await ctx.patchEntity(constraint.id, { isSemantic })
    }

    if (isSemantic) semantic++
    else deterministic++
  }

  return { total: constraints.length, semantic, deterministic }
}

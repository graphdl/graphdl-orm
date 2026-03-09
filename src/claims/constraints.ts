/**
 * Single Constraint Creator — consolidates 4 duplicate constraint creation
 * implementations into one pure parser + one Payload writer.
 *
 * parseMultiplicity() — pure function: parse multiplicity notation into ConstraintDefs
 * applyConstraints() — Payload function: create constraint + constraint-span records
 */

import type { Payload } from 'payload'

export interface ConstraintDef {
  kind: 'UC' | 'MC'
  modality: 'Alethic' | 'Deontic'
  /** Positional role indexes. -1 means "last role" (used for MC). */
  roles: number[]
}

/**
 * Pure function: parse multiplicity notation into constraint definitions.
 *
 * Multiplicity notation is shorthand for uniqueness/mandatory constraints:
 *   *:1  -> UC on role 0 (subject is unique per object)
 *   1:*  -> UC on role 1 (object is unique per subject)
 *   1:1  -> UC on role 0 AND UC on role 1 (two separate constraints)
 *   *:*  -> UC spanning both roles (pair uniqueness)
 *   D*:1 -> Deontic UC (prefix D)
 *   MC   -> Mandatory constraint on last role (Alethic)
 *   DMC  -> Mandatory constraint on last role (Deontic)
 *   AMC  -> Mandatory constraint on last role (Alethic, explicit)
 *   unary -> UC on single role (role 0)
 *   subtype, SS, DSS -> empty (handled elsewhere)
 *
 * Compound specs are space-separated: "*:1 MC" = Alethic UC + Alethic MC
 */
export function parseMultiplicity(spec: string): ConstraintDef[] {
  if (!spec) return []

  // subtype and subset constraints are handled elsewhere
  if (spec === 'subtype') return []
  if (/^D?SS$/i.test(spec.split(/\s+/)[0])) return []

  const parts = spec.split(/\s+/)
  const constraints: ConstraintDef[] = []

  for (const part of parts) {
    // Deontic UC shorthand: D*:1, D1:*, D*:*, D1:1
    const ducMatch = part.match(/^D([*1]:[*1])$/i)
    if (ducMatch) {
      expandUC(ducMatch[1], 'Deontic', constraints)
      continue
    }

    // Alethic UC shorthand: *:1, 1:*, *:*, 1:1
    if (/^[*1]:[*1]$/.test(part)) {
      expandUC(part, 'Alethic', constraints)
      continue
    }

    // Deontic MC
    if (/^DMC$/i.test(part)) {
      constraints.push({ kind: 'MC', modality: 'Deontic', roles: [-1] })
      continue
    }

    // Alethic MC (AMC or just MC)
    if (/^A?MC$/i.test(part)) {
      constraints.push({ kind: 'MC', modality: 'Alethic', roles: [-1] })
      continue
    }

    // Unary: UC on the single role
    if (/^unary$/i.test(part)) {
      constraints.push({ kind: 'UC', modality: 'Alethic', roles: [0] })
      continue
    }
  }

  return constraints
}

/** Expand a UC shorthand pattern into one or two ConstraintDefs. */
function expandUC(pattern: string, modality: 'Alethic' | 'Deontic', out: ConstraintDef[]): void {
  switch (pattern) {
    case '*:1':
      out.push({ kind: 'UC', modality, roles: [0] })
      break
    case '1:*':
      out.push({ kind: 'UC', modality, roles: [1] })
      break
    case '1:1':
      out.push({ kind: 'UC', modality, roles: [0] })
      out.push({ kind: 'UC', modality, roles: [1] })
      break
    case '*:*':
      out.push({ kind: 'UC', modality, roles: [0, 1] })
      break
  }
}

/**
 * Payload function: create constraint + constraint-span records for each ConstraintDef.
 *
 * Role indexes are resolved against the provided roleIds array.
 * Index -1 means "last role" (used for MC constraints).
 */
export async function applyConstraints(
  payload: Payload,
  opts: {
    constraints: ConstraintDef[]
    roleIds: string[]
    domainId?: string
  },
): Promise<void> {
  const { constraints, roleIds, domainId } = opts
  const domainData = domainId ? { domain: domainId } : {}

  for (const def of constraints) {
    // Resolve role indexes to actual IDs
    const resolvedIds = def.roles
      .map((idx) => {
        if (idx === -1) return roleIds[roleIds.length - 1]
        return roleIds[idx]
      })
      .filter((id): id is string => !!id)

    if (!resolvedIds.length) continue

    const constraint = await payload.create({
      collection: 'constraints',
      data: { kind: def.kind, modality: def.modality, ...domainData },
      disableTransaction: true,
    } as any)

    await payload.create({
      collection: 'constraint-spans',
      data: { constraint: constraint.id, roles: resolvedIds, ...domainData },
      disableTransaction: true,
    } as any)
  }
}

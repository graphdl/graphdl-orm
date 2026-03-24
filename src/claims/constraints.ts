/**
 * Constraint parsing and application.
 *
 * parseMultiplicity() — pure function, unchanged from original.
 * applyConstraints() — ported from Payload to GraphDLDBLike DO calls.
 */
import type { GraphDLDBLike } from './ingest'

export interface ConstraintDef {
  kind: 'UC' | 'MC' | 'IR' | 'SY' | 'AS' | 'TR' | 'IT' | 'ANS' | 'AC' | 'RF' | 'SS' | 'XC' | 'EQ' | 'OR' | 'XO'
  modality: 'Alethic' | 'Deontic'
  roles: number[]
}

/**
 * Parse multiplicity notation into constraint definitions.
 * Pure function — no DB dependency.
 */
export function parseMultiplicity(spec: string): ConstraintDef[] {
  if (!spec) return []
  if (spec === 'subtype') return []
  if (/^D?SS$/i.test(spec.split(/\s+/)[0])) return []

  const parts = spec.split(/\s+/)
  const constraints: ConstraintDef[] = []

  for (const part of parts) {
    const ducMatch = part.match(/^D([*1]:[*1])$/i)
    if (ducMatch) { expandUC(ducMatch[1], 'Deontic', constraints); continue }
    if (/^[*1]:[*1]$/.test(part)) { expandUC(part, 'Alethic', constraints); continue }
    if (/^DMC$/i.test(part)) { constraints.push({ kind: 'MC', modality: 'Deontic', roles: [-1] }); continue }
    if (/^A?MC$/i.test(part)) { constraints.push({ kind: 'MC', modality: 'Alethic', roles: [-1] }); continue }
    if (/^unary$/i.test(part)) { constraints.push({ kind: 'UC', modality: 'Alethic', roles: [0] }); continue }
  }

  return constraints
}

function expandUC(pattern: string, modality: 'Alethic' | 'Deontic', out: ConstraintDef[]): void {
  switch (pattern) {
    case '*:1': out.push({ kind: 'UC', modality, roles: [0] }); break
    case '1:*': out.push({ kind: 'UC', modality, roles: [1] }); break
    case '1:1':
      out.push({ kind: 'UC', modality, roles: [0] })
      out.push({ kind: 'UC', modality, roles: [1] })
      break
    case '*:*': out.push({ kind: 'UC', modality, roles: [0, 1] }); break
  }
}

/**
 * Apply constraint definitions by creating constraint + constraint_span records.
 * Ported from Payload's applyConstraints to use GraphDLDBLike directly.
 */
export async function applyConstraints(
  db: GraphDLDBLike,
  opts: {
    constraints: ConstraintDef[]
    roleIds: string[]
    domainId?: string
  },
): Promise<void> {
  const { constraints, roleIds, domainId } = opts

  for (const def of constraints) {
    const resolvedIds = def.roles
      .map(idx => idx === -1 ? roleIds[roleIds.length - 1] : roleIds[idx])
      .filter((id): id is string => !!id)

    if (!resolvedIds.length) continue

    const constraint = await db.createInCollection('constraints', {
      kind: def.kind,
      modality: def.modality,
      ...(domainId ? { domain: domainId } : {}),
    })

    for (const roleId of resolvedIds) {
      await db.createInCollection('constraint-spans', {
        constraint: constraint.id,
        role: roleId,
      })
    }
  }
}

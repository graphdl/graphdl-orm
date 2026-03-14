import { parseConstraintText, parseSetComparisonBlock } from './parse-constraint'
import { parseMultiplicity } from '../claims/constraints'
import { tokenizeReading } from '../claims/tokenize'
import type { HookResult, HookContext } from './index'
import { EMPTY_RESULT } from './index'

const SET_COMPARISON_KINDS = new Set(['SS', 'XC', 'EQ', 'OR', 'XO'])

/**
 * Constraint afterCreate hook.
 *
 * Accepts three input formats:
 * - Natural language (text field): parse via parseConstraintText() or parseSetComparisonBlock()
 * - Shorthand notation (multiplicity field): parse via parseMultiplicity()
 * - Pre-parsed set-comparison (kind is SS/XC/EQ/OR/XO with no reading): skip host reading lookup
 *
 * After parsing, finds the host reading and creates constraint-spans.
 */
export async function constraintAfterCreate(
  db: any,
  doc: Record<string, any>,
  context: HookContext,
): Promise<HookResult> {
  const result: HookResult = { created: {}, warnings: [] }
  const domainId = context.domainId || doc.domain

  // Pre-parsed set-comparison constraint (created via ingest pipeline or raw API)
  if (SET_COMPARISON_KINDS.has(doc.kind) && !doc.text && !doc.multiplicity) {
    return result
  }

  let parsedConstraints: Array<{ kind: string; modality: string; deonticOperator?: string; roles?: number[] }>
  let constraintNouns: string[] = []

  if (doc.text) {
    // Try set-comparison block parse first (multi-line XO/XC/OR/SS)
    const scBlock = parseSetComparisonBlock(doc.text)
    if (scBlock) {
      // Update the constraint record with parsed kind/modality
      await db.updateInCollection('constraints', doc.id, {
        kind: scBlock.kind,
        modality: scBlock.modality,
      })
      // Set-comparison constraints don't have a single host reading — done
      return result
    }

    // Natural language path
    const parsed = parseConstraintText(doc.text)
    if (!parsed) {
      result.warnings.push(`Unrecognized constraint pattern: "${doc.text}"`)
      return result
    }
    parsedConstraints = parsed
    constraintNouns = parsed[0]?.nouns || []
  } else if (doc.multiplicity) {
    // Shorthand notation path — preserve roles from parseMultiplicity
    const defs = parseMultiplicity(doc.multiplicity)
    if (!defs.length) return EMPTY_RESULT
    parsedConstraints = defs.map(d => ({ kind: d.kind, modality: d.modality, roles: d.roles }))
    // Extract nouns from the reading text if provided
    if (doc.reading) {
      const tokenized = tokenizeReading(doc.reading, context.allNouns)
      constraintNouns = tokenized.nounRefs.map(r => r.name)
    }
  } else {
    return EMPTY_RESULT
  }

  // Find the host reading
  const readingText = doc.reading || ''
  let hostReading: Record<string, any> | null = null
  let hostRoles: Record<string, any>[] = []

  if (readingText) {
    // Direct match by reading text
    const readings = await db.findInCollection('readings', {
      text: { equals: readingText },
      domain_id: { equals: domainId },
    }, { limit: 1 })
    if (readings.docs.length > 0) hostReading = readings.docs[0]
  }

  if (!hostReading && constraintNouns.length >= 2) {
    // Match by noun set: find readings containing the same nouns
    const allReadings = await db.findInCollection('readings', {
      domain_id: { equals: domainId },
    }, { limit: 10000 })

    for (const reading of allReadings.docs) {
      const tokenized = tokenizeReading(
        (reading.text || '').split('\n')[0].replace(/\.$/, ''),
        context.allNouns,
      )
      const readingNouns = tokenized.nounRefs.map(r => r.name)
      const sortedReading = [...readingNouns].sort()
      const sortedConstraint = [...constraintNouns].sort()
      if (sortedReading.length === sortedConstraint.length &&
          sortedReading.every((n, i) => n === sortedConstraint[i])) {
        hostReading = reading
        break
      }
    }
  }

  if (!hostReading) {
    if (context.batch) {
      // In batch mode, defer rather than reject
      context.deferred = context.deferred || []
      context.deferred.push({
        data: { ...doc },
        error: `host reading not found for constraint: "${doc.text || doc.multiplicity}"`,
      })
      return result
    }
    result.warnings.push(
      `Constraint rejected: host reading not found for "${doc.text || doc.multiplicity}"`
    )
    return result
  }

  // Fetch roles for the host reading
  const rolesResult = await db.findInCollection('roles', {
    reading_id: { equals: hostReading.id },
  }, { limit: 10000 })
  hostRoles = rolesResult.docs.sort((a: any, b: any) => a.roleIndex - b.roleIndex)

  if (hostRoles.length === 0) {
    result.warnings.push(`No roles found for reading "${hostReading.text}"`)
    return result
  }

  // Create constraint records and spans for each parsed constraint
  for (let i = 0; i < parsedConstraints.length; i++) {
    const parsed = parsedConstraints[i]
    let constraintId: string

    if (i === 0) {
      // Update the already-created constraint doc with parsed kind/modality
      await db.updateInCollection('constraints', doc.id, {
        kind: parsed.kind,
        modality: parsed.modality,
      })
      constraintId = doc.id
    } else {
      // "Exactly one" and similar produce multiple constraints — create new records
      const newConstraint = await db.createInCollection('constraints', {
        kind: parsed.kind,
        modality: parsed.modality,
        text: doc.text,
        domain: domainId,
      })
      constraintId = newConstraint.id
      result.created['constraints'] = [...(result.created['constraints'] || []), newConstraint]
    }

    // Determine which roles to span
    let roleIds: string[]
    if (parsed.roles && parsed.roles.length > 0) {
      // Shorthand path: use explicit role indices from parseMultiplicity
      roleIds = parsed.roles
        .map(idx => idx === -1 ? hostRoles[hostRoles.length - 1]?.id : hostRoles[idx]?.id)
        .filter((id): id is string => !!id)
    } else if (parsed.kind === 'RC') {
      // Ring constraint spans the first role (self-referential)
      roleIds = hostRoles.length > 0 ? [hostRoles[0].id] : []
    } else if (constraintNouns.length >= 1 && hostRoles.length >= 2) {
      // The constrained noun is the first noun in the constraint text.
      // Find the role in the host reading that plays that noun.
      const constrainedNoun = constraintNouns[0]
      const matchingRole = hostRoles.find((r: any) => {
        const nounName = typeof r.noun === 'object' ? r.noun?.name : null
        // If noun isn't populated, look it up from context
        if (!nounName) {
          const nounId = typeof r.noun === 'string' ? r.noun : r.noun?.id
          return context.allNouns.some(n => n.id === nounId && n.name === constrainedNoun)
        }
        return nounName === constrainedNoun
      })
      roleIds = matchingRole ? [matchingRole.id] : [hostRoles[0].id]
    } else {
      // Spanning or ternary: all roles
      roleIds = hostRoles.map((r: any) => r.id)
    }

    for (const roleId of roleIds) {
      const span = await db.createInCollection('constraint-spans', {
        constraint: constraintId,
        role: roleId,
      })
      result.created['constraint-spans'] = [
        ...(result.created['constraint-spans'] || []),
        span,
      ]
    }
  }

  return result
}

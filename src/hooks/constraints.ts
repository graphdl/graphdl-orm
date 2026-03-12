import { parseConstraintText } from './parse-constraint'
import { parseMultiplicity } from '../claims/constraints'
import { tokenizeReading } from '../claims/tokenize'
import type { HookResult, HookContext } from './index'
import { EMPTY_RESULT } from './index'

/**
 * Constraint afterCreate hook.
 *
 * Accepts two input formats:
 * - Natural language (text field): parse via parseConstraintText()
 * - Shorthand notation (multiplicity field): parse via parseMultiplicity()
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

  let parsedConstraints: Array<{ kind: string; modality: string; deonticOperator?: string }>
  let constraintNouns: string[] = []

  if (doc.text) {
    // Natural language path
    const parsed = parseConstraintText(doc.text)
    if (!parsed) {
      result.warnings.push(`Unrecognized constraint pattern: "${doc.text}"`)
      return result
    }
    parsedConstraints = parsed
    constraintNouns = parsed[0]?.nouns || []
  } else if (doc.multiplicity) {
    // Shorthand notation path
    const defs = parseMultiplicity(doc.multiplicity)
    if (!defs.length) return EMPTY_RESULT
    parsedConstraints = defs.map(d => ({ kind: d.kind, modality: d.modality }))
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
    }, { limit: 0 })

    for (const reading of allReadings.docs) {
      const tokenized = tokenizeReading(
        (reading.text || '').split('\n')[0].replace(/\.$/, ''),
        context.allNouns,
      )
      const readingNouns = tokenized.nounRefs.map(r => r.name)
      if (readingNouns.length === constraintNouns.length &&
          readingNouns.every((n, i) => n === constraintNouns[i])) {
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
  }, { limit: 0 })
  hostRoles = rolesResult.docs.sort((a: any, b: any) => a.roleIndex - b.roleIndex)

  if (hostRoles.length === 0) {
    result.warnings.push(`No roles found for reading "${hostReading.text}"`)
    return result
  }

  // Create constraint records and spans for each parsed constraint
  for (const parsed of parsedConstraints) {
    // Update the already-created constraint doc with parsed kind/modality
    await db.updateInCollection('constraints', doc.id, {
      kind: parsed.kind,
      modality: parsed.modality,
    })

    // Determine which roles to span
    let roleIds: string[]
    if (parsed.kind === 'RC') {
      // Ring constraint spans the first role (self-referential)
      roleIds = hostRoles.length > 0 ? [hostRoles[0].id] : []
    } else if (constraintNouns.length === 2 && hostRoles.length >= 2) {
      // Binary: "Each X ..." constrains role 0 (the X side)
      roleIds = [hostRoles[0].id]
    } else {
      // Spanning or ternary: all roles
      roleIds = hostRoles.map((r: any) => r.id)
    }

    for (const roleId of roleIds) {
      const span = await db.createInCollection('constraint-spans', {
        constraint: doc.id,
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

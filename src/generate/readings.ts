/**
 * generateReadings — exports the domain model back to FORML2 readings text.
 *
 * Consumes a DomainModel object (nouns, readings, constraints, state machines)
 * and formats them as FORML2.
 */

import type { NounDef, FactTypeDef, ConstraintDef, StateMachineDef, ReadingDef } from '../model/types'

// ---------------------------------------------------------------------------
// generateReadings
// ---------------------------------------------------------------------------

/**
 * Produce FORML2 readings text from domain model data.
 *
 * @param model - DomainModel providing nouns, factTypes, constraints, stateMachines, readings
 * @returns `{ text, format: 'forml2' }`
 */
export async function generateReadings(model: {
  nouns(): Promise<Map<string, NounDef>>
  factTypes(): Promise<Map<string, FactTypeDef>>
  constraints(): Promise<ConstraintDef[]>
  stateMachines(): Promise<Map<string, StateMachineDef>>
  readings(): Promise<ReadingDef[]>
}): Promise<{ text: string; format: string }> {
  // ------ 1. Fetch all domain model data ------
  const [nounMap, readingsList, constraints, smMap] = await Promise.all([
    model.nouns(),
    model.readings(),
    model.constraints(),
    model.stateMachines(),
  ])

  const nouns = [...nounMap.values()]
  const entities = nouns.filter((n) => n.objectType === 'entity')
  const values = nouns.filter((n) => n.objectType === 'value')

  const lines: string[] = []

  // ------ 2. Entity types ------
  if (entities.length) {
    lines.push('# Entity Types')
    lines.push('')
    for (const e of entities) {
      const refScheme = e.referenceScheme?.map((r) => r.name).join(', ') ?? null
      const superTypeName =
        typeof e.superType === 'string' ? e.superType : e.superType?.name ?? null

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
      if (v.valueType) parts.push(v.valueType)
      if (v.format) parts.push(`format: ${v.format}`)
      if (v.pattern) parts.push(`pattern: ${v.pattern}`)
      if (v.enumValues && v.enumValues.length) parts.push(`enum: ${v.enumValues.join(',')}`)
      if (parts.length) line += ` (${parts.join(', ')})`
      lines.push(line)
    }
    lines.push('')
  }

  // ------ 4. Readings with constraint annotations ------
  if (readingsList.length) {
    lines.push('# Readings')
    lines.push('')
    for (const r of readingsList) {
      if (!r.text) continue

      // Find constraints whose spans reference this reading's graphSchemaId
      const matchingConstraints = constraints.filter((c) =>
        c.spans.some((s) => s.factTypeId === r.graphSchemaId),
      )

      let constraintSuffix = ''
      for (const c of matchingConstraints) {
        const modality = c.modality === 'Deontic' ? 'D' : ''
        constraintSuffix += ` [${modality}${c.kind}]`
      }

      lines.push(r.text + constraintSuffix)
    }
    lines.push('')
  }

  // ------ 5. State machines ------
  const stateMachines = [...smMap.values()]
  for (const sm of stateMachines) {
    const smName = sm.nounName || sm.id
    lines.push(`# State Machine: ${smName}`)
    lines.push('')

    for (const t of sm.transitions) {
      if (t.from && t.to && t.event) {
        lines.push(`${smName} transitions from ${t.from} to ${t.to} on ${t.event}`)
      }
    }
    lines.push('')
  }

  return { text: lines.join('\n'), format: 'forml2' }
}

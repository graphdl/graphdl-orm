/**
 * generateReadings — exports the domain model back to canonical FORML2 readings text.
 *
 * Output matches the format used in source reading files (e.g. support.auto.dev/domains/support.md).
 * Roundtrip: readings file → parse → ingest → generateReadings → same readings file.
 */

import type { NounDef, FactTypeDef, ConstraintDef, StateMachineDef, ReadingDef } from '../model/types'

export async function generateReadings(model: {
  nouns(): Promise<Map<string, NounDef>>
  factTypes(): Promise<Map<string, FactTypeDef>>
  constraints(): Promise<ConstraintDef[]>
  stateMachines(): Promise<Map<string, StateMachineDef>>
  readings(): Promise<ReadingDef[]>
}): Promise<{ text: string; format: string }> {
  const [nounMap, readingsList, constraints, smMap] = await Promise.all([
    model.nouns(),
    model.readings(),
    model.constraints(),
    model.stateMachines(),
  ])

  const nouns = [...nounMap.values()]
  const entities = nouns.filter((n) => n.objectType === 'entity')
  const values = nouns.filter((n) => n.objectType === 'value')

  // Index constraints by fact type for annotation
  const constraintsByFactType = new Map<string, ConstraintDef[]>()
  for (const c of constraints) {
    for (const s of c.spans) {
      const list = constraintsByFactType.get(s.factTypeId) ?? []
      list.push(c)
      constraintsByFactType.set(s.factTypeId, list)
    }
  }

  // Build subtype map
  const subtypes = new Map<string, string>() // child name → parent name
  for (const n of entities) {
    if (n.superType) {
      const parentName = typeof n.superType === 'string' ? n.superType : n.superType.name
      if (parentName) subtypes.set(n.name, parentName)
    }
  }

  const lines: string[] = []

  // ── Entity Types ──────────────────────────────────────────────────
  if (entities.length) {
    lines.push('## Entity Types', '')
    for (const e of entities.sort((a, b) => a.name.localeCompare(b.name))) {
      lines.push(`${e.name}(.${e.name}Id) is an entity type.`)
    }
    lines.push('')
  }

  // ── Subtypes ──────────────────────────────────────────────────────
  if (subtypes.size > 0) {
    lines.push('## Subtypes', '')
    for (const [child, parent] of [...subtypes.entries()].sort((a, b) => a[0].localeCompare(b[0]))) {
      lines.push(`${child} is a subtype of ${parent}.`)
    }
    lines.push('')
  }

  // ── Value Types ───────────────────────────────────────────────────
  if (values.length) {
    lines.push('## Value Types', '')
    for (const v of values.sort((a, b) => a.name.localeCompare(b.name))) {
      lines.push(`${v.name} is a value type.`)
      if (v.enumValues && v.enumValues.length > 0) {
        const quoted = v.enumValues.map((e) => `'${e}'`).join(', ')
        lines.push(`  The possible values of ${v.name} are ${quoted}.`)
      }
    }
    lines.push('')
  }

  // ── Fact Types (readings grouped by first noun) ───────────────────
  if (readingsList.length) {
    lines.push('## Fact Types', '')

    // Group readings by the first noun (role 0)
    const byFirstNoun = new Map<string, ReadingDef[]>()
    for (const r of readingsList) {
      if (!r.text) continue
      const firstNoun = r.roles[0]?.nounName ?? '_ungrouped'
      const list = byFirstNoun.get(firstNoun) ?? []
      list.push(r)
      byFirstNoun.set(firstNoun, list)
    }

    for (const [nounName, readings] of [...byFirstNoun.entries()].sort((a, b) => a[0].localeCompare(b[0]))) {
      lines.push(`### ${nounName}`)
      for (const r of readings) {
        lines.push(r.text)
      }
      lines.push('')
    }
  }

  // ── Alethic Constraints ───────────────────────────────────────────
  const alethic = constraints.filter((c) => c.modality === 'Alethic')
  if (alethic.length) {
    lines.push('## Constraints', '')
    for (const c of alethic) {
      if (c.text) {
        lines.push(`${c.text}.`)
      } else {
        // Reconstruct from kind + spans
        const line = reconstructConstraint(c, nounMap, readingsList)
        if (line) lines.push(line)
      }
    }
    lines.push('')
  }

  // ── Deontic Constraints ───────────────────────────────────────────
  const deontic = constraints.filter((c) => c.modality === 'Deontic')
  if (deontic.length) {
    lines.push('## Deontic Constraints', '')
    for (const c of deontic) {
      if (c.text) {
        lines.push(`${c.text}.`)
      }
    }
    lines.push('')
  }

  // ── State Machines ────────────────────────────────────────────────
  const stateMachines = [...smMap.values()]
  for (const sm of stateMachines) {
    lines.push(`## State Machine: ${sm.nounName}`, '')
    lines.push(`Entity: ${sm.nounName}`)
    if (sm.statuses.length) {
      lines.push(`Initial state: ${sm.statuses[0].name}`)
      lines.push('')
      lines.push('### States')
      lines.push('')
      lines.push(sm.statuses.map((s) => s.name).join(', '))
      lines.push('')
    }
    if (sm.transitions.length) {
      lines.push('### Transitions')
      lines.push('')
      for (const t of sm.transitions) {
        lines.push(`${t.from} → ${t.to} via ${t.event}`)
      }
      lines.push('')
    }
  }

  return { text: lines.join('\n'), format: 'forml2' }
}

// ── Constraint reconstruction from spans ────────────────────────────

function reconstructConstraint(
  c: ConstraintDef,
  nounMap: Map<string, NounDef>,
  readings: ReadingDef[],
): string | null {
  if (c.spans.length === 0) return null

  // Find the reading for the first span
  const reading = readings.find((r) => r.graphSchemaId === c.spans[0].factTypeId)
  if (!reading || reading.roles.length < 2) return null

  const subjectNoun = reading.roles[0]?.nounName ?? ''
  const objectNoun = reading.roles[1]?.nounName ?? ''

  const predicate = extractPredicate(reading.text, subjectNoun, objectNoun)

  switch (c.kind) {
    case 'UC':
      if (c.spans.length > 1) {
        return `For each pair of ${subjectNoun} and ${objectNoun}, that ${subjectNoun} ${predicate} that ${objectNoun} at most once.`
      }
      return `Each ${subjectNoun} ${predicate} at most one ${objectNoun}.`
    case 'MC':
      return `Each ${subjectNoun} ${predicate} some ${objectNoun}.`
    case 'FC': {
      const min = c.minOccurrence ?? 1
      const max = c.maxOccurrence
      const quantifier = max === min ? `exactly ${min}` : max ? `between ${min} and ${max}` : `at least ${min}`
      return `Each ${subjectNoun} in the population of "${reading.text}" occurs there ${quantifier} times.`
    }
    case 'IR':
      return `No ${subjectNoun} ${predicate} the same ${subjectNoun}.`
    case 'AS':
      return `If ${subjectNoun}1 ${predicate} ${subjectNoun}2, then ${subjectNoun}2 is not ${predicate} ${subjectNoun}1.`
    case 'SY':
      return `If ${subjectNoun}1 ${predicate} ${subjectNoun}2, then ${subjectNoun}2 ${predicate} ${subjectNoun}1.`
    case 'TR':
      return `If ${subjectNoun}1 ${predicate} ${subjectNoun}2 and ${subjectNoun}2 ${predicate} ${subjectNoun}3, then ${subjectNoun}1 ${predicate} ${subjectNoun}3.`
    case 'IT':
      return `If ${subjectNoun}1 ${predicate} ${subjectNoun}2 and ${subjectNoun}2 ${predicate} ${subjectNoun}3, then ${subjectNoun}1 is not ${predicate} ${subjectNoun}3.`
    case 'SS':
      return `If some ${subjectNoun} ${predicate} some ${objectNoun} then that ${subjectNoun} ${predicate} that ${objectNoun}.`
    default:
      return null
  }
}

function extractPredicate(readingText: string, firstNoun: string, secondNoun: string): string {
  const afterFirst = readingText.indexOf(firstNoun)
  if (afterFirst === -1) return 'has'
  const start = afterFirst + firstNoun.length
  const beforeSecond = readingText.indexOf(secondNoun, start)
  if (beforeSecond === -1) return 'has'
  return readingText.slice(start, beforeSecond).trim() || 'has'
}

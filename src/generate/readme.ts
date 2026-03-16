/**
 * generateReadme — produces human-readable markdown documentation from a domain model.
 *
 * Consumes entity types, value types, readings, constraints, and state machines
 * to generate a README suitable for developers, stakeholders, or API consumers.
 */

import type { NounDef, FactTypeDef, ConstraintDef, StateMachineDef, ReadingDef } from '../model/types'

export async function generateReadme(model: {
  domainId: string
  nouns(): Promise<Map<string, NounDef>>
  factTypes(): Promise<Map<string, FactTypeDef>>
  constraints(): Promise<ConstraintDef[]>
  stateMachines(): Promise<Map<string, StateMachineDef>>
  readings(): Promise<ReadingDef[]>
}): Promise<{ text: string; format: string }> {
  const [nounMap, ftMap, constraints, smMap, readingsList] = await Promise.all([
    model.nouns(),
    model.factTypes(),
    model.constraints(),
    model.stateMachines(),
    model.readings(),
  ])

  const nouns = [...nounMap.values()]
  const entities = nouns.filter((n) => n.objectType === 'entity').sort((a, b) => a.name.localeCompare(b.name))
  const values = nouns.filter((n) => n.objectType === 'value').sort((a, b) => a.name.localeCompare(b.name))
  const stateMachines = [...smMap.values()]

  // Build supertype map
  const subtypeMap = new Map<string, string[]>() // parent → children
  const parentOf = new Map<string, string>() // child → parent
  for (const n of entities) {
    if (n.superType) {
      const parentName = typeof n.superType === 'string' ? n.superType : n.superType.name
      if (parentName) {
        parentOf.set(n.name, parentName)
        const children = subtypeMap.get(parentName) ?? []
        children.push(n.name)
        subtypeMap.set(parentName, children)
      }
    }
  }

  // Group readings by first noun
  const readingsByEntity = new Map<string, ReadingDef[]>()
  for (const r of readingsList) {
    if (!r.text) continue
    const firstNoun = r.roles[0]?.nounName ?? '_other'
    const list = readingsByEntity.get(firstNoun) ?? []
    list.push(r)
    readingsByEntity.set(firstNoun, list)
  }

  // Group fact types by subject entity for property extraction
  const propertiesByEntity = new Map<string, { name: string; type: string; description: string }[]>()
  for (const [, ft] of ftMap) {
    if (ft.arity < 2) continue
    const subject = ft.roles[0]?.nounName
    const object = ft.roles[1]?.nounDef
    if (!subject || !object) continue
    const props = propertiesByEntity.get(subject) ?? []
    const objectEntity = entities.find((e) => e.name === object.name)
    props.push({
      name: object.name,
      type: objectEntity ? `→ ${object.name}` : object.valueType || 'string',
      description: ft.reading,
    })
    propertiesByEntity.set(subject, props)
  }

  const lines: string[] = []

  // ── Title ──────────────────────────────────────────────────────────
  lines.push(`# ${model.domainId}`, '')
  lines.push(`Domain model documentation. Generated from ${entities.length} entities, ${values.length} value types, ${readingsList.length} readings, and ${stateMachines.length} state machines.`, '')

  // ── Entity Types ───────────────────────────────────────────────────
  if (entities.length) {
    lines.push('## Entities', '')

    for (const e of entities) {
      const parent = parentOf.get(e.name)
      const children = subtypeMap.get(e.name)
      const heading = parent ? `### ${e.name} ← ${parent}` : `### ${e.name}`
      lines.push(heading, '')

      if (e.description) lines.push(`${e.description}`, '')
      if (children?.length) lines.push(`**Subtypes:** ${children.join(', ')}`, '')

      // Properties table
      const props = propertiesByEntity.get(e.name)
      if (props?.length) {
        lines.push('| Property | Type | Reading |')
        lines.push('|----------|------|---------|')
        for (const p of props) {
          lines.push(`| ${p.name} | ${p.type} | ${p.description} |`)
        }
        lines.push('')
      }
    }
  }

  // ── Value Types ────────────────────────────────────────────────────
  if (values.length) {
    lines.push('## Value Types', '')
    lines.push('| Name | Type | Values |')
    lines.push('|------|------|--------|')
    for (const v of values) {
      const enumStr = v.enumValues?.length ? v.enumValues.map((e) => `\`${e}\``).join(', ') : ''
      lines.push(`| ${v.name} | ${v.valueType || 'string'} | ${enumStr} |`)
    }
    lines.push('')
  }

  // ── State Machines ─────────────────────────────────────────────────
  if (stateMachines.length) {
    lines.push('## State Machines', '')

    for (const sm of stateMachines) {
      lines.push(`### ${sm.nounName}`, '')

      if (sm.statuses.length) {
        lines.push(`**States:** ${sm.statuses.map((s) => s.name).join(' → ')}`, '')
      }

      if (sm.transitions.length) {
        lines.push('| From | To | Event |')
        lines.push('|------|----|-------|')
        for (const t of sm.transitions) {
          lines.push(`| ${t.from} | ${t.to} | \`${t.event}\` |`)
        }
        lines.push('')
      }
    }
  }

  // ── Constraints ────────────────────────────────────────────────────
  const alethic = constraints.filter((c) => c.modality === 'Alethic' && c.text)
  const deontic = constraints.filter((c) => c.modality === 'Deontic' && c.text)

  if (alethic.length || deontic.length) {
    lines.push('## Business Rules', '')

    if (alethic.length) {
      lines.push('### Structural Constraints', '')
      for (const c of alethic) {
        lines.push(`- ${c.text}`)
      }
      lines.push('')
    }

    if (deontic.length) {
      lines.push('### Policy Constraints', '')
      for (const c of deontic) {
        const icon = c.deonticOperator === 'forbidden' ? '🚫' : c.deonticOperator === 'permitted' ? '✅' : '📋'
        lines.push(`- ${icon} ${c.text}`)
      }
      lines.push('')
    }
  }

  return { text: lines.join('\n'), format: 'markdown' }
}

/**
 * generateWIT — compile domain model into WASM Interface Types definitions.
 *
 * Readings → .wit file. Each entity type becomes a WIT record.
 * Each fact type becomes a field. Constraints become documentation.
 * The WIT contract is a compiled projection of the readings —
 * generated, not hand-written.
 *
 * Also generates the FOL engine's typed interface (evaluate, query, etc.)
 * so the WASM boundary is typed instead of passing JSON strings.
 */

import type { NounDef, FactTypeDef, ConstraintDef, StateMachineDef } from '../model/types'

interface DomainModel {
  domain: string
  nouns(): Promise<Map<string, NounDef>>
  factTypes(): Promise<Map<string, FactTypeDef>>
  constraints(): Promise<ConstraintDef[]>
  stateMachines(): Promise<Map<string, StateMachineDef>>
}

export async function generateWIT(model: DomainModel): Promise<string> {
  const [nounMap, ftMap, constraints, smMap] = await Promise.all([
    model.nouns(),
    model.factTypes(),
    model.constraints(),
    model.stateMachines(),
  ])

  const nouns = [...nounMap.values()]
  const entities = nouns.filter(n => n.objectType === 'entity')
  const values = nouns.filter(n => n.objectType === 'value')

  // Build fact type index: noun → list of (factTypeId, otherNoun, isMulti)
  const nounFacts = new Map<string, { field: string; type: string; optional: boolean; list: boolean }[]>()
  for (const [ftId, ft] of ftMap) {
    if (ft.roles.length !== 2) continue
    const [role0, role1] = ft.roles
    const noun0 = role0.nounName
    const noun1 = role1.nounName

    // Check for UC (at most one) on this fact type
    const hasUC = constraints.some(c =>
      (c.kind === 'UC' || c.kind === 'FC') &&
      c.spans.some(s => s.factTypeId === ftId)
    )
    const hasMC = constraints.some(c =>
      c.kind === 'MC' &&
      c.spans.some(s => s.factTypeId === ftId)
    )

    const isValueNoun = values.some(v => v.name === noun1)
    const fieldName = toKebab(noun1)
    const fieldType = isValueNoun ? mapValueType(nounMap.get(noun1)) : toKebab(noun0 === noun1 ? `related-${noun1}` : noun1)

    const fields = nounFacts.get(noun0) ?? []
    fields.push({
      field: fieldName,
      type: fieldType,
      optional: !hasMC,
      list: !hasUC,
    })
    nounFacts.set(noun0, fields)
  }

  const lines: string[] = []
  const pkg = toKebab(model.domain)

  lines.push(`package graphdl:${pkg};`)
  lines.push('')
  lines.push(`/// Generated from ${model.domain} domain readings.`)
  lines.push(`/// Each record is an entity type. Each field is a fact type.`)
  lines.push('')

  // Enum types from value types with enum values
  for (const v of values) {
    if (v.enumValues && v.enumValues.length > 0) {
      const enumName = toKebab(v.name)
      lines.push(`enum ${enumName} {`)
      for (const val of v.enumValues) {
        lines.push(`    ${toKebab(val)},`)
      }
      lines.push(`}`)
      lines.push('')
    }
  }

  // Records from entity types
  for (const entity of entities) {
    const recordName = toKebab(entity.name)
    const fields = nounFacts.get(entity.name) ?? []
    lines.push(`record ${recordName} {`)
    lines.push(`    id: string,`)
    for (const f of fields) {
      const witType = f.list ? `list<${f.type}>` : f.type
      const finalType = f.optional && !f.list ? `option<${witType}>` : witType
      lines.push(`    ${f.field}: ${finalType},`)
    }
    lines.push(`}`)
    lines.push('')
  }

  // Violation record
  lines.push(`record violation {`)
  lines.push(`    constraint-id: string,`)
  lines.push(`    constraint-text: string,`)
  lines.push(`    detail: string,`)
  lines.push(`    alethic: bool,`)
  lines.push(`}`)
  lines.push('')

  // Derived fact record
  lines.push(`record derived-fact {`)
  lines.push(`    fact-type-id: string,`)
  lines.push(`    reading: string,`)
  lines.push(`    bindings: list<tuple<string, string>>,`)
  lines.push(`    derived-by: string,`)
  lines.push(`}`)
  lines.push('')

  // Query result record
  lines.push(`record query-result {`)
  lines.push(`    matches: list<string>,`)
  lines.push(`    count: u32,`)
  lines.push(`}`)
  lines.push('')

  // Transition record
  lines.push(`record transition {`)
  lines.push(`    from-status: string,`)
  lines.push(`    to-status: string,`)
  lines.push(`    event: string,`)
  lines.push(`}`)
  lines.push('')

  // Command result record
  lines.push(`record command-result {`)
  lines.push(`    status: option<string>,`)
  lines.push(`    violations: list<violation>,`)
  lines.push(`    transitions: list<transition>,`)
  lines.push(`    derived-count: u32,`)
  lines.push(`    rejected: bool,`)
  lines.push(`}`)
  lines.push('')

  // World interface — the engine's exports
  lines.push(`world fol-engine {`)
  lines.push(`    /// Load and compile domain IR`)
  lines.push(`    export load-ir: func(ir: string) -> result<_, string>;`)
  lines.push('')
  lines.push(`    /// Evaluate constraints against response + population`)
  lines.push(`    export evaluate: func(response-text: string, sender-id: option<string>, population: string) -> list<violation>;`)
  lines.push('')
  lines.push(`    /// Forward chain derivation rules to fixed point`)
  lines.push(`    export forward-chain: func(population: string) -> list<derived-fact>;`)
  lines.push('')
  lines.push(`    /// Query population by partial application of graph schema`)
  lines.push(`    export query: func(schema-id: string, target-role: u32, filters: list<tuple<u32, string>>, population: string) -> query-result;`)
  lines.push('')
  lines.push(`    /// Apply a command (create entity, transition)`)
  lines.push(`    export apply-command: func(command: string, population: string) -> command-result;`)
  lines.push('')
  lines.push(`    /// Get valid transitions from current status`)
  lines.push(`    export get-transitions: func(noun: string, status: string) -> list<transition>;`)
  lines.push('')
  lines.push(`    /// Synthesize all knowledge about a noun`)
  lines.push(`    export synthesize: func(noun: string, depth: u32) -> string;`)
  lines.push(`}`)
  lines.push('')

  return lines.join('\n')
}

// ── Helpers ──────────────────────────────────────────────────────────

function toKebab(name: string): string {
  return name
    .replace(/([a-z])([A-Z])/g, '$1-$2')
    .replace(/[\s_]+/g, '-')
    .toLowerCase()
    .replace(/[^a-z0-9-]/g, '')
}

function mapValueType(noun: NounDef | undefined): string {
  if (!noun) return 'string'
  if (noun.enumValues && noun.enumValues.length > 0) return toKebab(noun.name)
  const vt = noun.valueType?.toLowerCase() ?? ''
  if (vt.includes('int') || vt.includes('number')) return 'u32'
  if (vt.includes('bool')) return 'bool'
  if (vt.includes('float') || vt.includes('decimal')) return 'float64'
  return 'string'
}

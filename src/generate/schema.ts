// src/generate/schema.ts
import type { NounDef, FactTypeDef, ConstraintDef, StateMachineDef } from '../model/types'

// ── Types ──────────────────────────────────────────────────────────────

export interface DomainSchema {
  domain: string
  nouns: Record<string, {
    objectType: 'entity' | 'value'
    enumValues?: string[]
    valueType?: string
    superType?: string
    worldAssumption?: 'closed' | 'open'
  }>
  factTypes: Record<string, {
    reading: string
    roles: Array<{ nounName: string; roleIndex: number }>
  }>
  constraints: Array<{
    id: string
    kind: string
    modality: string
    deonticOperator?: 'obligatory' | 'forbidden' | 'permitted'
    text: string
    spans: Array<{ factTypeId: string; roleIndex: number; subsetAutofill?: boolean }>
    setComparisonArgumentLength?: number
    clauses?: string[]
    entity?: string
    minOccurrence?: number
    maxOccurrence?: number
  }>
  stateMachines: Record<string, {
    nounName: string
    statuses: string[]
    transitions: Array<{
      from: string
      to: string
      event: string
      guard?: {
        graphSchemaId: string
        constraintIds: string[]
      }
    }>
  }>
  derivationRules: Array<{
    id: string
    text: string
    antecedentFactTypeIds: string[]
    consequentFactTypeId: string
    kind: 'subtypeInheritance' | 'modusPonens' | 'transitivity' | 'closedWorldNegation'
  }>
}

// ── Generator ──────────────────────────────────────────────────────────

export async function generateSchema(model: {
  domainId: string
  nouns(): Promise<Map<string, NounDef>>
  factTypes(): Promise<Map<string, FactTypeDef>>
  constraints(): Promise<ConstraintDef[]>
  stateMachines(): Promise<Map<string, StateMachineDef>>
}): Promise<DomainSchema> {
  const [nouns, factTypes, constraints, stateMachines] = await Promise.all([
    model.nouns(),
    model.factTypes(),
    model.constraints(),
    model.stateMachines(),
  ])

  // ── Nouns ──
  const schemaNouns: DomainSchema['nouns'] = {}
  for (const [name, noun] of nouns) {
    const entry: DomainSchema['nouns'][string] = { objectType: noun.objectType }
    if (noun.enumValues && noun.enumValues.length > 0) entry.enumValues = noun.enumValues
    if (noun.valueType) entry.valueType = noun.valueType
    if (noun.superType) {
      entry.superType = typeof noun.superType === 'string'
        ? noun.superType
        : noun.superType.name
    }
    if (noun.worldAssumption) entry.worldAssumption = noun.worldAssumption
    schemaNouns[name] = entry
  }

  // ── FactTypes ──
  const schemaFactTypes: DomainSchema['factTypes'] = {}
  for (const [id, ft] of factTypes) {
    schemaFactTypes[id] = {
      reading: ft.reading,
      roles: ft.roles.map((r) => ({
        nounName: r.nounName,
        roleIndex: r.roleIndex,
      })),
    }
  }

  // ── Constraints ──
  const schemaConstraints: DomainSchema['constraints'] = []
  for (const c of constraints) {
    const entry: DomainSchema['constraints'][number] = {
      id: c.id,
      kind: c.kind,
      modality: c.modality || 'Alethic',
      text: c.text || '',
      spans: c.spans.map((s) => ({
        factTypeId: s.factTypeId,
        roleIndex: s.roleIndex,
        ...(s.subsetAutofill ? { subsetAutofill: true } : {}),
      })),
    }
    if (c.deonticOperator) entry.deonticOperator = c.deonticOperator
    if (c.setComparisonArgumentLength) entry.setComparisonArgumentLength = c.setComparisonArgumentLength
    if (c.entity) entry.entity = c.entity
    if (c.clauses) entry.clauses = c.clauses
    if (c.minOccurrence !== undefined) entry.minOccurrence = c.minOccurrence
    if (c.maxOccurrence !== undefined) entry.maxOccurrence = c.maxOccurrence
    schemaConstraints.push(entry)
  }

  // ── State Machines ──
  const schemaStateMachines: DomainSchema['stateMachines'] = {}
  for (const [id, sm] of stateMachines) {
    schemaStateMachines[id] = {
      nounName: sm.nounName,
      statuses: sm.statuses.map((s) => s.name),
      transitions: sm.transitions.map((t) => {
        const transition: DomainSchema['stateMachines'][string]['transitions'][number] = {
          from: t.from,
          to: t.to,
          event: t.event,
        }
        if (t.guard) {
          transition.guard = {
            graphSchemaId: t.guard.graphSchemaId,
            constraintIds: t.guard.constraintIds,
          }
        }
        return transition
      }),
    }
  }

  // ── Derivation Rules ──
  const derivationRules: DomainSchema['derivationRules'] = []

  // Subtype inheritance rules
  for (const [name, noun] of nouns) {
    const superType = noun.superType
      ? (typeof noun.superType === 'string' ? noun.superType : noun.superType.name)
      : undefined
    if (superType) {
      derivationRules.push({
        id: `derive-subtype-${name}`,
        text: `${name} inherits constraints of ${superType}`,
        antecedentFactTypeIds: [],
        consequentFactTypeId: '',
        kind: 'subtypeInheritance',
      })
    }
  }

  // Modus ponens from subset constraints (SS)
  for (const c of schemaConstraints) {
    if (c.kind === 'SS' && c.spans.length >= 2) {
      derivationRules.push({
        id: `derive-mp-${c.id}`,
        text: c.text,
        antecedentFactTypeIds: [c.spans[0].factTypeId],
        consequentFactTypeId: c.spans[1].factTypeId,
        kind: 'modusPonens',
      })
    }
  }

  // CWA negation for closed-world nouns
  for (const [name, noun] of nouns) {
    const assumption = noun.worldAssumption || 'closed'
    if (assumption === 'closed') {
      const factTypeIds = Object.entries(schemaFactTypes)
        .filter(([_, ft]) => ft.roles.some((r) => r.nounName === name))
        .map(([id]) => id)
      if (factTypeIds.length > 0) {
        derivationRules.push({
          id: `derive-cwa-${name}`,
          text: `Absence of fact for ${name} implies negation (Closed World)`,
          antecedentFactTypeIds: factTypeIds,
          consequentFactTypeId: '',
          kind: 'closedWorldNegation',
        })
      }
    }
  }

  return {
    domain: model.domainId,
    nouns: schemaNouns,
    factTypes: schemaFactTypes,
    constraints: schemaConstraints,
    stateMachines: schemaStateMachines,
    derivationRules,
  }
}

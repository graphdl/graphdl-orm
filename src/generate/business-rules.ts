// src/generate/business-rules.ts
import type { NounDef, FactTypeDef, ConstraintDef, StateMachineDef } from '../model/types'

// ── Types ──────────────────────────────────────────────────────────────

export interface BusinessRules {
  domain: string
  nouns: Record<string, {
    objectType: 'entity' | 'value'
    enumValues?: string[]
    valueType?: string
    superType?: string
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
}

// ── Generator ──────────────────────────────────────────────────────────

export async function generateBusinessRules(model: {
  domainId: string
  nouns(): Promise<Map<string, NounDef>>
  factTypes(): Promise<Map<string, FactTypeDef>>
  constraints(): Promise<ConstraintDef[]>
  stateMachines(): Promise<Map<string, StateMachineDef>>
}): Promise<BusinessRules> {
  const [nouns, factTypes, constraints, stateMachines] = await Promise.all([
    model.nouns(),
    model.factTypes(),
    model.constraints(),
    model.stateMachines(),
  ])

  // ── Nouns ──
  const irNouns: BusinessRules['nouns'] = {}
  for (const [name, noun] of nouns) {
    const entry: BusinessRules['nouns'][string] = { objectType: noun.objectType }
    if (noun.enumValues && noun.enumValues.length > 0) entry.enumValues = noun.enumValues
    if (noun.valueType) entry.valueType = noun.valueType
    if (noun.superType) {
      entry.superType = typeof noun.superType === 'string'
        ? noun.superType
        : noun.superType.name
    }
    irNouns[name] = entry
  }

  // ── FactTypes ──
  const irFactTypes: BusinessRules['factTypes'] = {}
  for (const [id, ft] of factTypes) {
    irFactTypes[id] = {
      reading: ft.reading,
      roles: ft.roles.map((r) => ({
        nounName: r.nounName,
        roleIndex: r.roleIndex,
      })),
    }
  }

  // ── Constraints ──
  const irConstraints: BusinessRules['constraints'] = []
  for (const c of constraints) {
    const entry: BusinessRules['constraints'][number] = {
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
    irConstraints.push(entry)
  }

  // ── State Machines ──
  const irStateMachines: BusinessRules['stateMachines'] = {}
  for (const [id, sm] of stateMachines) {
    irStateMachines[id] = {
      nounName: sm.nounName,
      statuses: sm.statuses.map((s) => s.name),
      transitions: sm.transitions.map((t) => {
        const transition: BusinessRules['stateMachines'][string]['transitions'][number] = {
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

  return {
    domain: model.domainId,
    nouns: irNouns,
    factTypes: irFactTypes,
    constraints: irConstraints,
    stateMachines: irStateMachines,
  }
}

export type ConstraintKind =
  | 'UC' | 'MC' | 'FC'
  | 'SS' | 'EQ' | 'XC' | 'OR' | 'XO'
  | 'IR' | 'AS' | 'AT' | 'SY' | 'IT' | 'TR' | 'AC'
  | 'VC'

export type WorldAssumption = 'closed' | 'open'

export interface NounDef {
  id: string
  name: string
  plural?: string
  description?: string
  objectType: 'entity' | 'value'
  domainId: string
  valueType?: string
  format?: string
  pattern?: string
  enumValues?: string[]
  minimum?: number
  exclusiveMinimum?: number
  maximum?: number
  exclusiveMaximum?: number
  minLength?: number
  maxLength?: number
  multipleOf?: number
  superType?: NounDef | string
  referenceScheme?: NounDef[]
  permissions?: string[]
  worldAssumption?: WorldAssumption
}

export interface FactTypeDef {
  id: string
  name?: string
  reading: string
  roles: RoleDef[]
  arity: number
}

export interface RoleDef {
  id: string
  nounName: string
  nounDef: NounDef
  roleIndex: number
}

export interface ConstraintDef {
  id: string
  kind: ConstraintKind
  modality: 'Alethic' | 'Deontic'
  deonticOperator?: 'obligatory' | 'forbidden' | 'permitted'
  text: string
  spans: SpanDef[]
  entity?: string
  clauses?: string[]
  setComparisonArgumentLength?: number
  minOccurrence?: number
  maxOccurrence?: number
}

export interface SpanDef {
  factTypeId: string
  roleIndex: number
  subsetAutofill?: boolean
}

export interface StateMachineDef {
  id: string
  nounName: string
  nounDef: NounDef
  statuses: StatusDef[]
  transitions: TransitionDef[]
}

export interface StatusDef {
  id: string
  name: string
}

export interface TransitionDef {
  from: string
  to: string
  event: string
  eventTypeId: string
  verb?: VerbDef
  guard?: { graphSchemaId: string; constraintIds: string[] }
}

export interface VerbDef {
  id: string
  name: string
  statusId?: string
  transitionId?: string
  graphId?: string
  agentDefinitionId?: string
  func?: {
    callbackUrl?: string
    httpMethod?: string
    functionType?: string
    headers?: Record<string, string>
  }
}

export interface ReadingDef {
  id: string
  text: string
  graphSchemaId: string
  roles: RoleDef[]
}

export type { Generator, NounRenderer, FactTypeRenderer } from './renderer'

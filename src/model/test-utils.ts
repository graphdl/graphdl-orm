import type {
  NounDef,
  FactTypeDef,
  ConstraintDef,
  ConstraintKind,
  SpanDef,
  StateMachineDef,
  StatusDef,
  TransitionDef,
  ReadingDef,
} from './types'
import type { Generator } from './renderer'
import { render } from './renderer'

// ---------------------------------------------------------------------------
// ID counter
// ---------------------------------------------------------------------------

let _idCounter = 0

function nextId(prefix: string): string {
  return `${prefix}_${++_idCounter}`
}

export function resetIds(): void {
  _idCounter = 0
}

// ---------------------------------------------------------------------------
// Factory functions
// ---------------------------------------------------------------------------

export function mkNounDef(overrides: Partial<NounDef> & { name?: string } = {}): NounDef {
  return {
    id: nextId('n'),
    objectType: 'entity',
    domainId: 'd1',
    name: 'Unnamed',
    ...overrides,
  }
}

export function mkValueNounDef(overrides: Partial<NounDef> & { name?: string } = {}): NounDef {
  return {
    id: nextId('n'),
    objectType: 'value',
    domainId: 'd1',
    name: 'Unnamed',
    ...overrides,
  }
}

interface RoleShorthand {
  nounDef: NounDef
  roleIndex: number
}

export function mkFactType(params: {
  reading: string
  roles: RoleShorthand[]
  id?: string
  name?: string
}): FactTypeDef {
  const { reading, roles, id, name } = params
  return {
    id: id ?? nextId('ft'),
    name,
    reading,
    roles: roles.map((r) => ({
      id: nextId('r'),
      nounName: r.nounDef.name,
      nounDef: r.nounDef,
      roleIndex: r.roleIndex,
    })),
    arity: roles.length,
  }
}

export function mkConstraint(params: {
  kind: ConstraintKind
  spans?: SpanDef[]
  id?: string
  modality?: 'Alethic' | 'Deontic'
  deonticOperator?: 'obligatory' | 'forbidden' | 'permitted'
  text?: string
  entity?: string
  clauses?: string[]
  setComparisonArgumentLength?: number
  minOccurrence?: number
  maxOccurrence?: number
}): ConstraintDef {
  const { kind, spans = [], id, modality = 'Alethic', text = '', ...rest } = params
  return {
    id: id ?? nextId('c'),
    kind,
    modality,
    text,
    spans,
    ...rest,
  }
}

export function mkStateMachine(params: {
  nounDef: NounDef
  statuses?: StatusDef[]
  transitions?: TransitionDef[]
  id?: string
}): StateMachineDef {
  const { nounDef, statuses = [], transitions = [], id } = params
  return {
    id: id ?? nextId('sm'),
    nounName: nounDef.name,
    nounDef,
    statuses,
    transitions,
  }
}

// ---------------------------------------------------------------------------
// createMockModel
// ---------------------------------------------------------------------------

export function createMockModel(data: {
  domainId?: string
  nouns?: NounDef[]
  factTypes?: FactTypeDef[]
  constraints?: ConstraintDef[]
  stateMachines?: StateMachineDef[]
  readings?: ReadingDef[]
}) {
  const nounMap = new Map((data.nouns ?? []).map((n) => [n.name, n]))
  const ftMap = new Map((data.factTypes ?? []).map((ft) => [ft.id, ft]))
  const smMap = new Map((data.stateMachines ?? []).map((sm) => [sm.id, sm]))

  const mockModel = {
    domainId: data.domainId ?? 'd1',
    nouns: async () => nounMap,
    noun: async (name: string) => nounMap.get(name),
    factTypes: async () => ftMap,
    factTypesFor: async (noun: NounDef) =>
      (data.factTypes ?? []).filter((ft) =>
        ft.roles.some((r) => r.nounDef.name === noun.name && r.roleIndex === 0),
      ),
    constraints: async () => data.constraints ?? [],
    constraintsFor: async (fts: FactTypeDef[]) => {
      const ids = new Set(fts.map((f) => f.id))
      return (data.constraints ?? []).filter((c) => c.spans.some((s) => ids.has(s.factTypeId)))
    },
    constraintSpans: async () => {
      const map = new Map<string, SpanDef[]>()
      for (const c of data.constraints ?? []) {
        if (c.spans.length > 0) map.set(c.id, c.spans)
      }
      return map
    },
    stateMachines: async () => smMap,
    readings: async () => data.readings ?? [],
    invalidate: () => {},
  }

  return {
    ...mockModel,
    render: async <T, Out>(gen: Generator<T, Out>) => render(mockModel, gen),
  }
}

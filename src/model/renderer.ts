import type { NounDef, FactTypeDef, RoleDef, ConstraintDef, StateMachineDef } from './types'

/** Renders a single noun (entity or value type) */
export interface NounRenderer<T> {
  entity(noun: NounDef, factTypes: FactTypeDef[], constraints: ConstraintDef[]): T
  value(noun: NounDef): T
}

/** Renders a single fact type — resolved roles with noun references */
export interface FactTypeRenderer<T> {
  unary?(role: RoleDef, constraints: ConstraintDef[]): T
  binary?(entity: RoleDef, value: RoleDef, constraints: ConstraintDef[]): T
  nary?(roles: RoleDef[], constraints: ConstraintDef[]): T
  custom?(factType: FactTypeDef, roles: RoleDef[], constraints: ConstraintDef[]): T
}

/** A complete generator */
export interface Generator<T, Out> {
  noun: NounRenderer<T>
  factType?: FactTypeRenderer<T>
  constraint?: (constraint: ConstraintDef) => T
  stateMachine?: (sm: StateMachineDef) => T
  combine(parts: T[]): Out
}

/** The subset of DomainModel methods that render() needs */
export interface ModelAccessors {
  nouns(): Promise<Map<string, NounDef>>
  factTypes(): Promise<Map<string, FactTypeDef>>
  factTypesFor(noun: NounDef): Promise<FactTypeDef[]>
  constraintsFor(fts: FactTypeDef[]): Promise<ConstraintDef[]>
  constraints(): Promise<ConstraintDef[]>
  stateMachines(): Promise<Map<string, StateMachineDef>>
}

export async function render<T, Out>(model: ModelAccessors, gen: Generator<T, Out>): Promise<Out> {
  const parts: T[] = []
  const nouns = await model.nouns()

  for (const [, noun] of nouns) {
    if (noun.objectType === 'entity') {
      const fts = await model.factTypesFor(noun)
      const cs = await model.constraintsFor(fts)
      parts.push(gen.noun.entity(noun, fts, cs))
    } else {
      parts.push(gen.noun.value(noun))
    }
  }

  if (gen.factType) {
    for (const [id, ft] of await model.factTypes()) {
      const cs = await model.constraintsFor([ft])
      const roles = ft.roles

      if (gen.factType.custom) {
        parts.push(gen.factType.custom(ft, roles, cs))
      } else if (ft.arity === 1 && gen.factType.unary) {
        parts.push(gen.factType.unary(roles[0], cs))
      } else if (ft.arity === 2 && gen.factType.binary) {
        parts.push(gen.factType.binary(roles[0], roles[1], cs))
      } else if (gen.factType.nary) {
        parts.push(gen.factType.nary(roles, cs))
      }
    }
  }

  if (gen.constraint) {
    for (const c of await model.constraints()) {
      parts.push(gen.constraint(c))
    }
  }

  if (gen.stateMachine) {
    for (const [, sm] of await model.stateMachines()) {
      parts.push(gen.stateMachine(sm))
    }
  }

  return gen.combine(parts)
}

import type { DeonticConstraintInstanceDef } from './parser'

export interface DeonticConstraintGroup {
  constraintText: string
  instances: string[]
}

export function groupDeonticInstances(
  constraints: string[],
  instances: DeonticConstraintInstanceDef[],
): DeonticConstraintGroup[] {
  return constraints.map((text) => ({
    constraintText: text,
    instances: instances
      .filter((i) => i.constraint === text)
      .map((i) => i.instance),
  }))
}

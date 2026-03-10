export { METAMODEL_DDL } from './metamodel'
export { STATE_DDL } from './state'
export { INSTANCE_DDL } from './instances'

import { METAMODEL_DDL } from './metamodel'
import { STATE_DDL } from './state'
import { INSTANCE_DDL } from './instances'

/** All DDL statements in dependency order. */
export const ALL_DDL: string[] = [
  ...METAMODEL_DDL,
  ...STATE_DDL,
  ...INSTANCE_DDL,
]

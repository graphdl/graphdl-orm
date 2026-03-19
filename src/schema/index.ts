export { METAMODEL_DDL } from './metamodel'
export { STATE_DDL } from './state'
export { AGENT_DDL } from './agents'
export { INSTANCE_DDL } from './instances'

import { METAMODEL_DDL } from './metamodel'
import { STATE_DDL } from './state'
import { AGENT_DDL } from './agents'
import { INSTANCE_DDL } from './instances'

/** All DDL statements in dependency order. */
export const ALL_DDL: string[] = [
  ...METAMODEL_DDL,
  ...STATE_DDL,
  ...AGENT_DDL,
  ...INSTANCE_DDL,
]

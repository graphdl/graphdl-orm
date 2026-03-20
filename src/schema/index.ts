export { BOOTSTRAP_DDL } from './bootstrap'

import { BOOTSTRAP_DDL } from './bootstrap'

/** All DDL statements in dependency order (generated from readings/*.md). */
export const ALL_DDL: string[] = [
  ...BOOTSTRAP_DDL,
]

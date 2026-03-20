import { GraphDLDB } from './do'
import { router } from './api/router'

export { GraphDLDB }
export { EntityDB } from './entity-do'
export { DomainDB } from './domain-do'

export default {
  fetch: router.fetch,
}

export type { Env } from './types'

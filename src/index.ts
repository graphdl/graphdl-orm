import { router } from './api/router'

export { EntityDB } from './entity-do'
export { DomainDB } from './domain-do'
export { RegistryDB } from './registry-do'

export default {
  fetch: router.fetch,
}

export type { Env } from './types'

import { GraphDLDB } from './do'
import { router } from './api/router'

export { GraphDLDB }

export default {
  fetch: router.fetch,
}

export type { Env } from './types'

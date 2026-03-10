import { GraphDLDB } from './do'

export { GraphDLDB }

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    return new Response(JSON.stringify({ status: 'ok', version: '0.1.0' }), {
      headers: { 'Content-Type': 'application/json' },
    })
  },
}

// Keep Env export for other modules
export type { Env } from './types'

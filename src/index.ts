import { router } from './api/router'
import type { Env } from './types'

export { EntityDB } from './entity-do'
export { DomainDB } from './domain-do'
export { RegistryDB } from './registry-do'

/**
 * Auth middleware — blocks direct HTTP access without a valid bearer token.
 * Service bindings (apis worker) bypass this entirely (they call DO methods
 * via env.GRAPHDL, not the fetch handler).
 *
 * Set API_SECRET as a Cloudflare secret:
 *   wrangler secret put API_SECRET
 *
 * Callers authenticate with:
 *   Authorization: Bearer <API_SECRET>
 *
 * When API_SECRET is not set (local dev), all requests are allowed.
 */
function withAuth(request: Request, env: Env): Response | void {
  const secret = env.API_SECRET
  if (!secret) return // no secret configured = allow all (local dev)

  // Health check is public
  const url = new URL(request.url)
  if (url.pathname === '/health') return

  const auth = request.headers.get('authorization')
  if (!auth || !auth.startsWith('Bearer ')) {
    return new Response(JSON.stringify({ error: 'Missing Authorization header' }), {
      status: 401,
      headers: { 'Content-Type': 'application/json' },
    })
  }

  const token = auth.slice(7)
  if (token !== secret) {
    return new Response(JSON.stringify({ error: 'Invalid token' }), {
      status: 403,
      headers: { 'Content-Type': 'application/json' },
    })
  }
}

export default {
  async fetch(request: Request, env: Env, ctx: ExecutionContext): Promise<Response> {
    const authResponse = withAuth(request, env)
    if (authResponse) return authResponse
    return router.fetch(request, env, ctx)
  },
}

export type { Env } from './types'

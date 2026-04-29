export interface Env {
  ENTITY_DB: DurableObjectNamespace
  REGISTRY_DB: DurableObjectNamespace
  /**
   * Subscriber registry for the kernel's signal-delivery layer. One
   * instance per scope — routes fetch `idFromName('global')` (or
   * per-App). Post-mutation hooks publish CellEvents; the /api/events
   * SSE route subscribes. See src/broadcast-do.ts.
   */
  BROADCAST: DurableObjectNamespace
  ENVIRONMENT: string
  API_SECRET?: string
  /**
   * Cloudflare AI Gateway base URL (#638). Read by `aiComplete` in
   * src/api/ai/complete.ts. Documented in wrangler.jsonc. May be the
   * empty string in local dev — `aiComplete` returns a config-error
   * envelope rather than throwing in that case.
   */
  AI_GATEWAY_URL?: string
  /**
   * Cloudflare AI Gateway bearer token (#638). Set as a secret via
   * `wrangler secret put AI_GATEWAY_TOKEN`; never committed.
   */
  AI_GATEWAY_TOKEN?: string
}

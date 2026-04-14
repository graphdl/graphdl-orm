export interface Env {
  ENTITY_DB: DurableObjectNamespace
  REGISTRY_DB: DurableObjectNamespace
  /**
   * Subscriber registry for the kernel's signal-delivery layer. One
   * instance per scope — routes fetch `idFromName('global')` (or
   * per-App). Post-mutation hooks publish CellEvents; the /api/events
   * SSE route subscribes. See src/broadcast-do.ts and docs/11.
   */
  BROADCAST: DurableObjectNamespace
  ENVIRONMENT: string
  API_SECRET?: string
}

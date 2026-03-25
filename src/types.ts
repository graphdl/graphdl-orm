export interface Env {
  ENTITY_DB: DurableObjectNamespace
  DOMAIN_DB: DurableObjectNamespace
  REGISTRY_DB: DurableObjectNamespace
  ENVIRONMENT: string
  API_SECRET?: string  // shared secret for direct HTTP access (service bindings bypass this)
}

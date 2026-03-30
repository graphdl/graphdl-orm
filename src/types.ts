export interface Env {
  ENTITY_DB: DurableObjectNamespace
  REGISTRY_DB: DurableObjectNamespace
  ENVIRONMENT: string
  API_SECRET?: string
}

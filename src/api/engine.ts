/**
 * Engine bridge — thin interface between HTTP dispatch and the WASM FOL engine.
 *
 * The TypeScript layer does NOT implement business logic. It:
 * 1. Loads the domain schema from EntityDB into a ConstraintIR
 * 2. Compiles it in the WASM engine (load_ir)
 * 3. Calls the appropriate WASM function (evaluate, forward_chain, run_machine, query)
 * 4. Returns the result
 *
 * All logic lives in the Rust AST. This file is plumbing.
 */

import { initSync, load_ir, evaluate_response, forward_chain_population, run_machine_wasm, query_schema_wasm, get_transitions_wasm, resolve_fact_event, prepare_entity, apply_command_wasm, debug_compiled_state, load_validation_model, validate_schema_wasm, project_entity_wasm, get_noun_schemas_wasm } from '../../crates/fol-engine/pkg/fol_engine.js'
import wasmModule from '../../crates/fol-engine/pkg/fol_engine_bg.wasm'

let wasmInitialized = false

function ensureWasm() {
  if (!wasmInitialized) {
    initSync({ module: wasmModule })
    wasmInitialized = true
  }
}

/**
 * Load a domain's schema into the WASM engine.
 * Builds the ConstraintIR from EntityDB entities via the schema generator.
 */
export async function loadDomainSchema(
  registry: any,
  getStub: (id: string) => any,
  domainSlug: string,
): Promise<void> {
  ensureWasm()
  const { buildSchemaFromEntities } = await import('../generate/schema-from-entities')

  // Fetch entities directly from Registry+EntityDB — no DomainModel, no field translation
  const fetchEntities = async (type: string, domain: string) => {
    const ids: string[] = await registry.getEntityIds(type, domain)
    const results: Array<{ id: string; type: string; data: Record<string, unknown> }> = []
    const settled = await Promise.allSettled(
      ids.map(async (id: string) => {
        const entity = await getStub(id).get()
        if (entity && !entity.deletedAt) {
          results.push({ id: entity.id, type: entity.type, data: entity.data })
        }
      }),
    )
    return results
  }

  const schema = await buildSchemaFromEntities(domainSlug, fetchEntities)
  load_ir(JSON.stringify(schema))
}

/**
 * Build a live population from EntityDB entities for a domain.
 *
 * Maps entity instances to fact type bindings:
 * - Each entity of type T with field F=V contributes to fact type "T has F"
 *   as binding [("T", entity.id), ("F", V)]
 * - The fact type IDs come from the schema IR (which must be loaded first)
 *
 * Returns a Population JSON string suitable for WASM evaluation.
 */
async function buildPopulation(
  registry: any,
  getStub: (id: string) => any,
  domainSlug: string,
): Promise<string> {
  // Get all entity IDs for this domain
  const counts = await registry.getEntityCounts(domainSlug) as Array<{ nounType: string; count: number }>
  const facts: Record<string, Array<{ factTypeId: string; bindings: Array<[string, string]> }>> = {}

  for (const { nounType } of counts) {
    const ids: string[] = await registry.getEntityIds(nounType, domainSlug)

    // Fan out to read entity data
    const entities = await Promise.allSettled(
      ids.map(async (id) => {
        const entity = await getStub(id).get()
        return entity && !entity.deletedAt ? entity : null
      }),
    )

    for (const result of entities) {
      if (result.status !== 'fulfilled' || !result.value) continue
      const entity = result.value

      // For each data field, create a fact binding:
      // fact_type = "NounType has FieldName", bindings = [(NounType, entityId), (FieldName, value)]
      for (const [field, value] of Object.entries(entity.data || {})) {
        if (field.startsWith('_')) continue // skip system fields
        if (typeof value !== 'string' && typeof value !== 'number' && typeof value !== 'boolean') continue

        const ftId = `${nounType}_${field}`
        if (!facts[ftId]) facts[ftId] = []
        facts[ftId].push({
          factTypeId: ftId,
          bindings: [[nounType, entity.id], [field, String(value)]],
        })
      }
    }
  }

  return JSON.stringify({ facts })
}

// ── Federation: External System resolution ──────────────────────────
// When a noun is backed by an External System, resolve its population
// from the existing service that already has the connection.
// The engine never calls backing stores directly. It calls the existing
// services that already have the connections and credentials.

export interface ServiceEndpoint {
  /** The existing service to call */
  service: string
  /** Full URL or path template. Use {id} for entity-specific lookups. */
  url: string
  /** Auth header name (default: X-API-Key) */
  authHeader?: string
  /** How to extract items from the response */
  responsePath?: string
  /** Map response fields to noun fields (response_field → noun_field) */
  fieldMap?: Record<string, string>
}

export interface FederatedSource {
  /** Noun name → service endpoint that serves it */
  endpoints: Record<string, ServiceEndpoint>
  /** Resolve a secret/API key for a service from DO storage */
  resolveSecret?: (service: string) => Promise<string | null>
}

/**
 * Fetch population facts for a noun from the service that already serves it.
 */
async function resolveFromService(
  nounType: string,
  endpoint: ServiceEndpoint,
  secret: string | null,
): Promise<Record<string, Array<{ factTypeId: string; bindings: Array<[string, string]> }>>> {
  const facts: Record<string, Array<{ factTypeId: string; bindings: Array<[string, string]> }>> = {}

  try {
    const headers: Record<string, string> = { 'Content-Type': 'application/json' }
    if (secret) {
      const headerName = endpoint.authHeader ?? 'X-API-Key'
      headers[headerName] = secret
    }

    const response = await fetch(endpoint.url, { headers })
    if (!response.ok) return facts

    const raw = await response.json() as any

    // Navigate response path (e.g., "data" or "data.subscriptions")
    let data = raw
    if (endpoint.responsePath) {
      for (const key of endpoint.responsePath.split('.')) {
        data = data?.[key]
      }
    }

    const items = Array.isArray(data) ? data
      : data && typeof data === 'object' ? [data]
      : []

    const fieldMap = endpoint.fieldMap ?? {}

    for (const item of items) {
      const entityId = item.id ?? item._id ?? item.vin ?? ''
      for (const [responseField, value] of Object.entries(item)) {
        if (responseField === 'id' || responseField === '_id' || responseField === 'object') continue
        if (typeof value !== 'string' && typeof value !== 'number' && typeof value !== 'boolean') continue
        if (value === null || value === undefined) continue

        const nounField = fieldMap[responseField] ?? responseField
        const ftId = `${nounType}_${nounField}`
        if (!facts[ftId]) facts[ftId] = []
        facts[ftId].push({
          factTypeId: ftId,
          bindings: [[nounType, String(entityId)], [nounField, String(value)]],
        })
      }

      // Flatten nested objects one level (e.g., vehicle.year → year)
      for (const [key, nested] of Object.entries(item)) {
        if (nested && typeof nested === 'object' && !Array.isArray(nested)) {
          for (const [subField, subValue] of Object.entries(nested as Record<string, unknown>)) {
            if (typeof subValue !== 'string' && typeof subValue !== 'number' && typeof subValue !== 'boolean') continue
            const nounField = fieldMap[`${key}.${subField}`] ?? subField
            const ftId = `${nounType}_${nounField}`
            if (!facts[ftId]) facts[ftId] = []
            facts[ftId].push({
              factTypeId: ftId,
              bindings: [[nounType, String(entityId)], [nounField, String(subValue)]],
            })
          }
        }
      }
    }
  } catch {
    // Service unavailable — return empty facts, don't fail
  }

  return facts
}

/**
 * Build a federated population from EntityDB + existing services.
 * Local entities come from DOs. Service-backed nouns come from
 * the existing services that already serve them.
 */
async function buildFederatedPopulation(
  registry: any,
  getStub: (id: string) => any,
  domainSlug: string,
  federation?: FederatedSource,
): Promise<string> {
  // Start with local EntityDB population
  const localPopJson = await buildPopulation(registry, getStub, domainSlug)
  if (!federation || Object.keys(federation.endpoints).length === 0) {
    return localPopJson
  }

  const localPop = JSON.parse(localPopJson) as { facts: Record<string, any[]> }

  // Resolve service-backed nouns in parallel
  const serviceResults = await Promise.allSettled(
    Object.entries(federation.endpoints).map(async ([nounType, endpoint]) => {
      const secret = federation.resolveSecret
        ? await federation.resolveSecret(endpoint.service)
        : null
      return resolveFromService(nounType, endpoint, secret)
    }),
  )

  // Merge service facts into the population
  for (const result of serviceResults) {
    if (result.status !== 'fulfilled') continue
    for (const [ftId, serviceFacts] of Object.entries(result.value)) {
      if (!localPop.facts[ftId]) localPop.facts[ftId] = []
      localPop.facts[ftId].push(...serviceFacts)
    }
  }

  return JSON.stringify(localPop)
}

/**
 * Load domain schema and build live population in one call.
 * Supports federation: nouns backed by External Systems are resolved
 * from those systems, not from EntityDB.
 */
export async function loadDomainAndPopulation(
  registry: any,
  getStub: (id: string) => any,
  domainSlug: string,
  federation?: FederatedSource,
): Promise<string> {
  await loadDomainSchema(registry, getStub, domainSlug)
  return buildFederatedPopulation(registry, getStub, domainSlug, federation)
}

/**
 * Prepare entity creation — single function application.
 * Returns initial state, violations, derived facts, and fact-triggered event.
 */
export function prepareEntity(
  nounName: string,
  fields: Record<string, unknown>,
  populationJson: string,
): {
  initialState: string | null
  violations: Array<{ constraintId: string; constraintText: string; detail: string }>
  derivedFacts: Array<{ factTypeId: string; reading: string; bindings: Array<[string, string]> }>
  factEvent: { eventName: string; factTypeId: string } | null
} {
  ensureWasm()
  const resultJson = prepare_entity(nounName, JSON.stringify(fields), populationJson)
  return JSON.parse(resultJson)
}

/**
 * AREST: Apply a command to the current population.
 * One function application. One state transfer.
 */
export function applyCommand(
  command: { type: string; [key: string]: unknown },
  populationJson: string,
): {
  entities: Array<{ id: string; type: string; data: Record<string, string> }>
  status: string | null
  transitions: Array<{ event: string; targetStatus: string; method: string; href: string }>
  violations: Array<{ constraintId: string; constraintText: string; detail: string }>
  derivedCount: number
  rejected: boolean
  population: any
} {
  ensureWasm()
  const population = JSON.parse(populationJson)
  const result = apply_command_wasm(command, population)
  return result
}

/**
 * Evaluate constraints against a response + population.
 * Returns violations.
 */
export function evaluateConstraints(
  responseText: string,
  populationJson: string,
): Array<{ constraintId: string; constraintText: string; detail: string }> {
  ensureWasm()
  const responseJson = JSON.stringify({ text: responseText, senderIdentity: null, fields: null })
  const resultJson = evaluate_response(responseJson, populationJson)
  return JSON.parse(resultJson)
}

/**
 * Forward chain derivation rules against a population.
 * Returns derived facts.
 */
export function forwardChain(
  populationJson: string,
): Array<{ factTypeId: string; reading: string; bindings: Array<[string, string]>; derivedBy: string }> {
  ensureWasm()
  const resultJson = forward_chain_population(populationJson)
  return JSON.parse(resultJson)
}

/**
 * Run a state machine by folding events through its transition function.
 * Returns the final state name.
 */
export function runStateMachine(
  nounName: string,
  events: Array<[string, string]>,
  populationJson: string,
): string {
  ensureWasm()
  const resultJson = run_machine_wasm(nounName, JSON.stringify(events), populationJson)
  return JSON.parse(resultJson)
}

/**
 * Get valid transitions from a compiled state machine for a given status.
 * Returns array of { from, to, event }.
 */
export function getTransitions(
  nounName: string,
  currentStatus: string,
): Array<{ from: string; to: string; event: string }> {
  ensureWasm()
  const resultJson = get_transitions_wasm(nounName, currentStatus)
  return JSON.parse(resultJson)
}

/**
 * Resolve what event should fire when a fact of the given type is created.
 * Returns { factTypeId, eventName, targetNoun } or null.
 */
export function resolveFactEvent(
  factTypeId: string,
): { factTypeId: string; eventName: string; targetNoun: string } | null {
  ensureWasm()
  const resultJson = resolve_fact_event(factTypeId)
  const result = JSON.parse(resultJson)
  return result === null ? null : result
}

/**
 * Query a fact type's population using partial application.
 * Returns matching entity references.
 */
export function querySchema(
  schemaId: string,
  targetRole: number,
  filterBindings: Array<[number, string]>,
  populationJson: string,
): { matches: string[]; count: number } {
  ensureWasm()
  const resultJson = query_schema_wasm(schemaId, targetRole, JSON.stringify(filterBindings), populationJson)
  return JSON.parse(resultJson)
}

/**
 * Load the validation model (compiled from core.md + validation.md).
 * Called once at startup. Persists across domain loads.
 */
export function loadValidationModel(irJson: string): void {
  ensureWasm()
  load_validation_model(irJson)
}

/**
 * Validate a domain IR against the validation model.
 * The validation model must be loaded first via loadValidationModel().
 */
export function validateSchema(domainIrJson: string): Array<{ constraint_id: string; constraint_text: string; detail: string }> {
  ensureWasm()
  const resultJson = validate_schema_wasm(domainIrJson)
  return JSON.parse(resultJson)
}

// ── Fact Projection ────────────────────────────────────────────────

export interface ProjectedFact {
  schemaId: string
  reading: string
  bindings: Array<[string, string]>
}

export interface FieldSchemaMapping {
  fieldName: string
  schemaId: string
  reading: string
  roleNames: string[]
}

/**
 * α(project) applied to a 3NF entity row.
 *
 * Maps each field to its compiled graph schema (Construction of Selectors)
 * and produces facts with proper schema references. Fields without a
 * compiled schema get provisional IDs (reading format).
 *
 * This is the projection half of the REST GET function:
 *   GET = α(project) ∘ load(DO[id])
 */
export function projectEntity(
  nounName: string,
  entityId: string,
  fields: Record<string, string>,
): ProjectedFact[] {
  ensureWasm()
  return project_entity_wasm(nounName, entityId, fields)
}

/**
 * Get the field-to-schema mapping for a noun.
 * Returns all compiled graph schemas where this noun plays role 0,
 * keyed by the field name (role 1's noun name).
 */
export function getNounSchemas(nounName: string): FieldSchemaMapping[] {
  ensureWasm()
  return get_noun_schemas_wasm(nounName)
}

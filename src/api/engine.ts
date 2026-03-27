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

import { initSync, load_ir, evaluate_response, forward_chain_population, run_machine_wasm, query_schema_wasm, get_transitions_wasm, resolve_fact_event, prepare_entity, apply_command_wasm, debug_compiled_state } from '../../crates/fol-engine/pkg/fol_engine.js'
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

/**
 * Load domain schema and build live population in one call.
 * Returns the population JSON string.
 */
export async function loadDomainAndPopulation(
  registry: any,
  getStub: (id: string) => any,
  domainSlug: string,
): Promise<string> {
  await loadDomainSchema(registry, getStub, domainSlug)
  return buildPopulation(registry, getStub, domainSlug)
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
} {
  ensureWasm()
  const commandJson = JSON.stringify(command)
  console.log('AREST applyCommand input:', commandJson.slice(0, 300))
  const resultJson = apply_command_wasm(commandJson, populationJson)
  console.log('AREST applyCommand output:', resultJson.slice(0, 300))
  const result = JSON.parse(resultJson)
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

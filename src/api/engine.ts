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

import { initSync, load_ir, evaluate_response, forward_chain_population, run_machine_wasm, query_schema_wasm, get_transitions_wasm, resolve_fact_event, prepare_entity, apply_command_wasm, debug_compiled_state, load_validation_model, validate_schema_wasm, project_entity_wasm, get_noun_schemas_wasm, parse_readings_wasm, parse_readings_with_nouns_wasm, rmap_wasm } from '../../crates/fol-engine/pkg/fol_engine.js'
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

  // Read the compiled IR cell — compile(parse(readings)) stored during seeding.
  // One cell, one fetch. No reconstruction from parts.
  const irCellId = `ir:${domainSlug}`
  const cell = await getStub(irCellId).get()
  if (!cell?.data?.ir) return

  const irJson = typeof cell.data.ir === 'string' ? cell.data.ir : JSON.stringify(cell.data.ir)
  load_ir(irJson)
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
export async function buildPopulation(
  registry: any,
  getStub: (id: string) => any,
  domainSlug: string,
): Promise<string> {
  // Build a lookup from (nounType, fieldName) → Graph Schema ID.
  // The IR has fact types like {id: "User_has_Org_Role", roles: [{nounName: "User"}, {nounName: "Org Role"}]}.
  // Entity fields are camelCase ("orgRole"). Role noun names are title case ("Org Role").
  // We match by normalizing both to lowercase.
  const irCell = await getStub(`ir:${domainSlug}`).get().catch(() => null)
  const ir = irCell?.data?.ir
    ? JSON.parse(typeof irCell.data.ir === 'string' ? irCell.data.ir : JSON.stringify(irCell.data.ir))
    : null

  const ftLookup = new Map<string, string>()
  Object.entries(ir?.factTypes || {}).forEach(([ftId, ft]: [string, any]) => {
    const firstRole = (ft.roles?.[0]?.nounName || '').toLowerCase()
    // Index by (firstRole, eachRoleNoun)
    ;(ft.roles || []).forEach((role: any) => {
      ftLookup.set(`${firstRole}:${role.nounName.toLowerCase()}`, ftId)
    })
    // Also index by (firstRole, verb) extracted from the reading
    // Reading format: "NounA verb NounB" → extract verb between first and second noun
    const reading = (ft.reading || ftId) as string
    const roles = (ft.roles || []) as Array<{ nounName: string }>
    if (roles.length >= 2) {
      const r = reading
      const first = roles[0].nounName
      const second = roles[roles.length - 1].nounName
      const afterFirst = r.indexOf(first) >= 0 ? r.substring(r.indexOf(first) + first.length).trim() : ''
      const beforeSecond = second && afterFirst.indexOf(second) >= 0 ? afterFirst.substring(0, afterFirst.indexOf(second)).trim() : afterFirst
      const verb = beforeSecond.replace(/\.$/, '').trim()
      ftLookup.set(`${firstRole}:${verb.toLowerCase()}`, ftId)
    }
  })

  const resolveFactTypeId = (nounType: string, fieldName: string): string => {
    // Try exact match: nounType + fieldName as role noun or verb (case-insensitive)
    const key = `${nounType.toLowerCase()}:${fieldName.toLowerCase()}`
    const match = ftLookup.get(key)
    if (match) return match
    // Fallback: try matching camelCase field to space-separated role name
    const spaced = fieldName.replace(/([A-Z])/g, ' $1').trim()
    const spacedKey = `${nounType.toLowerCase()}:${spaced.toLowerCase()}`
    const spacedMatch = ftLookup.get(spacedKey)
    if (spacedMatch) return spacedMatch
    // Last fallback: Graph Schema ID format
    return `${nounType}_has_${fieldName}`
  }

  // Get all entity IDs for this domain
  const counts = await registry.getEntityCounts(domainSlug) as Array<{ nounType: string; count: number }>
  const facts: Record<string, Array<{ factTypeId: string; bindings: Array<[string, string]> }>> = {}

  // Skip schema entities. Only domain entities contribute to the population.
  const schemaTypes = new Set(['Noun', 'Reading', 'Graph Schema', 'Role', 'Constraint', 'CompiledSchema', 'Derivation Rule', 'State Machine Definition', 'Status', 'Transition', 'External System'])
  const entitySettled = await Promise.allSettled(
    counts.filter(({ nounType }) => !schemaTypes.has(nounType)).flatMap(({ nounType }) =>
      registry.getEntityIds(nounType, domainSlug).then((ids: string[]) =>
        Promise.allSettled(ids.map(async (id: string) => {
          const entity = await getStub(id).get()
          return entity ? { ...entity, nounType } : null
        }))
      )
    )
  )

  // Flatten and process all entities
  entitySettled
    .filter((r): r is PromiseFulfilledResult<any> => r.status === 'fulfilled')
    .flatMap(r => r.value)
    .filter((r: any): r is PromiseFulfilledResult<any> => r.status === 'fulfilled' && r.value)
    .map((r: any) => r.value)
    .forEach((entity: any) => {
      Object.entries(entity.data || {}).forEach(([field, value]) => {
        if (field.startsWith('_')) return
        if (typeof value !== 'string' && typeof value !== 'number' && typeof value !== 'boolean') return

        const ftId = resolveFactTypeId(entity.nounType || entity.type, field)
        const list = facts[ftId] || []
        list.push({
          factTypeId: ftId,
          bindings: [[entity.nounType || entity.type, entity.id], [field, String(value)]],
        })
        facts[ftId] = list
      })
    })

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
        const ftId = `${nounType}_has_${nounField}`
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
            const ftId = `${nounType}_has_${nounField}`
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

/**
 * Determine which nouns are top-level from the compiled IR.
 * A noun is top-level if:
 * 1. It has objectType "entity" (not value, not abstract)
 * 2. No MC constraint makes it existentially dependent on another entity
 *
 * This uses the compiled IR directly — one cell, no fan-out.
 */
export function getTopLevelNouns(irJson: string): { topLevel: Set<string>; dependsOn: Map<string, string> } {
  const ir = JSON.parse(irJson)
  const topLevel = new Set<string>()
  const dependsOn = new Map<string, string>()

  // Find MC constraints — they express existential dependency
  const mcFactTypes = new Set<string>()
  for (const c of ir.constraints || []) {
    if (c.kind === 'MC') {
      for (const span of c.spans || []) {
        mcFactTypes.add(span.factTypeId || span.fact_type_id || '')
      }
    }
  }

  // For each MC constraint, find the dependent noun.
  // The constraint text says "Each X ... exactly one Y" — X is the constrained noun.
  // Look up the fact type roles, matching by ID or by noun overlap.
  const allFts = ir.factTypes || ir.fact_types || {}
  for (const ftId of mcFactTypes) {
    let ft = allFts[ftId]
    // If no direct match, find by noun overlap (inverse readings)
    if (!ft || !ft.roles || ft.roles.length < 2) {
      ft = Object.values(allFts).find((f: any) => {
        const roles = f.roles || []
        if (roles.length < 2) return false
        const r0 = roles[0]?.nounName || roles[0]?.noun_name || ''
        const r1 = roles[1]?.nounName || roles[1]?.noun_name || ''
        return ftId.includes(r0) && ftId.includes(r1)
      }) as any
    }
    if (!ft?.roles || ft.roles.length < 2) continue

    // The constrained noun is the one mentioned FIRST in the MC constraint text
    // which matches the noun at role 0 of the canonical fact type OR
    // the noun at role 1 if the constraint uses an inverse reading.
    // Find which noun in the fact type also appears first in the constraint's factTypeId
    const role0 = ft.roles[0]?.nounName || ft.roles[0]?.noun_name || ''
    const role1 = ft.roles[1]?.nounName || ft.roles[1]?.noun_name || ''

    // In "Each Message belongs to exactly one Support Request",
    // Message is the constrained noun (dependent), Support Request is the target.
    // The factTypeId starts with the constrained noun.
    const constrainedNoun = ftId.indexOf(role0) < ftId.indexOf(role1) ? role0 : role1
    const targetNoun = constrainedNoun === role0 ? role1 : role0

    if (constrainedNoun && targetNoun && constrainedNoun !== targetNoun) {
      const targetDef = (ir.nouns || {})[targetNoun]
      if (targetDef && (targetDef.objectType || targetDef.object_type) !== 'value') {
        dependsOn.set(constrainedNoun, targetNoun)
      }
    }
  }

  // Trace subtype chains
  for (const [name, def] of Object.entries(ir.nouns || {})) {
    const superType = (def as any).superType || (def as any).super_type
    if (superType && dependsOn.has(superType) && !dependsOn.has(name)) {
      dependsOn.set(name, dependsOn.get(superType)!)
    }
  }

  // Top-level = entity nouns that are not dependent
  for (const [name, def] of Object.entries(ir.nouns || {})) {
    const ot = (def as any).objectType || (def as any).object_type || ''
    if (ot === 'entity' && !dependsOn.has(name)) {
      topLevel.add(name)
    }
  }

  return { topLevel, dependsOn }
}

// ── Parse Readings via ρ ─────────────────────────────────────────────
// Per Theorem 2 (Specification Equivalence): parse: R → Φ.
// The Rust WASM engine is the ONLY parser. No TS parsing.

/**
 * Parse FORML 2 markdown readings into entities ready for materialization.
 * Returns an array of { id, type, domain, data } — cells for D.
 */
export function parseReadings(markdown: string, domain: string): Array<{ id: string; type: string; domain: string; data: Record<string, unknown> }> {
  ensureWasm()
  const result = parse_readings_wasm(markdown, domain)
  return typeof result === 'string' ? JSON.parse(result) : result
}

/**
 * Parse readings with cross-domain noun context.
 * Existing nouns from previously seeded domains are passed as JSON
 * so the parser resolves cross-domain references.
 */
export function parseReadingsWithNouns(
  markdown: string,
  domain: string,
  existingNounsJson: string,
): Array<{ id: string; type: string; domain: string; data: Record<string, unknown> }> {
  ensureWasm()
  const result = parse_readings_with_nouns_wasm(markdown, domain, existingNounsJson)
  return typeof result === 'string' ? JSON.parse(result) : result
}

// ── RMAP: Relational Mapping Procedure ───────────────────────────────
// RMAP (Halpin, Ch. 17) determines cell partitioning from UC structure.
// Each entity is a cell. Value-type fields are absorbed into entity tables.

export interface RmapColumn {
  name: string
  type: string
  nullable: boolean
  references?: string
}

export interface RmapTable {
  name: string
  columns: RmapColumn[]
  primaryKey: string[]
  checks?: string[]
}

/**
 * Compute RMAP table definitions from the loaded IR.
 * RMAP (Halpin, Ch. 17) determines cell partitioning from UC structure.
 */
export function computeRMAP(): RmapTable[] {
  ensureWasm()
  return rmap_wasm() as unknown as RmapTable[]
}

// ── Self-Describing Representations ──────────────────────────────────
// Per Theorem 4 (Derivability): every domain value v = (ρf):P.
// The representation includes _view metadata derived from readings in P.
// The client is a pure renderer — no procedural layout decisions.

/**
 * Derive the view metadata for an entity type from the population.
 * Queries Reading and Constraint cells to build the _view descriptor.
 *
 * A graph schema ⟨CONS, s₁, …, sₙ⟩ applied to roles gives the columns.
 * Constraints give validation rules. State machines give transitions.
 */
// DEPRECATED: replaced by deriveSchema in hateoas.ts
export async function deriveViewMetadata(
  registry: any,
  getStub: (id: string) => any,
  nounName: string,
  domainSlug: string,
): Promise<{
  type: string
  title: string
  fields: Array<{ name: string; required: boolean; role: string }>
  constraints: Array<{ text: string; kind: string; modality: string }>
  topLevel: boolean
  parent?: string
  children: string[]
  rmap: RmapTable | null
}> {
  // Query Reading cells for this noun's fact types
  const readingIds: string[] = await registry.getEntityIds('Reading', domainSlug)
  const readingSettled = await Promise.allSettled(
    readingIds.map(async (id: string) => {
      const cell = await getStub(id).get()
      return cell ? { id: cell.id, ...cell.data } : null
    }),
  )
  const readings = readingSettled
    .filter((r): r is PromiseFulfilledResult<any> => r.status === 'fulfilled' && r.value)
    .map(r => r.value)

  // Find readings that mention this noun — these define the fields
  const nounReadings = readings.filter((r: any) => {
    const text = r.text || ''
    return text.includes(nounName)
  })

  // Derive fields from readings: "Noun has FieldName" → field
  const fields: Array<{ name: string; required: boolean; role: string }> = []
  for (const r of nounReadings) {
    const text = r.text || ''
    // Pattern: "NounName has FieldName" or "Each NounName has exactly one FieldName"
    const hasMatch = text.match(new RegExp(`${nounName}\\s+has\\s+(.+?)$`, 'i'))
    if (hasMatch) {
      const fieldName = hasMatch[1].trim()
      fields.push({ name: fieldName, required: text.includes('exactly one'), role: 'attribute' })
    }
    // Pattern: "NounName is from OtherNoun" / "NounName belongs to OtherNoun"
    const relMatch = text.match(new RegExp(`${nounName}\\s+(?:is from|belongs to|is for|is of)\\s+(.+?)$`, 'i'))
    if (relMatch) {
      const fieldName = relMatch[1].trim()
      fields.push({ name: fieldName, required: text.includes('exactly one'), role: 'reference' })
    }
  }

  // Query Constraint cells
  const constraintIds: string[] = await registry.getEntityIds('Constraint', domainSlug)
  const constraintSettled = await Promise.allSettled(
    constraintIds.slice(0, 50).map(async (id: string) => {
      const cell = await getStub(id).get()
      return cell ? { id: cell.id, ...cell.data } : null
    }),
  )
  const constraints = constraintSettled
    .filter((r): r is PromiseFulfilledResult<any> => r.status === 'fulfilled' && r.value)
    .map(r => r.value)
    .filter((c: any) => {
      const text = (c.text || c.reading || '').toString()
      return text.includes(nounName)
    })
    .map((c: any) => ({
      text: c.text || c.reading || '',
      kind: c.kind || c.constraintType || '',
      modality: c.modality || 'Alethic',
    }))

  // Derive hierarchy from compiled IR — one cell fetch, no fan-out.
  // MC constraints determine existential dependency. Subtype chains propagate.
  const irCellId = `ir:${domainSlug}`
  const irCell = await getStub(irCellId).get()
  const irJson = irCell?.data?.ir as string || '{}'

  const { topLevel: topLevelSet, dependsOn } = getTopLevelNouns(irJson)
  const isTopLevel = topLevelSet.has(nounName)
  const parent = dependsOn.get(nounName)
  const children = [...dependsOn.entries()]
    .filter(([_, p]) => p === nounName)
    .map(([child]) => child)

  // RMAP: compute cell partitioning from UC structure (Halpin, Ch. 17).
  // The IR must already be loaded (done above via irCell fetch triggering load_ir).
  const rmapTables = computeRMAP()
  const nounTable = rmapTables.find(t => t.name === nounName) ?? null

  return {
    type: 'ListView',
    title: nounName,
    fields,
    constraints: constraints.map((c: any) => ({
      text: c.text || c.reading || '',
      kind: c.kind || c.constraintType || '',
      modality: c.modality || 'Alethic',
    })),
    topLevel: !parent,
    parent,
    children,
    rmap: nounTable,
  }
}

/**
 * Derive navigation context from access rules.
 * Returns the domains and apps visible to the user, plus the current breadcrumb.
 */
// DEPRECATED: replaced by HATEOAS links at each level
export async function deriveNavContext(
  registry: any,
  getStub: (id: string) => any,
  userEmail: string,
  currentDomain?: string,
  currentNoun?: string,
): Promise<{
  domains: string[]
  apps: Array<{ id: string; name: string; navigableDomains: string[] }>
  breadcrumb: string[]
}> {
  const { accessibleDomains, visibleApps } = await evaluateAccess(registry, getStub, userEmail)
  const breadcrumb: string[] = []
  if (currentDomain) breadcrumb.push(currentDomain)
  if (currentNoun) breadcrumb.push(currentNoun)

  return {
    domains: [...accessibleDomains],
    apps: visibleApps,
    breadcrumb,
  }
}

// ── Access Control via Derivation Rules ──────────────────────────────
// Per organizations.md:
//   "User accesses Domain iff User has Org Role in Organization and Domain belongs to that Organization."
//   "User accesses Domain if Domain has Visibility 'public'."
//
// The derivation rules are evaluated by ρ. The user email is part of input I.
// This function builds the access population from org/user/domain cells,
// forward-chains derivation rules, and returns which domains the user can access.

/**
 * Evaluate access control for a user by building a population from
 * Organization, User, App, and Domain cells, then forward-chaining
 * the access derivation rules.
 *
 * Returns the set of domain slugs the user can access.
 */
// DEPRECATED: replaced by /arest/ root resource
export async function evaluateAccess(
  registry: any,
  getStub: (id: string) => any,
  userEmail: string,
): Promise<{
  accessibleDomains: Set<string>
  userOrgs: Array<{ orgId: string; orgName: string; role: string }>
  visibleApps: Array<{ id: string; name: string; slug: string; organization: string; navigableDomains: string[] }>
}> {
  // Build population from the organizations domain.
  // The derivation rules in organizations.md determine access.
  try { await loadDomainSchema(registry, getStub, 'organizations') } catch {}
  const popJson = await buildPopulation(registry, getStub, 'organizations')

  // Forward-chain the derivation rules over P to derive access facts.
  let derived: Array<{ factTypeId: string; reading: string; bindings: Array<[string, string]>; derivedBy: string }> = []
  try { derived = forwardChain(popJson) } catch {}

  // Parse population for Graph Schema ID keyed facts.
  const pop = JSON.parse(popJson) as { facts: Record<string, Array<{ factTypeId: string; bindings: Array<[string, string]> }>> }

  // Helper: find binding value by noun name in a fact
  const bindVal = (fact: { bindings: Array<[string, string]> }, noun: string) =>
    fact.bindings.find(([k]) => k === noun)?.[1] || ''

  // Find User entity matching the authenticated email
  const userFacts = pop.facts['User_has_Email'] || []
  const userId = bindVal(
    userFacts.find(f => f.bindings.some(([, v]) => v === userEmail)) || { bindings: [] },
    'User',
  )

  // Extract org memberships from binary role fact types
  const roleFactTypes = ['User_owns_Organization', 'User_administers_Organization', 'User_belongs_to_Organization']
  const roleMap: Record<string, string> = {
    'User_owns_Organization': 'owner',
    'User_administers_Organization': 'admin',
    'User_belongs_to_Organization': 'member',
  }
  const userOrgs = roleFactTypes.flatMap(ftId =>
    (pop.facts[ftId] || [])
      .filter(f => bindVal(f, 'User') === userId)
      .map(f => {
        const orgId = bindVal(f, 'owns') || bindVal(f, 'administers') || bindVal(f, 'Organization') || ''
        const orgNameFacts = pop.facts['Organization_has_Name'] || []
        const orgName = bindVal(
          orgNameFacts.find(nf => bindVal(nf, 'Organization') === orgId) || { bindings: [] },
          'Name',
        ) || orgId
        return { orgId, orgName, orgSlug: orgId, role: roleMap[ftId] || 'member' }
      })
  )

  // Accessible domains from derived facts or public visibility
  const accessibleDomains = new Set<string>()
  const allDomainSlugs = await registry.listDomains() as string[]

  derived
    .filter(fact => fact.factTypeId === 'User_accesses_Domain')
    .forEach(fact => {
      const domain = fact.bindings.find(([k]: [string, string]) => k === 'Domain')?.[1]
      domain && accessibleDomains.add(domain)
    })

  // Fallback: public visibility
  if (accessibleDomains.size === 0) {
    const visFacts = pop.facts['Domain_has_Visibility'] || []
    allDomainSlugs.forEach(slug => {
      const vis = visFacts.find(f => bindVal(f, 'Domain') === slug)
      const visibility = vis ? bindVal(vis, 'Visibility') : null
      if (visibility === 'public' || !vis) accessibleDomains.add(slug)
    })
  }

  // Visible apps: filter by org membership
  const appOrgFacts = pop.facts['App_belongs_to_Organization'] || []
  const appNameFacts = pop.facts['App_has_Name'] || []
  const orgSlugs = new Set(userOrgs.map(uo => uo.orgSlug))

  const visibleApps = await Promise.all(
    appOrgFacts
      .filter(f => orgSlugs.has(bindVal(f, 'Organization') || bindVal(f, 'belongs to')))
      .map(async f => {
        const appId = bindVal(f, 'App')
        const name = bindVal(
          appNameFacts.find(nf => bindVal(nf, 'App') === appId) || { bindings: [] },
          'Name',
        ) || appId
        const appCell = await getStub(appId).get().catch(() => null)
        const slug = appCell?.data?.appSlug || appCell?.data?.slug || appId
        const nd = appCell?.data?.navigableDomains
        const navDomains = Array.isArray(nd) ? nd : (typeof nd === 'string' ? [nd] : [])
        return { id: appId, name, slug, organization: bindVal(f, 'Organization') || bindVal(f, 'belongs to'), navigableDomains: navDomains }
      })
  )

  return { accessibleDomains, userOrgs, visibleApps }
}

import { json, error } from 'itty-router'
import type { Env } from '../types'
import { persistViolations } from '../worker/outcomes'
// @ts-ignore — WASM module imported by wrangler's CompiledWasm rule
import wasmModule from '../../crates/fol-engine/pkg/fol_engine_bg.wasm'
// @ts-ignore — WASM JS bindings (web target with initSync)
import { initSync, load_ir, evaluate_response as wasmEvaluate, synthesize_noun as wasmSynthesize } from '../../crates/fol-engine/pkg/fol_engine.js'

// Initialize WASM module
let wasmInitialized = false
function initWasm() {
  if (wasmInitialized) return
  try {
    // Cloudflare Workers provide a WebAssembly.Module from the import
    // initSync from wasm-pack web target accepts a Module directly
    initSync({ module: wasmModule })
    wasmInitialized = true
  } catch { /* WASM not available */ }
}

function getDB(env: Env): DurableObjectStub {
  const id = env.DOMAIN_DB.idFromName('graphdl-primary')
  return env.DOMAIN_DB.get(id)
}

export async function handleEvaluate(request: Request, env: Env): Promise<Response> {
  if (request.method !== 'POST') {
    return error(405, { errors: [{ message: 'Method not allowed' }] })
  }

  const body = await request.json() as {
    domainId?: string
    response?: { text: string; senderIdentity?: string; fields?: Record<string, string> }
    population?: { facts: Record<string, any[]> }
  }

  if (!body.domainId) {
    return error(400, { errors: [{ message: 'domainId is required' }] })
  }
  if (!body.response?.text) {
    return error(400, { errors: [{ message: 'response.text is required' }] })
  }

  const db = getDB(env) as any

  // Load domain schema from generators collection
  const genResult = await db.findInCollection('generators', {
    domain: { equals: body.domainId },
    outputFormat: { equals: 'schema' },
  }, { limit: 1 })

  const schemaOutput = genResult?.docs?.[0]?.output
  if (!schemaOutput) {
    return json({
      violations: [],
      constraintCount: 0,
      domainId: body.domainId,
      warning: 'No domain schema generated for this domain. Run POST /api/generate with outputFormat: "schema" first.',
    })
  }

  let domainSchema: any
  try {
    domainSchema = typeof schemaOutput === 'string' ? JSON.parse(schemaOutput) : schemaOutput
  } catch {
    return error(500, { errors: [{ message: 'Failed to parse domain schema' }] })
  }

  // Deterministic text check — string matching against enum values
  const { checkDeterministicText, buildTextConstraints } = await import('../worker/deterministic-text-check')
  let textViolations: any[] = []
  try {
    const registry = env.REGISTRY_DB.get(env.REGISTRY_DB.idFromName('global')) as any
    // Resolve domain slug — entities are indexed by slug, not UUID
    let domainSlug = body.domainId!
    try {
      const slugResult: string | null = await registry.resolveSlugByUUID(body.domainId!)
      if (slugResult) domainSlug = slugResult
    } catch { /* use domainId as-is */ }
    const constraintIds: string[] = await registry.getEntityIds('Constraint', domainSlug)
    const nounIds: string[] = await registry.getEntityIds('Noun', domainSlug)
    const [constraintEntities, nounEntities] = await Promise.all([
      Promise.all(constraintIds.map(id => (env.ENTITY_DB.get(env.ENTITY_DB.idFromName(id)) as any).get())),
      Promise.all(nounIds.map(id => (env.ENTITY_DB.get(env.ENTITY_DB.idFromName(id)) as any).get())),
    ])
    const textConstraints = buildTextConstraints(
      constraintEntities.filter(Boolean),
      nounEntities.filter(Boolean),
    )
    if (textConstraints.length > 0 && body.response?.text) {
      textViolations = checkDeterministicText(body.response.text, textConstraints)
    }
  } catch { /* best-effort — continue to FOL evaluation */ }

  // Try WASM evaluation
  try {
    initWasm()
    if (!wasmInitialized) throw new Error('WASM not initialized')
    load_ir(JSON.stringify(domainSchema))

    const population = body.population || { facts: {} }
    const responseCtx = {
      text: body.response.text,
      senderIdentity: body.response.senderIdentity || null,
      fields: body.response.fields || null,
    }

    const violationJson = wasmEvaluate(
      JSON.stringify(responseCtx),
      JSON.stringify(population),
    )
    const violations = JSON.parse(violationJson)

    const mappedViolations = violations.map((v: any) => ({
      constraintId: v.constraintId,
      constraintText: v.constraintText,
      detail: v.detail,
    }))

    // Best-effort: persist violations as EntityDB DOs (don't block response)
    if (violations.length > 0) {
      persistViolations(env, violations.map((v: any) => ({
        domain: body.domainId!,
        constraintId: v.constraintId ?? null,
        text: v.detail || v.constraintText || 'Constraint violation',
        triggeredByResourceId: v.resourceId ?? undefined,
      }))).catch(() => { /* swallow — best-effort persistence */ })
    }

    // Merge text violations with FOL violations
    const allViolations = [
      ...mappedViolations,
      ...textViolations.map((v: any) => ({
        constraintId: v.constraintId,
        constraintText: v.constraintText,
        detail: `${v.operator}: found '${v.value}' — ${v.evidence}`,
      })),
    ]

    // Persist all violations
    if (allViolations.length > 0) {
      persistViolations(env, allViolations.map((v: any) => ({
        domain: body.domainId!,
        constraintId: v.constraintId ?? null,
        text: v.detail || v.constraintText || 'Constraint violation',
      }))).catch(() => { /* best-effort */ })
    }

    return json({
      violations: allViolations,
      constraintCount: domainSchema.constraints?.length || 0,
      textConstraintsChecked: textViolations.length > 0 ? true : false,
      domainId: body.domainId,
    })
  } catch (wasmErr: any) {
    // WASM not available — still return text violations
    const allViolations = textViolations.map((v: any) => ({
      constraintId: v.constraintId,
      constraintText: v.constraintText,
      detail: `${v.operator}: found '${v.value}' — ${v.evidence}`,
    }))

    return json({
      violations: allViolations,
      constraintCount: domainSchema.constraints?.length || 0,
      textConstraintsChecked: textViolations.length > 0 ? true : false,
      domainId: body.domainId,
      wasmError: wasmErr?.message || String(wasmErr),
      wasmInitialized,
      warning: 'WASM evaluator not available. Text constraints still checked.',
    })
  }
}

// ── Synthesize endpoint ──────────────────────────────────────────────

export async function handleSynthesize(request: Request, env: Env): Promise<Response> {
  if (request.method !== 'POST') {
    return error(405, { errors: [{ message: 'Method not allowed' }] })
  }

  const body = await request.json() as {
    domainId?: string
    nounName?: string
    depth?: number
  }

  if (!body.domainId || !body.nounName) {
    return error(400, { errors: [{ message: 'domainId and nounName are required' }] })
  }

  const db = getDB(env) as any

  // Load domain schema
  const genResult = await db.findInCollection('generators', {
    domain: { equals: body.domainId },
    outputFormat: { equals: 'schema' },
  }, { limit: 1 })

  const schemaOutput = genResult?.docs?.[0]?.output
  if (!schemaOutput) {
    return json({
      error: 'No domain schema generated for this domain.',
      suggestion: 'Run POST /api/generate with outputFormat: "schema" first.',
    })
  }

  let domainSchema: any
  try {
    domainSchema = typeof schemaOutput === 'string' ? JSON.parse(schemaOutput) : schemaOutput
  } catch {
    return error(500, { errors: [{ message: 'Failed to parse domain schema' }] })
  }

  // Try WASM synthesis
  try {
    initWasm()
    if (!wasmInitialized) throw new Error('WASM not initialized')
    load_ir(JSON.stringify(domainSchema))

    const resultJson = wasmSynthesize(body.nounName, body.depth || 2)
    const result = JSON.parse(resultJson)

    return json(result)
  } catch (wasmErr: any) {
    // Fallback: do synthesis in JS if WASM not available
    return json(synthesizeFallback(domainSchema, body.nounName, body.depth || 2))
  }
}

function synthesizeFallback(ir: any, nounName: string, depth: number): any {
  const noun = ir.nouns[nounName]
  if (!noun) return { error: `Noun '${nounName}' not found` }

  // Find fact types where this noun plays a role
  const participatesIn = Object.entries(ir.factTypes)
    .filter(([_, ft]: [string, any]) => ft.roles.some((r: any) => r.nounName === nounName))
    .map(([id, ft]: [string, any]) => ({
      id,
      reading: ft.reading,
      roleIndex: ft.roles.findIndex((r: any) => r.nounName === nounName),
    }))

  // Find constraints spanning those fact types
  const ftIds = new Set(participatesIn.map(p => p.id))
  const applicableConstraints = (ir.constraints || [])
    .filter((c: any) => c.spans.some((s: any) => ftIds.has(s.factTypeId)))
    .map((c: any) => ({
      id: c.id,
      text: c.text,
      kind: c.kind,
      modality: c.modality,
      deonticOperator: c.deonticOperator,
    }))

  // Find state machines
  const stateMachines = Object.values(ir.stateMachines || {})
    .filter((sm: any) => sm.nounName === nounName)

  // Find applicable derivation rules
  const derivationRules = (ir.derivationRules || [])
    .filter((dr: any) =>
      dr.antecedentFactTypeIds.some((id: string) => ftIds.has(id)) ||
      dr.id === `derive-subtype-${nounName}` ||
      dr.id === `derive-cwa-${nounName}`
    )

  // Find related nouns
  const relatedNouns: any[] = []
  for (const p of participatesIn) {
    const ft = ir.factTypes[p.id]
    for (const role of ft.roles) {
      if (role.nounName !== nounName) {
        const relatedNoun = ir.nouns[role.nounName]
        relatedNouns.push({
          name: role.nounName,
          viaFactType: p.id,
          viaReading: ft.reading,
          worldAssumption: relatedNoun?.worldAssumption || 'closed',
        })
      }
    }
  }

  return {
    nounName,
    worldAssumption: noun.worldAssumption || 'closed',
    participatesIn,
    applicableConstraints,
    stateMachines,
    derivationRules,
    derivedFacts: [],
    relatedNouns,
  }
}

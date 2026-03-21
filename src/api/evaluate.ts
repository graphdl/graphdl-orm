import { json, error } from 'itty-router'
import type { Env } from '../types'
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

  // Load constraint IR from generators collection
  const genResult = await db.findInCollection('generators', {
    domain: { equals: body.domainId },
    outputFormat: { equals: 'business-rules' },
  }, { limit: 1 })

  const irOutput = genResult?.docs?.[0]?.output
  if (!irOutput) {
    return json({
      violations: [],
      constraintCount: 0,
      domainId: body.domainId,
      warning: 'No constraint IR generated for this domain. Run POST /api/generate with outputFormat: "business-rules" first.',
    })
  }

  let constraintIR: any
  try {
    constraintIR = typeof irOutput === 'string' ? JSON.parse(irOutput) : irOutput
  } catch {
    return error(500, { errors: [{ message: 'Failed to parse constraint IR' }] })
  }

  // Try WASM evaluation
  try {
    initWasm()
    if (!wasmInitialized) throw new Error('WASM not initialized')
    load_ir(JSON.stringify(constraintIR))

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

    return json({
      violations: violations.map((v: any) => ({
        constraintId: v.constraintId,
        constraintText: v.constraintText,
        detail: v.detail,
      })),
      constraintCount: constraintIR.constraints?.length || 0,
      domainId: body.domainId,
    })
  } catch (wasmErr: any) {
    // WASM not available — return error details for debugging
    return json({
      violations: [],
      constraintCount: constraintIR.constraints?.length || 0,
      domainId: body.domainId,
      wasmError: wasmErr?.message || String(wasmErr),
      wasmInitialized,
      warning: 'WASM evaluator not available.',
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

  // Load constraint IR
  const genResult = await db.findInCollection('generators', {
    domain: { equals: body.domainId },
    outputFormat: { equals: 'business-rules' },
  }, { limit: 1 })

  const irOutput = genResult?.docs?.[0]?.output
  if (!irOutput) {
    return json({
      error: 'No constraint IR generated for this domain.',
      suggestion: 'Run POST /api/generate with outputFormat: "business-rules" first.',
    })
  }

  let constraintIR: any
  try {
    constraintIR = typeof irOutput === 'string' ? JSON.parse(irOutput) : irOutput
  } catch {
    return error(500, { errors: [{ message: 'Failed to parse constraint IR' }] })
  }

  // Try WASM synthesis
  try {
    initWasm()
    if (!wasmInitialized) throw new Error('WASM not initialized')
    load_ir(JSON.stringify(constraintIR))

    const resultJson = wasmSynthesize(body.nounName, body.depth || 2)
    const result = JSON.parse(resultJson)

    return json(result)
  } catch (wasmErr: any) {
    // Fallback: do synthesis in JS if WASM not available
    return json(synthesizeFallback(constraintIR, body.nounName, body.depth || 2))
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

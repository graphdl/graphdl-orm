import { json, error } from 'itty-router'
import type { Env } from '../types'
// @ts-ignore — WASM module imported by wrangler's CompiledWasm rule
import wasmModule from '../../crates/constraint-eval/pkg/constraint_eval_bg.wasm'
// @ts-ignore — WASM JS bindings (web target with initSync)
import { initSync, load_ir, evaluate_response as wasmEvaluate } from '../../crates/constraint-eval/pkg/constraint_eval.js'

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
  const id = env.GRAPHDL_DB.idFromName('graphdl-primary')
  return env.GRAPHDL_DB.get(id)
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

import { json, error } from 'itty-router'
import type { Env } from '../types'

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
    outputFormat: { equals: 'constraint-ir' },
  }, { limit: 1 })

  const irOutput = genResult?.docs?.[0]?.output
  if (!irOutput) {
    return json({
      violations: [],
      constraintCount: 0,
      domainId: body.domainId,
      warning: 'No constraint IR generated for this domain. Run POST /api/generate with outputFormat: "constraint-ir" first.',
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
    // @ts-ignore — dynamic WASM import resolved by wrangler bundler
    const wasmModule = await import('../../crates/constraint-eval/pkg/constraint_eval')
    wasmModule.load_ir(JSON.stringify(constraintIR))

    const population = body.population || { facts: {} }
    const responseCtx = {
      text: body.response.text,
      senderIdentity: body.response.senderIdentity || null,
      fields: body.response.fields || null,
    }

    const violationJson = wasmModule.evaluate_response(
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
  } catch (wasmErr) {
    // WASM not available (dev mode, missing build)
    console.error('WASM constraint evaluation unavailable:', wasmErr)
    return json({
      violations: [],
      constraintCount: constraintIR.constraints?.length || 0,
      domainId: body.domainId,
      warning: 'WASM evaluator not available. Build with: cd crates/constraint-eval && wasm-pack build --target bundler',
    })
  }
}

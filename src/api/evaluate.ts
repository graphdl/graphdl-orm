/**
 * Evaluate and Synthesize endpoints — WASM engine only.
 * Schema is loaded from Entity cells via loadDomainSchema (derived from readings in P).
 * No DomainDB dependency.
 */

import { json, error } from 'itty-router'
import type { Env } from '../types'
import { loadDomainSchema, evaluateConstraints, forwardChain } from './engine'

function getEntityDO(env: Env, id: string) {
  return env.ENTITY_DB.get(env.ENTITY_DB.idFromName(id))
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

  if (!body.domainId) return error(400, { errors: [{ message: 'domainId is required' }] })
  if (!body.response?.text) return error(400, { errors: [{ message: 'response.text is required' }] })

  const registry = env.REGISTRY_DB.get(env.REGISTRY_DB.idFromName('global')) as any
  const getStub = (id: string) => getEntityDO(env, id) as any

  // Load schema from Entity cells — compile(parse(readings)) derived from P
  try {
    await loadDomainSchema(registry, getStub, body.domainId)
  } catch (e) {
    return error(400, { errors: [{ message: `Failed to load schema for domain: ${body.domainId}` }] })
  }

  // Evaluate constraints against the response
  const violations = evaluateConstraints(body.response, body.population || { facts: {} })

  return json({ violations, evaluated: true })
}

export async function handleSynthesize(request: Request, env: Env): Promise<Response> {
  if (request.method !== 'POST') {
    return error(405, { errors: [{ message: 'Method not allowed' }] })
  }

  const body = await request.json() as { domainId?: string; noun?: string; depth?: number }
  if (!body.domainId || !body.noun) return error(400, { errors: [{ message: 'domainId and noun required' }] })

  const registry = env.REGISTRY_DB.get(env.REGISTRY_DB.idFromName('global')) as any
  const getStub = (id: string) => getEntityDO(env, id) as any

  try {
    await loadDomainSchema(registry, getStub, body.domainId)
  } catch {
    return error(400, { errors: [{ message: `Failed to load schema for domain: ${body.domainId}` }] })
  }

  const { synthesize_noun, current_domain_handle } = await import('../../crates/fol-engine/pkg/fol_engine.js')
  const result = synthesize_noun(current_domain_handle() ?? 0, body.noun, body.depth || 1)
  return json(result)
}

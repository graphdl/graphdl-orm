/**
 * Outcomes — Violation and Failure entity creation helpers.
 *
 * Violations and failures are first-class domain entities (EntityDB DOs),
 * not just error responses. Every evaluation path persists its outcomes
 * so they are queryable and auditable.
 *
 * Creation is best-effort: callers should use Promise.allSettled and
 * never let a persistence failure block the response.
 */

import type { Env } from '../types'

// ── Violation ────────────────────────────────────────────────────────

export interface ViolationInput {
  domain: string
  constraintId: string | null
  text: string
  severity?: 'error' | 'warning' | 'info'
  functionId?: string     // "is against Function" — the Function this violation is about
  batchId?: string
  triggeredByResourceId?: string  // "is triggered by Resource" — the specific resource that triggered it
}

/**
 * Create a Violation entity in an EntityDB DO and index it in the Registry.
 * Returns the entity id on success.
 */
export async function createViolation(env: Env, input: ViolationInput): Promise<string> {
  const id = crypto.randomUUID()
  const stub = env.ENTITY_DB.get(env.ENTITY_DB.idFromName(id)) as any
  await stub.put({
    id,
    type: 'Violation',
    data: {
      domain: input.domain,
      constraintId: input.constraintId,
      text: input.text,
      severity: input.severity ?? 'error',
      occurredAt: new Date().toISOString(),
      functionId: input.functionId ?? null,
      batchId: input.batchId ?? null,
      triggeredByResourceId: input.triggeredByResourceId ?? null,
    },
  })

  const registry = env.REGISTRY_DB.get(env.REGISTRY_DB.idFromName('global')) as any
  await registry.indexEntity('Violation', id, input.domain)

  return id
}

// ── Failure ──────────────────────────────────────────────────────────

export type FailureType = 'extraction' | 'evaluation' | 'transition' | 'parse' | 'induction'

export interface FailureInput {
  domain: string | null
  failureType: FailureType
  reason: string
  severity?: 'error' | 'warning' | 'info'
  functionId?: string       // "is against Function" — the Function this failure relates to
  causedByViolationId?: string  // "is caused by Violation" — the violation that caused this failure
  transitionId?: string     // "occurs during Transition" — the transition during which this failed
  input?: string
}

/**
 * Create a Failure entity in an EntityDB DO and index it in the Registry.
 * Returns the entity id on success.
 */
export async function createFailure(env: Env, input: FailureInput): Promise<string> {
  const id = crypto.randomUUID()
  const stub = env.ENTITY_DB.get(env.ENTITY_DB.idFromName(id)) as any
  await stub.put({
    id,
    type: 'Failure',
    data: {
      domain: input.domain,
      failureType: input.failureType,
      reason: input.reason,
      severity: input.severity ?? 'error',
      functionId: input.functionId ?? null,
      causedByViolationId: input.causedByViolationId ?? null,
      transitionId: input.transitionId ?? null,
      input: input.input ?? null,
      occurredAt: new Date().toISOString(),
    },
  })

  const registry = env.REGISTRY_DB.get(env.REGISTRY_DB.idFromName('global')) as any
  await registry.indexEntity('Failure', id, input.domain ?? '')

  return id
}

// ── Bulk helpers ─────────────────────────────────────────────────────

/**
 * Persist multiple violations best-effort. Returns the ids of successfully
 * created entities. Failures are silently swallowed — the caller's response
 * is never blocked by persistence errors.
 */
export async function persistViolations(
  env: Env,
  violations: ViolationInput[],
): Promise<string[]> {
  const results = await Promise.allSettled(
    violations.map(v => createViolation(env, v)),
  )
  return results
    .filter((r): r is PromiseFulfilledResult<string> => r.status === 'fulfilled')
    .map(r => r.value)
}

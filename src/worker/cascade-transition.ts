/**
 * Cascade transition pipeline — when a transition fires, execute the Verb's
 * callback (if any), match the HTTP response status code against Event Type
 * Patterns on outgoing transitions from the new status, and fire the next
 * transition automatically.
 *
 * Uses Registry+EntityDB fan-out (same pattern as everywhere else).
 */

import type { RegistryReadStub, EntityReadStub, EntityRecord } from '../api/entity-routes'
import { loadEntities, loadEntity } from './fan-out'
import type { GetStubFn } from './fan-out'

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export interface CascadeResult {
  finalState: string
  statesVisited: string[]
  callbackResults: Array<{ status: number; url: string }>
  failures: string[]
}

export interface CascadeContext {
  registry: RegistryReadStub
  getStub: (id: string) => EntityReadStub & { patch?(data: any): Promise<any> }
  fetchCallback?: (url: string, init?: RequestInit) => Promise<Response>
  domain?: string
  maxDepth?: number // prevent infinite loops, default 10
}

// ---------------------------------------------------------------------------
// Pattern matching
// ---------------------------------------------------------------------------

/**
 * Match an HTTP status code against an Event Type pattern.
 * 'X' characters are treated as single-digit wildcards, '*' matches anything.
 * Examples: '2XX' matches 200-299, '4XX' matches 400-499, '200' matches exactly 200.
 */
export function matchPattern(statusCode: number, pattern: string): boolean {
  if (pattern === '*') return true
  const regex = new RegExp('^' + pattern.replace(/X/gi, '\\d') + '$')
  return regex.test(statusCode.toString())
}

// ---------------------------------------------------------------------------
// Cascade execution
// ---------------------------------------------------------------------------

/**
 * Execute a cascade of transitions starting from the given event.
 *
 * Loop:
 * 1. Apply the transition (update entity status via EntityDB.patch)
 * 2. Load the Verb for the transition
 * 3. If the Verb has a callback URI (via its Function), execute it
 * 4. Get the HTTP response status code
 * 5. Load outgoing transitions from the new status
 * 6. For each outgoing transition, check if its Event Type Pattern matches
 * 7. If match found, fire that transition (goto step 1)
 * 8. If no match or no callback, stop
 * 9. On callback error, persist failure info and stop
 * 10. Enforce maxDepth to prevent infinite loops
 */
export async function executeCascade(
  entityId: string,
  initialEvent: string,
  context: CascadeContext,
): Promise<CascadeResult> {
  const { registry, getStub, domain, fetchCallback } = context
  const maxDepth = context.maxDepth ?? 10
  const fetcher = fetchCallback ?? globalThis.fetch

  const statesVisited: string[] = []
  const callbackResults: Array<{ status: number; url: string }> = []
  const failures: string[] = []

  // Load the entity to get current state machine info
  const entity = await loadEntity(getStub, entityId)
  if (!entity) {
    return { finalState: 'unknown', statesVisited, callbackResults, failures: ['Entity not found'] }
  }

  let currentStatusId = entity.data._statusId as string
  let currentStatusName = entity.data._status as string
  let definitionId = entity.data._stateMachineDefinition as string
  let currentEvent: string | null = initialEvent

  for (let depth = 0; depth < maxDepth && currentEvent; depth++) {
    // Find outgoing transitions from current status (by ID or name)
    const allTransitions = await loadEntities(registry, getStub, 'Transition', domain)
    const allStatuses = await loadEntities(registry, getStub, 'Status', domain)
    const outgoing = allTransitions.filter(t => {
      const fromMatch = t.data.from === currentStatusId || t.data.fromId === currentStatusId ||
        t.data.fromStatus === currentStatusName || t.data.from === currentStatusName
      return fromMatch
    })

    // Find the transition matching the current event (by ID or name)
    let matchedTransition: EntityRecord | null = null
    for (const t of outgoing) {
      const eventRef = (t.data.eventType || t.data.eventTypeId || t.data.triggeredByEventType) as string
      if (!eventRef) continue
      // Try loading as entity ID first
      const eventType = await loadEntity(getStub, eventRef).catch(() => null)
      if (eventType && eventType.data.name === currentEvent) {
        matchedTransition = t
        break
      }
      // Fall back to name-based match
      if (eventRef === currentEvent) {
        matchedTransition = t
        break
      }
    }

    if (!matchedTransition) break

    // Resolve target status (by ID or name)
    const toRef = (matchedTransition.data.to || matchedTransition.data.toId || matchedTransition.data.toStatus) as string
    let toStatus = toRef ? await loadEntity(getStub, toRef).catch(() => null) : null
    if (!toStatus) {
      toStatus = allStatuses.find(s => s.data.name === toRef) || null
    }
    const toStatusName = toStatus ? (toStatus.data.name as string) : toRef || 'unknown'
    const toStatusId = toStatus ? toStatus.id : toRef

    // Step 1: Apply the transition — update entity status
    const stub = getStub(entityId) as any
    if (stub.patch) {
      await stub.patch({ _status: toStatusName, _statusId: toStatusId })
    }

    currentStatusId = toStatusId
    currentStatusName = toStatusName
    statesVisited.push(toStatusName)

    // Step 2: Load the Verb for this transition
    const verbId = (matchedTransition.data.verb || matchedTransition.data.verbId) as string | undefined
    if (!verbId) {
      // No verb — transition applied, but no callback to cascade from
      currentEvent = null
      break
    }

    // Resolve callback URL: the Verb entity may have callbackUrl directly
    // (Verb is subtype of Function), or a separate Function entity references it.
    // In state.ts, the pattern is: load all Functions, find one where f.data.verb === verbId.
    const verb = await loadEntity(getStub, verbId)

    let callbackUrl: string | undefined
    let httpMethod = 'POST'
    let headers: Record<string, string> = { 'Content-Type': 'application/json' }

    if (verb?.data.callbackUrl) {
      // Verb itself has callback info (Verb is subtype of Function)
      callbackUrl = verb.data.callbackUrl as string
      httpMethod = (verb.data.httpMethod as string) || 'POST'
      if (verb.data.headers) {
        const parsed = typeof verb.data.headers === 'string'
          ? JSON.parse(verb.data.headers)
          : verb.data.headers
        headers = { ...headers, ...parsed }
      }
    }

    if (!callbackUrl) {
      // Look up via Function entity (verb → function relationship)
      const allFunctions = await loadEntities(registry, getStub, 'Function', domain)
      const func = allFunctions.find(f =>
        f.data.verb === verbId || f.data.verbId === verbId,
      )
      if (func?.data.callbackUrl) {
        callbackUrl = func.data.callbackUrl as string
        httpMethod = (func.data.httpMethod as string) || 'POST'
        if (func.data.headers) {
          const parsed = typeof func.data.headers === 'string'
            ? JSON.parse(func.data.headers)
            : func.data.headers
          headers = { ...headers, ...parsed }
        }
      }
    }

    if (!callbackUrl) {
      // No callback — stop cascading
      currentEvent = null
      break
    }

    // Step 3: Execute the callback
    let responseStatus: number
    try {
      const response = await fetcher(callbackUrl, {
        method: httpMethod,
        headers,
        body: JSON.stringify({
          entityId,
          transitionId: matchedTransition.id,
          previousStatus: statesVisited.length > 1 ? statesVisited[statesVisited.length - 2] : entity.data._status,
          currentStatus: toStatusName,
        }),
      })
      responseStatus = response.status
      callbackResults.push({ status: responseStatus, url: callbackUrl })
    } catch (err) {
      // Step 9: On callback error, record failure and stop
      failures.push(`Callback error for ${callbackUrl}: ${String(err)}`)
      currentEvent = null
      break
    }

    // Step 5-6: Load outgoing transitions from the new status and match patterns
    const nextTransitions = allTransitions.filter(t =>
      (t.data.from === toStatusId || t.data.fromId === toStatusId) &&
      (t.data.stateMachineDefinition === definitionId || t.data.stateMachineDefinitionId === definitionId),
    )

    currentEvent = null // Reset — will be set if a pattern matches

    for (const nt of nextTransitions) {
      const eventTypeId = (nt.data.eventType || nt.data.eventTypeId) as string
      if (!eventTypeId) continue
      const eventType = await loadEntity(getStub, eventTypeId)
      if (!eventType) continue

      const pattern = eventType.data.pattern as string | undefined
      if (pattern && matchPattern(responseStatus, pattern)) {
        // Found a matching pattern — this becomes the next event to fire
        currentEvent = eventType.data.name as string
        break
      }
    }
  }

  // If we exhausted maxDepth, note it
  if (statesVisited.length >= maxDepth) {
    failures.push(`Max cascade depth (${maxDepth}) reached`)
  }

  return {
    finalState: currentStatusName,
    statesVisited,
    callbackResults,
    failures,
  }
}

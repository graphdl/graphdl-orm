/**
 * State machine RPC — single-endpoint runtime for state machine operations.
 *
 * Replaces 400+ lines of HTTP-round-trip helpers in apis/state/.
 * All queries are in-process SQL — no service binding hops.
 *
 * GET    /api/state/:type/:id         → read current state + available transitions
 * POST   /api/state/:type/:id/:event  → send event (auto-creates instance on first event)
 * DELETE /api/state/:type/:id         → delete instance
 */

import { json, error } from 'itty-router'
import type { Env } from '../types'

function getDB(env: Env) {
  const id = env.DOMAIN_DB.idFromName('graphdl-primary')
  return env.DOMAIN_DB.get(id) as any
}

// ── Helpers ──────────────────────────────────────────────────────────

function slugMatch(a: string, b: string): boolean {
  const norm = (s: string) => s.toLowerCase().replace(/\s+/g, '-')
  return norm(a) === norm(b)
}

async function findDefinition(db: any, machineType: string) {
  const result = await db.findInCollection('state-machine-definitions', {}, { limit: 50 })
  return result.docs?.find((d: any) =>
    (d.title || d.name) && slugMatch(d.title || d.name, machineType)
  ) || null
}

async function findStatuses(db: any, definitionId: string) {
  const result = await db.findInCollection('statuses', {
    stateMachineDefinition: { equals: definitionId },
  }, { limit: 100 })
  return result.docs || []
}

async function findTransitionsFrom(db: any, statusId: string) {
  const result = await db.findInCollection('transitions', {
    from: { equals: statusId },
  }, { limit: 100 })
  return result.docs || []
}

async function findInitialStatus(db: any, definitionId: string) {
  const statuses = await findStatuses(db, definitionId)
  if (!statuses.length) return null

  // Heuristic: first status with no incoming transitions, else first in list
  for (const s of statuses) {
    const incoming = await db.findInCollection('transitions', {
      to: { equals: s.id },
    }, { limit: 1 })
    if (!incoming.docs?.length) return s
  }
  return statuses[0]
}

async function getInstance(db: any, machineType: string, instanceId: string) {
  // First try: find by name (entity ID) — unambiguous, works even with duplicate definitions
  const byName = await db.findInCollection('state-machines', {
    name: { equals: instanceId },
  }, { limit: 1 })
  let doc = byName.docs?.[0]

  // If multiple machines exist for this name (shouldn't happen), narrow by definition
  if (!doc) {
    const definition = await findDefinition(db, machineType)
    if (definition) {
      const result = await db.findInCollection('state-machines', {
        name: { equals: instanceId },
        stateMachineType: { equals: definition.id },
      }, { limit: 1 })
      doc = result.docs?.[0]
    }
  }

  if (!doc) return null

  // Resolve status name
  let currentStatusName: string | null = null
  const statusId = typeof doc.stateMachineStatus === 'object' ? doc.stateMachineStatus?.id : doc.stateMachineStatus
  if (statusId) {
    const status = await db.getFromCollection('statuses', statusId)
    currentStatusName = status?.name || null
  }

  return {
    id: doc.id,
    currentStatusId: statusId,
    currentStatusName,
    definitionId: typeof doc.stateMachineType === 'object' ? doc.stateMachineType?.id : doc.stateMachineType,
    domain: typeof doc.domain === 'object' ? doc.domain?.id : doc.domain,
  }
}

async function enrichTransitions(db: any, transitions: any[]) {
  return Promise.all(transitions.map(async (t: any) => {
    const eventTypeId = typeof t.eventType === 'object' ? t.eventType?.id : t.eventType
    const toStatusId = typeof t.to === 'object' ? t.to?.id : t.to

    const [eventType, toStatus, guards] = await Promise.all([
      eventTypeId ? db.getFromCollection('event-types', eventTypeId) : null,
      toStatusId ? db.getFromCollection('statuses', toStatusId) : null,
      db.findInCollection('guards', { transition: { equals: t.id } }, { limit: 10 })
        .then((r: any) => r.docs || []),
    ])

    const guardNames = guards.map((g: any) => g.name).filter(Boolean)

    return {
      id: t.id,
      event: eventType?.name || null,
      target: toStatus?.name || null,
      ...(guardNames.length ? { guards: guardNames } : {}),
      // Keep raw refs for sendEvent matching
      _eventTypeId: eventTypeId,
      _toStatusId: toStatusId,
      _verbId: typeof t.verb === 'object' ? t.verb?.id : t.verb,
    }
  }))
}

// ── GET /api/state/:type/:id ─────────────────────────────────────────

export async function handleGetState(request: Request, env: Env) {
  const url = new URL(request.url)
  const parts = url.pathname.replace('/api/state/', '').split('/')
  const machineType = parts[0]
  const instanceId = parts[1]
  const event = parts[2]

  if (!machineType || !instanceId) return error(400, { error: 'machineType and instanceId required' })

  // If an event is provided on GET, delegate to sendEvent (GET-based event firing)
  if (event) return handleSendEvent(request, env)

  const db = getDB(env)
  let instance = await getInstance(db, machineType, instanceId)

  // Auto-create instance at initial state if entity exists but no state machine yet
  if (!instance) {
    const definition = await findDefinition(db, machineType)
    if (!definition) return json({ error: `Machine type '${machineType}' not found` }, { status: 404 })

    const initialStatus = await findInitialStatus(db, definition.id)
    if (!initialStatus) return json({ error: `No statuses found for '${machineType}'` }, { status: 404 })

    const domainId = typeof definition.domain === 'object' ? definition.domain?.id : definition.domain
    await db.createInCollection('state-machines', {
      name: instanceId,
      stateMachineType: definition.id,
      stateMachineStatus: initialStatus.id,
      ...(domainId ? { domain: domainId } : {}),
    })

    instance = {
      id: '',
      currentStatusId: initialStatus.id,
      currentStatusName: initialStatus.name,
      definitionId: definition.id,
      domain: domainId,
    }
  }

  const rawTransitions = instance.currentStatusId
    ? await findTransitionsFrom(db, instance.currentStatusId)
    : []
  const availableTransitions = await enrichTransitions(db, rawTransitions)

  return json({
    machineType,
    instanceId,
    currentState: instance.currentStatusName || 'unknown',
    availableEvents: availableTransitions.map((t: any) => t.event).filter(Boolean),
    availableTransitions: availableTransitions.map(({ _eventTypeId, _toStatusId, _verbId, id, ...t }) => t),
  })
}

// ── POST /api/state/:type/:id/:event ─────────────────────────────────

export async function handleSendEvent(request: Request, env: Env) {
  const url = new URL(request.url)
  const parts = url.pathname.replace('/api/state/', '').split('/')
  const machineType = parts[0]
  const instanceId = parts[1]
  const event = parts[2]

  if (!machineType || !instanceId || !event) {
    return error(400, { error: 'machineType, instanceId, and event required' })
  }

  const body = request.method === 'GET'
    ? Object.fromEntries(url.searchParams)
    : await request.json().catch(() => ({})) as Record<string, unknown>

  const db = getDB(env)

  // Find or create instance
  let instance = await getInstance(db, machineType, instanceId)

  if (!instance) {
    const definition = await findDefinition(db, machineType)
    if (!definition) return json({ error: `Machine type '${machineType}' not found` }, { status: 404 })

    const initialStatus = await findInitialStatus(db, definition.id)
    if (!initialStatus) return json({ error: `No statuses found for '${machineType}'` }, { status: 404 })

    const domainId = typeof definition.domain === 'object' ? definition.domain?.id : definition.domain
    await db.createInCollection('state-machines', {
      name: instanceId,
      stateMachineType: definition.id,
      stateMachineStatus: initialStatus.id,
      ...(domainId ? { domain: domainId } : {}),
    })

    instance = {
      id: '',
      currentStatusId: initialStatus.id,
      currentStatusName: initialStatus.name,
      definitionId: definition.id,
      domain: domainId,
    }
  }

  // Resolve transition
  const rawTransitions = await findTransitionsFrom(db, instance.currentStatusId)
  const enriched = await enrichTransitions(db, rawTransitions)
  const matching = enriched.find((t: any) => t.event === event)

  if (!matching) {
    return json({
      error: `No transition for event '${event}' in state '${instance.currentStatusName}'`,
      currentState: instance.currentStatusName,
      availableEvents: enriched.map((t: any) => t.event).filter(Boolean),
    }, { status: 422 })
  }

  const previousState = instance.currentStatusName

  // Fire callback (verb → function → callbackUrl)
  let callbackResult: Record<string, unknown> | null = null
  if (matching._verbId) {
    const funcResult = await db.findInCollection('functions', {
      verb: { equals: matching._verbId },
    }, { limit: 1 })
    const func = funcResult.docs?.[0]

    if (func?.callbackUrl) {
      try {
        const callbackHeaders: Record<string, string> = { 'Content-Type': 'application/json' }
        if (func.headers) {
          const parsed = typeof func.headers === 'string' ? JSON.parse(func.headers) : func.headers
          for (const [key, val] of Object.entries(parsed)) {
            callbackHeaders[key] = String(val).replace(/\$\{(\w+)\}/g, (_, name) => (env as any)[name] || '')
          }
        }
        const res = await fetch(func.callbackUrl, {
          method: func.httpMethod || 'POST',
          headers: callbackHeaders,
          body: JSON.stringify({
            instanceId, machineType, event, previousState,
            targetState: matching.target,
            ...body,
          }),
        })
        callbackResult = { fired: true, url: func.callbackUrl, status: res.status, ok: res.ok }
      } catch (e) {
        callbackResult = { fired: true, url: func.callbackUrl, error: String(e) }
      }
    }
  }

  // Update instance state
  if (matching._toStatusId) {
    // Re-fetch to get the doc ID (instance may have been just created)
    const current = await getInstance(db, machineType, instanceId)
    if (current?.id) {
      await db.updateInCollection('state-machines', current.id, {
        stateMachineStatus: matching._toStatusId,
      })
    }
  }

  // Available events from new state
  const newTransitions = matching._toStatusId
    ? await enrichTransitions(db, await findTransitionsFrom(db, matching._toStatusId))
    : []

  return json({
    previousState,
    event,
    currentState: matching.target || previousState,
    ...(matching.guards ? { guards: matching.guards } : {}),
    callback: callbackResult,
    availableEvents: newTransitions.map((t: any) => t.event).filter(Boolean),
  })
}

// ── DELETE /api/state/:type/:id ──────────────────────────────────────

export async function handleDeleteState(request: Request, env: Env) {
  const url = new URL(request.url)
  const parts = url.pathname.replace('/api/state/', '').split('/')
  const machineType = parts[0]
  const instanceId = parts[1]

  if (!machineType || !instanceId) return error(400, { error: 'machineType and instanceId required' })

  const db = getDB(env)
  const instance = await getInstance(db, machineType, instanceId)
  if (!instance) return json({ error: 'Instance not found' }, { status: 404 })

  await db.deleteFromCollection('state-machines', instance.id)
  return json({ deleted: true, machineType, instanceId })
}

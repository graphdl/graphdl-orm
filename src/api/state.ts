/**
 * State machine RPC — single-endpoint runtime for state machine operations.
 *
 * Uses Registry+EntityDB fan-out instead of DomainDB SQL queries.
 *
 * GET    /api/state/:type/:id         → read current state + available transitions
 * POST   /api/state/:type/:id/:event  → send event (auto-creates instance on first event)
 * DELETE /api/state/:type/:id         → delete instance
 */

import { json, error } from 'itty-router'
import type { Env } from '../types'
import type { RegistryReadStub, EntityReadStub, EntityRecord } from './entity-routes'
import { createFailure } from '../worker/outcomes'

// ── DO helpers ───────────────────────────────────────────────────────

function getRegistryDO(env: Env): RegistryReadStub & { indexEntity(t: string, id: string, d?: string): Promise<void>; deindexEntity(t: string, id: string): Promise<void> } {
  const id = env.REGISTRY_DB.idFromName('global')
  return env.REGISTRY_DB.get(id) as any
}

function getEntityStub(env: Env, entityId: string): EntityReadStub & { put(input: any): Promise<any>; patch(data: any): Promise<any>; delete(): Promise<any> } {
  const id = env.ENTITY_DB.idFromName(entityId)
  return env.ENTITY_DB.get(id) as any
}

type GetStubFn = (id: string) => EntityReadStub

// ── Fan-out helpers ──────────────────────────────────────────────────

async function loadEntities(
  registry: RegistryReadStub,
  getStub: GetStubFn,
  entityType: string,
  domain?: string,
): Promise<EntityRecord[]> {
  const ids = await registry.getEntityIds(entityType, domain)
  const settled = await Promise.allSettled(
    ids.map(async (id) => {
      const stub = getStub(id)
      return stub.get()
    }),
  )
  const results: EntityRecord[] = []
  for (const r of settled) {
    if (r.status === 'fulfilled' && r.value && !r.value.deletedAt) {
      results.push(r.value)
    }
  }
  return results
}

async function loadEntity(
  getStub: GetStubFn,
  id: string,
): Promise<EntityRecord | null> {
  try {
    const stub = getStub(id)
    const entity = await stub.get()
    return entity && !entity.deletedAt ? entity : null
  } catch {
    return null
  }
}

// ── Helpers ──────────────────────────────────────────────────────────

function slugMatch(a: string, b: string): boolean {
  const norm = (s: string) => s.toLowerCase().replace(/\s+/g, '-')
  return norm(a) === norm(b)
}

async function findDefinition(
  registry: RegistryReadStub,
  getStub: GetStubFn,
  machineType: string,
  domain?: string,
): Promise<EntityRecord | null> {
  const defs = await loadEntities(registry, getStub, 'State Machine Definition', domain)
  return defs.find(d =>
    (d.data.title || d.data.name) &&
    slugMatch((d.data.title || d.data.name) as string, machineType),
  ) || null
}

async function findStatuses(
  registry: RegistryReadStub,
  getStub: GetStubFn,
  definitionId: string,
  domain?: string,
): Promise<EntityRecord[]> {
  const statuses = await loadEntities(registry, getStub, 'Status', domain)
  return statuses.filter(s =>
    s.data.stateMachineDefinition === definitionId ||
    s.data.stateMachineDefinitionId === definitionId,
  )
}

async function findTransitionsFrom(
  registry: RegistryReadStub,
  getStub: GetStubFn,
  statusId: string,
  domain?: string,
): Promise<EntityRecord[]> {
  const transitions = await loadEntities(registry, getStub, 'Transition', domain)
  return transitions.filter(t => t.data.from === statusId || t.data.fromId === statusId)
}

async function findInitialStatus(
  registry: RegistryReadStub,
  getStub: GetStubFn,
  definitionId: string,
  domain?: string,
): Promise<EntityRecord | null> {
  const statuses = await findStatuses(registry, getStub, definitionId, domain)
  if (!statuses.length) return null

  // Load all transitions to check for incoming
  const allTransitions = await loadEntities(registry, getStub, 'Transition', domain)

  // First status with no incoming transitions, else first in list
  for (const s of statuses) {
    const hasIncoming = allTransitions.some(t => t.data.to === s.id || t.data.toId === s.id)
    if (!hasIncoming) return s
  }
  return statuses[0]
}

async function getInstance(
  registry: RegistryReadStub,
  getStub: GetStubFn,
  machineType: string,
  instanceId: string,
  domain?: string,
): Promise<{ id: string; currentStatusId: string; currentStatusName: string | null; definitionId: string; domain?: string } | null> {
  // Load all state machine instances and find by name
  const instances = await loadEntities(registry, getStub, 'State Machine', domain)
  let doc = instances.find(i => i.data.name === instanceId)

  // If not found by name alone, also try filtering by definition
  if (!doc) {
    const definition = await findDefinition(registry, getStub, machineType, domain)
    if (definition) {
      doc = instances.find(i =>
        i.data.name === instanceId &&
        (i.data.stateMachineType === definition.id || i.data.stateMachineTypeId === definition.id),
      )
    }
  }

  if (!doc) return null

  // Resolve status name
  let currentStatusName: string | null = null
  const statusId = (typeof doc.data.stateMachineStatus === 'object'
    ? (doc.data.stateMachineStatus as any)?.id
    : doc.data.stateMachineStatus) as string | undefined
  if (statusId) {
    const status = await loadEntity(getStub, statusId)
    currentStatusName = status ? status.data.name as string : null
  }

  const definitionId = (typeof doc.data.stateMachineType === 'object'
    ? (doc.data.stateMachineType as any)?.id
    : doc.data.stateMachineType) as string
  const domainVal = (typeof doc.data.domain === 'object'
    ? (doc.data.domain as any)?.id
    : doc.data.domain) as string | undefined

  return {
    id: doc.id,
    currentStatusId: statusId || '',
    currentStatusName,
    definitionId,
    domain: domainVal,
  }
}

async function enrichTransitions(
  registry: RegistryReadStub,
  getStub: GetStubFn,
  transitions: EntityRecord[],
  domain?: string,
) {
  return Promise.all(transitions.map(async (t) => {
    const eventTypeId = (typeof t.data.eventType === 'object'
      ? (t.data.eventType as any)?.id : t.data.eventType) as string | undefined
    const toStatusId = (typeof t.data.to === 'object'
      ? (t.data.to as any)?.id : t.data.to) as string | undefined

    // Load guards for this transition
    const allGuards = await loadEntities(registry, getStub, 'Guard', domain)
    const guards = allGuards.filter(g => g.data.transition === t.id || g.data.transitionId === t.id)

    const [eventType, toStatus] = await Promise.all([
      eventTypeId ? loadEntity(getStub, eventTypeId) : null,
      toStatusId ? loadEntity(getStub, toStatusId) : null,
    ])

    const guardNames = guards.map(g => g.data.name).filter(Boolean)

    return {
      id: t.id,
      event: eventType?.data.name || null,
      target: toStatus?.data.name || null,
      ...(guardNames.length ? { guards: guardNames } : {}),
      _eventTypeId: eventTypeId,
      _toStatusId: toStatusId,
      _verbId: (typeof t.data.verb === 'object' ? (t.data.verb as any)?.id : t.data.verb) as string | undefined,
      _guardEntities: guards,
    }
  }))
}

/** Extract the Noun ID from a state machine definition entity. */
function getNounId(definition: EntityRecord | null): string | undefined {
  if (!definition) return undefined
  const d = definition.data
  return (typeof d.noun === 'object' ? (d.noun as any)?.id : d.noun ?? d.nounId ?? d.noun_id) as string | undefined
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

  const registry = getRegistryDO(env)
  const getStub: GetStubFn = (id) => getEntityStub(env, id)
  const domain = url.searchParams.get('domain') || undefined

  let instance = await getInstance(registry, getStub, machineType, instanceId, domain)

  // Auto-create instance at initial state if entity exists but no state machine yet
  if (!instance) {
    const definition = await findDefinition(registry, getStub, machineType, domain)
    if (!definition) return json({ error: `Machine type '${machineType}' not found` }, { status: 404 })

    const initialStatus = await findInitialStatus(registry, getStub, definition.id, domain)
    if (!initialStatus) return json({ error: `No statuses found for '${machineType}'` }, { status: 404 })

    const domainId = (typeof definition.data.domain === 'object'
      ? (definition.data.domain as any)?.id : definition.data.domain) as string | undefined

    // Create state machine instance as EntityDB DO + register in Registry
    const newId = crypto.randomUUID()
    const entityStub = getEntityStub(env, newId)
    await entityStub.put({
      id: newId,
      type: 'State Machine',
      data: {
        name: instanceId,
        stateMachineType: definition.id,
        stateMachineStatus: initialStatus.id,
        ...(domainId ? { domain: domainId } : {}),
      },
    })
    await registry.indexEntity('State Machine', newId, domainId)

    instance = {
      id: newId,
      currentStatusId: initialStatus.id,
      currentStatusName: initialStatus.data.name as string,
      definitionId: definition.id,
      domain: domainId,
    }
  }

  const rawTransitions = instance.currentStatusId
    ? await findTransitionsFrom(registry, getStub, instance.currentStatusId, domain)
    : []
  const availableTransitions = await enrichTransitions(registry, getStub, rawTransitions, domain)

  return json({
    machineType,
    instanceId,
    currentState: instance.currentStatusName || 'unknown',
    availableEvents: availableTransitions.map((t: any) => t.event).filter(Boolean),
    availableTransitions: availableTransitions.map(({ _eventTypeId, _toStatusId, _verbId, _guardEntities, id, ...t }) => t),
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

  const registry = getRegistryDO(env)
  const getStub: GetStubFn = (id) => getEntityStub(env, id)
  const domain = (body.domain as string) || url.searchParams.get('domain') || undefined
  const domainSlug = domain || null

  // Find or create instance
  let instance = await getInstance(registry, getStub, machineType, instanceId, domain)
  let definition: EntityRecord | null = null

  if (!instance) {
    definition = await findDefinition(registry, getStub, machineType, domain)
    if (!definition) {
      const reason = `State machine type '${machineType}' not found`
      createFailure(env, {
        domain: domainSlug,
        failureType: 'transition',
        reason,
      }).catch(() => {}) // best-effort, don't block the response
      return json({ error: reason }, { status: 404 })
    }

    const initialStatus = await findInitialStatus(registry, getStub, definition.id, domain)
    if (!initialStatus) {
      const reason = `No statuses found for '${machineType}'`
      createFailure(env, {
        domain: domainSlug,
        failureType: 'transition',
        reason,
        functionId: getNounId(definition),
      }).catch(() => {}) // best-effort, don't block the response
      return json({ error: reason }, { status: 404 })
    }

    const domainId = (typeof definition.data.domain === 'object'
      ? (definition.data.domain as any)?.id : definition.data.domain) as string | undefined

    // Create state machine instance as EntityDB DO + register in Registry
    const newId = crypto.randomUUID()
    const entityStub = getEntityStub(env, newId)
    await entityStub.put({
      id: newId,
      type: 'State Machine',
      data: {
        name: instanceId,
        stateMachineType: definition.id,
        stateMachineStatus: initialStatus.id,
        ...(domainId ? { domain: domainId } : {}),
      },
    })
    await registry.indexEntity('State Machine', newId, domainId)

    instance = {
      id: newId,
      currentStatusId: initialStatus.id,
      currentStatusName: initialStatus.data.name as string,
      definitionId: definition.id,
      domain: domainId,
    }
  }

  // Lazily resolve definition if we didn't load it above (instance already existed)
  if (!definition) {
    definition = await loadEntity(getStub, instance.definitionId)
  }
  const nounId = getNounId(definition)

  // Resolve transition
  const rawTransitions = await findTransitionsFrom(registry, getStub, instance.currentStatusId, domain)
  const enriched = await enrichTransitions(registry, getStub, rawTransitions, domain)
  const matching = enriched.find((t: any) => t.event === event)

  if (!matching) {
    const reason = `No transition for event '${event}' in state '${instance.currentStatusName}'`
    createFailure(env, {
      domain: domainSlug,
      failureType: 'transition',
      reason,
      functionId: nounId,
    }).catch(() => {}) // best-effort, don't block the response
    return json({
      error: reason,
      currentState: instance.currentStatusName,
      availableEvents: enriched.map((t: any) => t.event).filter(Boolean),
    }, { status: 422 })
  }

  // ── Guard evaluation ───────────────────────────────────────────────
  const guardEntities = (matching as any)._guardEntities as EntityRecord[] | undefined
  if (guardEntities && guardEntities.length > 0) {
    for (const guard of guardEntities) {
      const guardName = (guard.data.name || guard.id) as string
      const graphSchemaId = (guard.data.graphSchemaId || guard.data.graph_schema_id) as string | undefined

      // If the guard references a graph schema, verify the schema exists
      if (graphSchemaId) {
        const schema = await loadEntity(getStub, graphSchemaId)
        if (!schema) {
          const reason = `Guard '${guardName}' references unavailable graph schema '${graphSchemaId}'`
          createFailure(env, {
            domain: domainSlug,
            failureType: 'transition',
            reason,
            functionId: nounId,
          }).catch(() => {}) // best-effort, don't block the response
          return json({
            error: reason,
            currentState: instance.currentStatusName,
            guard: guardName,
          }, { status: 422 })
        }
      }

      // Guard is defined on this transition — it blocks the transition
      // (runtime guard evaluation requires constraint checking against entity data;
      // guards are treated as blocking until a guard-run confirms passage)
      const reason = `Guard '${guardName}' blocked transition from '${instance.currentStatusName}' to '${matching.target}'`
      createFailure(env, {
        domain: domainSlug,
        failureType: 'transition',
        reason,
        functionId: nounId,
      }).catch(() => {}) // best-effort, don't block the response
      return json({
        error: reason,
        currentState: instance.currentStatusName,
        guard: guardName,
      }, { status: 422 })
    }
  }

  const previousState = instance.currentStatusName

  // Fire callback (verb → function → callbackUrl)
  let callbackResult: Record<string, unknown> | null = null
  if (matching._verbId) {
    const allFunctions = await loadEntities(registry, getStub, 'Function', domain)
    const func = allFunctions.find(f =>
      f.data.verb === matching._verbId || f.data.verbId === matching._verbId,
    )

    if (func?.data.callbackUrl) {
      try {
        const callbackHeaders: Record<string, string> = { 'Content-Type': 'application/json' }
        if (func.data.headers) {
          const parsed = typeof func.data.headers === 'string'
            ? JSON.parse(func.data.headers)
            : func.data.headers
          for (const [key, val] of Object.entries(parsed)) {
            callbackHeaders[key] = String(val).replace(/\$\{(\w+)\}/g, (_, name) => (env as any)[name] || '')
          }
        }
        const res = await fetch(func.data.callbackUrl as string, {
          method: (func.data.httpMethod as string) || 'POST',
          headers: callbackHeaders,
          body: JSON.stringify({
            instanceId, machineType, event, previousState,
            targetState: matching.target,
            ...body,
          }),
        })
        callbackResult = { fired: true, url: func.data.callbackUrl, status: res.status, ok: res.ok }
      } catch (e) {
        callbackResult = { fired: true, url: func.data.callbackUrl, error: String(e) }
      }
    }
  }

  // Update instance state via EntityDB.patch()
  if (matching._toStatusId && instance.id) {
    const instanceStub = getEntityStub(env, instance.id)
    await instanceStub.patch({ stateMachineStatus: matching._toStatusId })
  }

  // Available events from new state
  const newTransitions = matching._toStatusId
    ? await enrichTransitions(
        registry, getStub,
        await findTransitionsFrom(registry, getStub, matching._toStatusId, domain),
        domain,
      )
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

  const registry = getRegistryDO(env)
  const getStub: GetStubFn = (id) => getEntityStub(env, id)
  const domain = url.searchParams.get('domain') || undefined

  const instance = await getInstance(registry, getStub, machineType, instanceId, domain)
  if (!instance) return json({ error: 'Instance not found' }, { status: 404 })

  // Soft-delete via EntityDB + deindex from Registry
  const entityStub = getEntityStub(env, instance.id)
  await entityStub.delete()
  await registry.deindexEntity('State Machine', instance.id)

  return json({ deleted: true, machineType, instanceId })
}

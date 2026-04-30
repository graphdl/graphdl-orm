import { AutoRouter, json, error } from 'itty-router'
import type { Env } from '../types'
import { registryIdForDomain, listDomains } from '../registry-do'
import { nounToSlug, nounToTable, resolveSlugToNoun } from '../collections'
import { handleParse } from './parse'
import { handleEvaluate, handleSynthesize } from './evaluate'
import { handleCreateEntity, handleDeleteEntity } from './entity-routes'
import { envelope } from './envelope'
import { loadDomainSchema, loadDomainAndPopulation, buildPopulation, getTransitions, applyCommand, querySchema, forwardChain, getNounSchemas, computeRMAP, system as wasmSystem, currentDomainHandle } from './engine'
import { handleArestRequest, handleArestReadFallback } from './arest-router'
import { handleMcpRequest } from '../mcp/remote'
import { dispatchVerb, UNIFIED_VERBS } from './verb-dispatcher'
import { aiComplete } from './ai/complete'
import { handleExtract } from './ai/extract'
import { AGENT_DEFINITIONS_STATE } from './ai/agent-seed'

// ── Collection slug → noun type resolution ───────────────────────────
// Resolved dynamically from the Registry via nounToSlug convention.
// Noun entities are materialized from parsed readings — no hardcoded maps.

// ── DO helpers ───────────────────────────────────────────────────────

import { cellKey } from './cell-key'

/** Get an EntityDB DO stub for the given entity ID. */
function getEntityDO(env: Env, entityId: string): DurableObjectStub {
  const id = env.ENTITY_DB.idFromName(entityId)
  return env.ENTITY_DB.get(id)
}

/**
 * Get the Durable Object stub for a cell identified by (nounType, entityId).
 * This is the #217 RMAP-derived routing form — it makes the cell naming
 * that's implicit in `getEntityDO` explicit by computing the canonical
 * DO name through `cellKey`. Use it at new call sites so the paper's
 * Definition 2 cell boundary is legible at the worker-routing layer.
 */
function getCellDO(env: Env, nounType: string, entityId: string): DurableObjectStub {
  const id = env.ENTITY_DB.idFromName(cellKey(nounType, entityId))
  return env.ENTITY_DB.get(id)
}

/** Decode URL-encoded entity ID from route params. Entity IDs contain colons (domain:name). */
function decodeId(params: Record<string, string>): string {
  return decodeURIComponent(params.id || '')
}

/** Get a RegistryDB DO stub for the given scope (e.g. app/org/global). */
function getRegistryDO(env: Env, scope: string): DurableObjectStub {
  const id = env.REGISTRY_DB.idFromName(scope)
  return env.REGISTRY_DB.get(id)
}

/**
 * Get a RegistryDB DO stub scoped to a (scope, domain) pair (#205).
 * When domain is absent falls back to the legacy 'global' shard.
 */
function getRegistryForDomain(env: Env, scope: string, domain?: string): DurableObjectStub {
  const id = registryIdForDomain(env.REGISTRY_DB, scope, domain)
  return env.REGISTRY_DB.get(id)
}


export const router = AutoRouter()

// ── Health ───────────────────────────────────────────────────────────
router.get('/health', () => json({ status: 'ok', version: '0.1.0' }))

// ── Remote MCP (ChatGPT / Claude Desktop custom connectors) ──────────
// Streamable HTTP transport, a single endpoint for both POST (client
// messages) and GET (SSE server stream). Bearer auth happens in
// src/worker.ts before the router sees the request. The `/sse` alias
// matches OpenAI's example URLs that end in `/sse/`.
router.all('/mcp', (request: Request) => handleMcpRequest(request))
router.all('/sse', (request: Request) => handleMcpRequest(request))
router.all('/sse/', (request: Request) => handleMcpRequest(request))

// ── Domain Connection: store secrets for External System access ──────
// Per core.md: Domain connects to External System with Secret Reference.
// Secrets are stored in the Domain entity's DO via connectSystem().
router.post('/api/connect/:domain/:system', async (request, env: Env) => {
  const { domain, system } = request.params
  const body = await request.json() as any
  const secret = body?.secret
  if (!secret) return error(400, { errors: [{ message: 'secret required' }] })

  // Domain secrets are stored in a cell keyed by ('domain-secrets', domain)
  // — one DO per domain (§5.4 Def 2: secret writes on distinct domains
  // never contend). Using `cellKey` instead of the inline `domain-secrets:${…}`
  // form keeps the #217 RMAP-derived naming authoritative in one place.
  const domainDO = getCellDO(env, 'domain-secrets', domain) as any
  await domainDO.connectSystem(system, secret)
  return json({ connected: true, domain, system })
})

router.get('/api/connect/:domain', async (request, env: Env) => {
  const { domain } = request.params
  const domainDO = getCellDO(env, 'domain-secrets', domain) as any
  const systems = await domainDO.connectedSystems()
  return json({ domain, connectedSystems: systems })
})

// REMOVED: /api/access is replaced by GET /arest/ (root resource with org membership links).
// See handleArestRequest in arest-router.ts.

// ── Derivation trace for a domain ────────────────────────────────────
// Per the paper: derivation chains are recorded and available on demand.
// Returns the full forward-chained derivation trace for a domain's population.
router.get('/api/trace/:domain', async (request, env: Env) => {
  const domain = decodeURIComponent(request.params.domain)
  const registry = getRegistryDO(env, 'global') as any
  const getStub = (id: string) => getEntityDO(env, id) as any

  await loadDomainSchema(registry, getStub, domain).catch(() => {})
  const popJson = await buildPopulation(registry, getStub, domain)
  let derived: Array<{ factTypeId: string; reading: string; bindings: Array<[string, string]>; derivedBy: string }> = []
  try { derived = forwardChain(popJson) } catch {}

  return json({
    domain,
    derivedFacts: derived.length,
    trace: derived.map(fact => ({
      rule: fact.derivedBy,
      reading: fact.reading,
      bindings: fact.bindings,
    })),
  })
})

// ── Debug: population fact type IDs ──────────────────────────────────
router.get('/api/debug/population/:domain', async (request, env: Env) => {
  const domain = decodeURIComponent(request.params.domain)
  const registry = getRegistryDO(env, 'global') as any
  const getStub = (id: string) => getEntityDO(env, id) as any
  await loadDomainSchema(registry, getStub, domain).catch(() => {})
  const popJson = await buildPopulation(registry, getStub, domain)
  const pop = JSON.parse(popJson) as { facts: Record<string, any[]> }
  const summary = Object.entries(pop.facts).map(([ftId, facts]) => ({ ftId, count: facts.length }))
  return json({ domain, factTypes: summary })
})

// ── Debug: entity counts by type from Registry ──────────────────────
router.get('/api/debug/counts/:domain', async (request, env: Env) => {
  const { domain } = request.params
  const registry = getRegistryDO(env, 'global') as any
  const counts = await registry.getEntityCounts(domain)
  return json({ domain, counts })
})

router.post('/api/debug/arest/:domain', async (request, env: Env) => {
  const { domain } = request.params
  const body = await request.json() as any
  const registry = getRegistryDO(env, 'global') as any
  const getStub = (id: string) => getEntityDO(env, id) as any
  const populationJson = await loadDomainAndPopulation(registry, getStub, domain)
  const result = applyCommand(body, populationJson)
  return json(result)
})

router.get('/api/debug/compiled/:domain', async (request, env: Env) => {
  const { domain } = request.params
  const registry = getRegistryDO(env, 'global') as any
  const handle = await loadDomainSchema(registry, (id: string) => getEntityDO(env, id) as any, domain)
  return json(JSON.parse(wasmSystem(handle, 'debug', '')))
})

router.get('/api/debug/schema/:domain', async (request, env: Env) => {
  const domain = decodeURIComponent(request.params.domain)
  const registry = getRegistryDO(env, 'global') as any
  const handle = await loadDomainSchema(registry, (id: string) => getEntityDO(env, id) as any, domain)
  const schema = JSON.parse(wasmSystem(handle, 'debug', ''))
  return json({
    domain,
    nounCount: Object.keys(schema.nouns).length,
    factTypeCount: Object.keys(schema.factTypes).length,
    constraintCount: schema.constraints.length,
    stateMachines: Object.entries(schema.stateMachines).map(([k, v]) => ({
      id: k, nounName: (v as any).nounName, statuses: (v as any).statuses, transitions: (v as any).transitions,
    })),
  })
})

// RMAP: show cell partitioning derived from UC structure (Halpin, Ch. 17).
router.get('/api/debug/rmap/:domain', async (request, env: Env) => {
  const domain = decodeURIComponent(request.params.domain)
  const registry = getRegistryDO(env, 'global') as any
  const getStub = (id: string) => getEntityDO(env, id) as any
  await loadDomainSchema(registry, getStub, domain).catch(() => {})
  const tables = computeRMAP()
  return json({ domain, tables })
})

router.get('/debug/table/:table', async (request, env: Env) => {
  const { table } = request.params
  const url = new URL(request.url)
  const domain = url.searchParams.get('domain') || undefined

  const registry = getRegistryDO(env, 'global') as any
  const counts = await registry.getEntityCounts(domain) as Array<{ nounType: string; count: number }>

  // Resolve: accept noun type name, collection slug, or table name
  const entityType = counts.find(c => c.nounType === table)?.nounType
    || counts.find(c => nounToSlug(c.nounType) === table || nounToTable(c.nounType) === table)?.nounType

  if (entityType) {
    const ids = await registry.getEntityIds(entityType, domain) as string[]
    return json({ table, entityType, domain: domain || 'all', count: ids.length, entityIds: ids })
  }

  return json({ table, domain: domain || 'all', counts })
})

// ── OpenAPI 3.1 schema exposure (per App) ───────────────────────────
//
// Generators are App-scoped: `App 'X' uses Generator 'openapi'.` is
// the opt-in assertion. The compiler emits one cell per opted-in App,
// keyed `openapi:{snake(app-slug)}`; this route plumbs that cell to
// the wire. Without an opt-in the cell is absent; the route reports
// 404 with guidance so callers see a clear remediation rather than a
// silent empty response.
//
// `?app=X` selects the App. `?domain=X` (optional, defaults to
// 'organizations') selects which compiled domain to query — the cell
// is looked up on that domain's state. A single compile may contain
// several Apps across many domains.

/// Mirrors rmap::to_snake in the Rust crate: insert `_` before an
/// uppercase letter that follows a lowercase one, replace space and
/// hyphen with `_`, lowercase everything. Used to form the cell key
/// from an App slug so the TS route lands on the same key the
/// compile gate emitted.
function appCellSuffix(app: string): string {
  let out = ''
  for (let i = 0; i < app.length; i++) {
    const ch = app[i]
    const prev = i > 0 ? app[i - 1] : ''
    if (/[A-Z]/.test(ch) && /[a-z]/.test(prev)) out += '_'
    out += ch === ' ' || ch === '-' ? '_' : ch.toLowerCase()
  }
  return out
}

// ── Live event stream (SSE) ────────────────────────────────────────
//
// GET /api/events?domain=X&noun=Y&entityId=Z opens a persistent
// text/event-stream. The worker forwards the request to BroadcastDO,
// which opens the stream and registers a subscription matching the
// query filter. Every post-mutation hook publishes into the DO;
// matching subscribers receive data frames.
//
// Narrower filters receive fewer events. `domain` is required; `noun`
// restricts to one noun type; `entityId` restricts to one entity.
router.get('/api/events', (request, env: Env) => {
  const broadcast = env.BROADCAST.get(env.BROADCAST.idFromName('global')) as unknown as {
    fetch(req: Request): Promise<Response>
  }
  return broadcast.fetch(request)
})

router.get('/api/openapi.json', async (request, env: Env) => {
  const url = new URL(request.url)
  const app = url.searchParams.get('app')
  if (!app) {
    return error(400, {
      errors: [{ message: "'app' query parameter required (e.g. ?app=my-app)" }],
    })
  }
  const domain = url.searchParams.get('domain') || 'organizations'
  const cellKey = `openapi:${appCellSuffix(app)}`

  const registry = getRegistryDO(env, 'global') as any
  const handle = await loadDomainSchema(registry, (id: string) => getEntityDO(env, id) as any, domain)
  const raw = wasmSystem(handle, cellKey, '')

  let doc: any = null
  try { doc = JSON.parse(raw) } catch { /* empty/bottom → doc stays null */ }

  if (!doc || typeof doc !== 'object' || !doc.openapi) {
    return error(404, {
      errors: [{
        message: `No OpenAPI document for App '${app}'. ` +
          `Add "App '${app}' uses Generator 'openapi'." to the App's readings to enable this endpoint.`,
      }],
    })
  }
  return json(doc)
})

// ── Unified MCP verb → HTTP route mapping (#200) ─────────────────────
// Every MCP tool registered in src/mcp/server-factory.ts gets a
// 1:1 HTTP route here via the shared `dispatchVerb` bridge. Body
// shape matches the MCP tool's inputSchema; response is a Theorem 5
// envelope per src/api/envelope.ts. CLI, local MCP (stdio), and HTTP
// clients thereby see the same input/output contract regardless of
// transport.
for (const verb of UNIFIED_VERBS) {
  router.post(`/api/${verb}`, async (request: Request, env: Env) => {
    let body: Record<string, unknown>
    try {
      body = (await request.json()) as Record<string, unknown>
    } catch {
      body = {}
    }
    try {
      const result = await dispatchVerb(verb, body)

      // #203: persist snapshot bytes to RegistryDB so they survive
      // worker restarts. The engine's in-memory snapshot store is
      // ephemeral; the DO copy is durable.
      // #205: use domain-scoped registry when a domain is present in body.
      if (verb === 'snapshot' && (env as any).REGISTRY) {
        try {
          const frozen = wasmSystem(currentDomainHandle(), 'freeze', '')
          if (frozen && frozen !== '⊥') {
            const label = (body.label as string) || new Date().toISOString()
            const snapshotDomain = (body.domain as string | undefined)
            const registryId = registryIdForDomain(
              (env as any).REGISTRY as DurableObjectNamespace,
              'global',
              snapshotDomain,
            )
            const registry = ((env as any).REGISTRY as DurableObjectNamespace).get(registryId) as any
            await registry.storeSnapshot(label, frozen)
          }
        } catch { /* best-effort — snapshot still succeeded in-memory */ }
      }

      return json(result)
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e)
      return error(400, message)
    }
  })
}

// #203: export frozen state bytes from DO storage
// #205: ?domain=<slug> selects the domain-scoped registry shard
router.get('/api/export/:label', async (request: Request, env: Env) => {
  if (!(env as any).REGISTRY) return error(501, 'no REGISTRY binding')
  const url = new URL(request.url)
  const label = url.pathname.split('/').pop() || ''
  const domain = url.searchParams.get('domain') || undefined
  const registryId = registryIdForDomain(
    (env as any).REGISTRY as DurableObjectNamespace,
    'global',
    domain,
  )
  const registry = ((env as any).REGISTRY as DurableObjectNamespace).get(registryId) as any
  const hex = await registry.fetchSnapshot(label)
  if (!hex) return error(404, `snapshot '${label}' not found`)
  return json({ label, frozen_hex: hex, byte_length: Math.floor(hex.length / 2) })
})

// #203: import frozen state bytes into the engine + persist to DO
// #205: body.domain selects the domain-scoped registry shard
router.post('/api/import', async (request: Request, env: Env) => {
  const body = (await request.json()) as { label?: string; frozen_hex?: string; domain?: string }
  if (!body.frozen_hex) return error(400, 'frozen_hex required')
  const label = body.label || new Date().toISOString()
  try {
    const raw = wasmSystem(currentDomainHandle(), 'thaw', body.frozen_hex)
    if (raw.startsWith('⊥')) return error(400, `thaw failed: ${raw}`)
    if ((env as any).REGISTRY) {
      const registryId = registryIdForDomain(
        (env as any).REGISTRY as DurableObjectNamespace,
        'global',
        body.domain,
      )
      const registry = ((env as any).REGISTRY as DurableObjectNamespace).get(registryId) as any
      await registry.storeSnapshot(label, body.frozen_hex)
    }
    return json({ ok: true, label, result: raw })
  } catch (e) {
    return error(500, e instanceof Error ? e.message : String(e))
  }
})

// ── Evaluate / Synthesize (WASM engine) ─────────────────────────────
router.post('/api/evaluate', handleEvaluate)
router.post('/api/synthesize', (request, env) => handleSynthesize(request, env))

// ── Induction (discover constraints from population) — uses WASM engine
router.post('/api/induce', async (request, env: Env) => {
  const body = await request.json() as { domain?: string }
  const registry = getRegistryDO(env, 'global') as any
  const getStub = (id: string) => getEntityDO(env, id) as any
  const handle = await loadDomainSchema(registry, getStub, body.domain || 'default')
  if (handle < 0) return error(400, { errors: [{ message: 'no domain loaded' }] })
  return json(JSON.parse(wasmSystem(handle, 'induce', '')))
})

// Entity creation goes through POST /api/entities/:noun (the AREST command path).

router.post('/api/_placeholder', async (request: Request, env: Env) => {
  const body = await request.json() as {
    domainId: string
    graphSchemaId?: string
    bindings?: Array<{ nounId: string; value?: string; resourceId?: string }>
    // Batch mode: resolve reading text + noun names
    facts?: Array<{
      reading: string
      bindings: Array<{ noun: string; value?: string; resourceId?: string }>
    }>
  }
  if (!body.domainId) {
    return error(400, { errors: [{ message: 'domainId required' }] })
  }

  const db = getRegistryDO(env, 'global') as any

  // Single fact with pre-resolved IDs
  if (body.graphSchemaId && body.bindings?.length) {
    const result = await db.createFact(body.domainId, body.graphSchemaId, body.bindings)
    return json(result, { status: 201 })
  }

  // Batch mode: resolve reading text + noun names → IDs
  if (!body.facts?.length) {
    return error(400, { errors: [{ message: 'graphSchemaId+bindings or facts[] required' }] })
  }

  // Load nouns and readings for this domain via Registry+EntityDB fan-out
  const registry = getRegistryDO(env, 'global') as any

  const [nounIds, readingIds] = await Promise.all([
    registry.getEntityIds('Noun', body.domainId) as Promise<string[]>,
    registry.getEntityIds('Reading', body.domainId) as Promise<string[]>,
  ])

  const [nounSettled, readingSettled] = await Promise.all([
    Promise.allSettled(nounIds.map(async (nid: string) => {
      const stub = getEntityDO(env, nid) as any
      return stub.get()
    })),
    Promise.allSettled(readingIds.map(async (rid: string) => {
      const stub = getEntityDO(env, rid) as any
      return stub.get()
    })),
  ])

  const nounByName = new Map<string, any>()
  for (const r of nounSettled) {
    if (r.status === 'fulfilled' && r.value) {
      const n = { id: r.value.id, ...r.value.data }
      if (n.name) nounByName.set(n.name, n)
    }
  }

  const schemaByReading = new Map<string, string>()
  for (const r of readingSettled) {
    if (r.status === 'fulfilled' && r.value) {
      const rd = r.value.data
      if (rd.text && rd.graphSchema) schemaByReading.set(rd.text, rd.graphSchema)
    }
  }

  const results: any[] = []
  const errors: string[] = []

  for (const fact of body.facts) {
    try {
      const graphSchemaId = schemaByReading.get(fact.reading)
      if (!graphSchemaId) {
        errors.push(`Reading not found: "${fact.reading}"`)
        continue
      }

      const resolvedBindings = fact.bindings.map(b => {
        const noun = nounByName.get(b.noun)
        if (!noun) return null
        return { nounId: noun.id, value: b.value, resourceId: b.resourceId }
      }).filter(Boolean)

      if (resolvedBindings.length < 2) {
        errors.push(`Not enough nouns resolved for: "${fact.reading}"`)
        continue
      }

      const result = await db.createFact(body.domainId, graphSchemaId, resolvedBindings)
      results.push(result)
    } catch (err: any) {
      errors.push(`"${fact.reading}": ${err.message}`)
    }
  }

  return json({ facts: results, errors }, { status: 201 })
})

// ── Entity (high-level: create noun instance with fields) ────────────
router.post('/api/entity', async (request, env: Env) => {
  const body = await request.json() as {
    noun: string        // noun name, e.g. "SupportRequest"
    domain: string      // domain ID
    fields: Record<string, string | string[] | Record<string, any>>  // string, array, or nested entity
    reference?: string  // display reference
    createdBy?: string  // creator identity
  }
  if (!body.noun || !body.domain || !body.fields) {
    return error(400, { errors: [{ message: 'noun, domain, and fields required' }] })
  }

  const registry = getRegistryDO(env, 'global') as any

  // Access control: engine evaluates "User can access Domain iff..." derivation rules

  // Extract array-of-objects fields (child entities) BEFORE creating the parent.
  // Children are created as separate Entity DOs to keep each DO = one entity.
  const childArrays: Array<{ fieldName: string; childNoun: string; items: Record<string, any>[] }> = []
  const parentFields: Record<string, any> = {}

  for (const [fieldName, fieldValue] of Object.entries(body.fields)) {
    if (Array.isArray(fieldValue) && fieldValue.length > 0 && typeof fieldValue[0] === 'object') {
      const singular = fieldName.replace(/s$/, '')
      const childNoun = singular.charAt(0).toUpperCase() + singular.slice(1)
      childArrays.push({ fieldName, childNoun, items: fieldValue as Record<string, any>[] })
    } else {
      parentFields[fieldName] = fieldValue
    }
  }

  // AREST: one command, one function application, one state transfer.
  const getStub = (id: string) => getEntityDO(env, id) as any

  // Engine path — runs the WASM parser to compile the domain readings,
  // then applies a createEntity command which validates constraints and
  // resolves reference-scheme ids. When the WASM parser is broken on
  // the deploy target (currently wasm32-unknown-unknown traps inside
  // the FORML 2 grammar bootstrap), we fall through to a degraded
  // direct-write path: skip constraint check, mint a UUID, persist.
  let arestResult: any = null
  let engineFailure: string | null = null
  try {
    const populationJson = await loadDomainAndPopulation(registry, getStub, body.domain)
    arestResult = applyCommand({
      type: 'createEntity',
      noun: body.noun,
      domain: body.domain,
      id: null, // resolved below from reference scheme
      fields: parentFields,
    }, populationJson)
  } catch (e) {
    engineFailure = `${e}`
  }

  if (arestResult?.rejected) {
    return error(422, { errors: arestResult.violations.map((v: any) => ({ message: v.detail, constraintId: v.constraintId })) })
  }

  // Persist all entities from the AREST result (Resource + State Machine + Violations).
  // In the engine-failure fallback, synthesize a single Entity record from the request
  // body — no constraint check, no derivation, no state machine, but the row lands in
  // EntityDB + Registry so reads see it.
  const entitiesToPersist: Array<{ id?: string; type: string; data: Record<string, any> }> =
    arestResult?.entities ?? [{ type: body.noun, data: parentFields }]
  for (const entity of entitiesToPersist) {
    const eid = entity.id || crypto.randomUUID()
    const eDO = getEntityDO(env, eid) as any
    await eDO.put({ id: eid, type: entity.type, data: entity.data })
    await registry.indexEntity(entity.type, eid, body.domain)
  }

  // The primary entity ID — first entity in the result, or the synthesized fallback.
  const primaryEntity = entitiesToPersist[0]
  const parentId = primaryEntity?.id || crypto.randomUUID()

  // Create children as separate Entity DOs
  if (childArrays.length > 0) {
    const parentFkField = body.noun.charAt(0).toLowerCase() + body.noun.slice(1)
    for (const { childNoun, items } of childArrays) {
      for (const childFields of items) {
        const childId = crypto.randomUUID()
        const childDO = getEntityDO(env, childId) as any
        await childDO.put({ id: childId, type: childNoun, data: { ...childFields, [parentFkField]: parentId } })
        await registry.indexEntity(childNoun, childId, body.domain)
      }
    }
  }

  return json({
    id: parentId,
    noun: body.noun,
    domain: body.domain,
    status: arestResult?.status ?? null,
    transitions: arestResult?.transitions ?? [],
    ...(arestResult?.derivedCount > 0 && { derived: arestResult.derivedCount }),
    ...(arestResult?.violations?.length > 0 && { deonticWarnings: arestResult.violations }),
    ...(engineFailure && { engineFailure }),
  }, { status: 201 })
})

// ── Entity state machine transitions ─────────────────────────────────
// GET available transitions
router.get('/api/entities/:noun/:id/transitions', async (request, env: Env) => {
  const noun = decodeURIComponent(request.params.noun); const id = decodeURIComponent(request.params.id)
  const entityDO = getEntityDO(env, id) as any
  const entity = await entityDO.get()
  if (!entity) return error(404, { errors: [{ message: 'Not Found' }] })

  if (!entity.data?._status) {
    return json({ transitions: [], message: 'Entity has no state machine' })
  }

  // Valid transitions from the engine's compiled state machine
  const registry = getRegistryDO(env, 'global') as any
  let transitions: Array<{ event: string; targetStatus: string }> = []
  try {
    const reqUrl = new URL(request.url)
    const domainSlug = reqUrl.searchParams.get('domain') || entity.data._domain as string || ''
    await loadDomainSchema(registry, (eid: string) => getEntityDO(env, eid) as any, domainSlug)
    transitions = getTransitions(entity.type, entity.data._status as string)
      .map((t: any) => ({ event: t.event, targetStatus: t.to }))
  } catch { /* schema load failed */ }

  return json({
    currentStatus: entity.data._status,
    transitions,
  })
})

// POST fire a transition event — AREST command
router.post('/api/entities/:noun/:id/transition', async (request, env: Env) => {
  const noun = decodeURIComponent(request.params.noun); const id = decodeURIComponent(request.params.id)
  const body = await request.json() as { event: string; domain?: string }
  if (!body.event) return error(400, { errors: [{ message: 'event required' }] })

  const entityDO = getEntityDO(env, id) as any
  const entity = await entityDO.get()
  if (!entity) return error(404, { errors: [{ message: 'Not Found' }] })

  const domainSlug = body.domain || entity.data?._domain as string || entity.data?.domain as string || ''
  const registry = getRegistryDO(env, 'global') as any
  const getStub = (eid: string) => getEntityDO(env, eid) as any

  // Find the State Machine instance for this entity
  const smIds: string[] = await registry.getEntityIds('State Machine', domainSlug)
  let currentStatus: string | undefined
  let smEntityId: string | undefined
  for (const smId of smIds) {
    try {
      const sm = await getStub(smId).get()
      if (sm?.data?.forResource === id) {
        currentStatus = sm.data.currentlyInStatus as string
        smEntityId = smId
        break
      }
    } catch { continue }
  }

  if (!currentStatus || !smEntityId) {
    return error(400, { errors: [{ message: 'Entity has no state machine' }] })
  }

  // AREST: apply transition command
  const populationJson = await loadDomainAndPopulation(registry, getStub, domainSlug)
  const cmd = {
    type: 'transition' as const,
    entityId: id,
    event: body.event,
    currentStatus,
    domain: domainSlug,
  }
  console.log('AREST transition cmd:', JSON.stringify(cmd))
  const arestResult = applyCommand(cmd, populationJson)
  console.log('AREST transition result:', JSON.stringify({ status: arestResult.status, entities: arestResult.entities?.length, error: (arestResult as any).error }))

  if (arestResult.rejected) {
    return error(422, { errors: arestResult.violations.map((v: any) => ({ message: v.detail })) })
  }

  if (!arestResult.status) {
    return error(400, { errors: [{
      message: `Invalid transition: event '${body.event}' not available from status '${currentStatus}'`,
    }], debug: { arestResult, cmd } })
  }

  // Update the State Machine entity's current status via cell store (↓n)
  const smCell = await getStub(smEntityId).get()
  if (smCell) {
    await getStub(smEntityId).put({ id: smCell.id, type: smCell.type, data: { ...smCell.data, currentlyInStatus: arestResult.status } })
  }

  // Persist any Event entities from the AREST result
  for (const e of arestResult.entities) {
    const eid = e.id || crypto.randomUUID()
    await getStub(eid).put({ id: eid, type: e.type, data: e.data })
    await registry.indexEntity(e.type, eid, domainSlug)
  }

  return json({
    id,
    noun,
    previousStatus: currentStatus,
    status: arestResult.status,
    event: body.event,
    transitions: arestResult.transitions,
  })
})

// ── Entity queries (fan-out via pure handlers) ───────────────────────
// POST /api/entities/:noun — create entity via direct DO write
router.post('/api/entities/:noun', async (request, env: Env) => {
  const noun = decodeURIComponent(request.params.noun)
  const body = await request.json() as { domain?: string; data?: Record<string, unknown>; id?: string }
  const domain = body.domain || new URL(request.url).searchParams.get('domain') || ''
  if (!domain) return error(400, { errors: [{ message: 'domain required in body or query param' }] })

  const entityId = body.id || crypto.randomUUID()
  const entityDO = getEntityDO(env, entityId) as any
  await entityDO.put({ id: entityId, type: noun, data: body.data || {} })

  const registry = getRegistryDO(env, 'global') as any
  await registry.indexEntity(noun, entityId, domain)

  const cell = await entityDO.get()
  return json(envelope({
    id: cell.id,
    type: cell.type,
    ...cell.data,
  }), { status: 201 })
})



// PATCH on entities: ↓n with merged data (cell store replaces contents)
router.patch('/api/entities/:noun/:id', async (request, env: Env) => {
  const { id } = request.params
  const body = await request.json() as Record<string, any>

  const entityDO = getEntityDO(env, id) as any
  const existing = await entityDO.get()
  if (!existing) return error(404, { errors: [{ message: 'Not Found' }] })

  const merged = { ...existing.data, ...body }
  const result = await entityDO.put({ id: existing.id, type: existing.type, data: merged })
  return json(envelope({ id: result.id, type: result.type, ...result.data }))
})

router.delete('/api/entities/:noun/:id', async (request, env: Env) => {
  const noun = decodeURIComponent(request.params.noun); const id = decodeURIComponent(request.params.id)
  const domain = new URL(request.url).searchParams.get('domain') || undefined

  const registry = getRegistryDO(env, 'global') as any
  const broadcast = env.BROADCAST.get(env.BROADCAST.idFromName('global')) as any
  const result = await handleDeleteEntity(id, getEntityDO(env, id) as any, registry, noun, domain, broadcast)

  if (!result) return error(404, { errors: [{ message: 'Not Found' }] })
  return json(envelope(result))
})

// ── Collection CRUD ──────────────────────────────────────────────────

// ── Fact Query: partial application of fact types ────────────────
// GET /api/facts/:schema?domain=X&bind[NounA]=value&target=NounB
// A query IS a partially applied Construction. Bind some roles, get the free roles.
// The engine loads the domain schema, federates to populate facts, forward-chains
// derivation rules, then applies Filter(predicate) to return matching values.

router.get('/api/facts/:schema', async (request, env: Env) => {
  const { schema } = request.params
  const url = new URL(request.url)
  const domainSlug = url.searchParams.get('domain')
  const targetRole = url.searchParams.get('target')

  if (!domainSlug) return error(400, { errors: [{ message: 'domain query param required' }] })
  if (!targetRole) return error(400, { errors: [{ message: 'target query param required (the role to extract)' }] })

  // Collect bind[NounName]=value pairs from query string
  const bindings: Array<[string, string]> = []
  for (const [key, value] of url.searchParams.entries()) {
    const match = key.match(/^bind\[(.+)\]$/)
    if (match) {
      bindings.push([match[1], value])
    }
  }

  try {
    const registry = getRegistryDO(env, 'global') as any
    const getStub = (eid: string) => getEntityDO(env, eid) as any

    // Load domain schema + build federated population
    const populationJson = await loadDomainAndPopulation(registry, getStub, domainSlug)

    // Forward-chain derivation rules (including joins)
    const derived = forwardChain(populationJson)

    // Merge derived facts into population
    const population = JSON.parse(populationJson)
    for (const fact of derived) {
      if (!population.facts[fact.factTypeId]) population.facts[fact.factTypeId] = []
      population.facts[fact.factTypeId].push({
        factTypeId: fact.factTypeId,
        bindings: fact.bindings,
      })
    }
    const enrichedPopJson = JSON.stringify(population)

    // Resolve the schema ID — try exact match, then reading-format match
    // The schema param can be a fact type ID or a reading like "Vehicle is resolved to Chrome Style Candidate"
    const schemaId = decodeURIComponent(schema)

    // Resolve role names to 1-indexed role positions using the compiled schema.
    // The schema's role_names array maps position → noun name.
    // We need to find the schema in the compiled model to get its role_names.
    // Try synthesize_noun for the first bound noun to discover the schema's roles.
    let roleNames: string[] = []
    try {
      // Use the schema ID to look up role names from the compiled model
      // getNounSchemas returns schemas where a noun plays role 0
      // We need a more direct lookup — use the population's fact type to infer roles
      const popFacts = population.facts[schemaId]
      if (popFacts && popFacts.length > 0) {
        roleNames = popFacts[0].bindings.map((b: [string, string]) => b[0])
      }
    } catch { /* fall through to index-based */ }

    const filterBindings: Array<[number, string]> = []
    for (const [noun, value] of bindings) {
      const idx = parseInt(noun)
      if (!isNaN(idx)) {
        filterBindings.push([idx, value])
      } else {
        // Map noun name to 1-indexed role position
        const roleIdx = roleNames.indexOf(noun)
        if (roleIdx >= 0) {
          filterBindings.push([roleIdx + 1, value])
        }
      }
    }

    // Resolve target role: name or 1-indexed number
    const targetIdx = parseInt(targetRole)
    let targetRoleIndex: number
    if (!isNaN(targetIdx)) {
      targetRoleIndex = targetIdx
    } else {
      const idx = roleNames.indexOf(targetRole)
      targetRoleIndex = idx >= 0 ? idx + 1 : 2
    }

    const result = querySchema(schemaId, targetRoleIndex, filterBindings, enrichedPopJson)

    return json({
      schema: schemaId,
      target: targetRole,
      bindings: Object.fromEntries(bindings),
      matches: result.matches,
      count: result.count,
      derived: derived.length,
      _links: {
        self: { href: url.pathname + url.search },
        domain: { href: `/api/entities/Domain?domain=${domainSlug}` },
      },
    })
  } catch (e: any) {
    return error(500, { errors: [{ message: e.message || 'Query failed' }] })
  }
})

/** GET /api/:collection — list/find */
// ── Conceptual Query (before generic :collection routes) ─────────────
// Conceptual query — uses WASM query_schema_wasm (θ₁ operations on P)

// ── Conceptual Query — θ₁ operations on P via WASM ─────────────────
// GET/POST /api/query?domain=X&q=<natural language query>
// Uses query_schema_wasm to evaluate partial application of fact types.
router.all('/api/query', async (request, env: Env) => {
  const url = new URL(request.url)
  let domain: string | undefined
  let query: string | undefined

  if (request.method === 'POST') {
    const body = await request.json() as any
    domain = body.domain || body.domainId
    query = body.query || body.q
  } else {
    domain = url.searchParams.get('domain') || undefined
    query = url.searchParams.get('q') || url.searchParams.get('query') || undefined
  }

  if (!domain || !query) {
    return error(400, { errors: [{ message: 'domain and q (query) are required' }] })
  }

  const registry = getRegistryDO(env, 'global') as any
  const getStub = (id: string) => getEntityDO(env, id) as any

  // Load schema + build population, then query via WASM
  const handle = await loadDomainSchema(registry, getStub, domain)
  const populationJson = await loadDomainAndPopulation(registry, getStub, domain)

  const result = JSON.parse(wasmSystem(handle, 'prove', `<${query}, closed>`))

  return json(result)
})

// Seed is the ONLY ingestion path — readings → WASM parse → cells in D
router.all('/api/parse', handleParse)



/** POST /api/:collection — create */
router.post('/api/:collection', async (request, env: Env) => {
  const collection = decodeURIComponent(request.params.collection)
  const body = await request.json() as Record<string, any>

  const registry = getRegistryDO(env, 'global') as any
  const entityType = await resolveSlugToNoun(registry, collection)
  if (!entityType) {
    return error(404, { errors: [{ message: `Collection "${collection}" not found` }] })
  }

  const domain = body.domain || ''
  const broadcast = env.BROADCAST.get(env.BROADCAST.idFromName('global')) as any
  const result = await handleCreateEntity(
    entityType,
    domain,
    body,
    (id) => getEntityDO(env, id) as any,
    registry,
    undefined,
    broadcast,
  )
  return json({ doc: { id: result.id, ...body }, message: 'Created successfully' }, { status: 201 })
})

/** PATCH /api/:collection/:id — update */
router.patch('/api/:collection/:id', async (request, env: Env) => {
  const collection = decodeURIComponent(request.params.collection); const id = decodeURIComponent(request.params.id)
  const body = await request.json() as Record<string, any>

  const registry = getRegistryDO(env, 'global') as any
  const entityType = await resolveSlugToNoun(registry, collection)
  if (!entityType) {
    return error(404, { errors: [{ message: `Collection "${collection}" not found` }] })
  }

  const entityDO = getEntityDO(env, id) as any
  const existing = await entityDO.get()
  if (!existing) return error(404, { errors: [{ message: 'Not Found' }] })
  const merged = { ...existing.data, ...body }
  const result = await entityDO.put({ id: existing.id, type: existing.type, data: merged })
  return json({ doc: { id: result.id, type: result.type, ...result.data }, message: 'Updated successfully' })
})

/** DELETE /api/:collection/:id — delete */
router.delete('/api/:collection/:id', async (request, env: Env) => {
  const collection = decodeURIComponent(request.params.collection); const id = decodeURIComponent(request.params.id)

  const registry = getRegistryDO(env, 'global') as any
  const entityType = await resolveSlugToNoun(registry, collection)
  if (!entityType) {
    return error(404, { errors: [{ message: `Collection "${collection}" not found` }] })
  }

  const domain = new URL(request.url).searchParams.get('domain') || undefined
  const broadcast = env.BROADCAST.get(env.BROADCAST.idFromName('global')) as any
  const result = await handleDeleteEntity(id, getEntityDO(env, id) as any, registry, entityType, domain, broadcast)
  if (!result) return error(404, { errors: [{ message: 'Not Found' }] })
  return json({ id, message: 'Deleted successfully' })
})


// ── Domain wipe (metamodel only — nouns, readings, constraints, roles, etc.) ──
// Deindex from Registry — EntityDB DOs become orphaned. Background cleanup.
router.delete('/api/domains/:domainId/metamodel', async (request, env: Env, ctx: ExecutionContext) => {
  const { domainId } = request.params
  const registry = getRegistryDO(env, 'global') as any

  // Count before wiping
  const entities = await registry.getAllEntityIdsForDomain(domainId) as Array<{ nounType: string; entityId: string }>

  // Deindex — entities become orphaned, instant
  const deindexedEntities = await registry.deindexEntitiesForDomain(domainId) as number
  const deindexedNouns = await registry.deindexNounsForDomain(domainId) as number


  // Background: soft-delete orphaned DOs
  if (entities.length > 0) {
    const BATCH = 50
    ctx.waitUntil((async () => {
      for (let i = 0; i < entities.length; i += BATCH) {
        const chunk = entities.slice(i, i + BATCH)
        await Promise.allSettled(
          chunk.map(({ entityId }) => (getEntityDO(env, entityId) as any).delete()),
        )
      }
    })())
  }

  return json({
    deleted: true,
    domainId,
    counts: {
      entitiesOrphaned: entities.length,
      entitiesDeindexed: deindexedEntities,
      nounsDeindexed: deindexedNouns,
      cleanupScheduled: entities.length > 0,
    },
  })
})

// ── Full reset (delete ALL data from ALL tables) ────────────────────
// Wipe the Registry index — EntityDB DOs become orphaned (unreachable).
// Background process cleans up orphaned DOs after response is sent.
router.delete('/api/reset', async (request, env: Env, ctx: ExecutionContext) => {
  const registry = getRegistryDO(env, 'global') as any

  // Collect entity IDs before wiping
  const entities = await registry.getAllEntityIds() as Array<{ nounType: string; entityId: string }>
  const entityCount = entities.length

  // Wipe all Registry tables — instant
  await registry.wipeAll()


  // Background: soft-delete orphaned DOs (fire and forget)
  if (entities.length > 0) {
    const BATCH = 50
    ctx.waitUntil((async () => {
      for (let i = 0; i < entities.length; i += BATCH) {
        const chunk = entities.slice(i, i + BATCH)
        await Promise.allSettled(
          chunk.map(({ entityId }) => (getEntityDO(env, entityId) as any).delete()),
        )
      }
    })())
  }

  return json({
    reset: true,
    counts: {
      entitiesOrphaned: entityCount,
      cleanupScheduled: entityCount > 0,
    },
  })
})

// ── HATEOAS API: /arest/ routes ─────────────────────────────────────
// All /arest/ paths resolve against the constraint graph derived from IR.
// This is additive — the old /api/ routes remain untouched.

async function handleArestRoute(request: Request, env: Env) {
  const url = new URL(request.url)
  const userEmail = request.headers.get('x-user-email') || ''

  // Load domain schema (default: organizations)
  const registryDO = getRegistryDO(env, 'global') as any
  const irDomain = url.searchParams.get('ir_domain') || 'organizations'
  const getStubLocal = (id: string) => getEntityDO(env, id) as any

  // Engine path needs IR (for slug → noun via constraint graph + _links).
  // When the WASM engine traps on the deploy target, fall through to a
  // direct registry/EntityDB read for GET /arest/{slug}[/{id}] so the
  // public read surface stays alive even when HATEOAS link derivation
  // can't run. Engine-failure response loses _links and _schema.
  const fallbackDomain = url.searchParams.get('domain') || undefined
  const tryReadFallback = async () => {
    const fallback = await handleArestReadFallback({
      path: url.pathname,
      method: request.method,
      registry: registryDO,
      getStub: getStubLocal,
      domain: fallbackDomain,
    }).catch(() => null)
    if (fallback) return json(fallback)
    return null
  }

  let schemaHandle: number = -1
  try {
    schemaHandle = await loadDomainSchema(registryDO, getStubLocal, irDomain)
  } catch {
    schemaHandle = -1
  }
  if (schemaHandle < 0) {
    const fallbackResponse = await tryReadFallback()
    if (fallbackResponse) return fallbackResponse
    return json({ error: 'No schema loaded' }, { status: 500 })
  }

  let ir: any = null
  try {
    ir = JSON.parse(wasmSystem(schemaHandle, 'debug', '')) as any
  } catch {
    const fallbackResponse = await tryReadFallback()
    if (fallbackResponse) return fallbackResponse
    return json({ error: 'No schema loaded' }, { status: 500 })
  }

  if (!ir) {
    const fallbackResponse = await tryReadFallback()
    if (fallbackResponse) return fallbackResponse
    return json({ error: 'No schema loaded' }, { status: 500 })
  }

  // Debug: return IR shape if ?debug=ir
  if (url.searchParams.get('debug') === 'ir') {
    return json({
      factTypeKeys: Object.keys(ir.factTypes || ir.fact_types || {}),
      constraintCount: (ir.constraints || []).length,
      nounKeys: Object.keys(ir.nouns || {}),
      nounDetails: Object.fromEntries(
        Object.entries(ir.nouns || {}).map(([k, v]: [string, any]) => [k, { objectType: v.objectType, superType: (ir.subtypes || {})[k] || null }])
      ),
      instanceFacts: (ir.generalInstanceFacts || []).length,
    })
  }

  // Build population for root resource (needed to resolve org memberships)
  let population = { facts: {} as Record<string, any[]> }
  if (url.pathname === '/arest/' || url.pathname === '/arest') {
    try {
      const popJson = await buildPopulation(registryDO, (id: string) => getEntityDO(env, id) as any, 'organizations')
      try { forwardChain(popJson) } catch {}
      population = JSON.parse(popJson)
    } catch {}
  }

  let result: any = null
  try {
    result = await handleArestRequest({
      path: url.pathname,
      method: request.method,
      ir,
      registry: registryDO,
      getStub: (id: string) => getEntityDO(env, id) as any,
      userEmail,
      population,
    })
  } catch {
    result = null
  }

  // The IR is loaded for a single domain (default: organizations). Slugs
  // belonging to other domains (e.g. /arest/support-requests when IR is
  // organizations) won't resolve and handleArestRequest returns null.
  // The fallback walks the global Registry which sees nouns from every
  // domain, so it can satisfy cross-domain reads. Caller can opt into
  // the engine-only path by passing ?ir_domain=<domain>.
  if (!result) {
    const fallbackResponse = await tryReadFallback()
    if (fallbackResponse) return fallbackResponse
    return error(404, { errors: [{ message: 'Not Found' }] })
  }
  return json(result)
}

router.get('/arest', async (request, env: Env) => {
  return handleArestRoute(request, env)
})

router.get('/arest/*', async (request, env: Env) => {
  return handleArestRoute(request, env)
})

// ── AI Gateway completion (#638 / Worker-AI-1) ───────────────────────
// Foundational primitive that #639-#642 build on (engine `Func::Def`
// migration of /arest/extract + /arest/chat, Agent Definition seeds,
// e2e tests). The handler lives in src/api/ai/complete.ts and is
// install-shaped to plug into the kernel's
// `register_async_platform_fn("ai_complete", …)` once that wire lands.
//
// Wire shape:
//   POST /api/ai/complete
//   { prompt: string, model?: string, temperature?: number,
//     max_tokens?: number, extras?: object }
//   →
//   { text, _meta?, citations? }                  on success
//   { error: { code, message, status? } }          on failure
//
// The handler NEVER throws — failures land as a structured envelope
// (the engine maps those onto `Object::Bottom`).
router.post('/api/ai/complete', async (request: Request, env: Env) => {
  let body: { prompt?: string; model?: string; temperature?: number; max_tokens?: number; extras?: Record<string, unknown> }
  try {
    body = await request.json() as typeof body
  } catch {
    return error(400, { errors: [{ message: 'invalid JSON body' }] })
  }
  if (!body.prompt || typeof body.prompt !== 'string') {
    return error(400, { errors: [{ message: 'prompt (string) required' }] })
  }
  const result = await aiComplete(body.prompt, {
    env: {
      AI_GATEWAY_URL: env.AI_GATEWAY_URL ?? '',
      AI_GATEWAY_TOKEN: env.AI_GATEWAY_TOKEN ?? '',
    },
    model: body.model,
    temperature: body.temperature,
    max_tokens: body.max_tokens,
    extras: body.extras,
  })
  // Map the structured error envelope onto an HTTP status so callers
  // get the right network-level signal. `auth` → 401, `config` → 503
  // (server-side misconfig, retryable after fix), upstream/network →
  // 502, shape → 502 (gateway protocol violation).
  if ('error' in result) {
    const status =
      result.error.code === 'auth' ? 401 :
      result.error.code === 'config' ? 503 :
      502
    return json(result, { status })
  }
  return json(result)
})

// ── MCP-verb proxies for /arest/chat and /arest/extract (#201) ───────
// Thin POST proxies that delegate through dispatchVerb so CLI, local
// MCP (stdio), and HTTP clients share the same input/output contract.
//   /arest/chat    → dispatchVerb('query', body)  — natural-language query
router.post('/arest/chat', async (request: Request) => {
  let body: Record<string, unknown>
  try { body = (await request.json()) as Record<string, unknown> } catch { body = {} }
  try {
    const result = await dispatchVerb('query', body)
    return json(result)
  } catch (e) {
    return error(400, e instanceof Error ? e.message : String(e))
  }
})

// ── /arest/extract (#639 / Worker-AI-2 + #641 / Worker-AI-4) ─────────
// Migrated from the dispatchVerb('compile', body) placeholder (#619
// spike) to the real LLM extract pipeline:
//
//   1. Resolve the Agent Definition for verb "extract" via the four-
//      cell walker `resolveAgentVerb` (TS port of
//      `arest::agent::resolve_agent_verb`).
//   2. Render the prompt template with body as input.
//   3. Call `aiComplete(rendered_prompt, { env, model: binding.modelCode })`.
//   4. Parse the LLM output as JSON; on parse failure surface `_raw`.
//
// 503 envelope shape mirrors the kernel-side #620 path so HATEOAS-aware
// clients can branch on a single envelope schema across both targets.
//
// #641 (this commit) wires the boot-time Agent Definition seed
// (`AGENT_DEFINITIONS_STATE` from `./ai/agent-seed`) so the four-cell
// walker resolves to the Extractor Agent Definition. With AI_GATEWAY_*
// env vars set, requests now return 200 with the parsed JSON payload;
// without them, the 503 envelope's `agentDefinition` block carries
// the resolved model code + agent id (introspection works without a
// live LLM call). Mirror of the kernel's `system::init` Agent
// Definition seed pattern at `crates/arest-kernel/src/system.rs:262`.
router.post('/arest/extract', async (request: Request, env: Env) => {
  return handleExtract(request, {
    AI_GATEWAY_URL: env.AI_GATEWAY_URL ?? '',
    AI_GATEWAY_TOKEN: env.AI_GATEWAY_TOKEN ?? '',
  }, {
    state: AGENT_DEFINITIONS_STATE,
  })
})

// ── #205: List all known domains from the global registry ────────────
router.get('/api/domains', async (_request: Request, env: Env) => {
  const registry = getRegistryDO(env, 'global') as any
  const domains: string[] = await registry.listDomains()
  return json({ domains })
})

// ── 404 fallback ─────────────────────────────────────────────────────
router.all('*', () => error(404, { errors: [{ message: 'Not Found' }] }))

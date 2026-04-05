/**
 * SYSTEM:x = (ρ (↑entity(x) : D)) : ↑op(x)
 *
 * The AREST system function. The fact drives the application.
 * A fact is λf. f(data) — it receives a function and applies it to itself.
 * The entity decides how, not the router.
 */

import { loadDomainSchema, buildPopulation, forwardChain } from './engine'

// ── Types ───────────────────────────────────────────────────────────

/** A fact is λf. f(data). It receives a function and applies it to itself. */
type Fact = (op: Operation) => Promise<SystemOutput>

/** An operation is a function the fact applies to its own data. */
type Operation = (data: FactData) => Promise<SystemOutput>

interface FactData {
  noun: string
  id?: string
  domain: string
  params: Record<string, string>
  body?: unknown
  env: SystemEnv
}

export interface SystemEnv {
  registry: any
  getStub: (id: string) => any
}

export interface SystemOutput {
  status: number
  body: unknown
}

// ── DEFS — named definitions registered by the runtime ──────────────

/** DEFS maps (noun, operation) to the function that handles it.
 *  ρ looks up DEFS to resolve a fact to its functional form.
 *  Runtime registers functions via ↓DEFS (registerDef). */
type Defs = Map<string, Operation>

function defKey(noun: string, op: string): string { return `${noun}:${op}` }
function defaultKey(op: string): string { return `*:${op}` }

function registerDef(defs: Defs, noun: string, op: string, fn: Operation): void {
  defs.set(defKey(noun, op), fn)
}

function registerDefault(defs: Defs, op: string, fn: Operation): void {
  defs.set(defaultKey(op), fn)
}

/** ρ resolves a fact to its functional form by looking up DEFS.
 *  The fact is λf. f(data). ρ finds the right f from DEFS. */
function rho(defs: Defs, data: FactData): Fact {
  return async (op: Operation) => {
    // ρ-lookup: noun-specific definition first, then default
    const opName = Object.entries(operations).find(([, v]) => v === op)?.[0] || ''
    const fn = defs.get(defKey(data.noun, opName))
      || defs.get(defaultKey(opName))
      || op
    return fn(data)
  }
}

// ── Resolution functions ────────────────────────────────────────────

async function resolveLocal(data: FactData, page: number, limit: number): Promise<SystemOutput> {
  const ids: string[] = await data.env.registry.getEntityIds(data.noun, data.domain)
  const start = (page - 1) * limit
  const pageIds = ids.slice(start, start + limit)
  const settled = await Promise.allSettled(
    pageIds.map(async (id: string) => {
      const cell = await data.env.getStub(id).get()
      return cell ? { id: cell.id, type: cell.type, ...cell.data } : null
    }),
  )
  const docs = settled
    .filter((r): r is PromiseFulfilledResult<any> => r.status === 'fulfilled' && r.value !== null)
    .map((r) => r.value)

  return {
    status: 200,
    body: {
      docs,
      totalDocs: ids.length,
      limit,
      page,
      totalPages: Math.ceil(ids.length / limit),
      hasNextPage: start + limit < ids.length,
      hasPrevPage: page > 1,
    },
  }
}

async function resolveExternalSystem(data: FactData): Promise<{
  baseUrl: string | null
  secret: string | null
  authHeader: string | null
  authPrefix: string | null
  uri: string
}> {
  // 1. Find which External System this domain is connected to
  const domainSecretsDO = data.env.getStub(`domain-secrets:${data.domain}`)
  const connectedSystems: string[] = await domainSecretsDO.connectedSystems().catch(() => [])
  const systemName = connectedSystems[0] || ''

  // 2. Look up the system entity for base URL and auth config
  const systemCell = systemName
    ? await data.env.getStub(systemName).get().catch(() => null)
    : null

  // 3. Resolve the noun's URI from Instance Fact entities.
  // The readings declare: Noun 'API Product' has URI '/api'.
  // These are stored as Instance Fact entities with subjectNoun, subjectValue, fieldName, objectValue.
  let uri = `/${encodeURIComponent(data.noun)}`
  const instanceFactIds: string[] = await data.env.registry.getEntityIds('Instance Fact', data.domain).catch(() => [])
  if (instanceFactIds.length > 0) {
    const settled = await Promise.allSettled(
      instanceFactIds.map(async (id: string) => {
        const cell = await data.env.getStub(id).get()
        return cell ? cell.data : null
      }),
    )
    const nounUriFact = settled
      .filter((r): r is PromiseFulfilledResult<any> => r.status === 'fulfilled' && r.value)
      .map(r => r.value)
      .find((f: any) => f.subjectNoun === 'Noun' && f.subjectValue === data.noun
        && (f.fieldName === 'uri' || f.fieldName === 'URI'))
    if (nounUriFact?.objectValue) {
      uri = nounUriFact.objectValue
    }
  }

  // 4. Resolve secret
  const secret = systemName
    ? await domainSecretsDO.resolveSystemSecret(systemName).catch(() => null)
    : null

  return {
    baseUrl: systemCell?.data?.url || systemCell?.data?.baseUrl || null,
    secret,
    authHeader: systemCell?.data?.header || null,
    authPrefix: systemCell?.data?.prefix || null,
    uri,
  }
}

async function resolveExternal(data: FactData, page: number, limit: number): Promise<SystemOutput> {
  const ext = await resolveExternalSystem(data)
  const { baseUrl, secret, authHeader, authPrefix, uri } = ext

  const fetchUrl = baseUrl
    ? `${baseUrl}${uri}?page=${page}&limit=${limit}`
    : null

  const headers: Record<string, string> = { Accept: 'application/json' }
  const combined = [authPrefix, secret].filter(Boolean).join(' ')
  combined && authHeader && (headers[authHeader] = combined)

  const response = fetchUrl
    ? await fetch(fetchUrl, { headers }).catch(() => null)
    : null

  const raw: any = response?.ok ? await response.json().catch(() => null) : null
  const rawDocs = Array.isArray(raw) ? raw : raw?.docs || (raw ? [raw] : [])
  const docs = rawDocs.map((item: any, i: number) => ({
    id: item.id || item._id || String(i),
    type: data.noun,
    ...item,
  }))

  return {
    status: 200,
    body: {
      docs,
      totalDocs: raw?.totalDocs || docs.length,
      limit,
      page,
      totalPages: raw?.totalPages || 1,
      hasNextPage: raw?.hasNextPage || false,
      hasPrevPage: raw?.hasPrevPage || false,
    },
  }
}

/**
 * Build a derivation trace for the given entity.
 * Loads the domain schema and population, runs the forward chainer,
 * and filters derived facts to those involving this entity.
 */
async function buildTrace(
  data: FactData,
): Promise<Array<{ rule: string; reading: string; bindings: Array<[string, string]> }>> {
  await loadDomainSchema(data.env.registry, data.env.getStub, data.domain)
  const popJson = await buildPopulation(data.env.registry, data.env.getStub, data.domain)
  const derived = forwardChain(popJson)
  const entityId = data.id || ''
  return derived
    .filter((fact: any) => fact.bindings.some(([, v]: [any, any]) => v === entityId))
    .map((fact: any) => ({ rule: fact.derivedBy, reading: fact.reading, bindings: fact.bindings }))
}

async function resolveLocalDetail(data: FactData): Promise<SystemOutput> {
  const cell = await data.env.getStub(data.id || '').get().catch(() => null)
  const includeTrace = data.params.trace === 'true'

  const trace = includeTrace
    ? await buildTrace(data).catch(() => [])
    : undefined

  return cell
    ? { status: 200, body: { id: cell.id, type: cell.type, ...cell.data, ...(trace ? { _trace: trace } : {}) } }
    : { status: 404, body: { errors: [{ message: 'Not found' }] } }
}

async function resolveExternalDetail(data: FactData): Promise<SystemOutput> {
  const { baseUrl, secret, authHeader, authPrefix } = await resolveExternalSystem(data)

  // First, fetch the local entity to get its endpoint path and other metadata
  const localCell = await data.env.getStub(data.id || '').get().catch(() => null)
  const endpointPath = localCell?.data?.endpointPath || localCell?.endpointPath

  // If the entity has an endpoint path, fetch live data from the backing service
  if (baseUrl && endpointPath) {
    const headers: Record<string, string> = { Accept: 'application/json' }
    const combined = [authPrefix, secret].filter(Boolean).join(' ')
    if (combined && authHeader) headers[authHeader] = combined

    const fetchUrl = `${baseUrl}${endpointPath}`
    const response = await fetch(fetchUrl, { headers }).catch(() => null)
    const raw = response?.ok ? await response.json().catch(() => null) : null

    if (raw) {
      // Merge live data with local entity metadata
      const merged = { id: data.id, type: data.noun, ...(localCell?.data || {}), _live: raw }
      return { status: 200, body: merged }
    }
  }

  // Fall back to local entity
  return localCell
    ? { status: 200, body: { id: localCell.id, type: localCell.type, ...localCell.data } }
    : { status: 404, body: { errors: [{ message: 'Not found' }] } }
}

// ── Operations ──────────────────────────────────────────────────────

/**
 * Operations are functions passed to facts as operands.
 * Each operation handles both local and backed resolution
 * through the same interface — the fact's data tells it what to do.
 */

const read: Operation = async (data) => {
  const page = parseInt(data.params.page || '1')
  const limit = parseInt(data.params.limit || '100')
  // List always resolves locally. Entities are seeded from readings.
  // Federation applies at the detail level (resolveExternalDetail).
  return resolveLocal(data, page, limit)
}

const readDetail: Operation = async (data) => {
  return resolveLocalDetail(data)
}

// ── Create pipeline — equation (10): create = emit ∘ validate ∘ derive ∘ resolve

type Stage = (ctx: PipelineContext) => Promise<PipelineContext>

interface PipelineContext {
  data: FactData
  entityId: string
  facts: Record<string, unknown>
  violations: string[]
  derived: any[]
}

const resolveStage: Stage = async (ctx) => {
  const entityId = crypto.randomUUID()
  return { ...ctx, entityId, facts: (ctx.data.body || {}) as Record<string, unknown> }
}

const deriveStage: Stage = async (ctx) => {
  // Forward chain will be wired when WASM evaluator is entity-driven
  return ctx
}

const validateStage: Stage = async (ctx) => {
  // Constraint evaluation will be wired when WASM evaluator is entity-driven
  return ctx
}

const emitStage: Stage = async (ctx) => {
  const entityDO = ctx.data.env.getStub(ctx.entityId)
  await entityDO.put({ id: ctx.entityId, type: ctx.data.noun, data: ctx.facts })
  await ctx.data.env.registry.materializeBatch([{
    id: ctx.entityId, type: ctx.data.noun, domain: ctx.data.domain, data: ctx.facts,
  }])
  return ctx
}

// create = emit ∘ validate ∘ derive ∘ resolve (equation 10)
const compose = (...stages: Stage[]): Stage =>
  stages.reduce((composed, stage) => async (ctx) => stage(await composed(ctx)))

const createPipeline = compose(resolveStage, deriveStage, validateStage, emitStage)

const create: Operation = async (data) => {
  const ctx = await createPipeline({ data, entityId: '', facts: {}, violations: [], derived: [] })
  const cell = await data.env.getStub(ctx.entityId).get()
  return { status: 201, body: cell }
}

const operations: Record<string, Operation> = { read, readDetail, create }

// ── SYSTEM function ─────────────────────────────────────────────────

/**
 * SYSTEM:x = (ρ (↑entity(x) : D)) : ↑op(x)
 *
 * Fetch the entity from D. Resolve its ρ (determined by type + DEFS).
 * Apply the operation as the operand. The entity handles dispatch.
 */
export async function system(
  noun: string,
  domain: string,
  op: string,
  params: Record<string, string>,
  env: SystemEnv,
  body?: unknown,
): Promise<SystemOutput> {
  // ↑DEFS — fetch persistent definitions
  const defsCell = await env.getStub(`defs:${domain}`).get().catch(() => null)
  const defsData: Record<string, string> = defsCell?.data || {}

  const defs: Defs = new Map()
  registerDefault(defs, 'read', read)
  registerDefault(defs, 'readDetail', readDetail)
  registerDefault(defs, 'create', create)

  // Register noun-specific operations from persisted DEFS
  for (const [key, value] of Object.entries(defsData)) {
    const [defNoun, defOp] = key.split(':')
    if (defNoun === '*') continue  // defaults already registered
    if (value === 'external' && defOp === 'readDetail') {
      registerDef(defs, defNoun, 'readDetail', resolveExternalDetail)
    }
    if (value === 'external' && defOp === 'read') {
      registerDef(defs, defNoun, 'read', async (d) => {
        const page = parseInt(d.params.page || '1')
        const limit = parseInt(d.params.limit || '100')
        return resolveExternal(d, page, limit)
      })
    }
  }

  // Build the fact's data
  const factData: FactData = {
    noun,
    id: params._id,
    domain,
    params,
    body,
    env,
  }

  // ↑op(x) — resolve the operation from the HTTP method
  const operation = operations[op]

  // (ρ fact) : op — ρ looks up DEFS for this noun and operation
  const fact = rho(defs, factData)
  return fact(operation)
}

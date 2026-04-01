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
  backedBy?: string
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

// ── ρ — resolve a fact to its functional form ───────────────────────

/**
 * ρ resolves a fact to its functional form.
 * The fact is λf. f(data). The type determines what f can do with data.
 */
const rho = (data: FactData): Fact =>
  (op: Operation) => op(data)

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

  // 3. Resolve the noun's URI from the IR instance facts.
  // The readings declare: Noun 'API Product' has URI '/api'.
  // Parsed as GeneralInstanceFact: { subjectNoun: "Noun", subjectValue: "API Product", fieldName: "uri", objectValue: "/api" }
  let uri = `/${encodeURIComponent(data.noun)}`
  const irCell = await data.env.getStub(`ir:${data.domain}`).get().catch(() => null)
  if (irCell?.data?.ir) {
    const ir = typeof irCell.data.ir === 'string' ? JSON.parse(irCell.data.ir) : irCell.data.ir
    const nounUriFact = (ir.generalInstanceFacts || []).find(
      (f: any) => f.subjectNoun === 'Noun' && f.subjectValue === data.noun
        && (f.fieldName === 'uri' || f.fieldName === 'URI')
    )
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

  const raw = response?.ok ? await response.json().catch(() => null) : null
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
    .filter(fact => fact.bindings.some(([, v]) => v === entityId))
    .map(fact => ({ rule: fact.derivedBy, reading: fact.reading, bindings: fact.bindings }))
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
  const resolve = data.backedBy ? resolveExternalDetail : resolveLocalDetail
  return resolve(data)
}

const create: Operation = async (data) => {
  const entityId = crypto.randomUUID()
  const entityDO = data.env.getStub(entityId)
  await entityDO.put({ id: entityId, type: data.noun, data: data.body || {} })
  await data.env.registry.materializeBatch([{
    id: entityId, type: data.noun, domain: data.domain, data: (data.body || {}) as Record<string, unknown>,
  }])
  const cell = await entityDO.get()
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
  // ↑entity(x) : D — fetch the noun definition from the IR cell
  const irCell = await env.getStub(`ir:${domain}`).get().catch(() => null)
  const ir = irCell?.data?.ir
    ? JSON.parse(typeof irCell.data.ir === 'string' ? irCell.data.ir : JSON.stringify(irCell.data.ir))
    : null
  const nounDef = ir?.nouns?.[noun]

  // Build the fact's data — everything the fact needs to apply an operation
  const factData: FactData = {
    noun,
    id: params._id,
    domain,
    params,
    body,
    backedBy: nounDef?.backedBy,
    env,
  }

  // ρ resolves the fact to a functional form based on its type.
  // The fact is λf. f(data). It receives the operation and applies it.
  const fact = rho(factData)

  // ↑op(x) — resolve the operation from the HTTP method
  const operation = operations[op]

  // (ρ fact) : op — the fact applies the operation
  return fact(operation)
}

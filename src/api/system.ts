/**
 * SYSTEM:x = (ρ (↑entity(x) : D)) : ↑op(x)
 *
 * The AREST system function. The fact drives the application.
 * A fact is λf. f(data) — it receives a function and applies it to itself.
 * The entity decides how, not the router.
 */

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

async function resolveExternal(data: FactData, page: number, limit: number): Promise<SystemOutput> {
  const systemIds: string[] = await data.env.registry.getEntityIds('External System', 'core').catch(() => [])
  const settled = await Promise.allSettled(
    systemIds.map(async (id: string) => {
      const cell = await data.env.getStub(id).get()
      return cell ? { id: cell.id, ...cell.data } : null
    }),
  )
  const systemEntity = settled
    .filter((r): r is PromiseFulfilledResult<any> => r.status === 'fulfilled' && r.value !== null)
    .map((r) => r.value)
    .find((s: any) => s.name === data.backedBy)

  const baseUrl = systemEntity?.baseUrl
  const fetchUrl = baseUrl
    ? `${baseUrl}/${encodeURIComponent(data.noun)}?page=${page}&limit=${limit}`
    : null

  const response = fetchUrl
    ? await fetch(fetchUrl, { headers: { Accept: 'application/json' } }).catch(() => null)
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

async function resolveLocalDetail(data: FactData): Promise<SystemOutput> {
  const cell = await data.env.getStub(data.id || '').get().catch(() => null)
  return cell
    ? { status: 200, body: { id: cell.id, type: cell.type, ...cell.data } }
    : { status: 404, body: { errors: [{ message: 'Not found' }] } }
}

async function resolveExternalDetail(data: FactData): Promise<SystemOutput> {
  const systemIds: string[] = await data.env.registry.getEntityIds('External System', 'core').catch(() => [])
  const settled = await Promise.allSettled(
    systemIds.map(async (id: string) => {
      const cell = await data.env.getStub(id).get()
      return cell ? { id: cell.id, ...cell.data } : null
    }),
  )
  const systemEntity = settled
    .filter((r): r is PromiseFulfilledResult<any> => r.status === 'fulfilled' && r.value !== null)
    .map((r) => r.value)
    .find((s: any) => s.name === data.backedBy)

  const baseUrl = systemEntity?.baseUrl
  const fetchUrl = baseUrl
    ? `${baseUrl}/${encodeURIComponent(data.noun)}/${encodeURIComponent(data.id || '')}`
    : null

  const response = fetchUrl
    ? await fetch(fetchUrl, { headers: { Accept: 'application/json' } }).catch(() => null)
    : null

  const raw = response?.ok ? await response.json().catch(() => null) : null
  return raw
    ? { status: 200, body: { id: raw.id || data.id, type: data.noun, ...raw } }
    : { status: 404, body: { errors: [{ message: 'Not found in External System' }] } }
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
  const resolve = data.backedBy ? resolveExternal : resolveLocal
  return resolve(data, page, limit)
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

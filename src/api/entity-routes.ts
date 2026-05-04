/**
 * Entity-type route handlers — pure async functions that operate on
 * injected stubs (RegistryStub, EntityStub) rather than env, so they
 * are fully testable without the Cloudflare runtime.
 *
 * Per the AREST whitepaper:
 *   - Each entity is a cell: ⟨CELL, id, contents⟩
 *   - ↑n fetches, ↓n stores
 *   - The population P is the set of all cells
 *   - If a cell isn't in the registry, it's not in the population
 */

// ---------------------------------------------------------------------------
// Stub interfaces (match EntityDB / RegistryDB DO RPCs)
// ---------------------------------------------------------------------------

export interface CellRecord {
  id: string
  type: string
  data: Record<string, unknown>
}

export interface EntityReadStub {
  get(): Promise<CellRecord | null>
}

export interface EntityWriteStub {
  put(input: { id: string; type: string; data: Record<string, unknown> }): Promise<CellRecord>
  delete(): Promise<{ id: string } | null>
}

export interface RegistryReadStub {
  getEntityIds(entityType: string, domainSlug?: string): Promise<string[]>
}

export interface RegistryWriteStub {
  indexEntity(entityType: string, entityId: string, domainSlug?: string): Promise<void>
  deindexEntity(entityType: string, entityId: string): Promise<void>
}

// ---------------------------------------------------------------------------
// Result types
// ---------------------------------------------------------------------------

export interface ListResult {
  docs: CellRecord[]
  totalDocs: number
  limit: number
  page: number
  totalPages: number
  hasNextPage: boolean
  hasPrevPage: boolean
  warnings?: string[]
}

export interface PaginationOpts {
  limit?: number
  page?: number
  depth?: number
}

export interface DepthOpts {
  depth?: number
  getStub?: (id: string) => EntityReadStub
}

// ---------------------------------------------------------------------------
// populateDepthForEntity
// ---------------------------------------------------------------------------

export async function populateDepthForEntity(
  entity: CellRecord,
  depth: number,
  getStub: (id: string) => EntityReadStub,
): Promise<Record<string, unknown>> {
  if (depth <= 0) return entity.data

  const populated = { ...entity.data }
  for (const [key, value] of Object.entries(populated)) {
    if (key.endsWith('Id') && typeof value === 'string') {
      try {
        const refStub = getStub(value)
        const refEntity = await refStub.get()
        if (refEntity) {
          populated[key.replace(/Id$/, '')] = {
            id: refEntity.id,
            ...refEntity.data,
          }
        }
      } catch {
        /* leave as ID if unreachable */
      }
    }
  }
  return populated
}

// ---------------------------------------------------------------------------
// handleListEntities
// ---------------------------------------------------------------------------

export async function handleListEntities(
  type: string,
  domain: string,
  registry: RegistryReadStub,
  getStub: (id: string) => EntityReadStub,
  opts?: PaginationOpts,
): Promise<ListResult> {
  const limit = opts?.limit ?? 100
  const page = opts?.page ?? 1

  const ids = await registry.getEntityIds(type, domain)

  const settled = await Promise.allSettled(
    ids.map(async (id) => {
      const stub = getStub(id)
      const cell = await stub.get()
      return { id, cell }
    }),
  )

  const docs: CellRecord[] = []
  const warnings: string[] = []

  for (const result of settled) {
    if (result.status === 'rejected') {
      const idx = settled.indexOf(result)
      warnings.push(ids[idx])
      continue
    }
    const { cell } = result.value
    if (cell) {
      docs.push(cell)
    }
  }

  // Depth population
  const depth = opts?.depth ?? 0
  if (depth >= 1) {
    await Promise.all(
      docs.map(async (doc) => {
        doc.data = await populateDepthForEntity(doc, depth, getStub)
      }),
    )
  }

  // Pagination
  const totalDocs = docs.length
  const offset = (page - 1) * limit
  const paged = limit > 0 ? docs.slice(offset, offset + limit) : docs
  const totalPages = limit > 0 ? Math.ceil(totalDocs / limit) : 1

  const hasNextPage = limit > 0 && offset + limit < totalDocs
  const hasPrevPage = page > 1

  return {
    docs: paged,
    totalDocs,
    limit,
    page,
    totalPages,
    hasNextPage,
    hasPrevPage,
    ...(warnings.length > 0 && { warnings }),
  }
}

// ---------------------------------------------------------------------------
// handleGetEntity
// ---------------------------------------------------------------------------

export async function handleGetEntity(
  stub: EntityReadStub,
  opts?: DepthOpts,
): Promise<CellRecord | null> {
  const cell = await stub.get()
  if (!cell) return null

  const depth = opts?.depth ?? 0
  if (depth >= 1 && opts?.getStub) {
    cell.data = await populateDepthForEntity(cell, depth, opts.getStub)
  }

  return cell
}

// ---------------------------------------------------------------------------
// Broadcast write stub — post-mutation signal delivery.
// ---------------------------------------------------------------------------

/**
 * Minimal interface the create/delete handlers need from BroadcastDO.
 * Accepts any object with the same shape so tests can pass a mock
 * without importing the DO runtime. The full DO type lives in
 * src/broadcast-do.ts.
 */
export interface BroadcastWriteStub {
  publish(event: {
    domain: string
    noun: string
    entityId: string
    operation: 'create' | 'update' | 'delete' | 'transition'
    facts: Record<string, unknown>
    timestamp: number
  }): Promise<unknown>
}

// ---------------------------------------------------------------------------
// handleCreateEntity
// ---------------------------------------------------------------------------

export interface CreateEntityResult {
  id: string
  type: string
}

export async function handleCreateEntity(
  type: string,
  domain: string,
  data: Record<string, unknown>,
  getStub: (id: string) => EntityWriteStub,
  registry: RegistryWriteStub,
  explicitId?: string,
  broadcast?: BroadcastWriteStub,
): Promise<CreateEntityResult> {
  const id = explicitId || crypto.randomUUID()
  const stub = getStub(id)
  await stub.put({ id, type, data })
  await registry.indexEntity(type, id, domain)
  // Kernel signal: fan out a 'create' event to any subscriber whose
  // filter matches. Best-effort — a broadcast failure must not roll
  // back the mutation (Definition 3-style: the write is committed).
  if (broadcast) {
    try {
      await broadcast.publish({
        domain, noun: type, entityId: id, operation: 'create',
        facts: data, timestamp: Date.now(),
      })
    } catch { /* signal-delivery is best-effort */ }
  }
  return { id, type }
}

// ---------------------------------------------------------------------------
// handleDeleteEntity
// ---------------------------------------------------------------------------

export async function handleDeleteEntity(
  id: string,
  stub: EntityWriteStub,
  registry: RegistryWriteStub,
  type: string,
  domain?: string,
  broadcast?: BroadcastWriteStub,
): Promise<{ id: string; deleted: boolean } | null> {
  const result = await stub.delete()
  if (!result) return null
  await registry.deindexEntity(type, id)
  if (broadcast && domain) {
    try {
      await broadcast.publish({
        domain, noun: type, entityId: id, operation: 'delete',
        facts: {}, timestamp: Date.now(),
      })
    } catch { /* signal-delivery is best-effort */ }
  }
  return { id: result.id, deleted: true }
}

// ---------------------------------------------------------------------------
// applyEntityCommand — engine-path entity write (#699 / Audit T1)
// ---------------------------------------------------------------------------

// Audit T1 (#699). The bypass routes used to call EntityDO.put() directly,
// skipping validate + derive + Theorem 5 envelope. This helper threads
// every entity write through the engine: build population from EntityDB,
// call applyCommand (which runs validate then derive), persist returned
// __state entities, broadcast the mutation.
//
// Wire-shape parity with `dispatchVerb('apply', ...)` — the audit text's
// "reroute every entity write through dispatchVerb('apply', ...)" maps
// 1:1 to this `{ operation, noun, id, fields }` input. dispatchVerb
// itself takes the engine-state path (no DO-backed population), which
// the worker target can't satisfy without round-tripping every cell on
// every request — hence this DO-aware sibling.

/** Engine surface this helper depends on — narrow types so a test
 * can pass a mock without touching the wasm-pack module. */
export interface EngineDeps {
  /** Build a population JSON from registry + DOs for `domain`. */
  loadDomainAndPopulation(
    registry: any,
    getStub: (id: string) => any,
    domain: string,
  ): Promise<string>
  /** Apply a Command against a population, returning the engine result. */
  applyCommand(command: any, populationJson: string): any
}

export type EntityCommandOperation = 'create' | 'update'

export interface ApplyEntityCommandInput {
  operation: EntityCommandOperation
  noun: string
  domain: string
  /** Required for 'update'; optional for 'create' (engine resolves via
   *  reference scheme when absent). */
  id?: string
  fields: Record<string, unknown>
}

export interface EngineEntity {
  id?: string
  type: string
  data: Record<string, unknown>
}

export interface EngineViolation {
  reading?: string
  constraintId?: string
  detail?: string
  modality?: 'alethic' | 'deontic'
}

export interface ApplyEntityCommandResult {
  /** True when validate rejected the command. Caller maps to 422. */
  rejected: boolean
  /** Primary entity id — first persisted entity, or the input id. */
  id: string
  status: string | null
  transitions: unknown[]
  derivedCount: number
  violations: EngineViolation[]
  /** Entities the engine asked to persist (after validate + derive).
   *  Already written to DOs + indexed before this returns. */
  entities: EngineEntity[]
}

export async function applyEntityCommand(
  input: ApplyEntityCommandInput,
  registry: RegistryWriteStub & RegistryReadStub & { [k: string]: any },
  getStub: (id: string) => EntityWriteStub & EntityReadStub,
  engine: EngineDeps,
  broadcast?: BroadcastWriteStub,
): Promise<ApplyEntityCommandResult> {
  if (input.operation === 'update' && !input.id) {
    throw new Error('applyEntityCommand: update requires `id`')
  }

  const populationJson = await engine.loadDomainAndPopulation(
    registry,
    getStub,
    input.domain,
  )

  const cmd = input.operation === 'create'
    ? {
        type: 'createEntity',
        noun: input.noun,
        domain: input.domain,
        id: input.id ?? null,
        fields: input.fields,
      }
    : {
        type: 'updateEntity',
        noun: input.noun,
        domain: input.domain,
        entityId: input.id,
        fields: input.fields,
      }

  const arestResult = engine.applyCommand(cmd, populationJson)

  if (arestResult?.rejected) {
    return {
      rejected: true,
      id: input.id ?? '',
      status: null,
      transitions: [],
      derivedCount: 0,
      violations: arestResult.violations ?? [],
      entities: [],
    }
  }

  const entities: EngineEntity[] = arestResult?.entities ?? []
  for (const entity of entities) {
    const eid = entity.id || crypto.randomUUID()
    entity.id = eid
    await getStub(eid).put({ id: eid, type: entity.type, data: entity.data })
    await registry.indexEntity(entity.type, eid, input.domain)
  }

  const primaryId = entities[0]?.id ?? input.id ?? crypto.randomUUID()

  if (broadcast) {
    try {
      await broadcast.publish({
        domain: input.domain,
        noun: input.noun,
        entityId: primaryId,
        operation: input.operation,
        facts: input.fields,
        timestamp: Date.now(),
      })
    } catch { /* signal-delivery is best-effort */ }
  }

  return {
    rejected: false,
    id: primaryId,
    status: arestResult?.status ?? null,
    transitions: arestResult?.transitions ?? [],
    derivedCount: arestResult?.derivedCount ?? 0,
    violations: arestResult?.violations ?? [],
    entities,
  }
}

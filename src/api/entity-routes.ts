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

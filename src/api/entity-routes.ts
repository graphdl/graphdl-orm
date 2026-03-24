/**
 * Entity-type route handlers — pure async functions that operate on
 * injected stubs (RegistryStub, EntityStub) rather than env, so they
 * are fully testable without the Cloudflare runtime.
 *
 * Uses Promise.allSettled for fan-out resilience (same pattern as materializer).
 */

// ---------------------------------------------------------------------------
// Stub interfaces (same shapes as EntityDB / RegistryDB DO RPCs)
// ---------------------------------------------------------------------------

export interface EntityReadStub {
  get(): Promise<EntityRecord | null>
}

export interface EntityWriteStub {
  put(input: { id: string; type: string; data: Record<string, unknown> }): Promise<{ id: string; version: number }>
  delete(): Promise<{ id: string; deleted: boolean } | null>
}

export interface RegistryReadStub {
  getEntityIds(entityType: string, domainSlug?: string): Promise<string[]>
}

export interface RegistryWriteStub {
  indexEntity(entityType: string, entityId: string, domainSlug?: string): Promise<void>
  deindexEntity(entityType: string, entityId: string): Promise<void>
}

export interface EntityRecord {
  id: string
  type: string
  data: Record<string, unknown>
  version: number
  deletedAt?: string
  createdAt?: string
  updatedAt?: string
}

// ---------------------------------------------------------------------------
// Result types
// ---------------------------------------------------------------------------

export interface ListResult {
  docs: EntityRecord[]
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
}

// ---------------------------------------------------------------------------
// handleListEntities
// ---------------------------------------------------------------------------

/**
 * Fan out to EntityDB DOs for a given entity type + domain.
 * Returns a Payload-style paginated result with an optional warnings array
 * tracking entity IDs whose DOs were unreachable.
 */
export async function handleListEntities(
  type: string,
  domain: string,
  registry: RegistryReadStub,
  getStub: (id: string) => EntityReadStub,
  opts?: PaginationOpts,
): Promise<ListResult> {
  const limit = opts?.limit ?? 100
  const page = opts?.page ?? 1

  // Ask the Registry which entity IDs match this type + domain
  const ids = await registry.getEntityIds(type, domain)

  // Fan out reads with allSettled for resilience
  const settled = await Promise.allSettled(
    ids.map(async (id) => {
      const stub = getStub(id)
      const entity = await stub.get()
      return { id, entity }
    }),
  )

  const docs: EntityRecord[] = []
  const warnings: string[] = []

  for (const result of settled) {
    if (result.status === 'rejected') {
      // Extract the entity ID from the original promise — we need to find it
      // by index since allSettled preserves order
      const idx = settled.indexOf(result)
      warnings.push(ids[idx])
      continue
    }
    const { entity } = result.value
    if (entity && !entity.deletedAt) {
      docs.push(entity)
    }
  }

  // Pagination
  const totalDocs = docs.length
  const offset = (page - 1) * limit
  const paged = limit > 0 ? docs.slice(offset, offset + limit) : docs
  const totalPages = limit > 0 ? Math.ceil(totalDocs / limit) : 1

  return {
    docs: paged,
    totalDocs,
    limit,
    page,
    totalPages,
    hasNextPage: limit > 0 && offset + limit < totalDocs,
    hasPrevPage: page > 1,
    ...(warnings.length > 0 && { warnings }),
  }
}

// ---------------------------------------------------------------------------
// handleGetEntity
// ---------------------------------------------------------------------------

/**
 * Fetch a single entity from its EntityDB DO.
 * Returns null if not found or soft-deleted.
 */
export async function handleGetEntity(
  stub: EntityReadStub,
): Promise<EntityRecord | null> {
  const entity = await stub.get()
  if (!entity || entity.deletedAt) return null
  return entity
}

// ---------------------------------------------------------------------------
// handleCreateEntity
// ---------------------------------------------------------------------------

/**
 * Create an entity in its EntityDB DO and index it in the Registry.
 * Generates a UUID for the new entity.
 */
export async function handleCreateEntity(
  type: string,
  domain: string,
  data: Record<string, unknown>,
  getStub: (id: string) => EntityWriteStub,
  registry: RegistryWriteStub,
): Promise<{ id: string; version: number }> {
  const id = crypto.randomUUID()
  const stub = getStub(id)
  const result = await stub.put({ id, type, data })
  await registry.indexEntity(type, id, domain)
  return { id, version: result.version }
}

// ---------------------------------------------------------------------------
// handleDeleteEntity
// ---------------------------------------------------------------------------

/**
 * Soft-delete an entity via its EntityDB DO and deindex it from the Registry.
 * Returns null if entity not found.
 */
export async function handleDeleteEntity(
  id: string,
  stub: EntityWriteStub,
  registry: RegistryWriteStub,
  type: string,
): Promise<{ id: string; deleted: boolean } | null> {
  const result = await stub.delete()
  if (!result) return null
  await registry.deindexEntity(type, id)
  return result
}

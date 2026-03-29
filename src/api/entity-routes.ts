/**
 * Entity-type route handlers — pure async functions that operate on
 * injected stubs (RegistryStub, EntityStub) rather than env, so they
 * are fully testable without the Cloudflare runtime.
 *
 * Uses Promise.allSettled for fan-out resilience (same pattern as materializer).
 */

// Deontic constraints evaluated by WASM engine, not procedural TS code.

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
  _links?: Record<string, { href: string; method?: string }>
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
  _links?: Record<string, string>  // List-level navigation links remain plain strings
}

export interface PaginationOpts {
  limit?: number
  page?: number
  depth?: number
}

export interface TransitionInfo {
  transitionId: string
  event: string
  targetStatus: string
  targetStatusId: string
}

export interface DepthOpts {
  depth?: number
  getStub?: (id: string) => EntityReadStub
  /** Valid transitions from current status — resolved by engine at router level. */
  transitions?: TransitionInfo[]
  /** Domain slug — used for HATEOAS link generation. */
  domain?: string
}

// ---------------------------------------------------------------------------
// HATEOAS link builders
// ---------------------------------------------------------------------------

/**
 * Reorder entity data fields by graph schema role order.
 * Fields in the order list come first (in order), then remaining fields alphabetically.
 * System fields (_status, _statusId, etc.) always go last.
 */
function orderData(data: Record<string, unknown>, fieldOrder: string[]): Record<string, unknown> {
  const ordered: Record<string, unknown> = {}
  const systemKeys = new Set<string>()
  const seen = new Set<string>()

  // 1. Ordered fields first
  for (const field of fieldOrder) {
    // Try camelCase variants
    const camel = field.split(' ').map((w, i) => i === 0 ? w.toLowerCase() : w.charAt(0).toUpperCase() + w.slice(1).toLowerCase()).join('')
    for (const key of [field, camel]) {
      if (key in data && !seen.has(key)) {
        ordered[key] = data[key]
        seen.add(key)
      }
    }
  }

  // 2. Remaining data fields alphabetically (skip system fields)
  for (const key of Object.keys(data).sort()) {
    if (seen.has(key)) continue
    if (key.startsWith('_')) { systemKeys.add(key); continue }
    ordered[key] = data[key]
    seen.add(key)
  }

  // 3. System fields last
  for (const key of [...systemKeys].sort()) {
    ordered[key] = data[key]
  }

  return ordered
}

/**
 * Navigation links for a single entity.
 * Derived from the entity's type (noun name from readings) and domain.
 * The URL structure is a projection of the graph schema — entity types
 * and their relationships are declared in readings, not hardcoded.
 */
export function buildEntityLinks(
  type: string,
  id: string,
  domain?: string,
  transitions?: TransitionInfo[],
): Record<string, { href: string; method?: string }> {
  const encoded = encodeURIComponent(type)
  const base = `/api/entities/${encoded}/${id}`
  const qs = domain ? `?domain=${domain}` : ''
  const links: Record<string, { href: string; method?: string }> = {
    self: { href: base },
    collection: { href: `/api/entities/${encoded}${qs}` },
  }

  // Transitions as links (per AREST whitepaper — transitions ARE links)
  if (transitions) {
    for (const t of transitions) {
      links[t.event] = { href: `${base}/transition`, method: 'POST' }
    }
  }

  return links
}

/** Navigation links for a paginated list. */
export function buildListLinks(
  type: string,
  domain: string,
  page: number,
  limit: number,
  hasNextPage: boolean,
  hasPrevPage: boolean,
): Record<string, string> {
  const encoded = encodeURIComponent(type)
  const base = `/api/entities/${encoded}`
  const qs = (p: number) => {
    const parts: string[] = []
    if (domain) parts.push(`domain=${domain}`)
    parts.push(`page=${p}`)
    parts.push(`limit=${limit}`)
    return parts.length > 0 ? `?${parts.join('&')}` : ''
  }
  const links: Record<string, string> = {
    self: `${base}${qs(page)}`,
    create: base,
  }
  if (hasNextPage) links.next = `${base}${qs(page + 1)}`
  if (hasPrevPage) links.prev = `${base}${qs(page - 1)}`
  if (domain) links.domain = `/api/entities/Domain?domain=${domain}`
  return links
}

// ---------------------------------------------------------------------------
// populateDepthForEntity
// ---------------------------------------------------------------------------

/**
 * Scan entity data for fields ending in `Id` (e.g., `graphSchemaId`).
 * For each, do a secondary fan-out to resolve the referenced entity.
 * The resolved object is added alongside the original ID field.
 */
export async function populateDepthForEntity(
  entity: { id: string; type: string; data: Record<string, unknown>; version: number },
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
    _links: buildListLinks(type, domain, page, limit, hasNextPage, hasPrevPage),
  }
}

// ---------------------------------------------------------------------------
// handleGetEntity
// ---------------------------------------------------------------------------

/**
 * Fetch a single entity from its EntityDB DO.
 * Returns null if not found or soft-deleted.
 * When depth >= 1, resolves Id-suffixed fields to full entity objects.
 * When opts.transitions is provided, they are folded into _links per the AREST whitepaper.
 */
export async function handleGetEntity(
  stub: EntityReadStub,
  opts?: DepthOpts,
): Promise<EntityRecord | null> {
  const entity = await stub.get()
  if (!entity || entity.deletedAt) return null

  const depth = opts?.depth ?? 0
  if (depth >= 1 && opts?.getStub) {
    entity.data = await populateDepthForEntity(entity, depth, opts.getStub)
  }

  // HATEOAS links — transitions folded in per AREST whitepaper
  entity._links = buildEntityLinks(entity.type, entity.id, opts?.domain, opts?.transitions)

  return entity
}

// ---------------------------------------------------------------------------
// handleCreateEntity
// ---------------------------------------------------------------------------

export interface CreateEntityResult {
  id: string
  version: number
  _links?: Record<string, { href: string; method?: string }>
}

/**
 * Create an entity in its EntityDB DO and index it in the Registry.
 * Generates a UUID for the new entity.
 *
 * Deontic constraint checking is done by the WASM engine at the router level,
 * not here. This function is pure CRUD — create + index.
 */
export async function handleCreateEntity(
  type: string,
  domain: string,
  data: Record<string, unknown>,
  getStub: (id: string) => EntityWriteStub,
  registry: RegistryWriteStub,
  explicitId?: string,
): Promise<CreateEntityResult> {
  const id = explicitId || crypto.randomUUID()
  const stub = getStub(id)
  const result = await stub.put({ id, type, data })
  await registry.indexEntity(type, id, domain)
  return { id, version: result.version, _links: buildEntityLinks(type, id, domain) }
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

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
  _links?: Record<string, { href: string; method?: string }>
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
  _links?: Record<string, string>
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
  transitions?: TransitionInfo[]
  domain?: string
}

// ---------------------------------------------------------------------------
// HATEOAS link builders
// ---------------------------------------------------------------------------

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

  if (transitions) {
    for (const t of transitions) {
      links[t.event] = { href: `${base}/transition`, method: 'POST' }
    }
  }

  return links
}

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
    _links: buildListLinks(type, domain, page, limit, hasNextPage, hasPrevPage),
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

  cell._links = buildEntityLinks(cell.type, cell.id, opts?.domain, opts?.transitions)

  return cell
}

// ---------------------------------------------------------------------------
// handleCreateEntity
// ---------------------------------------------------------------------------

export interface CreateEntityResult {
  id: string
  type: string
  _links?: Record<string, { href: string; method?: string }>
}

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
  await stub.put({ id, type, data })
  await registry.indexEntity(type, id, domain)
  return { id, type, _links: buildEntityLinks(type, id, domain) }
}

// ---------------------------------------------------------------------------
// handleDeleteEntity
// ---------------------------------------------------------------------------

export async function handleDeleteEntity(
  id: string,
  stub: EntityWriteStub,
  registry: RegistryWriteStub,
  type: string,
): Promise<{ id: string; deleted: boolean } | null> {
  const result = await stub.delete()
  if (!result) return null
  await registry.deindexEntity(type, id)
  return { id: result.id, deleted: true }
}

import { deriveLinks, deriveSchema } from './hateoas'
import { resolveSlugToNoun } from '../collections'

export interface ChildRelation {
  noun: string
  slug: string
  factType: string
}

export interface ConstraintGraph {
  children: Map<string, ChildRelation[]>
}

export interface PathSegment {
  noun: string
  slug: string
  id?: string
}

export interface ResolvedPath {
  level: 'root' | 'collection' | 'entity'
  noun?: string
  id?: string
  parentNoun?: string
  parentId?: string
  segments: PathSegment[]
}

/**
 * Slugify a noun name for URL segments.
 * "Support Request" -> "support-requests"
 */
export function nounToSlug(noun: string): string {
  return noun.toLowerCase().replace(/ /g, '-') + 's'
}

/**
 * Build the constraint graph from IR.
 *
 * For each binary fact type between two entity nouns,
 * if there is a UC on the child noun's role,
 * the child is navigable under the parent.
 */
export function buildConstraintGraph(ir: any): ConstraintGraph {
  // UC "Each X verb at most one Y" constrains X (first noun, span[0])
  const ucIndex = new Map<string, number>()
  ;(ir.constraints || [])
    .filter((c: any) => c.kind === 'UC')
    .forEach((c: any) => {
      const span = (c.spans || [])[0]
      if (span) ucIndex.set(span.factTypeId, span.roleIndex)
    })

  const children = new Map<string, ChildRelation[]>()

  Object.entries(ir.factTypes || {}).forEach(([schemaId, ft]: [string, any]) => {
    const roles = ft.roles || []
    if (roles.length !== 2) return

    const constrainedIdx = ucIndex.get(schemaId)
    if (constrainedIdx === undefined) return

    const childNoun = roles[constrainedIdx].nounName
    const parentNoun = roles[1 - constrainedIdx].nounName

    const childDef = (ir.nouns || {})[childNoun]
    const parentDef = (ir.nouns || {})[parentNoun]
    if (!childDef || childDef.objectType !== 'entity') return
    if (!parentDef || parentDef.objectType !== 'entity') return

    const existing = children.get(parentNoun) || []
    existing.push({ noun: childNoun, slug: nounToSlug(childNoun), factType: schemaId })
    children.set(parentNoun, existing)
  })

  return { children }
}

/**
 * Resolve an /arest/ path against the constraint graph.
 *
 * Path segments alternate between slugs and IDs:
 *   /organizations/acme/domains/support/support-requests/sr-123
 *   [slug]        [id]  [slug]  [id]   [slug]           [id]
 *
 * Odd part count means the last segment is a collection (no ID).
 * Even part count means the last segment is an entity (has ID).
 * The constraint graph validates that each slug is a valid child
 * of the previous segment's noun.
 */
export function resolvePath(path: string, graph: ConstraintGraph): ResolvedPath | null {
  const trimmed = path.replace(/^\/arest\/?/, '').replace(/\/$/, '')
  if (!trimmed) return { level: 'root', segments: [] }

  const parts = trimmed.split('/')
  const segments: PathSegment[] = []

  // Collect all known nouns from the graph
  const allNouns = new Set<string>()
  graph.children.forEach((childList, parent) => {
    allNouns.add(parent)
    childList.forEach(c => allNouns.add(c.noun))
  })

  let i = 0
  while (i < parts.length) {
    const slug = parts[i]
    const parentNoun = segments.length > 0 ? segments[segments.length - 1].noun : null

    const noun = parentNoun
      ? findChildBySlug(slug, parentNoun, graph)
      : findNounBySlug(slug, allNouns)

    if (!noun) return null

    const id = parts[i + 1]
    if (id !== undefined) {
      segments.push({ noun, slug, id })
      i += 2
    } else {
      segments.push({ noun, slug })
      i += 1
    }
  }

  const last = segments[segments.length - 1]
  const secondToLast = segments.length >= 2 ? segments[segments.length - 2] : undefined

  if (last.id) {
    return {
      level: 'entity',
      noun: last.noun,
      id: last.id,
      parentNoun: secondToLast?.noun,
      parentId: secondToLast?.id,
      segments,
    }
  }

  return {
    level: 'collection',
    noun: last.noun,
    parentNoun: secondToLast?.noun,
    parentId: secondToLast?.id,
    segments,
  }
}

function findNounBySlug(slug: string, allNouns: Set<string>): string | null {
  for (const noun of allNouns) {
    if (nounToSlug(noun) === slug) return noun
  }
  return null
}

function findChildBySlug(slug: string, parentNoun: string, graph: ConstraintGraph): string | null {
  const children = graph.children.get(parentNoun) || []
  const match = children.find(c => c.slug === slug)
  return match?.noun || null
}

// ── Root Resource ──────────────────────────────────────────────────

export interface RootResource {
  type: string
  id: string
  data: Record<string, unknown>
  _links: Record<string, any>
}

/**
 * Build the root resource for an authenticated user.
 *
 * Projects org membership facts from the population.
 * Each org link carries the factType (Fact Type ID).
 */
export async function handleRoot(
  userEmail: string,
  population: any,
  getStub: (id: string) => { get(): Promise<any> },
): Promise<RootResource> {
  const orgFactTypes = [
    'User_owns_Organization',
    'User_administers_Organization',
    'User_belongs_to_Organization',
  ]

  const orgLinks = await Promise.all(
    orgFactTypes.flatMap(ftId =>
      (population.facts?.[ftId] || [])
        .filter((f: any) => f.bindings.some(([, v]: [string, string]) => v === userEmail))
        .map(async (f: any) => {
          // The binding that is not the user's email is the org ID
          const orgId = f.bindings.find(([, v]: [string, string]) => v !== userEmail)?.[1] || ''
          const orgEntity = await getStub(orgId).get().catch(() => null)
          const title = orgEntity?.data?.name || orgEntity?.data?.slug || orgId
          return { href: `/arest/organizations/${orgId}`, title, factType: ftId }
        })
    )
  )

  return {
    type: 'User',
    id: userEmail,
    data: { email: userEmail },
    _links: {
      self: { href: '/arest/' },
      organizations: orgLinks,
    },
  }
}

// ── AREST Route Handler ────────────────────────────────────────────

export interface ArestRequestInput {
  path: string
  method: string
  ir: any
  registry: any
  getStub: (id: string) => any
  userEmail?: string
  population?: any
  body?: any
}

/**
 * Handle an /arest/ request by resolving the path against the constraint
 * graph and returning the appropriate resource with derived links.
 *
 * Entity responses get entity data + _links (navigation + transitions).
 * Collection responses get docs array + _links + _schema.
 * Root gets user resource + org membership links.
 * Invalid paths return null.
 */
export async function handleArestRequest(input: ArestRequestInput): Promise<any> {
  const { path, method, ir, registry, getStub } = input

  const graph = buildConstraintGraph(ir)
  const resolved = resolvePath(path, graph)
  if (!resolved) return null

  if (resolved.level === 'root') {
    return handleRoot(input.userEmail || '', input.population || { facts: {} }, getStub)
  }

  if (resolved.level === 'entity' && method === 'GET') {
    const entity = await getStub(resolved.id!).get().catch(() => null)
    if (!entity) return null

    const collectionPath = buildBasePath(resolved.segments.map((s, i) =>
      i === resolved.segments.length - 1 ? { noun: s.noun, slug: s.slug } : s
    ))

    const parentPath = resolved.segments.length >= 2
      ? buildBasePath(resolved.segments.slice(0, -1))
      : undefined

    const links = deriveLinks({
      noun: resolved.noun!,
      id: resolved.id!,
      basePath: collectionPath,
      ir,
      parentPath,
    })

    // Flatten data fields onto the top-level response. Consumers don't
    // need to know about the EntityDB cell envelope — they want
    // `body.name`, not `body.data.name`. Matches the fallback shape.
    return { id: entity.id, type: entity.type, ...(entity.data || {}), _links: links }
  }

  if (resolved.level === 'collection' && method === 'GET') {
    const basePath = buildBasePath(resolved.segments)
    const allIds = await registry.getEntityIds(resolved.noun!).catch(() => [])

    const entities = await Promise.all(
      allIds.map(async (id: string) => {
        const entity = await getStub(id).get().catch(() => null)
        return entity
      })
    )
    const docs = entities
      .filter((e: any): e is { id: string; type: string; data: Record<string, any> } => Boolean(e))
      .map((e) => ({ id: e.id, type: e.type, ...(e.data || {}) }))

    const links = deriveLinks({ noun: resolved.noun!, basePath, ir })
    const schema = deriveSchema(resolved.noun!, ir)

    return {
      type: resolved.noun,
      docs,
      totalDocs: docs.length,
      _links: links,
      _schema: schema,
    }
  }

  return null
}

function buildBasePath(segments: PathSegment[]): string {
  return '/arest/' + segments
    .map(s => s.id ? `${s.slug}/${s.id}` : s.slug)
    .join('/')
}

// ── HATEOAS read fallback (engine-less) ────────────────────────────
//
// `handleArestRequest` requires IR to resolve slugs through the
// constraint graph and to derive _links / _schema. When the WASM
// engine traps on the deploy target — currently the FORML 2 stage-2
// grammar bootstrap on wasm32 — IR can't be built, so the regular
// path returns 500.
//
// This fallback covers the two read shapes the public API needs to
// keep working under that failure: GET /arest/{slug} (collection)
// and GET /arest/{slug}/{id} (entity). Slug → noun resolution goes
// through the Registry's getRegisteredNouns() (no IR), and entity
// data fields are flattened onto the response so consumers don't
// have to know about the cell envelope.
//
// Trade-off: no _links, no _schema, no nested-segment navigation.
// The full HATEOAS surface returns when the engine path works.

export interface ArestReadFallbackInput {
  path: string
  method: string
  registry: any
  getStub: (id: string) => any
  domain?: string
}

export async function handleArestReadFallback(
  input: ArestReadFallbackInput,
): Promise<any> {
  if (input.method !== 'GET') return null

  const trimmed = input.path.replace(/^\/arest\/?/, '').replace(/\/$/, '')
  if (!trimmed) return null

  const parts = trimmed.split('/')
  if (parts.length < 1 || parts.length > 2) return null

  const slug = parts[0]
  const id = parts[1]

  const noun = await resolveSlugToNoun(input.registry, slug)
  if (!noun) return null

  if (id !== undefined) {
    const entity = await input.getStub(id).get().catch(() => null)
    if (!entity) return null
    return { id: entity.id, type: entity.type, ...(entity.data || {}) }
  }

  const ids: string[] = await input.registry
    .getEntityIds(noun, input.domain)
    .catch(() => [])

  const entities = await Promise.all(
    ids.map((eid: string) => input.getStub(eid).get().catch(() => null)),
  )
  const docs = entities
    .filter((e: any): e is { id: string; type: string; data: Record<string, any> } => Boolean(e))
    .map((e) => ({ id: e.id, type: e.type, ...(e.data || {}) }))

  return { type: noun, docs, totalDocs: docs.length }
}

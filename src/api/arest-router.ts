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
  const ucIndex = new Map<string, number>()
  ;(ir.constraints || [])
    .filter((c: any) => c.kind === 'UC')
    .forEach((c: any) => {
      ;(c.spans || []).forEach((span: any) => {
        ucIndex.set(span.factTypeId, span.roleIndex)
      })
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

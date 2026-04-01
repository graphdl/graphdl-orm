export interface TransitionInfo {
  transitionId: string
  event: string
  targetStatus: string
  targetStatusId: string
}

export interface LinkEntry {
  href: string
  method?: string
  factType?: string
}

export interface DeriveLinksInput {
  noun: string
  id?: string
  basePath: string
  ir: any
  transitions?: TransitionInfo[]
  parentPath?: string
}

/**
 * Derive HATEOAS links from the constraint graph.
 *
 * Navigation links: project over binary fact types involving this noun.
 *   - UC on other noun's role = other noun is child -> collection link
 *   - UC on this noun's role = this noun is child -> parent link
 *   - Value type nouns excluded from navigation
 *
 * Transition links: state machine events as POST actions (Theorem 3).
 */
export function deriveLinks(input: DeriveLinksInput): Record<string, LinkEntry> {
  const { noun, id, basePath, ir, transitions, parentPath } = input
  const links: Record<string, LinkEntry> = {}

  const selfHref = id ? `${basePath}/${id}` : basePath
  links.self = { href: selfHref }

  if (!id) {
    links.create = { href: basePath, method: 'POST' }
  }

  // Build UC index: factTypeId -> constrained role index
  const ucIndex = new Map<string, number>()
  ;(ir.constraints || [])
    .filter((c: any) => c.kind === 'UC')
    .forEach((c: any) => {
      ;(c.spans || []).forEach((span: any) => {
        ucIndex.set(span.factTypeId, span.roleIndex)
      })
    })

  // Project links from binary fact types involving this noun
  Object.entries(ir.factTypes || {}).forEach(([schemaId, ft]: [string, any]) => {
    const roles = ft.roles || []
    if (roles.length !== 2) return

    const thisRoleIdx = roles.findIndex((r: any) => r.nounName === noun)
    if (thisRoleIdx < 0) return

    const otherRoleIdx = 1 - thisRoleIdx
    const otherNoun = roles[otherRoleIdx].nounName

    const otherDef = (ir.nouns || {})[otherNoun]
    if (!otherDef || otherDef.objectType === 'value') return

    const constrainedRoleIdx = ucIndex.get(schemaId)

    if (constrainedRoleIdx === thisRoleIdx && id) {
      // UC on this noun's role = this noun is child. Parent link.
      if (parentPath) {
        const key = otherNoun.toLowerCase().replace(/ /g, '-')
        links[key] = { href: parentPath, factType: schemaId }
      }
    } else if (constrainedRoleIdx === otherRoleIdx && id) {
      // UC on other noun's role = other noun is child. Collection link.
      const slug = otherNoun.toLowerCase().replace(/ /g, '-') + 's'
      links[slug] = { href: `${selfHref}/${slug}`, factType: schemaId }
    }
  })

  // Transition links (Theorem 3)
  if (transitions && id) {
    transitions.forEach((t) => {
      links[t.event] = { href: `${selfHref}/transition`, method: 'POST' }
    })
  }

  return links
}

export interface SchemaField {
  name: string
  role: 'attribute' | 'reference'
  required: boolean
  factType: string
}

export interface SchemaConstraint {
  text: string
  kind: string
  modality: string
}

export interface DerivedSchema {
  fields: SchemaField[]
  constraints: SchemaConstraint[]
}

/**
 * Derive _schema for a noun collection from the IR.
 *
 * Fields: project over binary fact types where this noun plays a role.
 *   - Other noun is value type -> role 'attribute'
 *   - Other noun is entity type -> role 'reference'
 *   - MC constraint on this noun's role -> required: true
 *
 * Constraints: filter all constraints whose spans reference
 * a fact type involving this noun.
 */
export function deriveSchema(noun: string, ir: any): DerivedSchema {
  const mcIndex = new Set<string>()
  ;(ir.constraints || [])
    .filter((c: any) => c.kind === 'MC')
    .forEach((c: any) => {
      ;(c.spans || []).forEach((span: any) => {
        mcIndex.add(span.factTypeId)
      })
    })

  const involvedFtIds = new Set<string>()

  const fields: SchemaField[] = Object.entries(ir.factTypes || {})
    .filter(([_, ft]: [string, any]) => {
      const roles = ft.roles || []
      return roles.length === 2 && roles.some((r: any) => r.nounName === noun)
    })
    .map(([schemaId, ft]: [string, any]) => {
      involvedFtIds.add(schemaId)
      const roles = ft.roles || []
      const thisIdx = roles.findIndex((r: any) => r.nounName === noun)
      const otherNoun = roles[1 - thisIdx].nounName
      const otherDef = (ir.nouns || {})[otherNoun]
      const isValue = otherDef?.objectType === 'value'
      return {
        name: otherNoun,
        role: (isValue ? 'attribute' : 'reference') as 'attribute' | 'reference',
        required: mcIndex.has(schemaId),
        factType: schemaId,
      }
    })

  const constraints: SchemaConstraint[] = (ir.constraints || [])
    .filter((c: any) =>
      (c.spans || []).some((span: any) => involvedFtIds.has(span.factTypeId))
    )
    .map((c: any) => ({
      text: c.text,
      kind: c.kind,
      modality: c.modality,
    }))

  return { fields, constraints }
}

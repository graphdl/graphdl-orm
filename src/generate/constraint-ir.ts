// src/generate/constraint-ir.ts
import { parseConstraintText, parseSetComparisonBlock } from '../hooks/parse-constraint'

// ── Types ──────────────────────────────────────────────────────────────

export interface ConstraintIR {
  domain: string
  nouns: Record<string, {
    objectType: 'entity' | 'value'
    enumValues?: string[]
    valueType?: string
    superType?: string
  }>
  factTypes: Record<string, {
    reading: string
    roles: Array<{ nounName: string; roleIndex: number }>
  }>
  constraints: Array<{
    id: string
    kind: string
    modality: string
    deonticOperator?: 'obligatory' | 'forbidden' | 'permitted'
    text: string
    spans: Array<{ factTypeId: string; roleIndex: number; subsetAutofill?: boolean }>
    setComparisonArgumentLength?: number
    clauses?: string[]
    entity?: string
  }>
  stateMachines: Record<string, {
    nounName: string
    statuses: string[]
    transitions: Array<{
      from: string
      to: string
      event: string
      guard?: {
        graphSchemaId: string
        constraintIds: string[]
      }
    }>
  }>
}

// ── Helpers ────────────────────────────────────────────────────────────

async function fetchAll(db: any, slug: string, where?: any): Promise<any[]> {
  const result = await db.findInCollection(slug, where, { limit: 10000 })
  return result?.docs || []
}

// ── Generator ──────────────────────────────────────────────────────────

export async function generateConstraintIR(db: any, domainId: string): Promise<ConstraintIR> {
  const domainFilter = { domain: { equals: domainId } }

  // Fetch all domain data in parallel
  const [
    nouns,
    graphSchemas,
    readings,
    roles,
    constraints,
    constraintSpans,
    smDefs,
    statuses,
    transitions,
    eventTypes,
    guards,
  ] = await Promise.all([
    fetchAll(db, 'nouns', domainFilter),
    fetchAll(db, 'graph-schemas', domainFilter),
    fetchAll(db, 'readings', domainFilter),
    fetchAll(db, 'roles'),
    fetchAll(db, 'constraints', domainFilter),
    fetchAll(db, 'constraint-spans'),
    fetchAll(db, 'state-machine-definitions', domainFilter),
    fetchAll(db, 'statuses', domainFilter),
    fetchAll(db, 'transitions', domainFilter),
    fetchAll(db, 'event-types', domainFilter),
    fetchAll(db, 'guards', domainFilter),
  ])

  // Build noun lookup
  const nounById = new Map(nouns.map((n: any) => [n.id, n]))

  // Build role lookup: roleId → { nounId, graphSchemaId, roleIndex }
  const roleById = new Map(
    roles.map((r: any) => [r.id, {
      nounId: r.noun,
      graphSchemaId: r.graphSchema,
      roleIndex: r.roleIndex,
      readingId: r.reading,
    }])
  )

  // Filter roles to those belonging to domain readings
  const domainReadingIds = new Set(readings.map((r: any) => r.id))
  const domainRoles = roles.filter((r: any) => domainReadingIds.has(r.reading))

  // Filter constraint-spans to those whose role belongs to this domain (prevents cross-domain bleed)
  const domainRoleIds = new Set(domainRoles.map((r: any) => r.id))
  const domainConstraintSpans = constraintSpans.filter((s: any) => domainRoleIds.has(s.role))

  // ── Nouns ──
  const irNouns: ConstraintIR['nouns'] = {}
  for (const noun of nouns) {
    const entry: any = { objectType: noun.objectType || 'entity' }
    if (noun.enumValues) {
      try {
        const parsed = typeof noun.enumValues === 'string' ? JSON.parse(noun.enumValues) : noun.enumValues
        if (Array.isArray(parsed) && parsed.length > 0) entry.enumValues = parsed
      } catch { /* skip malformed enum */ }
    }
    if (noun.valueType) entry.valueType = noun.valueType
    if (noun.superType) {
      const parent = nounById.get(noun.superType)
      if (parent) entry.superType = parent.name
    }
    irNouns[noun.name] = entry
  }

  // ── FactTypes ──
  const irFactTypes: ConstraintIR['factTypes'] = {}
  for (const gs of graphSchemas) {
    const gsRoles = domainRoles
      .filter((r: any) => r.graphSchema === gs.id)
      .sort((a: any, b: any) => (a.roleIndex || 0) - (b.roleIndex || 0))

    const gsReadings = readings.filter((r: any) => r.graphSchema === gs.id)
    const readingText = gsReadings[0]?.text || gs.name || ''

    irFactTypes[gs.id] = {
      reading: readingText,
      roles: gsRoles.map((r: any) => ({
        nounName: nounById.get(r.noun)?.name || 'Unknown',
        roleIndex: r.roleIndex || 0,
      })),
    }
  }

  // ── Constraints ──
  // Build span lookup: constraintId → Array<{ roleId, ... }>
  const spansByConstraint = new Map<string, any[]>()
  for (const span of domainConstraintSpans) {
    const cid = span.constraint
    if (!spansByConstraint.has(cid)) spansByConstraint.set(cid, [])
    spansByConstraint.get(cid)!.push(span)
  }

  const irConstraints: ConstraintIR['constraints'] = []
  for (const c of constraints) {
    const spans = spansByConstraint.get(c.id) || []

    // Resolve spans to factType + roleIndex
    const irSpans = spans
      .map((span: any) => {
        const role = roleById.get(span.role)
        if (!role) return null
        return {
          factTypeId: role.graphSchemaId,
          roleIndex: role.roleIndex,
          ...(span.subsetAutofill ? { subsetAutofill: true } : {}),
        }
      })
      .filter(Boolean) as Array<{ factTypeId: string; roleIndex: number; subsetAutofill?: boolean }>

    // Re-derive deonticOperator from text
    let deonticOperator: 'obligatory' | 'forbidden' | 'permitted' | undefined
    if (c.modality === 'Deontic' && c.text) {
      const parsed = parseConstraintText(c.text)
      if (parsed?.[0]?.deonticOperator) {
        deonticOperator = parsed[0].deonticOperator
      } else {
        // Fallback: extract directly from deontic wrapper when inner text is unrecognized
        const m = c.text.match(/^It is (obligatory|forbidden|permitted) that\b/i)
        if (m) deonticOperator = m[1].toLowerCase() as 'obligatory' | 'forbidden' | 'permitted'
      }
    }

    const entry: ConstraintIR['constraints'][number] = {
      id: c.id,
      kind: c.kind,
      modality: c.modality || 'Alethic',
      text: c.text || '',
      spans: irSpans,
    }
    if (deonticOperator) entry.deonticOperator = deonticOperator
    if (c.setComparisonArgumentLength) entry.setComparisonArgumentLength = c.setComparisonArgumentLength

    // Populate clauses and entity for set-comparison constraints
    if (['XO', 'XC', 'OR', 'SS', 'EQ'].includes(c.kind) && c.text) {
      const setComparison = parseSetComparisonBlock(c.text)
      if (setComparison) {
        if (setComparison.entity) entry.entity = setComparison.entity
        if (setComparison.clauses) entry.clauses = setComparison.clauses
      }
    }

    irConstraints.push(entry)
  }

  // ── State Machines ──
  const irStateMachines: ConstraintIR['stateMachines'] = {}

  for (const smDef of smDefs) {
    const nounName = nounById.get(smDef.noun)?.name || smDef.title || 'Unknown'
    const smStatuses = statuses
      .filter((s: any) => s.stateMachineDefinition === smDef.id)
      .sort((a: any, b: any) => (a.createdAt || '').localeCompare(b.createdAt || ''))

    const statusById = new Map(smStatuses.map((s: any) => [s.id, s.name]))

    const smTransitions = transitions
      .filter((t: any) => smStatuses.some((s: any) => s.id === t.from))

    const irTransitions = smTransitions
      .map((t: any) => {
        const from = statusById.get(t.from)
        const to = statusById.get(t.to)
        const eventType = eventTypes.find((e: any) => e.id === t.eventType)
        if (!from || !to || !eventType) return null

        const transition: any = { from, to, event: eventType.name }

        // Resolve guards for this transition
        const transitionGuards = guards.filter((g: any) => g.transition === t.id)
        if (transitionGuards.length > 0) {
          const guard = transitionGuards[0]
          // Resolve graph_schema → constraint_spans → constraints
          const gsId = guard.graphSchema
          if (gsId) {
            const gsRoleIds = domainRoles
              .filter((r: any) => r.graphSchema === gsId)
              .map((r: any) => r.id)
            const guardSpans = domainConstraintSpans.filter((s: any) =>
              gsRoleIds.includes(s.role)
            )
            const constraintIds = [...new Set(guardSpans.map((s: any) => s.constraint))]
              .filter((cid: string) => constraints.some((c: any) => c.id === cid))

            if (constraintIds.length > 0) {
              transition.guard = { graphSchemaId: gsId, constraintIds }
            }
          }
        }

        return transition
      })
      .filter(Boolean)

    irStateMachines[smDef.id] = {
      nounName,
      statuses: smStatuses.map((s: any) => s.name),
      transitions: irTransitions,
    }
  }

  return {
    domain: domainId,
    nouns: irNouns,
    factTypes: irFactTypes,
    constraints: irConstraints,
    stateMachines: irStateMachines,
  }
}

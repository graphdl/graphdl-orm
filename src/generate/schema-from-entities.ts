/**
 * Build a ConstraintIR (DomainSchema) directly from EntityDB entities.
 *
 * No DomainModel, no SQL column conventions, no field name translation.
 * Reads entities as-is — field names come from the readings.
 *
 * This replaces generateSchema(DomainModel) for the engine bridge.
 */

import type { DomainSchema } from './schema'

interface EntityData {
  id: string
  type: string
  data: Record<string, unknown>
}

type FetchEntities = (type: string, domain: string) => Promise<EntityData[]>

export async function buildSchemaFromEntities(
  domain: string,
  fetchEntities: FetchEntities,
): Promise<DomainSchema> {
  const [nouns, readings, roles, constraints, constraintSpans, graphSchemas,
    smDefs, statuses, transitions, eventTypes, guards, verbs, functions] = await Promise.all([
    fetchEntities('Noun', domain),
    fetchEntities('Reading', domain),
    fetchEntities('Role', domain),
    fetchEntities('Constraint', domain),
    fetchEntities('Constraint Span', domain),
    fetchEntities('Graph Schema', domain),
    fetchEntities('State Machine Definition', domain),
    fetchEntities('Status', domain),
    fetchEntities('Transition', domain),
    fetchEntities('Event Type', domain),
    fetchEntities('Guard', domain),
    fetchEntities('Verb', domain),
    fetchEntities('Function', domain),
  ])

  // ── Nouns ── keyed by NAME (not entity ID) — the noun name IS the identity in the IR
  const schemaNouns: DomainSchema['nouns'] = {}
  for (const n of nouns) {
    const d = n.data
    const nounName = (d.name || n.id) as string
    schemaNouns[nounName] = {
      objectType: (d.objectType as string) === 'value' ? 'value' : 'entity',
      ...(d.enumValues && { enumValues:
        Array.isArray(d.enumValues) ? d.enumValues : JSON.parse(d.enumValues as string)
      }),
      ...(d.valueType && { valueType: d.valueType as string }),
      ...(d.superType && { superType: d.superType as string }),
      ...(d.worldAssumption && { worldAssumption: d.worldAssumption as 'closed' | 'open' }),
      ...(d.referenceScheme && { refScheme:
        Array.isArray(d.referenceScheme) ? d.referenceScheme : JSON.parse(d.referenceScheme as string)
      }),
    }
  }

  // ── Fact Types (Graph Schemas + Roles → Reading) ──
  const schemaFactTypes: DomainSchema['factTypes'] = {}
  for (const gs of graphSchemas) {
    const gsRoles = roles.filter(r =>
      r.data.graphSchema === gs.id || r.data.graphSchemaId === gs.id
    )
    const reading = readings.find(r =>
      r.data.graphSchema === gs.id || r.data.graphSchemaId === gs.id
    )
    if (gsRoles.length === 0) continue

    schemaFactTypes[gs.id] = {
      reading: (reading?.data?.text || gs.data.name || gs.data.title || gs.id) as string,
      roles: gsRoles
        .sort((a, b) => ((a.data.roleIndex as number) || 0) - ((b.data.roleIndex as number) || 0))
        .map((r, i) => {
          const nounId = (r.data.noun || r.data.nounId) as string
          const nounEntity = nouns.find(n => n.id === nounId)
          return {
            nounName: (nounEntity?.data?.name || nounId || '') as string,
            roleIndex: (r.data.roleIndex as number) ?? i,
          }
        }),
    }
  }

  // ── Constraints ──
  const schemaConstraints: DomainSchema['constraints'] = []
  for (const c of constraints) {
    const d = c.data
    const spans = constraintSpans
      .filter(cs => cs.data.constraint === c.id || cs.data.constraintId === c.id)
      .map(cs => {
        const roleId = (cs.data.role || cs.data.roleId) as string
        const roleEntity = roles.find(r => r.id === roleId)
        const gsId = roleEntity ? (roleEntity.data.graphSchema || roleEntity.data.graphSchemaId) as string : ''
        return {
          factTypeId: gsId,
          roleIndex: (roleEntity?.data?.roleIndex as number) ?? 0,
          ...(cs.data.subsetAutofill === true && { subsetAutofill: true }),
        }
      })

    schemaConstraints.push({
      id: c.id,
      kind: (d.kind || d.constraintType || '') as string,
      modality: (d.modality || 'Alethic') as string,
      text: (d.text || '') as string,
      spans,
      ...(d.deonticOperator && { deonticOperator: d.deonticOperator as any }),
      ...(d.entity && { entity: d.entity as string }),
    })
  }

  // ── State Machines ──
  const schemaStateMachines: DomainSchema['stateMachines'] = {}
  for (const sm of smDefs) {
    const d = sm.data
    // The noun name — from the readings field, not a translated column
    const nounName = (d.forNoun || d.noun || d.name || '') as string

    // Statuses belonging to this SM definition
    const smStatuses = statuses.filter(s =>
      s.data.definedInStateMachineDefinition === sm.id ||
      s.data.inStateMachineDefinition === sm.id ||
      s.data.stateMachineDefinition === sm.id ||
      // Also match by SM name for cross-referencing
      s.data.definedInStateMachineDefinition === (d.name || sm.id) ||
      s.data.inStateMachineDefinition === (d.name || sm.id) ||
      s.data.stateMachineDefinition === (d.name || sm.id)
    )

    const statusNames = smStatuses.map(s => (s.data.name || s.id) as string)
    const statusIds = new Set(smStatuses.map(s => s.id))

    // Find initial status
    const initialStatus = smStatuses.find(s => s.data.isInitial === true || s.data.isInitial === 'true')
    if (initialStatus) {
      // Move initial to front
      const initialName = (initialStatus.data.name || initialStatus.id) as string
      const idx = statusNames.indexOf(initialName)
      if (idx > 0) {
        statusNames.splice(idx, 1)
        statusNames.unshift(initialName)
      }
    }

    // Transitions — match by from status being in this SM's statuses
    const smTransitions = transitions.filter(t => {
      const from = (t.data.fromStatus || t.data.from) as string
      return statusIds.has(from) || statusNames.includes(from)
    })

    schemaStateMachines[sm.id] = {
      nounName,
      statuses: statusNames,
      transitions: smTransitions.map(t => {
        const from = (t.data.fromStatus || t.data.from || '') as string
        const to = (t.data.toStatus || t.data.to || '') as string
        const eventTypeId = (t.data.triggeredByEventType || t.data.eventType || '') as string
        const et = eventTypes.find(e => e.id === eventTypeId)
        const eventName = (et?.data?.name || eventTypeId || '') as string

        const entry: DomainSchema['stateMachines'][string]['transitions'][number] = {
          from,
          to,
          event: eventName,
        }

        // Guards
        const tGuards = guards.filter(g =>
          g.data.preventsTransition === t.id ||
          g.data.transition === t.id ||
          g.data.transitionId === t.id
        )
        if (tGuards.length > 0) {
          entry.guard = {
            graphSchemaId: (tGuards[0].data.graphSchemaId || '') as string,
            constraintIds: tGuards.map(g => g.id),
          }
        }

        return entry
      }),
    }
  }

  // ── Derivation Rules ──
  const derivationRules: DomainSchema['derivationRules'] = []

  // Subtype inheritance
  for (const [name, noun] of Object.entries(schemaNouns)) {
    if (noun.superType) {
      derivationRules.push({
        id: `derive-subtype-${name}`,
        text: `${name} inherits constraints of ${noun.superType}`,
        antecedentFactTypeIds: [],
        consequentFactTypeId: '',
        kind: 'subtypeInheritance',
      })
    }
  }

  return {
    domain,
    nouns: schemaNouns,
    factTypes: schemaFactTypes,
    constraints: schemaConstraints,
    stateMachines: schemaStateMachines,
    derivationRules,
  }
}

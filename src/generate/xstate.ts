/**
 * generateXState — generates XState machine configs, agent tool schemas,
 * and agent system prompts from state machine definitions.
 *
 * Ported from Generator.ts.bak lines 1354-1558. Adapted for DO findInCollection API.
 */

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/** Fetch all docs from a collection, handling pagination by using a large limit. */
async function fetchAll(db: any, slug: string, where?: any): Promise<any[]> {
  const result = await db.findInCollection(slug, where, { limit: 10000 })
  return result?.docs || []
}

/** Convert PascalCase to kebab-case: SupportRequest → support-request */
function toKebab(name: string): string {
  return name
    .replace(/([A-Z])/g, '-$1')
    .toLowerCase()
    .replace(/^-/, '')
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

export async function generateXState(db: any, domainId: string): Promise<{ files: Record<string, string> }> {
  const stateMachineDefinitions = await fetchAll(db, 'state-machine-definitions', { domain: { equals: domainId } })
  const nouns = await fetchAll(db, 'nouns', { domain: { equals: domainId } })
  const files: Record<string, string> = {}

  for (const smDef of stateMachineDefinitions) {
    // Fetch statuses for this state machine definition, sorted by createdAt
    const statuses = await fetchAll(db, 'statuses', { stateMachineDefinition: { equals: smDef.id } })
    // Sort by createdAt ascending (string comparison works for ISO dates)
    statuses.sort((a: any, b: any) => (a.createdAt || '').localeCompare(b.createdAt || ''))

    if (!statuses.length) continue

    // Collect all transitions across all statuses
    const allTransitions: { from: string; to: string; event: string; callback?: { url: string; method: string } }[] = []

    for (const status of statuses) {
      const transitions = await fetchAll(db, 'transitions', { from: { equals: status.id } })

      for (const t of transitions) {
        // Resolve toStatus name
        const toStatusId = t.to
        const toStatus = statuses.find((s: any) => s.id === toStatusId)

        // Resolve eventType name
        const eventTypeId = t.eventType
        let eventType: any = null
        if (eventTypeId) {
          const eventTypes = await fetchAll(db, 'event-types', { id: { equals: eventTypeId } })
          eventType = eventTypes[0] || null
        }

        // Resolve verb → function chain for callback metadata
        let callback: { url: string; method: string } | undefined
        const verbId = t.verb
        if (verbId) {
          const verbs = await fetchAll(db, 'verbs', { id: { equals: verbId } })
          const verb = verbs[0]
          if (verb?.function) {
            const funcId = typeof verb.function === 'string' ? verb.function : verb.function?.id
            if (funcId) {
              const funcs = await fetchAll(db, 'functions', { id: { equals: funcId } })
              const func = funcs[0]
              if (func?.callbackUrl) {
                callback = { url: func.callbackUrl, method: func.httpMethod || 'POST' }
              }
            }
          }
        }

        if (toStatus?.name && eventType?.name) {
          allTransitions.push({
            from: status.name,
            to: toStatus.name,
            event: eventType.name,
            callback,
          })
        }
      }
    }

    // Build states from statuses + transitions
    const states: Record<string, any> = {}
    for (const status of statuses) {
      const outgoing = allTransitions.filter((t) => t.from === status.name)
      const on: Record<string, any> = {}
      for (const t of outgoing) {
        const transition: Record<string, any> = { target: t.to }
        if (t.callback) {
          transition.meta = { callback: t.callback }
        }
        on[t.event] = transition
      }
      states[status.name] = Object.keys(on).length ? { on } : {}
    }

    // Determine initial state: the status with no incoming transitions
    const statesWithIncoming = new Set(allTransitions.map((t) => t.to))
    const initialStatus = statuses.find((s: any) => !statesWithIncoming.has(s.name)) || statuses[0]

    // Resolve noun name from the state machine definition
    const nounId = smDef.noun
    const nounValue = nouns.find((n: any) => n.id === nounId)
    const machineName = toKebab(nounValue?.name || 'unknown')

    const xstateConfig = {
      id: machineName,
      initial: initialStatus.name,
      states,
    }

    files[`state-machines/${machineName}.json`] = JSON.stringify(xstateConfig, null, 2)

    // Generate agent tool schemas from unique events
    const uniqueEvents = new Map<string, { from: string[]; to: string[] }>()
    for (const t of allTransitions) {
      if (!t.event) continue
      if (!uniqueEvents.has(t.event)) {
        uniqueEvents.set(t.event, { from: [], to: [] })
      }
      const entry = uniqueEvents.get(t.event)!
      if (!entry.from.includes(t.from)) entry.from.push(t.from)
      if (!entry.to.includes(t.to)) entry.to.push(t.to)
    }

    const tools = Array.from(uniqueEvents.entries()).map(([event, { from, to }]) => ({
      name: event,
      description: `Transition from ${from.join(' or ')} to ${to.join(' or ')}`,
      parameters: {
        type: 'object' as const,
        properties: {},
      },
    }))

    files[`agents/${machineName}-tools.json`] = JSON.stringify(tools, null, 2)

    // Generate system prompt from RELEVANT readings + state machine
    // 1. Find graph schemas where the machine's entity noun plays a role
    const allRoles = await fetchAll(db, 'roles')

    const directSchemaIds = new Set<string>()
    const relatedNounIds = new Set<string>()
    if (nounId) relatedNounIds.add(nounId)

    for (const role of allRoles) {
      const roleNounId = role.noun
      const gsId = role.graphSchema
      if (roleNounId === nounId && gsId) {
        directSchemaIds.add(gsId)
      }
    }

    // 2. Collect all noun IDs that participate in those schemas
    for (const role of allRoles) {
      const gsId = role.graphSchema
      if (directSchemaIds.has(gsId)) {
        const roleNounId = role.noun
        if (roleNounId) relatedNounIds.add(roleNounId)
      }
    }

    // 3. Expand one level — schemas where any related noun participates
    const expandedSchemaIds = new Set(directSchemaIds)
    for (const role of allRoles) {
      const roleNounId = role.noun
      const gsId = role.graphSchema
      if (relatedNounIds.has(roleNounId) && gsId) {
        expandedSchemaIds.add(gsId)
      }
    }

    // 4. Filter readings to those in the expanded schema set
    const allReadings = await fetchAll(db, 'readings')
    const readings = allReadings.filter((r: any) => {
      const gsId = r.graphSchema
      return expandedSchemaIds.has(gsId)
    })

    const readingTexts = [...new Set(readings.map((r: any) => r.text).filter(Boolean))] as string[]
    const stateNames = statuses.map((s: any) => s.name)
    const eventNames = Array.from(uniqueEvents.keys())

    const prompt = [
      `# ${nounValue?.name || 'Agent'} Agent`,
      '',
      '## Domain Model',
      ...readingTexts.map((r: string) => `- ${r}`),
      '',
      '## State Machine',
      `States: ${stateNames.join(', ')}`,
      '',
      '## Available Actions',
      ...eventNames.map((e) => {
        const { from, to } = uniqueEvents.get(e)!
        return `- **${e}**: ${from.join('/')} → ${to.join('/')}`
      }),
      '',
      '## Current State: {{currentState}}',
      '',
      '## Instructions',
      'You operate within the domain model above. Use the available actions to transition the state machine. Do not take actions outside the defined transitions for the current state.',
      '',
    ].join('\n')

    files[`agents/${machineName}-prompt.md`] = prompt
  }

  return { files }
}

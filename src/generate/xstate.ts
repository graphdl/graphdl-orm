/**
 * generateXState — generates XState machine configs, agent tool schemas,
 * and agent system prompts from state machine definitions.
 *
 * Consumes a DomainModel object instead of raw DB queries.
 */

import type { NounDef, FactTypeDef, StateMachineDef, ReadingDef } from '../model/types'

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

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

export async function generateXState(model: {
  nouns(): Promise<Map<string, NounDef>>
  factTypes(): Promise<Map<string, FactTypeDef>>
  stateMachines(): Promise<Map<string, StateMachineDef>>
  readings(): Promise<ReadingDef[]>
}): Promise<{ files: Record<string, string> }> {
  const stateMachines = await model.stateMachines()
  const files: Record<string, string> = {}

  for (const [, sm] of stateMachines) {
    const { statuses, transitions } = sm

    if (!statuses.length) continue

    // Build transition list with callback metadata
    const allTransitions: { from: string; to: string; event: string; callback?: { url: string; method: string } }[] = []

    for (const t of transitions) {
      let callback: { url: string; method: string } | undefined
      if (t.verb?.func?.callbackUrl) {
        callback = { url: t.verb.func.callbackUrl, method: t.verb.func.httpMethod || 'POST' }
      }

      allTransitions.push({
        from: t.from,
        to: t.to,
        event: t.event,
        callback,
      })
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
    const initialStatus = statuses.find((s) => !statesWithIncoming.has(s.name)) || statuses[0]

    const machineName = toKebab(sm.nounName)

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
    // 1. Find fact types where the SM's noun participates in a role
    const allFactTypes = await model.factTypes()

    const directFactTypeIds = new Set<string>()
    const relatedNounNames = new Set<string>()
    relatedNounNames.add(sm.nounName)

    for (const [ftId, ft] of allFactTypes) {
      if (ft.roles.some((r) => r.nounName === sm.nounName)) {
        directFactTypeIds.add(ftId)
      }
    }

    // 2. Collect all noun names that participate in those fact types
    for (const [ftId, ft] of allFactTypes) {
      if (directFactTypeIds.has(ftId)) {
        for (const r of ft.roles) {
          relatedNounNames.add(r.nounName)
        }
      }
    }

    // 3. Expand one level — fact types where any related noun participates
    const expandedFactTypeIds = new Set(directFactTypeIds)
    for (const [ftId, ft] of allFactTypes) {
      if (ft.roles.some((r) => relatedNounNames.has(r.nounName))) {
        expandedFactTypeIds.add(ftId)
      }
    }

    // 4. Filter readings to those in the expanded fact type set
    const allReadings = await model.readings()
    const readings = allReadings.filter((r) => expandedFactTypeIds.has(r.graphSchemaId))

    const readingTexts = [...new Set(readings.map((r) => r.text).filter(Boolean))] as string[]
    const stateNames = statuses.map((s) => s.name)
    const eventNames = Array.from(uniqueEvents.keys())

    const prompt = [
      `# ${sm.nounName} Agent`,
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

/**
 * generateXState — generates XState machine configs in state.do format,
 * agent tool schemas, and agent system prompts from state machine definitions.
 *
 * Output format matches state.do: { id, initial, states: { name: { on, callback } } }
 * - Callbacks from Verb (Function subtype) with callback URI
 * - Event Type Patterns for status code matching in cascades
 * - Guards reference Graph Schemas
 * - Status isInitial flag (falls back to "no incoming transitions" heuristic)
 */

import type { NounDef, FactTypeDef, StateMachineDef, ReadingDef } from '../model/types'

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/** Convert PascalCase/spaced name to kebab-case: Support Request → support-request */
function toKebab(name: string): string {
  return name
    .replace(/\s+/g, '-')
    .replace(/([A-Z])/g, '-$1')
    .toLowerCase()
    .replace(/^-/, '')
    .replace(/--+/g, '-')
}

// ---------------------------------------------------------------------------
// state.do format types
// ---------------------------------------------------------------------------

export interface StateDotDoConfig {
  id: string
  initial: string
  states: Record<string, StateDotDoState>
}

export interface StateDotDoState {
  on?: Record<string, string | StateDotDoTransition>
  callback?: string | StateDotDoCallback
  type?: 'final'
}

export interface StateDotDoTransition {
  target: string
  guard?: string
  meta?: Record<string, any>
}

export interface StateDotDoCallback {
  url: string
  method?: string
  headers?: Record<string, string>
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

    const machineName = toKebab(sm.nounName)

    // ── Build state.do config ──────────────────────────────────────────

    const states: Record<string, StateDotDoState> = {}

    for (const status of statuses) {
      const outgoing = transitions.filter((t) => t.from === status.name)
      const state: StateDotDoState = {}

      // Events (transitions from this state)
      if (outgoing.length > 0) {
        const on: Record<string, string | StateDotDoTransition> = {}
        for (const t of outgoing) {
          // If transition has a guard or event pattern, use object form
          if (t.guard || t.eventPattern) {
            const transition: StateDotDoTransition = { target: t.to }
            if (t.guard) {
              transition.guard = t.guard.graphSchemaId
            }
            if (t.eventPattern) {
              transition.meta = { ...transition.meta, pattern: t.eventPattern }
            }
            on[t.event] = transition
          } else {
            // Simple form: event → target state name
            on[t.event] = t.to
          }
        }
        state.on = on
      }

      // Callback from Status Verb (Moore semantics — action on entry)
      // Verb is a subtype of Function, so it has callback URI directly
      if (status.verb?.func?.callbackUrl) {
        const cb = status.verb.func
        if (cb.httpMethod && cb.httpMethod !== 'POST' || cb.headers) {
          state.callback = {
            url: cb.callbackUrl!,
            ...(cb.httpMethod && { method: cb.httpMethod }),
            ...(cb.headers && { headers: cb.headers }),
          }
        } else {
          state.callback = cb.callbackUrl!
        }
      }

      // Also check Mealy callbacks (Verb performed during transition TO this state)
      // These become callbacks on the target state in state.do format
      const incomingWithCallback = transitions.filter(
        (t) => t.to === status.name && t.verb?.func?.callbackUrl
      )
      if (!state.callback && incomingWithCallback.length > 0) {
        const cb = incomingWithCallback[0].verb!.func!
        if (cb.httpMethod && cb.httpMethod !== 'POST' || cb.headers) {
          state.callback = {
            url: cb.callbackUrl!,
            ...(cb.httpMethod && { method: cb.httpMethod }),
            ...(cb.headers && { headers: cb.headers }),
          }
        } else {
          state.callback = cb.callbackUrl!
        }
      }

      // Final state: no outgoing transitions
      if (outgoing.length === 0) {
        state.type = 'final'
      }

      states[status.name] = state
    }

    // ── Determine initial state ──────────────────────────────────────

    // 1. Check isInitial flag first
    let initialStatus = statuses.find((s) => s.isInitial)

    // 2. Fall back to "no incoming transitions" heuristic
    if (!initialStatus) {
      const statesWithIncoming = new Set(transitions.map((t) => t.to))
      initialStatus = statuses.find((s) => !statesWithIncoming.has(s.name)) || statuses[0]
    }

    const xstateConfig: StateDotDoConfig = {
      id: machineName,
      initial: initialStatus.name,
      states,
    }

    files[`state-machines/${machineName}.json`] = JSON.stringify(xstateConfig, null, 2)

    // ── Generate agent tools from transitions ────────────────────────

    const uniqueEvents = new Map<string, { from: string[]; to: string[]; guards: string[] }>()
    for (const t of transitions) {
      if (!t.event) continue
      if (!uniqueEvents.has(t.event)) {
        uniqueEvents.set(t.event, { from: [], to: [], guards: [] })
      }
      const entry = uniqueEvents.get(t.event)!
      if (!entry.from.includes(t.from)) entry.from.push(t.from)
      if (!entry.to.includes(t.to)) entry.to.push(t.to)
      if (t.guard && !entry.guards.includes(t.guard.graphSchemaId)) {
        entry.guards.push(t.guard.graphSchemaId)
      }
    }

    const tools = Array.from(uniqueEvents.entries()).map(([event, { from, to, guards }]) => ({
      name: event,
      description: `Transition from ${from.join(' or ')} to ${to.join(' or ')}`,
      ...(guards.length > 0 && { guards }),
      parameters: {
        type: 'object' as const,
        properties: {},
      },
    }))

    files[`agents/${machineName}-tools.json`] = JSON.stringify(tools, null, 2)

    // ── Generate agent system prompt ─────────────────────────────────

    const allFactTypes = await model.factTypes()
    const directFactTypeIds = new Set<string>()
    const relatedNounNames = new Set<string>()
    relatedNounNames.add(sm.nounName)

    for (const [ftId, ft] of allFactTypes) {
      if (ft.roles.some((r) => r.nounName === sm.nounName)) {
        directFactTypeIds.add(ftId)
      }
    }

    for (const [ftId, ft] of allFactTypes) {
      if (directFactTypeIds.has(ftId)) {
        for (const r of ft.roles) relatedNounNames.add(r.nounName)
      }
    }

    const expandedFactTypeIds = new Set(directFactTypeIds)
    for (const [ftId, ft] of allFactTypes) {
      if (ft.roles.some((r) => relatedNounNames.has(r.nounName))) {
        expandedFactTypeIds.add(ftId)
      }
    }

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
      `Initial: ${initialStatus.name}`,
      '',
      '## Available Actions',
      ...eventNames.map((e) => {
        const { from, to, guards } = uniqueEvents.get(e)!
        const guardNote = guards.length > 0 ? ` (guarded by: ${guards.join(', ')})` : ''
        return `- **${e}**: ${from.join('/')} → ${to.join('/')}${guardNote}`
      }),
      '',
      '## Current State: {{currentState}}',
      '',
      '## Instructions',
      'You operate within the domain model above. Your available actions are the transitions valid from the current state. Do not take actions outside the defined transitions.',
      '',
    ].join('\n')

    files[`agents/${machineName}-prompt.md`] = prompt
  }

  return { files }
}

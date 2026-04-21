/**
 * Convert an AREST State Machine Definition + its Transitions into
 * an xstate 5 machine config. Pure — no fetch, no React.
 *
 * AREST representation (readings/state.md):
 *   - State Machine Definition(.id) — entity keyed by id, has
 *     initial status
 *   - Transition(.id)               — is defined in SMD, is from
 *     Status, is to Status, is triggered by Event Type
 *   - Status(.Name)                 — defined / terminal are derived
 *
 * Derived facts (readings/state.md §Derivation Rules):
 *   - Status is defined in SMD iff some Transition in SMD is from or
 *     to that Status.
 *   - Status is terminal in SMD iff that Status is defined in SMD
 *     and no Transition in SMD is from that Status.
 *
 * xstate output — mirrors the Backus fold (whitepaper §5.1 eq. 4):
 *   machine(s0, E) = foldl transition s0 E
 *
 * States absent from transitions but named as initial are still
 * included so they render as source nodes.
 */

export interface ArestTransition {
  /** Transition id / name (used as the event when no explicit event). */
  id: string
  /** Source Status name. */
  from: string
  /** Target Status name. */
  to: string
  /** Explicit event name; falls back to transition id. */
  event?: string
  /** Whether a Guard prevents this transition. Surfaced for UI only. */
  guarded?: boolean
}

export interface ArestStateMachineDefinition {
  id: string
  noun: string
  /** Initial status name. */
  initial: string
}

export interface XStateConfig {
  id: string
  initial: string
  states: Record<string, XStateNode>
}

export interface XStateNode {
  type?: 'final'
  on?: Record<string, string>
}

/**
 * Convert SMD + transitions to an xstate config. Terminal states
 * (no outgoing transitions) are marked `type: 'final'` so xstate's
 * standard semantics line up with AREST's Status-is-terminal rule
 * (readings/state.md §Derivation Rules).
 */
export function arestToXStateConfig(
  smd: ArestStateMachineDefinition,
  transitions: readonly ArestTransition[],
): XStateConfig {
  const states: Record<string, XStateNode> = {}

  // Seed every status mentioned as from / to so every node appears.
  for (const t of transitions) {
    if (!states[t.from]) states[t.from] = {}
    if (!states[t.to]) states[t.to] = {}
  }
  // Seed initial even if it has no outgoing transitions yet.
  if (!states[smd.initial]) states[smd.initial] = {}

  // Populate `on` for each outgoing transition.
  for (const t of transitions) {
    const node = states[t.from]
    const event = t.event ?? t.id
    if (!node.on) node.on = {}
    node.on[event] = t.to
  }

  // Mark terminal states: present in the graph, no outgoing transitions.
  for (const node of Object.values(states)) {
    if (!node.on || Object.keys(node.on).length === 0) {
      node.type = 'final'
    }
  }

  return {
    id: smd.id,
    initial: smd.initial,
    states,
  }
}

/**
 * Enumerate all status names referenced by an SMD (as initial) or
 * any of its transitions (as from or to). Useful for rendering a
 * legend or seeding an add-transition dropdown.
 */
export function listStatuses(
  smd: ArestStateMachineDefinition,
  transitions: readonly ArestTransition[],
): string[] {
  const set = new Set<string>([smd.initial])
  for (const t of transitions) {
    set.add(t.from)
    set.add(t.to)
  }
  return Array.from(set).sort()
}

/**
 * Classify each status as initial / intermediate / terminal for
 * display. Terminal = no outgoing transitions (per AREST's
 * derivation rule).
 */
export interface StatusInfo {
  name: string
  isInitial: boolean
  isTerminal: boolean
  outgoing: ArestTransition[]
}

export function describeStatuses(
  smd: ArestStateMachineDefinition,
  transitions: readonly ArestTransition[],
): StatusInfo[] {
  const names = listStatuses(smd, transitions)
  return names.map((name) => {
    const outgoing = transitions.filter((t) => t.from === name)
    return {
      name,
      isInitial: name === smd.initial,
      isTerminal: outgoing.length === 0,
      outgoing,
    }
  })
}

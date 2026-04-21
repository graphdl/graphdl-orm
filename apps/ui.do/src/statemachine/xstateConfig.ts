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

/**
 * Find cycles that have no exit transition.
 *
 * Whitepaper §7 Theorem 3 proof: "Transition graphs may contain
 * cycles. It is obligatory that each cycle has some exit
 * transition, a deontic constraint that ensures liveness without
 * requiring acyclicity." This helper surfaces violating cycles so
 * the editor can warn on save before a machine deadlocks.
 *
 * Implementation: Tarjan-style SCC detection, then filter SCCs
 * that have >1 node (or 1 node with a self-loop) and whose every
 * outgoing transition from members stays inside the SCC — no exit.
 * Returns the status-name lists for each offending SCC.
 */
export function findDeadCycles(
  smd: ArestStateMachineDefinition,
  transitions: readonly ArestTransition[],
): string[][] {
  const adj = new Map<string, string[]>()
  const statuses = listStatuses(smd, transitions)
  for (const s of statuses) adj.set(s, [])
  for (const t of transitions) {
    const out = adj.get(t.from)
    if (out && !out.includes(t.to)) out.push(t.to)
  }

  // Tarjan's strongly-connected-components algorithm.
  const index = new Map<string, number>()
  const lowlink = new Map<string, number>()
  const onStack = new Set<string>()
  const stack: string[] = []
  let counter = 0
  const sccs: string[][] = []

  function strongconnect(v: string): void {
    index.set(v, counter)
    lowlink.set(v, counter)
    counter++
    stack.push(v)
    onStack.add(v)

    for (const w of adj.get(v) ?? []) {
      if (!index.has(w)) {
        strongconnect(w)
        lowlink.set(v, Math.min(lowlink.get(v)!, lowlink.get(w)!))
      } else if (onStack.has(w)) {
        lowlink.set(v, Math.min(lowlink.get(v)!, index.get(w)!))
      }
    }

    if (lowlink.get(v) === index.get(v)) {
      const scc: string[] = []
      let w: string
      do {
        w = stack.pop()!
        onStack.delete(w)
        scc.push(w)
      } while (w !== v)
      sccs.push(scc)
    }
  }

  for (const s of statuses) if (!index.has(s)) strongconnect(s)

  const dead: string[][] = []
  for (const scc of sccs) {
    // Skip trivial SCCs (single node, no self-loop) — they can't cycle.
    if (scc.length === 1) {
      const only = scc[0]
      const hasSelfLoop = transitions.some((t) => t.from === only && t.to === only)
      if (!hasSelfLoop) continue
    }
    const set = new Set(scc)
    // A cycle has an exit iff some transition from an SCC member
    // lands outside the SCC.
    const hasExit = transitions.some((t) => set.has(t.from) && !set.has(t.to))
    if (!hasExit) dead.push(scc.sort())
  }
  return dead
}

/**
 * Build a Stately Studio URL (stately.ai) with the machine config
 * serialised into the URL hash. Lets users hand a machine off to
 * Stately's full visual editor for a second-opinion edit session.
 *
 * Format matches stately.ai/viz: base64-encoded JSON payload in the
 * hash. The URL opens in the viz tab of Stately Studio; the
 * "Open in editor" control on that page lets the user fork it.
 */
export function buildStatelyDeeplink(config: XStateConfig): string {
  const payload = JSON.stringify({ version: 5, machine: config })
  // Stately's viz URL takes URL-safe base64 in the hash.
  const base64 = typeof btoa !== 'undefined'
    ? btoa(payload)
    : Buffer.from(payload).toString('base64')
  const urlSafe = base64.replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '')
  return `https://stately.ai/viz?machine=${urlSafe}`
}

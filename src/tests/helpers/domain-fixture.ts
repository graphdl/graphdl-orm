/**
 * domain-fixture.ts — Reusable WASM-compiled domain helper for paper verification tests.
 *
 * Parses FORML2 readings via the WASM engine and returns the compiled IR + entity list.
 * Every paper verification test file imports from here.
 *
 * Note on the WASM debug format: system(handle, 'debug', '') returns a display string
 * of the form <<nouns, <N1, N2>>, <factTypes, <<id, reading>>>, <constraints, <...>>, <totalFacts, N>>.
 * parseDebugIR() converts this into the { nouns, factTypes, constraints } shape used throughout.
 */

import {
  compileDomainReadings,
  system,
  release_domain,
} from '../../api/engine'

// ── Metamodel readings — the system's own vocabulary ───────────────────────
// Minimal subset of state.md needed to parse state machine instance facts.

export const STATE_READINGS = `# State

## Entity Types

Status(.Name) is an entity type.
State Machine Definition(.Name) is an entity type.
Transition(.id) is an entity type.
Noun(.Name) is an entity type.

## Fact Types

### State Machine Definition
State Machine Definition is for Noun.

### Status
Status is initial in State Machine Definition.

### Transition
Transition is defined in State Machine Definition.
Transition is from Status.
Transition is to Status.
`.trim()

// ── Domain reading strings ──────────────────────────────────────────────────

export const ORDER_READINGS = `# Orders

A minimal Order domain for paper verification tests.

## Entity Types
Order(.OrderId) is an entity type.
Customer(.Name) is an entity type.
Priority(.Label) is an entity type.

## Value Types
OrderId is a value type.
Label is a value type.
Amount is a value type.

## Fact Types

### Order
Order was placed by Customer.
Order has Priority.
Order has Amount.

## Constraints
Each Order was placed by exactly one Customer.
Each Order has at most one Priority.
Each Order has at most one Amount.

## Instance Facts
State Machine Definition 'Order' is for Noun 'Order'.
Status 'In Cart' is initial in State Machine Definition 'Order'.
Transition 'place' is defined in State Machine Definition 'Order'.
Transition 'place' is from Status 'In Cart'.
Transition 'place' is to Status 'Placed'.
Transition 'ship' is defined in State Machine Definition 'Order'.
Transition 'ship' is from Status 'Placed'.
Transition 'ship' is to Status 'Shipped'.
Transition 'deliver' is defined in State Machine Definition 'Order'.
Transition 'deliver' is from Status 'Shipped'.
Transition 'deliver' is to Status 'Delivered'.
`

export const SUPPORT_READINGS = `# Support

A support domain with deontic constraints for paper verification tests.

## Entity Types
Ticket(.TicketId) is an entity type.
Agent(.Name) is an entity type.

## Value Types
TicketId is a value type.
ResponseText is a value type.

## Fact Types

### Ticket
Ticket is assigned to Agent.
Ticket has ResponseText.

## Constraints
Each Ticket is assigned to at most one Agent.

## Mandatory Constraints
Each Ticket is assigned to exactly one Agent.

## Deontic Constraints
It is obligatory that each Ticket has some ResponseText.
`

// ── Types ───────────────────────────────────────────────────────────────────

export interface CompiledDomain {
  ir: {
    nouns: string[]
    factTypes: Array<{ id: string; reading: string }>
    constraints: Array<{ kind: string; text: string }>
    totalFacts: number
    raw: string
  }
  entities: string[]
  handle: number
}

// ── Debug format parser ─────────────────────────────────────────────────────

/**
 * Parse the WASM debug display format into a structured IR object.
 *
 * The format is: <<nouns, <N1, N2>>, <factTypes, <<id, reading>>>, <constraints, <...>>, <totalFacts, N>>
 *
 * This is NOT JSON — it is GraphDL's internal display notation.
 * We extract the sections by finding the named top-level fields.
 */
function parseDebugIR(raw: string): CompiledDomain['ir'] {
  // Extract nouns section: <<nouns, <N1, N2, ...>>
  const nounsMatch = raw.match(/<nouns,\s*<([^>]*)>/)
  const nouns: string[] = nounsMatch
    ? nounsMatch[1].split(',').map(s => s.trim()).filter(Boolean)
    : []

  // Extract factTypes section: <factTypes, <<id, reading>, <id, reading>>>
  const factTypes: Array<{ id: string; reading: string }> = []
  const ftSection = raw.match(/<factTypes,\s*(<<.*?>>|φ)/)
  if (ftSection && ftSection[1] !== 'φ') {
    const ftContent = ftSection[1]
    // Each entry is <id, reading text>
    const ftEntries = ftContent.matchAll(/<([^,<>]+),\s*([^<>]+)>/g)
    for (const m of ftEntries) {
      factTypes.push({ id: m[1].trim(), reading: m[2].trim() })
    }
  }

  // Extract constraints section
  const constraints: Array<{ kind: string; text: string }> = []
  const cSection = raw.match(/<constraints,\s*(<<.*?>>|φ)/)
  if (cSection && cSection[1] !== 'φ') {
    const cContent = cSection[1]
    const cEntries = cContent.matchAll(/<([^,<>]+),\s*([^<>]+)>/g)
    for (const m of cEntries) {
      constraints.push({ kind: m[1].trim(), text: m[2].trim() })
    }
  }

  // Extract totalFacts
  const tfMatch = raw.match(/<totalFacts,\s*(\d+)>/)
  const totalFacts = tfMatch ? parseInt(tfMatch[1], 10) : 0

  return { nouns, factTypes, constraints, totalFacts, raw }
}

// ── Core fixture function ───────────────────────────────────────────────────

/**
 * Parse FORML2 readings via the WASM engine and compile to IR.
 *
 * Steps:
 *   1. compileDomainReadings([[domain, readings]]) → handle
 *   2. system(handle, 'debug', '') → display string
 *   3. parseDebugIR() → structured { nouns, factTypes, constraints }
 *
 * Note: parseReadings(system(0, 'parse', ...)) returns [] in the current WASM build.
 * The entity/noun list is extracted from the debug IR instead.
 */
export function compileDomain(readings: string, ...prereqs: string[]): CompiledDomain {
  const handle: number = compileDomainReadings(...prereqs, readings)
  const raw: string = system(handle, 'debug', '')
  const ir = parseDebugIR(raw)
  const entities: string[] = ir.nouns
  return { ir, entities, handle }
}

// ── Utility functions ───────────────────────────────────────────────────────

/** Try JSON.parse; if not JSON, return the raw display string. */
function parseResult(raw: string): any {
  try { return JSON.parse(raw) } catch { return raw }
}

/** Evaluate constraints against a population. */
export function evaluate(handle: number, text: string, population: string): any {
  return parseResult(system(handle, 'evaluate', text))
}

/** Get available transitions for a noun in a given status.
 *  Returns array of { from, to, event } parsed from display notation.
 *  Terminal status returns empty array (φ). */
export function transitions(handle: number, noun: string, status: string): Array<{ from: string; to: string; event: string }> {
  const raw = system(handle, `transitions:${noun}`, status)
  if (raw === 'φ' || raw === '⊥') return []
  const matches = [...raw.matchAll(/<([^<>,]+),\s*([^<>,]+),\s*([^<>,]+)>/g)]
  return matches.map(m => ({ from: m[1].trim(), to: m[2].trim(), event: m[3].trim() }))
}

/** Run forward chaining over a population. */
export function forwardChain(handle: number, population: string): any {
  return parseResult(system(handle, 'forward_chain', population))
}

/** Raw system call for testing arbitrary def keys. */
export function systemRaw(handle: number, key: string, input: string): string {
  return system(handle, key, input)
}

/**
 * Release a compiled domain handle.
 */
export function releaseDomain(handle: number): void {
  release_domain(handle)
}

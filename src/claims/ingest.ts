/**
 * Claims ingestion — ported from Payload to GraphDLDBLike.
 *
 * Two entry points:
 * - ingestClaims()   — bulk structured claims
 */
import type { GraphDLDBLike } from '../do-adapter'
import { createScope } from './scope'
import { ingestNouns, ingestSubtypes, ingestReadings, ingestConstraints, ingestTransitions, ingestFacts } from './steps'
import type { DerivationRule } from '../derivation/parse-rule'

export interface ExtractedClaims {
  nouns: Array<{
    name: string
    objectType: 'entity' | 'value'
    plural?: string
    valueType?: string
    format?: string
    enum?: string[]
    enumValues?: string[]
    minimum?: number
    maximum?: number
    pattern?: string
    worldAssumption?: 'closed' | 'open'
  }>
  readings: Array<{
    text: string
    nouns: string[]
    predicate: string
    multiplicity?: string
    derivation?: string
    ruleIR?: DerivationRule
  }>
  constraints: Array<{
    kind: 'UC' | 'MC' | 'IR' | 'SS' | 'XC' | 'EQ' | 'OR' | 'XO'
    modality: 'Alethic' | 'Deontic'
    deonticOperator?: 'obligatory' | 'forbidden' | 'permitted'
    reading: string
    roles: number[]
    /** Full verbalized text (set-comparison constraints) */
    text?: string
    /** For XO/XC/OR: the individual clause texts */
    clauses?: string[]
    /** For set-comparison: the constrained entity name */
    entity?: string
    /** For set-comparison: role spans across multiple readings */
    spans?: Array<{ reading: string; roles: number[] }>
  }>
  subtypes?: Array<{ child: string; parent: string }>
  transitions?: Array<{ entity: string; from: string; to: string; event: string }>
  facts?: Array<{
    reading?: string
    values?: Array<{ noun: string; value: string }>
    /** Entity-centric format from FORML2 parser */
    entity?: string
    entityValue?: string
    predicate?: string
    valueType?: string
    value?: string
  }>
}

export interface IngestClaimsResult {
  nouns: number
  readings: number
  stateMachines: number
  skipped: number
  errors: string[]
}

/**
 * Ingest bulk structured claims.
 */
export async function ingestClaims(
  db: GraphDLDBLike,
  opts: { claims: ExtractedClaims; domainId: string },
): Promise<IngestClaimsResult> {
  const { claims, domainId } = opts
  const scope = createScope()

  const nouns = await ingestNouns(db, claims.nouns, domainId, scope)
  await ingestSubtypes(db, claims.subtypes || [], domainId, scope)
  const nounsBefore = scope.nouns.size
  const readings = await ingestReadings(db, claims.readings, domainId, scope)
  const autoCreatedNouns = scope.nouns.size - nounsBefore
  await ingestConstraints(db, claims.constraints || [], domainId, scope)
  const stateMachines = await ingestTransitions(db, claims.transitions || [], domainId, scope)

  if (claims.facts?.length) {
    try { await (db as any).applySchema(domainId) } catch { /* may fail if no readings yet */ }
  }
  await ingestFacts(db, claims.facts || [], domainId, scope)

  // Strip [domainId] prefix from errors for backward compatibility
  const prefix = `[${domainId}] `

  return {
    nouns: nouns + autoCreatedNouns,
    readings,
    stateMachines,
    skipped: scope.skipped,
    errors: scope.errors.map(e => e.startsWith(prefix) ? e.slice(prefix.length) : e),
  }
}

// ---------------------------------------------------------------------------
// Multi-domain project ingestion
// ---------------------------------------------------------------------------

export interface ProjectResult {
  domains: Map<string, IngestClaimsResult>
  totals: { nouns: number; readings: number; stateMachines: number; errors: string[] }
}

/**
 * Ingest claims from multiple domains within a single shared scope.
 *
 * Unlike calling ingestClaims() per domain, this function shares a single
 * Scope across all domains so that cross-domain noun references resolve
 * correctly (e.g. Domain B can reference a noun defined in Domain A).
 *
 * Steps are executed in phase order (all nouns first, then all subtypes, etc.)
 * so that later phases can rely on earlier phases across all domains.
 */
export async function ingestProject(
  db: GraphDLDBLike,
  domains: Array<{ domainId: string; claims: ExtractedClaims }>,
): Promise<ProjectResult> {
  const scope = createScope()
  const counters = new Map<string, { nouns: number; readings: number; stateMachines: number }>()
  for (const d of domains) counters.set(d.domainId, { nouns: 0, readings: 0, stateMachines: 0 })

  // Step 1: All nouns across all domains
  for (const { domainId, claims } of domains) {
    counters.get(domainId)!.nouns = await ingestNouns(db, claims.nouns, domainId, scope)
  }
  // Step 2: All subtypes
  for (const { domainId, claims } of domains) {
    await ingestSubtypes(db, claims.subtypes || [], domainId, scope)
  }
  // Step 3: All readings
  for (const { domainId, claims } of domains) {
    counters.get(domainId)!.readings = await ingestReadings(db, claims.readings, domainId, scope)
  }
  // Step 4: All constraints
  for (const { domainId, claims } of domains) {
    await ingestConstraints(db, claims.constraints || [], domainId, scope)
  }
  // Step 5: All transitions
  for (const { domainId, claims } of domains) {
    counters.get(domainId)!.stateMachines = await ingestTransitions(db, claims.transitions || [], domainId, scope)
  }
  // Step 5.5: Apply schema for ALL domains before facts
  for (const { domainId } of domains) {
    try { await (db as any).applySchema(domainId) } catch {}
  }
  // Step 6: All facts
  for (const { domainId, claims } of domains) {
    await ingestFacts(db, claims.facts || [], domainId, scope)
  }

  // Build per-domain results with domain-prefixed error attribution
  const perDomain = new Map<string, IngestClaimsResult>()
  for (const { domainId } of domains) {
    const c = counters.get(domainId)!
    const prefix = `[${domainId}] `
    perDomain.set(domainId, {
      nouns: c.nouns,
      readings: c.readings,
      stateMachines: c.stateMachines,
      skipped: scope.skipped,
      errors: scope.errors.filter(e => e.startsWith(prefix)).map(e => e.slice(prefix.length)),
    })
  }

  return {
    domains: perDomain,
    totals: {
      nouns: [...counters.values()].reduce((s, c) => s + c.nouns, 0),
      readings: [...counters.values()].reduce((s, c) => s + c.readings, 0),
      stateMachines: [...counters.values()].reduce((s, c) => s + c.stateMachines, 0),
      errors: [...scope.errors],
    },
  }
}

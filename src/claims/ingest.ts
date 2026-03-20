/**
 * Claims ingestion — ported from Payload to GraphDLDB.
 *
 * Two entry points:
 * - ingestClaims()   — bulk structured claims
 */
import type { GraphDLDB } from '../do'
import { createScope } from './scope'
import { ingestNouns, ingestSubtypes, ingestReadings, ingestConstraints, ingestTransitions, ingestFacts } from './steps'

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
  }>
  constraints: Array<{
    kind: 'UC' | 'MC' | 'RC' | 'SS' | 'XC' | 'EQ' | 'OR' | 'XO'
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
  db: GraphDLDB,
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

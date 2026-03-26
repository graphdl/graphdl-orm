/**
 * Claims ingestion — bulk structured claims.
 *
 * Metamodel entities are accumulated via BatchBuilder (committed by the caller).
 * Instance facts still go through FactWriterLike.createEntity() directly.
 */

/** Minimal DB interface needed only for instance facts (ingestFacts). */
export interface FactWriterLike {
  createEntity?(domainId: string, entityName: string, fields: any, reference?: string): Promise<any>
  applySchema?(domainId: string): Promise<any>
}

import { BatchBuilder } from './batch-builder'
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
    refScheme?: string[]
    objectifies?: string
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
    kind: 'UC' | 'MC' | 'IR' | 'SY' | 'AS' | 'TR' | 'IT' | 'ANS' | 'AC' | 'RF' | 'SS' | 'XC' | 'EQ' | 'OR' | 'XO'
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
  /** The batch of metamodel entities accumulated during ingestion. */
  batch: ReturnType<BatchBuilder['toBatch']>
}

/**
 * Ingest bulk structured claims.
 *
 * Metamodel entities are accumulated in a BatchBuilder and returned in the
 * result. The caller is responsible for committing the batch to storage.
 * Instance facts are written via db.createEntity() as before.
 */
export async function ingestClaims(
  db: FactWriterLike,
  opts: { claims: ExtractedClaims; domainId: string },
): Promise<IngestClaimsResult> {
  const { claims, domainId } = opts
  const scope = createScope()
  const builder = new BatchBuilder(domainId)

  const nouns = ingestNouns(builder, claims.nouns, domainId, scope)
  ingestSubtypes(builder, claims.subtypes || [], domainId, scope)

  // Build objectification map: reading text → noun ID
  // For nouns with objectifies, map the reading text to the noun's ID
  // so the graph schema shares the noun's identity (Graph Schema IS a Noun)
  const objectificationMap = new Map<string, string>()
  for (const noun of claims.nouns) {
    if (noun.objectifies) {
      const nounRecord = scope.nouns.get(`${domainId}:${noun.name}`)
      if (nounRecord) objectificationMap.set(noun.objectifies, nounRecord.id)
    }
  }

  const nounsBefore = scope.nouns.size
  const readings = ingestReadings(builder, claims.readings, domainId, scope, objectificationMap)
  const autoCreatedNouns = scope.nouns.size - nounsBefore
  ingestConstraints(builder, claims.constraints || [], domainId, scope)
  const stateMachines = ingestTransitions(builder, claims.transitions || [], domainId, scope)

  if (claims.facts?.length) {
    try { await db.applySchema?.(domainId) } catch { /* may fail if no readings yet */ }
  }
  await ingestFacts(db, claims.facts || [], domainId, scope, builder)

  // Strip [domainId] prefix from errors for backward compatibility
  const prefix = `[${domainId}] `

  return {
    nouns: nouns + autoCreatedNouns,
    readings,
    stateMachines,
    skipped: scope.skipped,
    errors: scope.errors.map(e => e.startsWith(prefix) ? e.slice(prefix.length) : e),
    batch: builder.toBatch(),
  }
}

// ---------------------------------------------------------------------------
// Multi-domain project ingestion
// ---------------------------------------------------------------------------

export interface ProjectResult {
  domains: Map<string, IngestClaimsResult>
  totals: { nouns: number; readings: number; stateMachines: number; errors: string[] }
  /** Combined batch of all metamodel entities across all domains. */
  batch: ReturnType<BatchBuilder['toBatch']>
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
 *
 * A single BatchBuilder is shared across all domains so that cross-domain
 * entity references (e.g. roles pointing to nouns from other domains) resolve.
 */
export async function ingestProject(
  db: FactWriterLike,
  domains: Array<{ domainId: string; claims: ExtractedClaims }>,
): Promise<ProjectResult> {
  const scope = createScope()
  // Use a shared builder across all domains so cross-domain references work.
  // The domain field is set per-entity via ensureEntity data, not the builder's
  // top-level domain. Use a sentinel domain for the builder itself.
  const builder = new BatchBuilder('__project__')
  const counters = new Map<string, { nouns: number; readings: number; stateMachines: number }>()
  for (const d of domains) counters.set(d.domainId, { nouns: 0, readings: 0, stateMachines: 0 })

  // Step 1: All nouns across all domains
  for (const { domainId, claims } of domains) {
    counters.get(domainId)!.nouns = ingestNouns(builder, claims.nouns, domainId, scope)
  }
  // Step 2: All subtypes
  for (const { domainId, claims } of domains) {
    ingestSubtypes(builder, claims.subtypes || [], domainId, scope)
  }
  // Step 3: All readings
  for (const { domainId, claims } of domains) {
    const objMap = new Map<string, string>()
    for (const noun of claims.nouns) {
      if (noun.objectifies) {
        const rec = scope.nouns.get(`${domainId}:${noun.name}`)
        if (rec) objMap.set(noun.objectifies, rec.id)
      }
    }
    counters.get(domainId)!.readings = ingestReadings(builder, claims.readings, domainId, scope, objMap.size > 0 ? objMap : undefined)
  }
  // Step 4: All constraints
  for (const { domainId, claims } of domains) {
    ingestConstraints(builder, claims.constraints || [], domainId, scope)
  }
  // Step 5: All transitions
  for (const { domainId, claims } of domains) {
    counters.get(domainId)!.stateMachines = ingestTransitions(builder, claims.transitions || [], domainId, scope)
  }
  // Step 5.5: Apply schema for ALL domains before facts
  for (const { domainId } of domains) {
    try { await db.applySchema?.(domainId) } catch {}
  }
  // Step 6: All facts
  for (const { domainId, claims } of domains) {
    await ingestFacts(db, claims.facts || [], domainId, scope, builder)
  }

  // Build per-domain results with domain-prefixed error attribution
  const perDomain = new Map<string, IngestClaimsResult>()
  const emptyBatch = { domain: '', entities: [] as any[] }
  for (const { domainId } of domains) {
    const c = counters.get(domainId)!
    const prefix = `[${domainId}] `
    perDomain.set(domainId, {
      nouns: c.nouns,
      readings: c.readings,
      stateMachines: c.stateMachines,
      skipped: scope.skipped,
      errors: scope.errors.filter(e => e.startsWith(prefix)).map(e => e.slice(prefix.length)),
      batch: emptyBatch,
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
    batch: builder.toBatch(),
  }
}

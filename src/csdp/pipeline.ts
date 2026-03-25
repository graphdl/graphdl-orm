/**
 * CSDP Pipeline — orchestrates CSDP validation, batch building, RMAP,
 * and constraint induction into a single pure-function call.
 *
 * Input:  ExtractedClaims (from the FORML2 parser)
 * Output: PipelineResult (valid batch + tables, or rejection with violations)
 */

import { validateCsdp, type SchemaIR, type CsdpViolation } from './validate'
import { induceConstraints, type InductionResult } from './induce'
import { rmap, type RmapSchemaIR, type TableDef } from '../rmap/procedure'
import { BatchBuilder } from '../claims/batch-builder'
import type { BatchEntity } from '../batch-wal'
import type { ExtractedClaims } from '../claims/ingest'

// ── Public types ─────────────────────────────────────────────────────

export interface ViolationEntity {
  id: string
  type: 'Violation'
  data: {
    constraintId: string | null
    text: string
    severity: 'error'
    failureType: string
    fix: string
    occurredAt: string
  }
}

export interface PipelineResult {
  valid: boolean
  violations: CsdpViolation[]
  /** Violation entities ready for persistence as EntityDB DOs. */
  violationEntities: ViolationEntity[]
  batch?: { domain: string; entities: BatchEntity[] }
  tables?: TableDef[]
  induced?: InductionResult
}

// ── Pipeline entry point ─────────────────────────────────────────────

/**
 * Run the full CSDP pipeline on extracted claims.
 *
 * 1. Build SchemaIR from ExtractedClaims
 * 2. Validate via CSDP checks
 * 3. Build a batch via BatchBuilder
 * 4. Run RMAP to produce relational tables
 * 5. Run constraint induction if population facts are available
 */
export function runCsdpPipeline(claims: ExtractedClaims, domain: string): PipelineResult {
  // Step 1: Build SchemaIR from claims
  const schemaIR = buildSchemaIR(claims)

  // Step 2: Validate via CSDP
  const validation = validateCsdp(schemaIR)
  if (!validation.valid) {
    const now = new Date().toISOString()
    const violationEntities: ViolationEntity[] = validation.violations.map(v => ({
      id: crypto.randomUUID(),
      type: 'Violation' as const,
      data: {
        constraintId: v.constraintId ?? null,
        text: v.message,
        severity: 'error' as const,
        failureType: v.type,
        fix: v.fix,
        occurredAt: now,
      },
    }))
    return { valid: false, violations: validation.violations, violationEntities }
  }

  // Step 3: Build batch via BatchBuilder
  const builder = new BatchBuilder(domain)

  for (const noun of claims.nouns) {
    builder.ensureEntity('Noun', 'name', noun.name, {
      name: noun.name,
      objectType: noun.objectType,
      ...(noun.refScheme ? { refScheme: noun.refScheme } : {}),
      ...(noun.worldAssumption ? { worldAssumption: noun.worldAssumption } : {}),
    })
  }

  for (const reading of claims.readings) {
    builder.addEntity('Reading', {
      text: reading.text,
      ...(reading.nouns ? { nouns: reading.nouns } : {}),
      ...(reading.predicate ? { predicate: reading.predicate } : {}),
      ...(reading.multiplicity ? { multiplicity: reading.multiplicity } : {}),
    })
  }

  for (const constraint of (claims.constraints ?? [])) {
    builder.addEntity('Constraint', {
      kind: constraint.kind,
      reading: constraint.reading,
      roles: constraint.roles,
      modality: constraint.modality,
      ...(constraint.text ? { text: constraint.text } : {}),
    })
  }

  for (const subtype of (claims.subtypes ?? [])) {
    builder.addEntity('Subtype', {
      child: subtype.child,
      parent: subtype.parent,
    })
  }

  // Step 4: Run RMAP
  const rmapIR = buildRmapSchemaIR(claims, schemaIR)
  const tables = rmap(rmapIR)

  // Step 5: Run induction if population available
  let induced: InductionResult | undefined
  if (claims.facts && claims.facts.length > 0) {
    const irForInduction = buildInductionIR(claims, schemaIR)
    const population = buildPopulation(claims, schemaIR)
    induced = induceConstraints(
      JSON.stringify(irForInduction),
      JSON.stringify(population),
    )
  }

  return {
    valid: true,
    violations: [],
    violationEntities: [],
    batch: builder.toBatch(),
    tables,
    induced,
  }
}

// ── Internal helpers ─────────────────────────────────────────────────

/**
 * Generate a stable fact-type ID from a reading text.
 * Slugifies the reading into a kebab-style id.
 */
function readingToFactTypeId(text: string): string {
  return 'ft-' + text
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-|-$/g, '')
}

/**
 * Extract role noun names from a reading.
 * If the reading has an explicit `nouns` array, use that.
 * Otherwise, match declared noun names found in the reading text.
 */
function extractRoles(
  reading: { text: string; nouns?: string[] },
  declaredNouns: Map<string, { name: string; objectType: string }>,
): Array<{ nounName: string; roleIndex: number }> {
  // If explicit nouns array provided, use it
  if (reading.nouns && reading.nouns.length > 0) {
    return reading.nouns.map((name, idx) => ({ nounName: name, roleIndex: idx }))
  }

  // Otherwise scan the reading text for declared noun names (longest first to
  // avoid partial matches like "Name" matching inside "NameSpace")
  const sortedNouns = [...declaredNouns.keys()].sort((a, b) => b.length - a.length)
  const roles: Array<{ nounName: string; roleIndex: number }> = []
  let remaining = reading.text

  for (const nounName of sortedNouns) {
    // Use word-boundary matching
    const pattern = new RegExp(`\\b${escapeRegex(nounName)}\\b`)
    if (pattern.test(remaining)) {
      roles.push({ nounName, roleIndex: roles.length })
      // Remove the matched noun to avoid double-matching
      remaining = remaining.replace(pattern, '')
    }
  }

  return roles
}

function escapeRegex(str: string): string {
  return str.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
}

/**
 * Build a SchemaIR from ExtractedClaims.
 * This is the bridge between the parser output and the CSDP/RMAP inputs.
 */
export function buildSchemaIR(claims: ExtractedClaims): SchemaIR {
  const declaredNouns = new Map(
    claims.nouns.map(n => [n.name, { name: n.name, objectType: n.objectType }]),
  )

  // Build fact types from readings
  const factTypes: SchemaIR['factTypes'] = claims.readings.map(reading => {
    const id = readingToFactTypeId(reading.text)
    const roles = extractRoles(reading, declaredNouns)
    return { id, reading: reading.text, roles }
  })

  // Build a lookup from reading text to fact type id
  const readingToId = new Map(factTypes.map(ft => [ft.reading, ft.id]))

  // Build constraints — map reading to factTypeId
  const constraints: SchemaIR['constraints'] = (claims.constraints ?? []).map(c => ({
    kind: c.kind,
    factTypeId: readingToId.get(c.reading) ?? '',
    roles: c.roles,
    modality: c.modality,
    text: c.text,
  }))

  // Build subtypes — ExtractedClaims uses child/parent, SchemaIR uses subtype/supertype
  const subtypes: SchemaIR['subtypes'] = (claims.subtypes ?? []).map(st => ({
    subtype: st.child,
    supertype: st.parent,
  }))

  return {
    nouns: claims.nouns.map(n => ({ name: n.name, objectType: n.objectType })),
    factTypes,
    constraints,
    ...(subtypes.length > 0 ? { subtypes } : {}),
  }
}

/**
 * Build an RmapSchemaIR from ExtractedClaims and the already-built SchemaIR.
 * RmapSchemaIR extends SchemaIR with refScheme on nouns.
 */
function buildRmapSchemaIR(claims: ExtractedClaims, schema: SchemaIR): RmapSchemaIR {
  return {
    nouns: claims.nouns.map(n => ({
      name: n.name,
      objectType: n.objectType,
      ...(n.refScheme ? { refScheme: n.refScheme.join(', ') } : {}),
    })),
    factTypes: schema.factTypes,
    constraints: schema.constraints,
    ...(schema.subtypes ? { subtypes: schema.subtypes } : {}),
  }
}

/**
 * Build the IR JSON structure expected by induceConstraints.
 */
function buildInductionIR(
  claims: ExtractedClaims,
  schema: SchemaIR,
): { nouns: Record<string, { worldAssumption?: string }>; factTypes: Record<string, { reading: string; roles: Array<{ nounName: string }> }>; constraints: any[] } {
  const nouns: Record<string, { worldAssumption?: string }> = {}
  for (const n of claims.nouns) {
    nouns[n.name] = { worldAssumption: n.worldAssumption ?? 'closed' }
  }

  const factTypes: Record<string, { reading: string; roles: Array<{ nounName: string }> }> = {}
  for (const ft of schema.factTypes) {
    factTypes[ft.id] = {
      reading: ft.reading,
      roles: ft.roles.map(r => ({ nounName: r.nounName })),
    }
  }

  return { nouns, factTypes, constraints: schema.constraints }
}

/**
 * Build a population JSON structure from ExtractedClaims facts.
 *
 * Maps each fact to the appropriate fact type, producing the bindings
 * array that induceConstraints expects.
 */
function buildPopulation(
  claims: ExtractedClaims,
  schema: SchemaIR,
): { facts: Record<string, Array<{ bindings: Array<[string, string]> }>> } {
  const facts: Record<string, Array<{ bindings: Array<[string, string]> }>> = {}

  // Initialize empty arrays for each fact type
  for (const ft of schema.factTypes) {
    facts[ft.id] = []
  }

  for (const fact of (claims.facts ?? [])) {
    if (fact.reading && fact.values) {
      // Reading-based fact: match by reading text
      const ftId = schema.factTypes.find(ft => ft.reading === fact.reading)?.id
      if (ftId) {
        const bindings: Array<[string, string]> = fact.values.map(v => [v.noun, v.value])
        facts[ftId].push({ bindings })
      }
    } else if (fact.entity && fact.predicate) {
      // Entity-centric fact: find matching fact type by entity + predicate
      const matchingFt = schema.factTypes.find(ft => {
        const hasEntity = ft.roles.some(r => r.nounName === fact.entity)
        const readingContainsPredicate = ft.reading.toLowerCase().includes(
          (fact.predicate ?? '').toLowerCase(),
        )
        return hasEntity && readingContainsPredicate
      })
      if (matchingFt) {
        const bindings: Array<[string, string]> = [
          [fact.entity!, fact.entityValue ?? ''],
        ]
        if (fact.valueType && fact.value) {
          bindings.push([fact.valueType, fact.value])
        }
        facts[matchingFt.id]!.push({ bindings })
      }
    }
  }

  return { facts }
}

/**
 * Converters: transform parsed domain/state-machine/FORML2 results into
 * ExtractedClaims format so all ingestion flows through ingestClaims().
 *
 * Three converter functions:
 * - domainParseToClaims()        — DomainParseResult -> ExtractedClaims
 * - stateMachineParseToClaims()  — StateMachineParseResult -> ExtractedClaims
 * - readingDefsToClaims()        — ReadingDef[] -> ExtractedClaims
 */

import type { DomainParseResult, StateMachineParseResult, ReadingDef } from '../seed/parser'
import type { ExtractedClaims } from './ingest'

// ── Helpers ────────────────────────────────────────────────────────────────────

/**
 * Extract PascalCase noun names from a reading text, using a set of known noun
 * names. Matches whole words only; longest-first to avoid partial matches
 * (e.g., "SupportRequest" before "Request").
 */
function extractNounsFromText(text: string, knownNouns: Set<string>): string[] {
  // Sort longest first so "SupportRequest" matches before "Request"
  const sorted = [...knownNouns].sort((a, b) => b.length - a.length)
  if (!sorted.length) return []

  const pattern = sorted.map((n) => n.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')).join('|')
  const regex = new RegExp('\\b(' + pattern + ')\\b', 'g')

  const found: string[] = []
  let match: RegExpExecArray | null
  while ((match = regex.exec(text)) !== null) {
    found.push(match[1])
  }
  return found
}

/**
 * Extract the predicate (verb phrase) between the first two nouns in a reading text.
 */
function extractPredicate(text: string, nouns: string[]): string {
  if (nouns.length < 2) return ''
  const firstEnd = text.indexOf(nouns[0]) + nouns[0].length
  const secondStart = text.indexOf(nouns[1], firstEnd)
  if (secondStart <= firstEnd) return ''
  return text.slice(firstEnd, secondStart).trim()
}

/**
 * Discover PascalCase words in text for cases where we don't have a pre-built
 * noun set (e.g., FORML2 plain text). Matches sequences of uppercase-led
 * segments like "Customer", "SupportRequest", "EmailAddress".
 */
function discoverPascalCaseWords(text: string): string[] {
  // Match PascalCase words: starts with uppercase letter, may contain lowercase
  // Handles multi-segment names like "SupportRequest", "EmailAddress"
  const regex = /\b([A-Z][a-zA-Z0-9]*)\b/g
  const found = new Set<string>()
  let match: RegExpExecArray | null

  // Common English words that should not be treated as noun names
  const ignore = new Set([
    'Each', 'It', 'That', 'The', 'If', 'A', 'An', 'Some',
    'Is', 'Has', 'Was', 'Are', 'Were', 'Be', 'Been',
    'No', 'Not', 'Or', 'And', 'For', 'In', 'On', 'Of',
    'To', 'At', 'By', 'With', 'From',
    'UC', 'MC', 'SS', 'DMC', 'DSS', 'AMC',
  ])

  while ((match = regex.exec(text)) !== null) {
    const word = match[1]
    if (!ignore.has(word)) {
      found.add(word)
    }
  }
  return [...found]
}

// ── domainParseToClaims ────────────────────────────────────────────────────────

/**
 * Convert a DomainParseResult to ExtractedClaims format.
 *
 * Maps entity types and value types to nouns, readings to claims readings
 * (extracting noun references and predicates from text), multiplicity to
 * the reading's multiplicity field (ingestClaims handles constraint creation),
 * and explicit UC notation to claims.constraints.
 */
export function domainParseToClaims(parsed: DomainParseResult): ExtractedClaims {
  const claims: ExtractedClaims = {
    nouns: [],
    readings: [],
    constraints: [],
    subtypes: [],
    facts: [],
  }

  // Build a set of all known noun names for extraction
  const knownNouns = new Set<string>()

  // Entity types -> nouns
  for (const e of parsed.entityTypes) {
    knownNouns.add(e.name)
    claims.nouns.push({
      name: e.name,
      objectType: 'entity',
      plural: e.name.replace(/([A-Z])/g, '-$1').replace(/^-/, '').toLowerCase() + 's',
    })
  }

  // Value types -> nouns
  for (const v of parsed.valueTypes) {
    knownNouns.add(v.name)
    const noun: ExtractedClaims['nouns'][number] = {
      name: v.name,
      objectType: 'value',
      valueType: v.valueType,
    }
    if (v.format) noun.format = v.format
    if (v.enum) noun.enum = v.enum.split(',').map((s) => s.trim())
    if (v.pattern) noun.pattern = v.pattern
    if (v.minimum !== undefined) noun.minimum = v.minimum
    if (v.maximum !== undefined) noun.maximum = v.maximum
    claims.nouns.push(noun)
  }

  // Readings -> claims.readings, claims.subtypes, claims.constraints
  for (const r of parsed.readings) {
    // Subtype readings
    if (r.multiplicity === 'subtype') {
      const subtypeMatch = r.text.match(/^(\S+)\s+is\s+a\s+subtype\s+of\s+(\S+)$/i)
      if (subtypeMatch) {
        claims.subtypes!.push({ child: subtypeMatch[1], parent: subtypeMatch[2] })
      }
      continue
    }

    // Subset constraint readings — pass through as readings (ingestClaims doesn't
    // handle SS natively, but keeping them preserves behavior)
    // Actually, ingestClaims uses parseMultiplicity which returns [] for SS.
    // These need special handling. For now we skip SS readings since ingestClaims
    // doesn't support them directly. They are edge cases.
    if (/^D?SS$/i.test(r.multiplicity.split(/\s+/)[0])) {
      // Skip subset constraints — not supported in ExtractedClaims format
      continue
    }

    const nouns = extractNounsFromText(r.text, knownNouns)
    const predicate = extractPredicate(r.text, nouns)

    claims.readings.push({
      text: r.text,
      nouns,
      predicate,
      multiplicity: r.multiplicity,
    })

    // Explicit UC notation (ternary constraints) -> claims.constraints
    // We need to convert UC(NounA, NounB) to role indexes.
    // Since roles are created in noun-appearance order, we can map noun names
    // to their positional index in the reading's nouns array.
    if (r.ucs?.length) {
      for (const ucRoleNames of r.ucs) {
        const roleIndexes = ucRoleNames
          .map((roleName) => nouns.indexOf(roleName))
          .filter((idx) => idx !== -1)

        if (roleIndexes.length) {
          claims.constraints.push({
            kind: 'UC',
            modality: 'Alethic',
            reading: r.text,
            roles: roleIndexes,
          })
        }
      }
      // Clear the multiplicity so ingestClaims doesn't also create duplicate
      // constraints from the multiplicity field (since we've handled them explicitly)
      const lastReading = claims.readings[claims.readings.length - 1]
      lastReading.multiplicity = undefined
    }
  }

  // Deontic constraints -> additional readings with DMC multiplicity
  for (const text of parsed.deonticConstraints) {
    const nouns = extractNounsFromText(text, knownNouns)
    const predicate = extractPredicate(text, nouns)
    claims.readings.push({
      text,
      nouns,
      predicate,
      multiplicity: '*:1',
    })
  }

  // Deontic constraint instances -> additional readings (violation message annotations)
  for (const d of parsed.deonticConstraintInstances) {
    let inst = d.instance
    // Strip surrounding quotes
    if (
      inst.length >= 2 &&
      ((inst.startsWith('"') && inst.endsWith('"')) ||
        (inst.startsWith("'") && inst.endsWith("'")) ||
        (inst.startsWith('\u201C') && inst.endsWith('\u201D')) ||
        (inst.startsWith('\u2018') && inst.endsWith('\u2019')))
    ) {
      inst = inst.slice(1, -1)
    }
    const text = `${d.constraint} '${inst}'`
    const nouns = extractNounsFromText(text, knownNouns)
    const predicate = extractPredicate(text, nouns)
    claims.readings.push({
      text,
      nouns,
      predicate,
      multiplicity: '*:1',
    })
  }

  // Instance facts -> claims.facts
  for (const factText of parsed.instanceFacts) {
    const instances: { entityType: string; value: string }[] = []
    const pattern = /(\b[A-Z]\w*)\s+'([^']+)'/g
    let match
    while ((match = pattern.exec(factText)) !== null) {
      instances.push({ entityType: match[1], value: match[2] })
    }

    if (instances.length === 0) {
      // No quoted values — treat as a plain reading (e.g., verb/function wiring)
      const nouns = extractNounsFromText(factText, knownNouns)
      const predicate = extractPredicate(factText, nouns)
      claims.readings.push({
        text: factText,
        nouns,
        predicate,
        multiplicity: '*:1',
      })
      continue
    }

    // Build the base reading text by removing quoted values
    let baseReading = factText
    for (const inst of instances) {
      baseReading = baseReading.replace(` '${inst.value}'`, '')
    }
    baseReading = baseReading.replace(/\s+/g, ' ').trim()

    claims.facts!.push({
      reading: baseReading,
      values: instances.map((inst) => ({ noun: inst.entityType, value: inst.value })),
    })
  }

  return claims
}

// ── stateMachineParseToClaims ──────────────────────────────────────────────────

/**
 * Convert a StateMachineParseResult to ExtractedClaims format.
 *
 * Maps transitions to claims.transitions with the provided entity noun name.
 * Adds the entity noun to claims.nouns.
 */
export function stateMachineParseToClaims(
  parsed: StateMachineParseResult,
  entityNoun: string,
): ExtractedClaims {
  return {
    nouns: [{ name: entityNoun, objectType: 'entity' }],
    readings: [],
    constraints: [],
    transitions: parsed.transitions.map((t) => ({
      entity: entityNoun,
      from: t.from,
      to: t.to,
      event: t.event,
    })),
  }
}

// ── readingDefsToClaims ────────────────────────────────────────────────────────

/**
 * Convert ReadingDef[] (FORML2 plain text format) to ExtractedClaims format.
 *
 * Since FORML2 only provides reading text + multiplicity, we discover noun names
 * from PascalCase words in the text. All nouns default to objectType: 'entity'
 * since we can't distinguish entity vs. value from FORML2 alone.
 */
export function readingDefsToClaims(readings: ReadingDef[]): ExtractedClaims {
  const claims: ExtractedClaims = {
    nouns: [],
    readings: [],
    constraints: [],
    subtypes: [],
  }

  // First pass: discover all noun names from all reading texts
  const allNouns = new Set<string>()
  for (const r of readings) {
    for (const word of discoverPascalCaseWords(r.text)) {
      allNouns.add(word)
    }
  }

  // Create noun entries (all entity type since FORML2 doesn't distinguish)
  for (const name of allNouns) {
    claims.nouns.push({ name, objectType: 'entity' })
  }

  // Second pass: convert readings
  for (const r of readings) {
    // Subtype readings
    if (r.multiplicity === 'subtype') {
      const subtypeMatch = r.text.match(/^(\S+)\s+is\s+a\s+subtype\s+of\s+(\S+)$/i)
      if (subtypeMatch) {
        claims.subtypes!.push({ child: subtypeMatch[1], parent: subtypeMatch[2] })
      }
      continue
    }

    // Skip subset constraints (not supported in ExtractedClaims)
    if (/^D?SS$/i.test(r.multiplicity.split(/\s+/)[0])) {
      continue
    }

    const nouns = extractNounsFromText(r.text, allNouns)
    const predicate = extractPredicate(r.text, nouns)

    claims.readings.push({
      text: r.text,
      nouns,
      predicate,
      multiplicity: r.multiplicity,
    })

    // Explicit UC notation -> claims.constraints
    if (r.ucs?.length) {
      for (const ucRoleNames of r.ucs) {
        const roleIndexes = ucRoleNames
          .map((roleName) => nouns.indexOf(roleName))
          .filter((idx) => idx !== -1)

        if (roleIndexes.length) {
          claims.constraints.push({
            kind: 'UC',
            modality: 'Alethic',
            reading: r.text,
            roles: roleIndexes,
          })
        }
      }
      // Clear multiplicity to avoid duplicate constraint creation
      const lastReading = claims.readings[claims.readings.length - 1]
      lastReading.multiplicity = undefined
    }
  }

  return claims
}

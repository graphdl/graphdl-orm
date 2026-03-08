/**
 * FORML2 Verbal Constraint Parser
 *
 * Parses natural-language ORM readings in FORML2 verbal notation and extracts:
 * - Noun roles (ordered)
 * - Predicate text
 * - Uniqueness constraints (UC) from "at most one", "exactly one"
 * - Mandatory constraints (MC) from "some", "exactly one", "one or more", "at least one"
 * - Modality: Alethic (default) or Deontic ("obligatory", "forbidden", etc.)
 * - Subtype declarations: "X is a subtype of Y"
 * - State transitions: "X transitions from S1 to S2 on event"
 * - Instance facts with quoted values: "X with Y 'val' has Z 'val'"
 */

// ─── Types ──────────────────────────────────────────────────────────────────

export interface ParsedConstraint {
  kind: 'UC' | 'MC'
  roles: number[] // indexes into nouns array
  modality: 'Alethic' | 'Deontic'
}

export interface TransitionDef {
  subject: string
  from: string
  to: string
  event: string
}

export interface InstanceValue {
  noun: string
  value: string
}

export interface ParsedReading {
  nouns: string[]
  predicate: string
  constraints: ParsedConstraint[]
  isSubtype: boolean
  isTransition: boolean
  transition?: TransitionDef
  isInstanceFact: boolean
  instanceValues: InstanceValue[]
}

// ─── Helpers ────────────────────────────────────────────────────────────────

/** Sort longest first so "ListingChannel" matches before "Listing" */
function buildNounRegex(knownNouns: string[]): RegExp {
  const sorted = [...knownNouns].sort((a, b) => b.length - a.length)
  return new RegExp(
    '\\b(' + sorted.map((n) => n.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')).join('|') + ')\\b',
    'g',
  )
}

// ─── Parser ─────────────────────────────────────────────────────────────────

export function parseReading(text: string, knownNouns: string[]): ParsedReading {
  const result: ParsedReading = {
    nouns: [],
    predicate: '',
    constraints: [],
    isSubtype: false,
    isTransition: false,
    isInstanceFact: false,
    instanceValues: [],
  }

  // ── Subtype pattern ───────────────────────────────────────────────────
  const subtypeMatch = text.match(/^(\w+)\s+is\s+a\s+subtype\s+of\s+(\w+)$/i)
  if (subtypeMatch) {
    result.isSubtype = true
    result.nouns = [subtypeMatch[1], subtypeMatch[2]]
    return result
  }

  // ── Transition pattern ────────────────────────────────────────────────
  const transitionMatch = text.match(
    /^(\w+)\s+transitions\s+from\s+(\w+)\s+to\s+(\w+)\s+on\s+(\w+)$/i,
  )
  if (transitionMatch) {
    result.isTransition = true
    result.nouns = [transitionMatch[1], transitionMatch[2], transitionMatch[3]]
    result.transition = {
      subject: transitionMatch[1],
      from: transitionMatch[2],
      to: transitionMatch[3],
      event: transitionMatch[4],
    }
    return result
  }

  // ── Instance facts with quoted values ─────────────────────────────────
  const quotedPattern = /(?:with\s+)?(\w+)\s+[''\u2018\u201C]([^''\u2019\u201D]+)[''\u2019\u201D]/g
  let quotedMatch
  while ((quotedMatch = quotedPattern.exec(text)) !== null) {
    if (knownNouns.includes(quotedMatch[1])) {
      result.instanceValues.push({ noun: quotedMatch[1], value: quotedMatch[2] })
    }
  }
  if (result.instanceValues.length > 0) {
    result.isInstanceFact = true
  }

  // ── Tokenize to find nouns in order ───────────────────────────────────
  const nounRegex = buildNounRegex(knownNouns)
  let match
  while ((match = nounRegex.exec(text)) !== null) {
    if (!result.nouns.includes(match[1])) {
      result.nouns.push(match[1])
    }
  }

  // ── Determine modality ────────────────────────────────────────────────
  const isDeontic = /\b(obligatory|forbidden|permitted|prohibited)\b/i.test(text)
  const modality = isDeontic ? ('Deontic' as const) : ('Alethic' as const)

  // ── Extract verbal constraints ────────────────────────────────────────
  if (/\bat\s+most\s+one\b/i.test(text)) {
    result.constraints.push({ kind: 'UC', roles: [0], modality })
  }

  if (/\bhas\s+some\b/i.test(text)) {
    result.constraints.push({ kind: 'MC', roles: [0], modality })
  }

  if (/\bexactly\s+one\b/i.test(text)) {
    result.constraints.push({ kind: 'UC', roles: [0], modality })
    result.constraints.push({ kind: 'MC', roles: [0], modality })
  }

  if (/\b(one\s+or\s+more|at\s+least\s+one)\b/i.test(text)) {
    result.constraints.push({ kind: 'MC', roles: [0], modality })
  }

  // Deontic obligation without explicit constraint quantifiers implies MC
  if (isDeontic && result.constraints.length === 0) {
    result.constraints.push({ kind: 'MC', roles: [0], modality })
  }

  // ── Extract predicate ─────────────────────────────────────────────────
  // Text between the first and second noun, minus constraint quantifier words
  if (result.nouns.length >= 2) {
    const firstNounEnd = text.indexOf(result.nouns[0]) + result.nouns[0].length
    const secondNounStart = text.indexOf(result.nouns[1], firstNounEnd)
    if (secondNounStart > firstNounEnd) {
      result.predicate = text
        .slice(firstNounEnd, secondNounStart)
        .replace(/\b(each|at most one|some|exactly one|one or more|at least one)\b/gi, '')
        .trim()
    }
  }

  return result
}

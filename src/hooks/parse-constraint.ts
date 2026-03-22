/**
 * Deterministic natural language constraint parser.
 *
 * Recognizes canonical FORML2 constraint patterns and returns structured
 * ParsedConstraint objects. Returns null for unrecognized text.
 *
 * Pure function — no DB or LLM dependency.
 */

export type ConstraintKind = 'UC' | 'MC' | 'IR' | 'SY' | 'AS' | 'TR' | 'IT' | 'ANS' | 'AC' | 'RF' | 'SS' | 'XC' | 'EQ' | 'OR' | 'XO'

export interface ParsedConstraint {
  kind: ConstraintKind
  modality: 'Alethic' | 'Deontic'
  deonticOperator?: 'obligatory' | 'forbidden' | 'permitted'
  nouns: string[]
  /**
   * The noun whose role is constrained. In ORM2 verbalization:
   * - "Each A R at most one B" → constrainedNoun = A (UC on A's role)
   * - "Each A R some B" → constrainedNoun = A (MC on A's role)
   * The role index is resolved by finding constrainedNoun's position
   * in the primary reading (graph schema role order = primary reading noun order).
   */
  constrainedNoun?: string
  /** For set-comparison (XO/XC/OR): the constrained entity name */
  entity?: string
  /** For set-comparison (XO/XC/OR): the individual clause texts */
  clauses?: string[]
}

// Match noun names — may be multi-word with spaces (e.g., "Support Request", "API Product")
// Stops before lowercase stopwords (per, that, for, via, etc.) to avoid over-matching
// Noun name: PascalCase words separated by spaces.
// Stopwords (per, that, for, etc.) prevent over-matching into predicates.
// Note: "Of" in "Terms Of Service" is blocked by the 'i' flag on containing regexes.
// Multi-word nouns with "Of" must be handled by the parser (parse.ts), not here.
const NOUN = '([A-Z][a-zA-Z0-9]*(?:\\s+(?!per\\b|that\\b|for\\b|of\\b|the\\b|at\\b|in\\b|via\\b|to\\b|from\\b|by\\b|with\\b|on\\b|or\\b|and\\b|some\\b|each\\b|is\\b|has\\b|are\\b|was\\b|no\\b|not\\b|more\\b|most\\b)[A-Z][a-zA-Z0-9]*)*)'

// "Each X has/belongs to at most one Y."
const AT_MOST_ONE = new RegExp(
  `^Each ${NOUN} (?:has|belongs to|is [a-z]+ (?:by|to|in|for|of|via)) at most one ${NOUN}`,
  'i'
)

// "Each X has exactly one Y."
const EXACTLY_ONE = new RegExp(
  `^Each ${NOUN} (?:has|belongs to|is [a-z]+ (?:by|to|in|for|of|via)) exactly one ${NOUN}`,
  'i'
)

// "Each X has at least one Y." / "Each X has some Y."
const AT_LEAST_ONE = new RegExp(
  `^Each ${NOUN} (?:has|belongs to|is [a-z]+ (?:by|to|in|for|of|via)) at least one ${NOUN}`,
  'i'
)

// "For each pair of X and Y, that X ... that Y at most once."
const SPANNING_UC = new RegExp(
  `^For each pair of ${NOUN} and ${NOUN},.*at most once`,
  'i'
)

// "For each combination of X and Y, that X has at most one Z per that Y."
const TERNARY_UC = new RegExp(
  `^For each combination of ${NOUN} and ${NOUN},.*at most one ${NOUN}`,
  'i'
)

// Ring constraints — all operate on binary facts where subject and object are the same type
// Subscripted noun: PascalCase word optionally followed by a digit (e.g., Person1, Person2)
const RING_NOUN = '([A-Z][a-zA-Z]*\\d?)'

/** Strip trailing digit subscript from a ring-constraint noun: "Person1" → "Person" */
function stripSubscript(noun: string): string {
  return noun.replace(/\d+$/, '')
}

// "No X [verb] itself."
const RING_IRREFLEXIVE = new RegExp(
  `^No ${RING_NOUN} .+? itself`,
  'i'
)
// "If X1 [verb] X2, then X2 is not [verb] X1" / "If X1 [verb] X2, then X2 [verb] not X1"
const RING_ASYMMETRIC = new RegExp(
  `^If ${RING_NOUN} .+? ${RING_NOUN},? then ${RING_NOUN} (?:is not|does not|not) .+? ${RING_NOUN}`,
  'i'
)
// "If X1 [verb] X2, then X2 [verb] X1"
const RING_SYMMETRIC = new RegExp(
  `^If ${RING_NOUN} .+? ${RING_NOUN},? then ${RING_NOUN} .+? ${RING_NOUN}$`,
  'i'
)
// "If X1 [verb] X2 and X2 [verb] X3, then X1 is not [verb] X3"
const RING_INTRANSITIVE = new RegExp(
  `^If ${RING_NOUN} .+? ${RING_NOUN} and ${RING_NOUN} .+? ${RING_NOUN},? then ${RING_NOUN} (?:is not|does not|not) .+? ${RING_NOUN}`,
  'i'
)
// "If X1 [verb] X2 and X2 [verb] X3, then X1 [verb] X3"
const RING_TRANSITIVE = new RegExp(
  `^If ${RING_NOUN} .+? ${RING_NOUN} and ${RING_NOUN} .+? ${RING_NOUN},? then ${RING_NOUN} .+? ${RING_NOUN}$`,
  'i'
)
// "If X1 [verb] X2 and X2 [verb] X1, then X1 is the same as X2"
const RING_ANTISYMMETRIC = new RegExp(
  `^If ${RING_NOUN} .+? ${RING_NOUN} and ${RING_NOUN} .+? ${RING_NOUN},? then ${RING_NOUN} is the same (?:as )?${RING_NOUN}`,
  'i'
)
// "No chain of [verb] links returns to its starting point" / "It is impossible that a cycle of [verb] exists"
const RING_ACYCLIC = /^(?:No chain of .+ (?:returns|leads back)|It is impossible that .*cycl)/i
// "Each X [verb] itself"
const RING_REFLEXIVE = new RegExp(
  `^Each ${RING_NOUN} .+? itself`,
  'i'
)

// "Each X has some Y." → MC (mandatory)
const HAS_SOME = new RegExp(
  `^Each ${NOUN} (?:has|belongs to|authenticates via|provides|supports) some ${NOUN}`,
  'i'
)

// Deontic wrappers
const DEONTIC = /^It is (obligatory|forbidden|permitted) that (.+)$/i

// "... if and only if ..." → EQ (equality/biconditional)
const IF_AND_ONLY_IF = /\bif and only if\b/i

export function parseConstraintText(text: string): ParsedConstraint[] | null {
  if (!text || !text.trim()) return null

  const clean = text.trim().replace(/\.$/, '')

  // Check for deontic wrapper first
  const deonticMatch = clean.match(DEONTIC)
  if (deonticMatch) {
    const operator = deonticMatch[1].toLowerCase() as 'obligatory' | 'forbidden' | 'permitted'
    const inner = parseConstraintText(deonticMatch[2])
    if (!inner) return null
    return inner.map(c => ({ ...c, modality: 'Deontic' as const, deonticOperator: operator }))
  }

  // "Each X has exactly one Y" → UC + MC
  // Constrained noun is X (the noun after "Each") — UC on X's role in the primary reading
  let m = clean.match(EXACTLY_ONE)
  if (m) {
    const nouns = [m[1], m[2]]
    return [
      { kind: 'UC', modality: 'Alethic', nouns, constrainedNoun: m[1] },
      { kind: 'MC', modality: 'Alethic', nouns, constrainedNoun: m[1] },
    ]
  }

  // "Each X has at most one Y" → UC on X's role
  m = clean.match(AT_MOST_ONE)
  if (m) {
    return [{ kind: 'UC', modality: 'Alethic', nouns: [m[1], m[2]], constrainedNoun: m[1] }]
  }

  // "Each X has at least one Y" → MC on X's role
  m = clean.match(AT_LEAST_ONE)
  if (m) {
    return [{ kind: 'MC', modality: 'Alethic', nouns: [m[1], m[2]], constrainedNoun: m[1] }]
  }

  // "For each combination of X and Y, ... at most one Z ..."
  // UC spans X and Y roles (the nouns in the "For each" clause)
  m = clean.match(TERNARY_UC)
  if (m) {
    return [{ kind: 'UC', modality: 'Alethic', nouns: [m[1], m[2], m[3]], constrainedNoun: m[1] }]
  }

  // "For each pair of X and Y, ... at most once"
  // Spanning UC — both roles are constrained (the combination is unique)
  m = clean.match(SPANNING_UC)
  if (m) {
    return [{ kind: 'UC', modality: 'Alethic', nouns: [m[1], m[2]] }]
  }

  // "No X [verb] itself"
  m = clean.match(RING_IRREFLEXIVE)
  if (m) {
    const base = stripSubscript(m[1])
    return [{ kind: 'IR', modality: 'Alethic', nouns: [base], constrainedNoun: base }]
  }

  // Ring constraints — order matters: more specific patterns first

  // "If X1 ... X2 and X2 ... X1, then X1 is the same as X2" → Antisymmetric (before transitive/intransitive)
  m = clean.match(RING_ANTISYMMETRIC)
  if (m) {
    const base = stripSubscript(m[1])
    return [{ kind: 'ANS', modality: 'Alethic', nouns: [base], constrainedNoun: base }]
  }

  // "If X1 ... X2 and X2 ... X3, then X1 is not ... X3" → Intransitive (before transitive)
  m = clean.match(RING_INTRANSITIVE)
  if (m) {
    const base = stripSubscript(m[1])
    return [{ kind: 'IT', modality: 'Alethic', nouns: [base], constrainedNoun: base }]
  }

  // "If X1 ... X2 and X2 ... X3, then X1 ... X3" → Transitive
  m = clean.match(RING_TRANSITIVE)
  if (m) {
    const base = stripSubscript(m[1])
    return [{ kind: 'TR', modality: 'Alethic', nouns: [base], constrainedNoun: base }]
  }

  // "If X1 ... X2, then X2 is not ... X1" → Asymmetric (before symmetric)
  m = clean.match(RING_ASYMMETRIC)
  if (m) {
    const base = stripSubscript(m[1])
    return [{ kind: 'AS', modality: 'Alethic', nouns: [base], constrainedNoun: base }]
  }

  // "If X1 ... X2, then X2 ... X1" → Symmetric
  m = clean.match(RING_SYMMETRIC)
  if (m) {
    const base = stripSubscript(m[1])
    return [{ kind: 'SY', modality: 'Alethic', nouns: [base], constrainedNoun: base }]
  }

  // "No chain of ... returns/leads back" → Acyclic
  m = clean.match(RING_ACYCLIC)
  if (m) {
    // Extract noun from the text heuristically
    const nounMatch = clean.match(/No chain of (\w+)/i) || clean.match(/cycle of (\w+)/i)
    const noun = nounMatch ? nounMatch[1] : ''
    return [{ kind: 'AC', modality: 'Alethic', nouns: [noun], constrainedNoun: noun }]
  }

  // "Each X [verb] itself" → Purely reflexive
  m = clean.match(RING_REFLEXIVE)
  if (m) {
    const base = stripSubscript(m[1])
    return [{ kind: 'RF', modality: 'Alethic', nouns: [base], constrainedNoun: base }]
  }

  // "Each X has some Y" → MC on X's role
  m = clean.match(HAS_SOME)
  if (m) {
    return [{ kind: 'MC', modality: 'Alethic', nouns: [m[1], m[2]], constrainedNoun: m[1] }]
  }

  // "... if and only if ..." → EQ (equality/biconditional)
  if (IF_AND_ONLY_IF.test(clean)) {
    // Extract multi-word noun names by finding sequences of capitalized words
    const allNouns = [...new Set(clean.match(/[A-Z][a-zA-Z0-9]*(?:\s+[A-Z][a-zA-Z0-9]*)*/g) || [])]
      .filter(n => !['Each', 'For', 'The', 'That', 'Some', 'If', 'No', 'It', 'And', 'Or'].includes(n))
    return [{ kind: 'EQ', modality: 'Alethic', nouns: allNouns }]
  }

  return null
}

// ── Set-comparison block patterns ────────────────────────────────────

// Multi-word entity name: "Lead Message Match" or "LeadMessageMatch"
const ENTITY_NAME = '([A-Z][a-zA-Z0-9]*(?:\\s+[A-Z][a-zA-Z0-9]*)*)'

// "For each X, exactly one of the following holds:"
const XO_HEADER = new RegExp(
  `^For each ${ENTITY_NAME},\\s*exactly one of the following holds`,
  'i'
)

// "For each X, at most one of the following holds:"
const XC_HEADER = new RegExp(
  `^For each ${ENTITY_NAME},\\s*at most one of the following holds`,
  'i'
)

// "For each X, at least one of the following holds:"
const OR_HEADER = new RegExp(
  `^For each ${ENTITY_NAME},\\s*at least one of the following holds`,
  'i'
)

// "If some X ... then that X ..."
const SS_PATTERN = /^If some\b/i

/**
 * Parse a multi-line set-comparison constraint block.
 *
 * Handles XO ("exactly one of the following holds"),
 * XC ("at most one of the following holds"),
 * OR ("at least one of the following holds"),
 * and SS ("If some ... then that ... where ...").
 *
 * Returns null if the block doesn't match any pattern.
 */
export function parseSetComparisonBlock(blockText: string): ParsedConstraint | null {
  if (!blockText || !blockText.trim()) return null

  const trimmed = blockText.trim()
  const firstLine = trimmed.split('\n')[0].trim()

  // Try XO/XC/OR headers
  for (const [regex, kind] of [
    [XO_HEADER, 'XO'],
    [XC_HEADER, 'XC'],
    [OR_HEADER, 'OR'],
  ] as const) {
    const m = firstLine.match(regex)
    if (m) {
      const entity = m[1]
      const clauses = extractClauses(trimmed)
      const allNouns = new Set<string>()
      allNouns.add(toPascalCase(entity))
      for (const clause of clauses) {
        for (const noun of extractPascalNouns(clause)) {
          allNouns.add(noun)
        }
      }
      return {
        kind,
        modality: 'Alethic',
        nouns: [...allNouns],
        entity: toPascalCase(entity),
        clauses,
      }
    }
  }

  // Try SS pattern: "If some X ... then that X ..."
  if (SS_PATTERN.test(firstLine)) {
    const fullText = trimmed.replace(/\n/g, ' ')
    const allNouns = extractPascalNouns(fullText)
    return {
      kind: 'SS',
      modality: 'Alethic',
      nouns: allNouns,
    }
  }

  return null
}

/**
 * Extract semicolon-delimited clauses from a set-comparison block.
 * Clauses appear on indented lines after the header, each ending with ; or .
 */
function extractClauses(blockText: string): string[] {
  const lines = blockText.split('\n').slice(1) // skip header line
  const clauses: string[] = []

  // Join all remaining lines, split by semicolons
  const body = lines.map(l => l.trim()).join(' ')
  const parts = body.split(/;/)
  for (const part of parts) {
    const clean = part.trim().replace(/\.$/, '').trim()
    if (clean) clauses.push(clean)
  }
  return clauses
}

/** Extract PascalCase noun names from text. */
function extractPascalNouns(text: string): string[] {
  const nouns: string[] = []
  const words = text.split(/\s+/)
  let i = 0
  while (i < words.length) {
    const raw = words[i]
    // Skip quoted values like 'Pending' or "Confirmed"
    if (/^['"]/.test(raw) || /['"]$/.test(raw)) { i++; continue }
    const word = raw.replace(/[^a-zA-Z0-9]/g, '')
    if (/^[A-Z][a-zA-Z0-9]*$/.test(word)) {
      let name = word
      let j = i + 1
      while (j < words.length) {
        const nextRaw = words[j]
        if (/^['"]/.test(nextRaw) || /['"]$/.test(nextRaw)) break
        const next = nextRaw.replace(/[^a-zA-Z0-9]/g, '')
        if (/^[A-Z][a-zA-Z0-9]*$/.test(next)) {
          name += next
          j++
        } else {
          break
        }
      }
      nouns.push(name)
      i = j
    } else {
      i++
    }
  }
  return [...new Set(nouns)]
}

/** Convert multi-word entity name to PascalCase: "Lead Message Match" → "LeadMessageMatch" */
function toPascalCase(name: string): string {
  return name.split(/\s+/).map(w => w.charAt(0).toUpperCase() + w.slice(1)).join('')
}

// ── Informational pattern detection ──────────────────────────────────

const INFORMATIONAL_PATTERNS = [
  /^It is possible that\b/i,
  /^In each population of\b/i,
  /^This association with\b.*provides the preferred identification scheme/i,
  /^Data Type:/i,
  /^Reference Scheme:/i,
  /^Reference Mode:/i,
  /^Fact Types:\s*$/i,
  /^##\s/,
  /^\w+ is an entity type\.?$/i,
  /^\w+ is a value type\.?$/i,
]

/**
 * Check if text is an informational FORML2 pattern that should be
 * silently skipped (not treated as a reading or constraint).
 */
export function isInformationalPattern(text: string): boolean {
  if (!text || !text.trim()) return true
  const clean = text.trim()
  return INFORMATIONAL_PATTERNS.some(p => p.test(clean))
}

/**
 * Deterministic natural language constraint parser.
 *
 * Recognizes canonical FORML2 constraint patterns and returns structured
 * ParsedConstraint objects. Returns null for unrecognized text.
 *
 * Pure function — no DB or LLM dependency.
 */

export type ConstraintKind = 'UC' | 'MC' | 'RC' | 'SS' | 'XC' | 'EQ' | 'OR' | 'XO'

export interface ParsedConstraint {
  kind: ConstraintKind
  modality: 'Alethic' | 'Deontic'
  deonticOperator?: 'obligatory' | 'forbidden' | 'permitted'
  nouns: string[]
  /** For set-comparison (XO/XC/OR): the constrained entity name */
  entity?: string
  /** For set-comparison (XO/XC/OR): the individual clause texts */
  clauses?: string[]
}

// Match PascalCase noun names (e.g., "Customer", "SupportRequest", "APIKey")
const NOUN = '([A-Z][a-zA-Z0-9]*)'

// "Each X has/belongs to at most one Y."
const AT_MOST_ONE = new RegExp(
  `^Each ${NOUN} (?:has|belongs to|is [a-z]+ (?:by|to|in|for|of)) at most one ${NOUN}`,
  'i'
)

// "Each X has exactly one Y."
const EXACTLY_ONE = new RegExp(
  `^Each ${NOUN} (?:has|belongs to|is [a-z]+ (?:by|to|in|for|of)) exactly one ${NOUN}`,
  'i'
)

// "Each X has at least one Y."
const AT_LEAST_ONE = new RegExp(
  `^Each ${NOUN} (?:has|belongs to|is [a-z]+ (?:by|to|in|for|of)) at least one ${NOUN}`,
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

// "No X [verb] itself."
const RING_IRREFLEXIVE = new RegExp(
  `^No ${NOUN} [a-z]+ itself`,
  'i'
)

// "Each X has some Y." → MC (mandatory, alternative phrasing)
const HAS_SOME = new RegExp(
  `^Each ${NOUN} (?:has|belongs to) some ${NOUN}`,
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
  let m = clean.match(EXACTLY_ONE)
  if (m) {
    const nouns = [m[1], m[2]]
    return [
      { kind: 'UC', modality: 'Alethic', nouns },
      { kind: 'MC', modality: 'Alethic', nouns },
    ]
  }

  // "Each X has at most one Y" → UC
  m = clean.match(AT_MOST_ONE)
  if (m) {
    return [{ kind: 'UC', modality: 'Alethic', nouns: [m[1], m[2]] }]
  }

  // "Each X has at least one Y" → MC
  m = clean.match(AT_LEAST_ONE)
  if (m) {
    return [{ kind: 'MC', modality: 'Alethic', nouns: [m[1], m[2]] }]
  }

  // "For each combination of X and Y, ... at most one Z ..."
  m = clean.match(TERNARY_UC)
  if (m) {
    return [{ kind: 'UC', modality: 'Alethic', nouns: [m[1], m[2], m[3]] }]
  }

  // "For each pair of X and Y, ... at most once"
  m = clean.match(SPANNING_UC)
  if (m) {
    return [{ kind: 'UC', modality: 'Alethic', nouns: [m[1], m[2]] }]
  }

  // "No X [verb] itself"
  m = clean.match(RING_IRREFLEXIVE)
  if (m) {
    return [{ kind: 'RC', modality: 'Alethic', nouns: [m[1]] }]
  }

  // "Each X has some Y" → MC (alternative mandatory phrasing)
  m = clean.match(HAS_SOME)
  if (m) {
    return [{ kind: 'MC', modality: 'Alethic', nouns: [m[1], m[2]] }]
  }

  // "... if and only if ..." → EQ (equality/biconditional)
  if (IF_AND_ONLY_IF.test(clean)) {
    const allNouns = [...new Set(clean.match(/[A-Z][a-zA-Z0-9]*/g) || [])]
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

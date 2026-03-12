/**
 * Deterministic natural language constraint parser.
 *
 * Recognizes canonical FORML2 constraint patterns and returns structured
 * ParsedConstraint objects. Returns null for unrecognized text.
 *
 * Pure function — no DB or LLM dependency.
 */

export interface ParsedConstraint {
  kind: 'UC' | 'MC' | 'RC'
  modality: 'Alethic' | 'Deontic'
  deonticOperator?: 'obligatory' | 'forbidden' | 'permitted'
  nouns: string[]
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

// Deontic wrappers
const DEONTIC = /^It is (obligatory|forbidden|permitted) that (.+)$/i

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

  return null
}

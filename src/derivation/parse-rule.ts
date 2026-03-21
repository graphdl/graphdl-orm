/**
 * Derivation Rule Parser
 *
 * Parses ORM derivation rules of the form:
 *   Consequent := Antecedent1 and Antecedent2 and ...
 *
 * Produces structured IR (DerivationRule) from free-text rules
 * using known noun names for tokenization (longest-first, same
 * strategy as src/claims/tokenize.ts).
 */

export interface RuleTriple {
  subject: string
  predicate: string
  object: string
  qualifier?: { predicate: string; object: string }
  comparison?: { op: '>' | '<' | '>=' | '<=' | '=' | '!='; value: number }
  literalValue?: string
}

export interface DerivationRule {
  text: string
  consequent: RuleTriple
  antecedents: RuleTriple[]
  kind: 'join' | 'comparison' | 'aggregate' | 'identity'
  aggregate?: { fn: string; noun: string }
}

type ComparisonOp = '>' | '<' | '>=' | '<=' | '=' | '!='

// ─── Helpers ────────────────────────────────────────────────────────

/**
 * Find all known nouns in text, returning them in order of appearance.
 * Uses longest-first matching to avoid partial matches (same as tokenize.ts).
 */
function findNouns(text: string, nouns: string[]): Array<{ name: string; start: number; end: number }> {
  if (nouns.length === 0) return []

  // Sort longest first so "Graph Schema" matches before "Graph"
  const sorted = [...nouns].sort((a, b) => b.length - a.length)

  const pattern = sorted.map((n) => n.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')).join('|')
  const regex = new RegExp('\\b(' + pattern + ')\\b', 'g')

  const found: Array<{ name: string; start: number; end: number }> = []
  let match: RegExpExecArray | null
  while ((match = regex.exec(text)) !== null) {
    found.push({ name: match[1], start: match.index, end: match.index + match[0].length })
  }
  return found
}

/**
 * Mask known noun names in text with null characters, so that " and " inside
 * noun names (e.g. "Supply and Demand") is not treated as a conjunct separator.
 * Returns { masked, unmask } where unmask() restores original noun names.
 */
function maskNouns(text: string, nouns: string[]): { masked: string; unmask: (s: string) => string } {
  if (nouns.length === 0) return { masked: text, unmask: (s) => s }

  const sorted = [...nouns].sort((a, b) => b.length - a.length)

  // Build replacement map: noun → placeholder with null chars
  const replacements: Array<{ original: string; placeholder: string }> = []
  let masked = text

  for (const noun of sorted) {
    if (!masked.includes(noun)) continue
    // Replace spaces in the noun with \0 to protect from splitting
    const placeholder = noun.replace(/ /g, '\0')
    if (placeholder === noun) continue // no spaces, no masking needed
    replacements.push({ original: noun, placeholder })
    // Use word-boundary-aware replacement
    const re = new RegExp('\\b' + noun.replace(/[.*+?^${}()|[\]\\]/g, '\\$&') + '\\b', 'g')
    masked = masked.replace(re, placeholder)
  }

  const unmask = (s: string): string => {
    let result = s
    for (const { original, placeholder } of replacements) {
      result = result.split(placeholder).join(original)
    }
    return result
  }

  return { masked, unmask }
}

/**
 * Split RHS conjuncts on " and ", with noun-name masking to protect
 * noun names that contain "and".
 */
function splitConjuncts(rhs: string, nouns: string[]): string[] {
  const { masked, unmask } = maskNouns(rhs, nouns)
  const parts = masked.split(' and ')
  return parts.map((p) => unmask(p.trim()))
}

/**
 * Extract a comparison operator and value from the tail of a conjunct.
 * e.g. "Layer State has Valence > 0.3" → { op: '>', value: 0.3 } and cleaned text
 */
function extractComparison(text: string): { cleaned: string; comparison?: { op: ComparisonOp; value: number } } {
  const re = /\s*(>=|<=|!=|>|<|=)\s*(-?\d+(?:\.\d+)?)\s*$/
  const m = text.match(re)
  if (m) {
    return {
      cleaned: text.slice(0, m.index!).trim(),
      comparison: { op: m[1] as ComparisonOp, value: parseFloat(m[2]) },
    }
  }
  return { cleaned: text }
}

/**
 * Extract a literal value in single quotes from a conjunct.
 * e.g. "Layer State has Affect Region 'Excited'" → literalValue = 'Excited'
 */
function extractLiteral(text: string): { cleaned: string; literalValue?: string } {
  const re = /\s+'([^']+)'\s*$/
  const m = text.match(re)
  if (m) {
    return {
      cleaned: text.slice(0, m.index!).trim(),
      literalValue: m[1],
    }
  }
  return { cleaned: text }
}

/**
 * Parse a single clause (conjunct) into a RuleTriple.
 * Finds known nouns and extracts predicates between them.
 * Handles binary (2 nouns), ternary (3 nouns → qualifier), comparisons, and literals.
 */
function parseTriple(text: string, nouns: string[]): RuleTriple {
  // Strip leading "that " (used in identity rules)
  let cleaned = text.replace(/^that\s+/, '')

  // Extract literal and comparison
  const litResult = extractLiteral(cleaned)
  cleaned = litResult.cleaned
  const literalValue = litResult.literalValue

  const cmpResult = extractComparison(cleaned)
  cleaned = cmpResult.cleaned
  const comparison = cmpResult.comparison

  // Find nouns in cleaned text
  const found = findNouns(cleaned, nouns)

  if (found.length < 2) {
    // Fallback: return what we can
    return {
      subject: found.length > 0 ? found[0].name : '',
      predicate: cleaned,
      object: '',
      ...(literalValue !== undefined && { literalValue }),
      ...(comparison !== undefined && { comparison }),
    }
  }

  const subject = found[0].name
  const objectNoun = found.length >= 3 ? found[1] : found[found.length - 1]

  // Predicate: text between end of subject and start of object
  const subjectEnd = found[0].end
  const objectStart = objectNoun.start
  const predicate = cleaned.slice(subjectEnd, objectStart).trim()

  const triple: RuleTriple = {
    subject,
    predicate,
    object: objectNoun.name,
    ...(literalValue !== undefined && { literalValue }),
    ...(comparison !== undefined && { comparison }),
  }

  // Ternary: 3 nouns → qualifier (e.g. "Graph uses Resource for Role")
  if (found.length >= 3) {
    const qualifierNoun = found[found.length - 1]
    const qualifierPredicateStart = objectNoun.end
    const qualifierPredicateEnd = qualifierNoun.start
    const qualifierPredicate = cleaned.slice(qualifierPredicateStart, qualifierPredicateEnd).trim()
    triple.qualifier = {
      predicate: qualifierPredicate,
      object: qualifierNoun.name,
    }
  }

  return triple
}

// ─── Main Parser ────────────────────────────────────────────────────

export function parseRule(text: string, nouns: string[]): DerivationRule {
  // Strip trailing period
  const cleaned = text.replace(/\.\s*$/, '')

  // Split on :=
  const splitIdx = cleaned.indexOf(':=')
  if (splitIdx === -1) {
    throw new Error(`Derivation rule must contain ':=': ${text}`)
  }
  const lhs = cleaned.slice(0, splitIdx).trim()
  const rhs = cleaned.slice(splitIdx + 2).trim()

  // Parse consequent
  const consequent = parseTriple(lhs, nouns)

  // ── Detect rule kind ──

  // Identity: RHS contains "the same"
  const identityMatch = rhs.match(/\bthe same\b/)
  if (identityMatch) {
    const antecedent = parseIdentityTriple(rhs, nouns)
    return {
      text,
      consequent,
      antecedents: [antecedent],
      kind: 'identity',
    }
  }

  // Aggregate: RHS starts with aggregate function (count/sum/avg/min/max) of Noun where ...
  const aggregateMatch = rhs.match(/^(count|sum|avg|min|max)\s+of\s+(\w[\w\s]*?)\s+where\s+(.+)$/i)
  if (aggregateMatch) {
    const fn = aggregateMatch[1].toLowerCase()
    const aggNounText = aggregateMatch[2].trim()
    const whereClause = aggregateMatch[3].trim()

    // Resolve the aggregate noun against known nouns
    const aggNounMatches = findNouns(aggNounText, nouns)
    const aggNoun = aggNounMatches.length > 0 ? aggNounMatches[0].name : aggNounText

    // Parse where-clause conjuncts
    const conjuncts = splitConjuncts(whereClause, nouns)
    const antecedents = conjuncts.map((c) => parseTriple(c, nouns))

    return {
      text,
      consequent,
      antecedents,
      kind: 'aggregate',
      aggregate: { fn, noun: aggNoun },
    }
  }

  // Default: split RHS into conjuncts
  const conjuncts = splitConjuncts(rhs, nouns)
  const antecedents = conjuncts.map((c) => parseTriple(c, nouns))

  // Classify: if any antecedent has a comparison → 'comparison', else → 'join'
  const hasComparison = antecedents.some((a) => a.comparison !== undefined)
  const kind = hasComparison ? 'comparison' : 'join'

  return {
    text,
    consequent,
    antecedents,
    kind,
  }
}

/**
 * Parse an identity-form RHS like "that Domain is the same Domain".
 * The predicate is "is the same" and we find the noun on each side.
 */
function parseIdentityTriple(text: string, nouns: string[]): RuleTriple {
  // Strip leading "that "
  let cleaned = text.replace(/^that\s+/, '')

  // Find "is the same" and split around it
  const sameIdx = cleaned.indexOf('is the same')
  if (sameIdx === -1) {
    // Fallback: parse as normal triple
    return parseTriple(text, nouns)
  }

  const beforeSame = cleaned.slice(0, sameIdx).trim()
  const afterSame = cleaned.slice(sameIdx + 'is the same'.length).trim()

  // Find subject noun in beforeSame
  const subjectNouns = findNouns(beforeSame, nouns)
  const subject = subjectNouns.length > 0 ? subjectNouns[0].name : beforeSame

  // Find object noun in afterSame
  const objectNouns = findNouns(afterSame, nouns)
  const object = objectNouns.length > 0 ? objectNouns[0].name : afterSame

  return {
    subject,
    predicate: 'is the same',
    object,
  }
}

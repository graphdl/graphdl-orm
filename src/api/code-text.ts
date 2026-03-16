/**
 * codeText — replace parsed FORML2 lines with numbered shortcodes,
 * producing a coded document + legend for LLM residue extraction.
 *
 * The LLM receives:
 * 1. Legend: shortcode → parsed claim mapping (FORML2 document)
 * 2. Coded text: original document with parsed lines as shortcodes
 * 3. Instruction: extract only from non-coded lines
 *
 * Shortcode format:
 * - [N1] [N2] ... — noun declarations (entity/value types)
 * - [R1] [R2] ... — readings (fact types)
 * - [C1] [C2] ... — constraints
 * - [S1] [S2] ... — subtypes
 */

interface ParsedNoun { name: string; objectType: string; valueType?: string; enumValues?: string[]; plural?: string }
interface ParsedReading { text: string; nouns: string[]; predicate: string }
interface ParsedConstraint { kind: string; modality?: string; reading?: string; text?: string; roles?: number[] }
interface ParsedSubtype { child: string; parent: string }

interface ParseResult {
  nouns: ParsedNoun[]
  readings: ParsedReading[]
  constraints: ParsedConstraint[]
  subtypes: ParsedSubtype[]
  unparsed: string[]
  coverage: number
}

export interface CodedOutput {
  /** Original text with parsed lines replaced by shortcodes */
  coded: string
  /** Shortcode → claim legend as FORML2 */
  legend: string
  /** Residue lines only (no shortcodes, no parsed lines) */
  residue: string
  stats: { parsedLines: number; unparsedLines: number; totalLines: number }
}

export function codeText(original: string, parsed: ParseResult): CodedOutput {
  const lines = original.split('\n')

  // Build lookup sets from parsed content
  const parsedNounNames = new Set(parsed.nouns.map(n => n.name))
  const parsedReadingTexts = new Set<string>()
  for (const r of parsed.readings) {
    parsedReadingTexts.add(r.text)
    // Also match with/without trailing period
    if (r.text.endsWith('.')) parsedReadingTexts.add(r.text.slice(0, -1))
    else parsedReadingTexts.add(r.text + '.')
  }
  const parsedConstraintTexts = new Set<string>()
  for (const c of parsed.constraints) {
    if (c.text) {
      parsedConstraintTexts.add(c.text)
      if (c.text.endsWith('.')) parsedConstraintTexts.add(c.text.slice(0, -1))
      else parsedConstraintTexts.add(c.text + '.')
    }
  }
  const parsedSubtypeChildren = new Set(parsed.subtypes.map(s => s.child))

  // Assign shortcodes
  const nounCodes = new Map<string, string>()
  parsed.nouns.forEach((n, i) => nounCodes.set(n.name, `N${i + 1}`))
  const readingCodes = new Map<string, string>()
  parsed.readings.forEach((r, i) => readingCodes.set(r.text, `R${i + 1}`))
  const constraintCodes = new Map<string, string>()
  parsed.constraints.forEach((c, i) => { if (c.text) constraintCodes.set(c.text, `C${i + 1}`) })
  const subtypeCodes = new Map<string, string>()
  parsed.subtypes.forEach((s, i) => subtypeCodes.set(s.child, `S${i + 1}`))

  // Patterns
  const ENTITY_RE = /^(\w[\w\s]*?)(?:\(\.?\w+\))?\s+is an? entity type\.?$/
  const VALUE_RE = /^(\w[\w\s]*?)\s+is an? value type\.?$/
  const SUBTYPE_RE = /^(\w[\w\s]*?)\s+is a subtype of\s+(\w[\w\s]*?)\.?$/
  const SECTION_RE = /^##?\s+/
  const SUBSECTION_RE = /^###\s+/
  const ENUM_RE = /^\s+The possible values of/
  const EMPTY_RE = /^\s*$/

  const coded: string[] = []
  const residueLines: string[] = []
  let parsedCount = 0

  for (const line of lines) {
    const trimmed = line.trim()

    // Preserve structure
    if (EMPTY_RE.test(trimmed) || SECTION_RE.test(trimmed) || SUBSECTION_RE.test(trimmed)) {
      coded.push(line)
      continue
    }

    // Entity type → [N1]
    const entityMatch = trimmed.match(ENTITY_RE)
    if (entityMatch && parsedNounNames.has(entityMatch[1].trim())) {
      const code = nounCodes.get(entityMatch[1].trim())
      if (code) { coded.push(`[${code}]`); parsedCount++; continue }
    }

    // Value type → [N2]
    const valueMatch = trimmed.match(VALUE_RE)
    if (valueMatch && parsedNounNames.has(valueMatch[1].trim())) {
      const code = nounCodes.get(valueMatch[1].trim())
      if (code) { coded.push(`[${code}]`); parsedCount++; continue }
    }

    // Enum line → skip (captured by noun)
    if (ENUM_RE.test(trimmed)) { parsedCount++; continue }

    // Subtype → [S1]
    const subtypeMatch = trimmed.match(SUBTYPE_RE)
    if (subtypeMatch && parsedSubtypeChildren.has(subtypeMatch[1].trim())) {
      const code = subtypeCodes.get(subtypeMatch[1].trim())
      if (code) { coded.push(`[${code}]`); parsedCount++; continue }
    }

    // Reading → [R1]
    if (parsedReadingTexts.has(trimmed)) {
      const text = trimmed.endsWith('.') ? trimmed.slice(0, -1) : trimmed
      const code = readingCodes.get(text) || readingCodes.get(trimmed)
      if (code) { coded.push(`[${code}]`); parsedCount++; continue }
    }

    // Constraint → [C1]
    if (parsedConstraintTexts.has(trimmed)) {
      const text = trimmed.endsWith('.') ? trimmed.slice(0, -1) : trimmed
      const code = constraintCodes.get(text) || constraintCodes.get(trimmed)
      if (code) { coded.push(`[${code}]`); parsedCount++; continue }
    }

    // Unparsed — preserve in coded text AND collect as residue
    coded.push(line)
    if (trimmed) residueLines.push(trimmed)
  }

  // Build legend
  const legendLines: string[] = ['## Legend', '']
  for (const [name, code] of nounCodes) {
    const noun = parsed.nouns.find(n => n.name === name)!
    const detail = noun.objectType === 'value'
      ? `${name} is a value type.${noun.enumValues?.length ? ` Values: ${noun.enumValues.join(', ')}` : ''}`
      : `${name} is an entity type.`
    legendLines.push(`[${code}] ${detail}`)
  }
  for (const [child, code] of subtypeCodes) {
    const parent = parsed.subtypes.find(s => s.child === child)!.parent
    legendLines.push(`[${code}] ${child} is a subtype of ${parent}.`)
  }
  for (const [text, code] of readingCodes) {
    legendLines.push(`[${code}] ${text}`)
  }
  for (const [text, code] of constraintCodes) {
    legendLines.push(`[${code}] ${text}`)
  }

  const totalLines = lines.filter(l => {
    const t = l.trim()
    return t && !SECTION_RE.test(t) && !SUBSECTION_RE.test(t) && !EMPTY_RE.test(t)
  }).length

  return {
    coded: coded.join('\n'),
    legend: legendLines.join('\n'),
    residue: residueLines.join('\n'),
    stats: {
      parsedLines: parsedCount,
      unparsedLines: residueLines.length,
      totalLines,
    },
  }
}

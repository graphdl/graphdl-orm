import { json, error } from 'itty-router'
import type { Env } from '../types'
import type { ExtractedClaims } from '../claims/ingest'
import { tokenizeReading } from '../claims/tokenize'
import { parseConstraintText, parseSetComparisonBlock } from '../hooks/parse-constraint'

interface ParseResult extends ExtractedClaims {
  warnings: string[]
  /** Lines that were not parsed — candidates for LLM semantic extraction */
  unparsed: string[]
  /** Coverage: ratio of parsed lines to total non-empty, non-comment lines */
  coverage: number
}

type Section = 'entity-types' | 'value-types' | 'subtypes' | 'fact-types' | 'constraints'
  | 'mandatory-constraints' | 'deontic-constraints' | 'derivation-rules' | 'instance-facts' | 'unknown'

// ── Section header detection ──────────────────────────────────────────

const SECTION_MAP: Array<[RegExp, Section]> = [
  [/^##\s*Entity\s*Types?/i, 'entity-types'],
  [/^##\s*Value\s*Types?/i, 'value-types'],
  [/^##\s*Subtypes?/i, 'subtypes'],
  [/^##\s*Fact\s*Types?/i, 'fact-types'],
  [/^##\s*(?:Mandatory\s+)?Constraints?/i, 'constraints'],
  [/^##\s*Deontic\s*Constraints?/i, 'deontic-constraints'],
  [/^##\s*Derivation\s*Rules?/i, 'derivation-rules'],
  [/^##\s*Instance\s*Facts?/i, 'instance-facts'],
]

function detectSection(line: string): Section | null {
  for (const [pattern, section] of SECTION_MAP) {
    if (pattern.test(line)) return section
  }
  return null
}

// ── Line-level patterns ───────────────────────────────────────────────

// Entity type: "EntityName(.RefScheme) is an entity type."
const ENTITY_TYPE = /^([A-Z][a-zA-Z0-9]*)(?:\(\.([A-Z][a-zA-Z0-9]*)\))?\s+is an entity type\.?$/i

// Value type: "ValueName is a value type."
const VALUE_TYPE = /^([A-Z][a-zA-Z0-9]*)\s+is a value type\.?$/i

// Enum values: "The possible values of X are 'a', 'b', 'c'."
const ENUM_VALUES = /^The possible values of ([A-Z][a-zA-Z0-9]*) are (.+)\.?$/i

// Subtype: "Child is a subtype of Parent."
const SUBTYPE = /^([A-Z][a-zA-Z0-9]*)\s+is a subtype of\s+([A-Z][a-zA-Z0-9]*)\.?$/i

// Partition: "Parent is partitioned into Child1, Child2."
const PARTITION = /^([A-Z][a-zA-Z0-9]*)\s+is partitioned into\s+(.+)\.?$/i

// Subtype exclusion: "No Parent is both a Child1 and a Child2."
const SUBTYPE_EXCLUSION = /^No ([A-Z][a-zA-Z0-9]*) is both (?:a |an )?([A-Z][a-zA-Z0-9]*) and (?:a |an )?([A-Z][a-zA-Z0-9]*)\.?$/i

// Subtype totality: "Each Parent is a Child1 or a Child2."
const SUBTYPE_TOTALITY = /^Each ([A-Z][a-zA-Z0-9]*) is (?:a |an )?([A-Z][a-zA-Z0-9]*) or (?:a |an )?([A-Z][a-zA-Z0-9]*)\.?$/i

// Derivation rule: "X has Y := expression."
const DERIVATION = /^(.+?)\s*:=\s*(.+?)\.?$/

// Instance fact: EntityName 'value' has ValueType 'value'.
const INSTANCE_FACT = /^([A-Z][a-zA-Z0-9]*)\s+'([^']+)'\s+has\s+([A-Z][a-zA-Z0-9]*)\s+'([^']+)'\.?$/

// Instance fact (verb): Verb 'name' runs Function 'name'. / Function 'name' has FunctionType 'type'.
const INSTANCE_FACT_VERB = /^([A-Z][a-zA-Z0-9]*)\s+'([^']+)'\s+(\w+(?:\s+\w+)*)\s+([A-Z][a-zA-Z0-9]*)\s+'([^']+)'\.?$/

// Instance fact (simple assignment): EntityName 'name' has ValueType 'value'.
const INSTANCE_FACT_SIMPLE = /^([A-Z][a-zA-Z0-9]*)\s+(?:has\s+)?([A-Z][a-zA-Z0-9]*)\s+'([^']+)'\.?$/

// Markdown subsection header (### EntityName) — sets fact type grouping context
const SUBSECTION = /^###\s+(.+)/

// Comment or description line (starts with lowercase or is a markdown heading)
const COMMENT_LINE = /^(?:#\s|[a-z]|$|\[)/

// Skip patterns — lines that are structural but not claims
const SKIP_PATTERNS = [
  /^#\s/,           // Top-level markdown heading
  /^---/,           // Horizontal rule
  /^\s*$/,          // Empty line
]

/**
 * Pure-function FORML2 parser.
 *
 * Parses multi-line FORML2 text into structured ExtractedClaims.
 * Tracks section context from ## headers to correctly classify lines.
 * Returns unparsed lines for optional LLM semantic extraction.
 *
 * No DB writes, no hooks — read-only.
 */
export function parseFORML2(
  text: string,
  existingNouns: Array<{ name: string; id: string; objectType?: 'entity' | 'value' }>,
): ParseResult {
  const warnings: string[] = []
  const unparsed: string[] = []
  const nounMap = new Map<string, { name: string; objectType: 'entity' | 'value'; plural?: string; valueType?: string; enumValues?: string[] }>()
  const readings: ParseResult['readings'] = []
  const constraints: ParseResult['constraints'] = []
  const subtypes: ParseResult['subtypes'] = []
  const facts: ParseResult['facts'] = []

  // Initialize nounMap with existing nouns
  for (const n of existingNouns) {
    if (!nounMap.has(n.name)) {
      nounMap.set(n.name, { name: n.name, objectType: n.objectType || 'entity' })
    }
  }

  let section: Section = 'unknown'
  let totalLines = 0
  let parsedLines = 0

  const lines = text.split('\n')

  for (let i = 0; i < lines.length; i++) {
    const raw = lines[i]
    const line = raw.trim()

    // Skip empty and structural lines
    if (SKIP_PATTERNS.some(p => p.test(line))) continue
    if (!line) continue

    // Track section headers
    const newSection = detectSection(line)
    if (newSection) {
      section = newSection
      parsedLines++
      continue
    }

    // Subsection headers (### EntityName) — just context, not a claim
    if (SUBSECTION.test(line)) {
      parsedLines++
      continue
    }

    // Skip description lines (lowercase start, markdown links)
    if (COMMENT_LINE.test(line)) continue

    totalLines++

    // ── Entity type declaration ──────────────────────────────────────
    let m = line.replace(/\.$/, '').match(ENTITY_TYPE)
    if (m) {
      const name = m[1]
      const refScheme = m[2]
      nounMap.set(name, { name, objectType: 'entity' })
      if (refScheme && !nounMap.has(refScheme)) {
        nounMap.set(refScheme, { name: refScheme, objectType: 'value', valueType: 'string' })
      }
      parsedLines++
      continue
    }

    // ── Value type declaration ────────────────────────────────────────
    m = line.replace(/\.$/, '').match(VALUE_TYPE)
    if (m) {
      const name = m[1]
      if (!nounMap.has(name)) {
        nounMap.set(name, { name, objectType: 'value', valueType: 'string' })
      } else {
        // Update existing to value type
        const existing = nounMap.get(name)!
        existing.objectType = 'value'
      }
      parsedLines++
      continue
    }

    // ── Enum values ──────────────────────────────────────────────────
    m = line.replace(/\.$/, '').match(ENUM_VALUES)
    if (m) {
      const name = m[1]
      const valuesStr = m[2]
      const values = valuesStr.match(/'([^']+)'/g)?.map(v => v.replace(/'/g, '')) || []
      const existing = nounMap.get(name)
      if (existing) {
        existing.enumValues = values
      } else {
        nounMap.set(name, { name, objectType: 'value', valueType: 'string', enumValues: values })
      }
      parsedLines++
      continue
    }

    // ── Subtype declaration ──────────────────────────────────────────
    m = line.replace(/\.$/, '').match(SUBTYPE)
    if (m) {
      subtypes.push({ child: m[1], parent: m[2] })
      if (!nounMap.has(m[1])) nounMap.set(m[1], { name: m[1], objectType: 'entity' })
      if (!nounMap.has(m[2])) nounMap.set(m[2], { name: m[2], objectType: 'entity' })
      parsedLines++
      continue
    }

    // ── Partition declaration ────────────────────────────────────────
    m = line.replace(/\.$/, '').match(PARTITION)
    if (m) {
      const parent = m[1]
      const children = m[2].split(/,\s*/).map(c => c.trim()).filter(Boolean)
      for (const child of children) {
        subtypes.push({ child, parent })
        if (!nounMap.has(child)) nounMap.set(child, { name: child, objectType: 'entity' })
      }
      if (!nounMap.has(parent)) nounMap.set(parent, { name: parent, objectType: 'entity' })
      parsedLines++
      continue
    }

    // ── Subtype exclusion ────────────────────────────────────────────
    m = line.replace(/\.$/, '').match(SUBTYPE_EXCLUSION)
    if (m) {
      constraints.push({ kind: 'XC', modality: 'Alethic', reading: '', roles: [], text: line, entity: m[1], clauses: [m[2], m[3]] })
      parsedLines++
      continue
    }

    // ── Subtype totality ─────────────────────────────────────────────
    m = line.replace(/\.$/, '').match(SUBTYPE_TOTALITY)
    if (m) {
      constraints.push({ kind: 'OR', modality: 'Alethic', reading: '', roles: [], text: line, entity: m[1], clauses: [m[2], m[3]] })
      parsedLines++
      continue
    }

    // ── Derivation rule ──────────────────────────────────────────────
    m = line.match(DERIVATION)
    if (m) {
      // Store as a reading with a derivation marker
      readings.push({ text: line, nouns: [], predicate: ':=', derivation: m[2].trim() })
      parsedLines++
      continue
    }

    // ── Instance facts ───────────────────────────────────────────────
    m = line.replace(/\.$/, '').match(INSTANCE_FACT)
    if (m) {
      facts.push({ entity: m[1], entityValue: m[2], valueType: m[3], value: m[4] })
      parsedLines++
      continue
    }
    m = line.replace(/\.$/, '').match(INSTANCE_FACT_VERB)
    if (m) {
      facts.push({ entity: m[1], entityValue: m[2], predicate: m[3], valueType: m[4], value: m[5] })
      parsedLines++
      continue
    }
    m = line.replace(/\.$/, '').match(INSTANCE_FACT_SIMPLE)
    if (m) {
      facts.push({ entity: m[1], valueType: m[2], value: m[3] })
      parsedLines++
      continue
    }

    // ── Set-comparison block ─────────────────────────────────────────
    // Look ahead for multi-line block
    const blockEnd = findBlockEnd(lines, i)
    if (blockEnd > i) {
      const block = lines.slice(i, blockEnd + 1).join('\n')
      const scBlock = parseSetComparisonBlock(block)
      if (scBlock) {
        for (const name of scBlock.nouns) {
          if (!nounMap.has(name)) nounMap.set(name, { name, objectType: 'entity' })
        }
        constraints.push({
          kind: scBlock.kind,
          modality: scBlock.modality,
          reading: '',
          roles: [],
          text: block.trim(),
          clauses: scBlock.clauses,
          entity: scBlock.entity,
        })
        i = blockEnd // skip the block lines
        parsedLines++
        continue
      }
    }

    // ── Standalone constraint (in ## Constraints or ## Deontic Constraints section) ──
    if (section === 'constraints' || section === 'mandatory-constraints' || section === 'deontic-constraints') {
      const parsed = parseConstraintText(line.replace(/\.$/, ''))
      if (parsed) {
        for (const pc of parsed) {
          // For standalone constraints, try to match nouns to existing readings
          const matchedReading = findMatchingReading(readings, pc.nouns)
          constraints.push({
            kind: pc.kind,
            modality: section === 'deontic-constraints' ? 'Deontic' : pc.modality,
            deonticOperator: pc.deonticOperator,
            reading: matchedReading || '',
            roles: matchedReading ? resolveRoles(matchedReading, pc.nouns, readings) : [],
            text: line,
          })
        }
        parsedLines++
        continue
      }

      // Deontic constraints that don't match standard UC/MC patterns —
      // resolve the inner clause to an existing reading
      if (section === 'deontic-constraints' || /^It is (forbidden|obligatory|permitted) that\b/i.test(line)) {
        const deonticMatch = line.match(/^It is (forbidden|obligatory|permitted) that (.+?)\.?$/i)
        if (deonticMatch) {
          const operator = deonticMatch[1].toLowerCase() as 'forbidden' | 'obligatory' | 'permitted'
          const innerClause = deonticMatch[2].replace(/^each\s+/i, '').trim()

          // Try to match inner clause to an existing reading
          const matchedReading = readings.find(r => {
            // Exact match
            if (r.text === innerClause) return true
            // Check if reading text appears within the inner clause
            if (innerClause.includes(r.text)) return true
            // Check if the inner clause nouns match a reading's nouns
            const clauseNouns = (innerClause.match(/[A-Z][a-zA-Z0-9]*/g) || [])
            return clauseNouns.length >= 2 && clauseNouns.every(n => r.nouns.includes(n))
          })

          constraints.push({
            kind: 'UC',
            modality: 'Deontic',
            deonticOperator: operator,
            reading: matchedReading?.text || '',
            roles: matchedReading ? matchedReading.nouns.map((_, idx) => idx) : [],
            text: line,
          })
          parsedLines++
          continue
        }
      }
    }

    // ── Reading (fact type) ──────────────────────────────────────────
    // Try to parse as a reading — this is the catch-all for fact type lines
    const cleanLine = line.replace(/\.$/, '')

    // Build current noun list for tokenization
    const currentNouns = [
      ...existingNouns,
      ...[...nounMap.values()]
        .filter(n => !existingNouns.some(e => e.name === n.name))
        .map(n => ({ name: n.name, id: '' })),
    ]

    const tokenized = tokenizeReading(cleanLine, currentNouns)
    let nounNames = tokenized.nounRefs.map(r => r.name)

    // PascalCase fallback if tokenization found fewer than 2 nouns
    if (nounNames.length < 2) {
      const pascalWords = cleanLine.match(/[A-Z][a-zA-Z0-9]*/g) || []
      if (pascalWords.length >= 2) nounNames = pascalWords
    }

    // Unary fact (1 noun)
    if (nounNames.length === 1) {
      const predicate = tokenized.predicate || cleanLine.replace(nounNames[0], '').trim()
      if (!nounMap.has(nounNames[0])) nounMap.set(nounNames[0], { name: nounNames[0], objectType: 'entity' })
      readings.push({ text: cleanLine, nouns: nounNames, predicate })
      parsedLines++
      continue
    }

    // Binary+ fact (2+ nouns)
    if (nounNames.length >= 2) {
      const predicate = tokenized.predicate || extractPredicate(cleanLine, nounNames)
      const isHasPredicate = /^has$/i.test(predicate.trim())

      for (let j = 0; j < nounNames.length; j++) {
        const name = nounNames[j]
        if (!nounMap.has(name)) {
          const objectType = (isHasPredicate && j === nounNames.length - 1) ? 'value' as const : 'entity' as const
          nounMap.set(name, { name, objectType })
        }
      }

      readings.push({ text: cleanLine, nouns: nounNames, predicate })

      // Parse indented constraint lines following this reading
      while (i + 1 < lines.length && /^\s+\S/.test(lines[i + 1])) {
        i++
        const constraintLine = lines[i].trim().replace(/\.$/, '')
        const parsed = parseConstraintText(constraintLine)
        if (parsed) {
          for (const pc of parsed) {
            const constraintNouns = (pc.kind === 'UC' || pc.kind === 'MC') && pc.nouns.length > 0
              ? [pc.nouns[0]] : pc.nouns
            const roles = constraintNouns
              .map(cn => nounNames.indexOf(cn))
              .filter(idx => idx !== -1)
            constraints.push({
              kind: pc.kind,
              modality: pc.modality,
              deonticOperator: pc.deonticOperator,
              reading: cleanLine,
              roles,
            })
          }
          parsedLines++
        } else {
          warnings.push(`Unrecognized constraint: "${constraintLine}"`)
        }
      }

      parsedLines++
      continue
    }

    // ── Unparsed line ────────────────────────────────────────────────
    unparsed.push(line)
  }

  // Build nouns array from map
  const nouns = [...nounMap.values()]
  const coverage = totalLines > 0 ? parsedLines / totalLines : 1

  return {
    nouns,
    readings,
    constraints,
    subtypes,
    transitions: [],
    facts,
    warnings,
    unparsed,
    coverage,
  }
}

// ── Helpers ──────────────────────────────────────────────────────────

/** Extract predicate between first two nouns. */
function extractPredicate(text: string, nounNames: string[]): string {
  if (nounNames.length < 2) return ''
  const first = text.indexOf(nounNames[0])
  if (first === -1) return ''
  const afterFirst = first + nounNames[0].length
  const second = text.indexOf(nounNames[1], afterFirst)
  if (second === -1) return ''
  return text.slice(afterFirst, second).trim()
}

/** Find the end of a multi-line block (indented continuation lines). */
function findBlockEnd(lines: string[], start: number): number {
  let end = start
  for (let i = start + 1; i < lines.length; i++) {
    const line = lines[i]
    if (/^\s+\S/.test(line) || /^[-•]\s/.test(line.trim())) {
      end = i
    } else if (line.trim() === '') {
      break
    } else {
      break
    }
  }
  return end
}

/** Find a reading whose nouns match the constraint's nouns. */
function findMatchingReading(
  readings: ParseResult['readings'],
  constraintNouns: string[],
): string {
  for (const r of readings) {
    if (constraintNouns.every(cn => r.nouns.includes(cn))) {
      return r.text
    }
  }
  return ''
}

/** Resolve role indices for constraint nouns within a matched reading. */
function resolveRoles(
  readingText: string,
  constraintNouns: string[],
  readings: ParseResult['readings'],
): number[] {
  const reading = readings.find(r => r.text === readingText)
  if (!reading) return []
  return constraintNouns
    .map(cn => reading.nouns.indexOf(cn))
    .filter(idx => idx !== -1)
}

// ── HTTP Handler ───────────────────────────────────────────────────

function getDB(env: Env): DurableObjectStub {
  const id = env.GRAPHDL_DB.idFromName('graphdl-primary')
  return env.GRAPHDL_DB.get(id)
}

export async function handleParse(request: Request, env: Env): Promise<Response> {
  if (request.method !== 'POST') {
    return error(405, { errors: [{ message: 'Method not allowed' }] })
  }

  const body = await request.json() as { text?: string; domain?: string }
  if (!body.text) {
    return error(400, { errors: [{ message: 'text is required' }] })
  }
  if (!body.domain) {
    return error(400, { errors: [{ message: 'domain is required' }] })
  }

  // Load existing nouns for tokenization context (read-only)
  const db = getDB(env) as any
  const existingNouns = await db.findInCollection('nouns', {
    domain_id: { equals: body.domain },
  }, { limit: 10000 })
  const nouns = existingNouns.docs.map((n: any) => ({ name: n.name, id: n.id, objectType: n.objectType }))

  const result = parseFORML2(body.text, nouns)

  // Generate coded text + legend for hybrid LLM pipeline
  const { codeText } = await import('./code-text')
  const { coded, legend, residue, stats } = codeText(body.text, result)

  return json({ ...result, coded, legend, residue, codeStats: stats })
}

import { json, error } from 'itty-router'
import type { Env } from '../types'
import type { ExtractedClaims } from '../claims/ingest'
import { tokenizeReading } from '../claims/tokenize'
import { parseConstraintText, parseSetComparisonBlock } from '../hooks/parse-constraint'
import { parseRule } from '../derivation/parse-rule'

interface ParseResult extends ExtractedClaims {
  warnings: string[]
  /** Lines that were not parsed — candidates for LLM semantic extraction */
  unparsed: string[]
  /** Coverage: ratio of parsed lines to total non-empty, non-comment lines */
  coverage: number
}

type Section = 'entity-types' | 'value-types' | 'subtypes' | 'fact-types' | 'constraints'
  | 'mandatory-constraints' | 'deontic-constraints' | 'derivation-rules' | 'instance-facts'
  | 'states' | 'transitions' | 'unknown'

// ── Section header detection ──────────────────────────────────────────

const SECTION_MAP: Array<[RegExp, Section]> = [
  [/^##\s*Entity\s*Types?/i, 'entity-types'],
  [/^##\s*Value\s*Types?/i, 'value-types'],
  [/^##\s*Subtypes?/i, 'subtypes'],
  [/^##\s*Fact\s*Types?/i, 'fact-types'],
  [/^##\s*(?:(?:Mandatory|Ring|Subset|Disjunctive(?:\s+Mandatory)?|Frequency|Value|Equality|Exclusion|Set\s*Comparison)\s+)?Constraints?/i, 'constraints'],
  [/^##\s*Deontic\s*Constraints?/i, 'deontic-constraints'],
  [/^##\s*Derivation\s*Rules?/i, 'derivation-rules'],
  [/^##\s*Instance\s*Facts?/i, 'instance-facts'],
  [/^##\s*States?$/i, 'states'],
  [/^##\s*Transitions?$/i, 'transitions'],
]

function detectSection(line: string): Section | null {
  for (const [pattern, section] of SECTION_MAP) {
    if (pattern.test(line)) return section
  }
  return null
}

// ── Line-level patterns ───────────────────────────────────────────────
// Noun name pattern: one or more PascalCase words separated by spaces.
// Used ONLY for structured declarations (entity/value type, subtype, instance fact).
// Fact-type readings resolve nouns against the declared nounMap — no PascalCase guessing.
const N = '(?:[A-Z][a-zA-Z0-9]*(?:\\s+[A-Z][a-zA-Z0-9]*)*)'

// Entity type: "Support Request(.Request Id) is an entity type."
// Also handles compound ref schemes: "Layer State(.Layer, .Timestamp) is an entity type."
// And "within" syntax: "Status(.Name within State Machine Definition) is an entity type."
const ENTITY_TYPE = new RegExp(`^(${N})(?:\\(([^)]+)\\))?\\s+is an entity type\\.?$`, 'i')

// Value type: "Request Id is a value type."
const VALUE_TYPE = new RegExp(`^(${N})\\s+is a value type\\.?$`, 'i')

// Enum values: "The possible values of Plan Tier are 'a', 'b', 'c'."
const ENUM_VALUES = new RegExp(`^The possible values of (${N}) are (.+)\\.?$`, 'i')

// Subtype: "Support Request is a subtype of Request."
const SUBTYPE = new RegExp(`^(${N})\\s+is a subtype of\\s+(${N})\\.?$`, 'i')

// Partition: "Parent is partitioned into Child1, Child2."
const PARTITION = new RegExp(`^(${N})\\s+is partitioned into\\s+(.+)\\.?$`, 'i')

// Subtype exclusion: "No Request is both a Support Request and a Feature Request."
const SUBTYPE_EXCLUSION = new RegExp(`^No (${N}) is both (?:a |an )?(${N}) and (?:a |an )?(${N})\\.?$`, 'i')

// Subtype totality: "Each Person is a Male or a Female."
const SUBTYPE_TOTALITY = new RegExp(`^Each (${N}) is (?:a |an )?(${N}) or (?:a |an )?(${N})\\.?$`, 'i')

// Derivation rule: "X has Y := expression."
const DERIVATION = /^(.+?)\s*:=\s*(.+?)\.?$/

// Instance fact: Entity 'value' has Value Type 'value'.
const INSTANCE_FACT = new RegExp(`^(${N})\\s+'([^']+)'\\s+has\\s+(${N})\\s+'([^']+)'\\.?$`)

// Instance fact (verb): Verb 'name' runs Function 'name'.
const INSTANCE_FACT_VERB = new RegExp(`^(${N})\\s+'([^']+)'\\s+(\\w+(?:\\s+\\w+)*)\\s+(${N})\\s+'([^']+)'\\.?$`)

// Instance fact (simple assignment): Entity has Value Type 'value'. / Entity 'ref' has Value Type number.
const INSTANCE_FACT_SIMPLE = new RegExp(`^(${N})\\s+(?:has\\s+)?(${N})\\s+'([^']+)'\\.?$`)

// Markdown subsection header (### EntityName) — sets fact type grouping context
const SUBSECTION = /^###\s+(.+)/

// Comment or description line — not a FORML2 claim
const COMMENT_LINE = /^(?:#\s|[a-z]|$|\[|Note:\s)/

// Lines that look like documentation comments, not FORML2 readings
const DESCRIPTION_LINE = /^Cross-domain references:/

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
  const nounMap = new Map<string, { name: string; objectType: 'entity' | 'value'; plural?: string; valueType?: string; enumValues?: string[]; refScheme?: string[] }>()
  const readings: ParseResult['readings'] = []
  const constraints: ParseResult['constraints'] = []
  const subtypes: ParseResult['subtypes'] = []
  const transitions: Array<{ entity: string; from: string; to: string; event: string }> = []
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

    // Skip unrecognized ## headers that didn't match any section
    if (/^##\s/.test(line)) {
      parsedLines++
      continue
    }

    // Subsection headers (### EntityName) — just context, not a claim
    if (SUBSECTION.test(line)) {
      parsedLines++
      continue
    }

    // Skip comment lines (lowercase start, markdown links)
    if (COMMENT_LINE.test(line)) continue

    totalLines++

    // ── State machine states (comma-separated list) ──────────────────
    if (section === 'states') {
      // Parse "Triaging, Waiting On Customer, Resolved"
      const stateNames = line.split(',').map(s => s.trim()).filter(Boolean)
      if (stateNames.length >= 2) {
        // Derive entity name from the document title (# Support Request Lifecycle → Support Request)
        const titleMatch = text.match(/^#\s+(.+?)(?:\s+Lifecycle)?$/m)
        const entityName = titleMatch ? titleMatch[1].trim() : ''
        for (const state of stateNames) {
          if (!nounMap.has(state)) nounMap.set(state, { name: state, objectType: 'entity' })
        }
        parsedLines++
        continue
      }
    }

    // ── State machine transitions (markdown table) ───────────────────
    if (section === 'transitions') {
      // Skip table header and separator rows
      if (line.startsWith('|') && (line.includes('From') || line.includes('---'))) {
        parsedLines++
        continue
      }
      // Parse "| Triaging | Resolved | resolve |"
      if (line.startsWith('|')) {
        const cells = line.split('|').map(c => c.trim()).filter(Boolean)
        if (cells.length >= 3) {
          const titleMatch = text.match(/^#\s+(.+?)(?:\s+Lifecycle)?$/m)
          const entityName = titleMatch ? titleMatch[1].trim() : ''
          transitions.push({
            entity: entityName,
            from: cells[0],
            to: cells[1],
            event: cells[2],
          })
          parsedLines++
          continue
        }
      }
    }

    // ── Entity type declaration ──────────────────────────────────────
    let m = line.replace(/\.$/, '').match(ENTITY_TYPE)
    if (m) {
      const name = m[1]
      const refSchemeRaw = m[2]
      const nounEntry: Record<string, any> = { name, objectType: 'entity' }
      if (refSchemeRaw) {
        // Parse compound ref schemes: ".Layer, .Timestamp" — comma-separated value type IDs
        const parts = refSchemeRaw.split(/,/).map(p => p.trim().replace(/^\./, '').trim()).filter(Boolean)
        nounEntry.refScheme = parts
        for (const part of parts) {
          if (part && !nounMap.has(part)) {
            nounMap.set(part, { name: part, objectType: 'value', valueType: 'string' })
          }
        }
      }
      nounMap.set(name, nounEntry)
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

    // ── Subtype annotations (totality, exclusion, non-totality) ──────
    // "Not every X is a Y" — non-totality
    // "Each X is a Y, a Z, or a W" — totality (partition enumeration)
    // "No X belongs to more than one of these subtypes" — exclusion
    if (/^Not every\b/i.test(line) ||
        /^Each .+ is (?:a |an ).+(?:,|or )/i.test(line) ||
        /^No .+ belongs to more than one/i.test(line)) {
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
      // Parse into structured IR and store alongside text
      const nounNames = [...nounMap.keys()]
      const ruleIR = parseRule(line, nounNames)
      const ruleNouns = [ruleIR.consequent.subject, ruleIR.consequent.object].filter(Boolean)
      readings.push({ text: line, nouns: ruleNouns, predicate: ':=', derivation: m[2].trim(), ruleIR })
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
          // Match constraint nouns to the primary reading
          // Graph schema role order = primary reading noun order
          const matchedReading = findMatchingReading(readings, pc.nouns)
          const roles = matchedReading
            ? pc.constrainedNoun
              ? resolveConstrainedRole(matchedReading, pc.constrainedNoun, readings)
              : resolveRoles(matchedReading, pc.nouns, readings) // spanning UC: all roles
            : []
          constraints.push({
            kind: pc.kind,
            modality: section === 'deontic-constraints' ? 'Deontic' : pc.modality,
            deonticOperator: pc.deonticOperator,
            reading: matchedReading || '',
            roles,
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

      // Ring constraints ("No X verb itself") and conditional constraints ("If X1 verb X2, then ...")
      // that weren't matched by parseConstraintText — store as generic constraints
      if (/^No\s/i.test(line) || /^If\s/i.test(line)) {
        constraints.push({
          kind: 'IR',
          modality: section === 'deontic-constraints' ? 'Deontic' : 'Alethic',
          reading: '',
          roles: [],
          text: line,
        })
        parsedLines++
        continue
      }

      // Any remaining line in a constraints section should NOT fall through to readings.
      // Store as a generic constraint with the verbalization text preserved.
      if (/^Each\s/i.test(line) || /^For each\s/i.test(line)) {
        // Attempt to infer kind from text pattern
        const kind = /exactly one/i.test(line) ? 'UC' as const
          : /at most one/i.test(line) ? 'UC' as const
          : /at least one/i.test(line) ? 'MC' as const
          : /some/i.test(line) ? 'MC' as const
          : 'UC' as const
        constraints.push({
          kind,
          modality: section === 'deontic-constraints' ? 'Deontic' : 'Alethic',
          reading: '',
          roles: [],
          text: line,
        })
        parsedLines++
        continue
      }
    }

    // Skip prose descriptions and cross-domain reference comments.
    // Checked after structured patterns so "X is an entity type" is not falsely skipped.
    if (DESCRIPTION_LINE.test(line)) continue

    // Lines in instance-facts section that didn't match any instance pattern
    // should NOT fall through to reading parsing — they're data, not schema
    if (section === 'instance-facts') {
      unparsed.push(line)
      continue
    }

    // ── Reading (fact type) ──────────────────────────────────────────
    // Try to parse as a reading — this is the catch-all for fact type lines
    const cleanLine = line.replace(/\.$/, '')

    // Build current noun list for tokenization (declared nouns only)
    const currentNouns = [
      ...existingNouns,
      ...[...nounMap.values()]
        .filter(n => !existingNouns.some(e => e.name === n.name))
        .map(n => ({ name: n.name, id: '' })),
    ]

    // Tokenize against declared nouns — longest-match-first via tokenizeReading
    const tokenized = tokenizeReading(cleanLine, currentNouns)
    const nounNames = tokenized.nounRefs.map(r => r.name)

    // Unary fact (1 noun)
    if (nounNames.length === 1) {
      const predicate = tokenized.predicate || cleanLine.replace(nounNames[0], '').trim()
      readings.push({ text: cleanLine, nouns: nounNames, predicate })
      parsedLines++
      continue
    }

    // Binary+ fact (2+ nouns)
    if (nounNames.length >= 2) {
      const predicate = tokenized.predicate || extractPredicate(cleanLine, nounNames)

      readings.push({ text: cleanLine, nouns: nounNames, predicate })

      // Parse indented constraint lines following this reading
      while (i + 1 < lines.length && /^\s+\S/.test(lines[i + 1])) {
        i++
        const constraintLine = lines[i].trim().replace(/\.$/, '')
        const parsed = parseConstraintText(constraintLine)
        if (parsed) {
          for (const pc of parsed) {
            // Use constrainedNoun to find the role index in the primary reading.
            // For spanning UCs (no constrainedNoun), all roles are constrained.
            const roles = pc.constrainedNoun
              ? [nounNames.indexOf(pc.constrainedNoun)].filter(idx => idx !== -1)
              : pc.nouns.map(cn => nounNames.indexOf(cn)).filter(idx => idx !== -1)
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
    transitions,
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

/**
 * Resolve the role index of a constrained noun in a primary reading.
 *
 * In ORM2: "Each A R at most one B" → UC on A's role.
 * The role index is A's position in the primary reading's noun order,
 * which matches the graph schema's role order.
 */
function resolveConstrainedRole(
  readingText: string,
  constrainedNoun: string,
  readings: ParseResult['readings'],
): number[] {
  const reading = readings.find(r => r.text === readingText)
  if (!reading) return []
  const idx = reading.nouns.indexOf(constrainedNoun)
  return idx !== -1 ? [idx] : []
}

// ── HTTP Handler ───────────────────────────────────────────────────

function getDB(env: Env): DurableObjectStub {
  const id = env.DOMAIN_DB.idFromName('graphdl-primary')
  return env.DOMAIN_DB.get(id)
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

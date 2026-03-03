/**
 * Parse domain markdown and FORML2 plain text into structured ORM definitions.
 *
 * Supports two input formats:
 * 1. Domain markdown — Entity Types, Value Types, Readings, Instance Facts, Deontic Constraints tables
 * 2. State machine markdown — States list + Transitions table
 * 3. Plain text FORML2 — one reading per line: "Customer has Name *:1"
 */

// ─── Types ──────────────────────────────────────────────────────────────────

export interface EntityTypeDef {
  name: string
  referenceScheme: string[]
  notes?: string
}

export interface ValueTypeDef {
  name: string
  valueType: string
  format?: string
  enum?: string
  pattern?: string
  minimum?: number
  maximum?: number
}

export interface ReadingDef {
  text: string
  multiplicity: string
}

export interface DomainParseResult {
  entityTypes: EntityTypeDef[]
  valueTypes: ValueTypeDef[]
  readings: ReadingDef[]
  instanceFacts: string[]
  deonticConstraints: string[]
}

export interface StateMachineParseResult {
  states: string[]
  transitions: { from: string; to: string; event: string; guard?: string }[]
}

// ─── Markdown Table Parser ──────────────────────────────────────────────────

function parseTableRows(lines: string[]): string[][] {
  return lines
    .filter((line) => line.includes('|') && !line.match(/^\s*\|[-\s|]+\|\s*$/))
    .map((line) =>
      line
        .split('|')
        .slice(1, -1)
        .map((cell) => cell.trim()),
    )
}

function findSection(lines: string[], heading: string): string[] {
  const headingPattern = new RegExp(`^##\\s+${heading}`, 'i')
  const start = lines.findIndex((l) => headingPattern.test(l))
  if (start === -1) return []
  const end = lines.findIndex((l, i) => i > start && /^##\s/.test(l))
  return lines.slice(start + 1, end === -1 ? undefined : end)
}

// ─── Domain Parser ──────────────────────────────────────────────────────────

export function parseDomainMarkdown(markdown: string): DomainParseResult {
  const lines = markdown.split('\n')

  // Entity Types
  const entityLines = findSection(lines, 'Entity Types')
  const entityRows = parseTableRows(entityLines)
  const entityTypes: EntityTypeDef[] = entityRows
    .filter((row) => row.length >= 2 && row[0] !== 'Entity')
    .map((row) => ({
      name: row[0],
      referenceScheme: row[1].split(',').map((s) => s.trim()).filter(Boolean),
      ...(row[2] ? { notes: row[2] } : {}),
    }))

  // Value Types
  const valueLines = findSection(lines, 'Value Types')
  const valueRows = parseTableRows(valueLines)
  const valueTypes: ValueTypeDef[] = valueRows
    .filter((row) => row.length >= 2 && row[0] !== 'Value')
    .map((row) => {
      const def: ValueTypeDef = { name: row[0], valueType: row[1] }
      if (row[2]) {
        const constraints = row[2]
        const formatMatch = constraints.match(/format:\s*(\S+)/)
        const enumMatch = constraints.match(/enum:\s*(.+?)(?:,\s*(?:pattern|format|minimum|maximum)|$)/)
        const patternMatch = constraints.match(/pattern:\s*(\S+)/)
        const minMatch = constraints.match(/minimum:\s*(\d+)/)
        const maxMatch = constraints.match(/maximum:\s*(\d+)/)

        if (formatMatch) def.format = formatMatch[1]
        if (enumMatch) def.enum = enumMatch[1].trim()
        if (patternMatch) def.pattern = patternMatch[1]
        if (minMatch) def.minimum = parseInt(minMatch[1])
        if (maxMatch) def.maximum = parseInt(maxMatch[1])
      }
      return def
    })

  // Collect all readings from any section that starts with "## Readings"
  const readings: ReadingDef[] = []
  let inReadingSection = false
  for (const line of lines) {
    if (/^##\s+Readings/.test(line)) {
      inReadingSection = true
      continue
    }
    if (/^##\s/.test(line) && !/^##\s+Readings/.test(line)) {
      inReadingSection = false
      continue
    }
    if (inReadingSection && line.includes('|') && !line.match(/^\s*\|[-\s|]+\|\s*$/) && !line.match(/^\s*\|\s*Reading\s*\|/)) {
      const cells = line.split('|').slice(1, -1).map((c) => c.trim())
      if (cells.length >= 2 && cells[0]) {
        const text = cells[0]
        const mult = cells[1].replace(/\\/g, '')
        readings.push({ text, multiplicity: mult })
      }
    }
  }

  // Instance Facts
  const factLines = findSection(lines, 'Instance Facts')
  const factRows = parseTableRows(factLines)
  const instanceFacts = factRows
    .filter((row) => row.length >= 1 && row[0] !== 'Fact')
    .map((row) => row[0])

  // Deontic Constraints
  const constraintLines = findSection(lines, 'Deontic Constraints')
  const constraintRows = parseTableRows(constraintLines)
  const deonticConstraints = constraintRows
    .filter((row) => row.length >= 1 && row[0] !== 'Constraint')
    .map((row) => row[0])

  return { entityTypes, valueTypes, readings, instanceFacts, deonticConstraints }
}

// ─── State Machine Parser ───────────────────────────────────────────────────

export function parseStateMachineMarkdown(markdown: string): StateMachineParseResult {
  const lines = markdown.split('\n')

  // States: comma-separated line after ## States
  const stateLines = findSection(lines, 'States')
  const statesLine = stateLines.find((l) => l.trim() && !l.startsWith('|'))
  const states = statesLine
    ? statesLine.split(',').map((s) => s.trim()).filter(Boolean)
    : []

  // Transitions table
  const transitionLines = findSection(lines, 'Transitions')
  const transitionRows = parseTableRows(transitionLines)
  const transitions = transitionRows
    .filter((row) => row.length >= 3 && row[0] !== 'From')
    .map((row) => ({
      from: row[0],
      to: row[1],
      event: row[2],
      ...(row[3] ? { guard: row[3] } : {}),
    }))

  return { states, transitions }
}

// ─── FORML2 Plain Text Parser ───────────────────────────────────────────────

export function parseFORML2(text: string): ReadingDef[] {
  return text
    .split('\n')
    .map((line) => line.trim())
    .filter((line) => line && !line.startsWith('#') && !line.startsWith('//'))
    .map((line) => {
      // Match: "Reading text *:1" or "Reading text | *:1"
      const pipeMatch = line.match(/^(.+?)\s*\|\s*([*1]:[*1])\s*$/)
      if (pipeMatch) return { text: pipeMatch[1].trim(), multiplicity: pipeMatch[2] }

      const spaceMatch = line.match(/^(.+?)\s+([*1]:[*1])\s*$/)
      if (spaceMatch) return { text: spaceMatch[1].trim(), multiplicity: spaceMatch[2] }

      // No multiplicity — default to *:1
      return { text: line, multiplicity: '*:1' }
    })
}

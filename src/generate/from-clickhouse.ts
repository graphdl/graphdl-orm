/**
 * fromClickHouse — compile ClickHouse CREATE TABLE statements into FORML2 readings.
 *
 * ClickHouse schema → readings. The ontology is a semantic projection over
 * the existing ClickHouse data. Tables become entity types, columns become
 * fact types, CHECK constraints become value constraints, foreign key comments
 * become relationship fact types.
 *
 * The generated readings don't create a new store — they describe the shape
 * of data that already exists in ClickHouse.
 */

export interface ClickHouseTable {
  database?: string
  name: string
  columns: ClickHouseColumn[]
  orderBy?: string[]
  partitionBy?: string
  engine?: string
}

export interface ClickHouseColumn {
  name: string
  type: string
  default?: string
  comment?: string
  constraint?: string // CHECK constraint text
  nullable?: boolean
  fk?: string // foreign key target: "table.column"
}

// ── SQL Parser (minimal, for CREATE TABLE) ──────────────────────────

export function parseClickHouseSQL(sql: string): ClickHouseTable[] {
  const tables: ClickHouseTable[] = []
  // Match CREATE TABLE statements
  const tableRegex = /CREATE\s+TABLE\s+(?:IF\s+NOT\s+EXISTS\s+)?(?:(\w+)\.)?(\w+)\s*\(([\s\S]*?)\)\s*ENGINE\s*=\s*(\w+(?:\([^)]*\))?)/gi

  let match: RegExpExecArray | null
  while ((match = tableRegex.exec(sql)) !== null) {
    const database = match[1] || undefined
    const name = match[2]
    const columnBlock = match[3]
    const engine = match[4]

    const columns = parseColumns(columnBlock)

    // Extract ORDER BY and PARTITION BY from after ENGINE
    const afterEngine = sql.slice(match.index + match[0].length, match.index + match[0].length + 200)
    const orderMatch = afterEngine.match(/ORDER\s+BY\s+(?:\(([^)]+)\)|(\w+))/i)
    const partitionMatch = afterEngine.match(/PARTITION\s+BY\s+(\w+)/i)

    tables.push({
      database,
      name,
      columns,
      engine,
      orderBy: orderMatch ? (orderMatch[1] || orderMatch[2]).split(',').map(s => s.trim()) : undefined,
      partitionBy: partitionMatch?.[1],
    })
  }

  return tables
}

function parseColumns(block: string): ClickHouseColumn[] {
  const columns: ClickHouseColumn[] = []

  // Split by newlines first (each column is typically on its own line),
  // then by commas within lines. This preserves inline comments.
  const rawLines = block.split('\n').map(l => l.trim()).filter(l => l.length > 0)
  const parts: string[] = []
  for (const line of rawLines) {
    // Remove trailing comma but keep the rest (including comments)
    const cleaned = line.replace(/,\s*$/, '').trim()
    if (cleaned) parts.push(cleaned)
  }

  for (const part of parts) {
    // Skip CONSTRAINT lines
    if (/^\s*CONSTRAINT/i.test(part)) {
      // Extract constraint and attach to previous column if possible
      const constraintMatch = part.match(/CONSTRAINT\s+\w+\s+CHECK\s+(.+)/i)
      if (constraintMatch && columns.length > 0) {
        columns[columns.length - 1].constraint = constraintMatch[1].trim()
      }
      continue
    }

    // Skip comments
    if (/^\s*--/.test(part)) continue

    // Extract FK hint from comment BEFORE stripping comments
    const commentMatch = part.match(/--\s*FK\s*\??\s*(\w+\.\w+)/i)

    // Parse column: name type [DEFAULT value] [-- comment]
    const cleaned = part.replace(/--.*$/gm, '').trim()
    if (!cleaned) continue

    const colMatch = cleaned.match(/^(\w+)\s+([\w()]+)(?:\s+DEFAULT\s+(\S+))?/i)
    if (!colMatch) continue

    const name = colMatch[1]
    const type = colMatch[2]
    const defaultVal = colMatch[3]

    columns.push({
      name,
      type,
      default: defaultVal,
      fk: commentMatch?.[1],
      nullable: defaultVal !== undefined,
    })
  }

  return columns
}

// ── Readings Generator ──────────────────────────────────────────────

export function fromClickHouse(tables: ClickHouseTable[], domainName: string): string {
  const lines: string[] = []
  lines.push(`# ${domainName} — ClickHouse Projection`)
  lines.push('')
  lines.push('Semantic projection over existing ClickHouse tables.')
  lines.push('The engine queries ClickHouse; it does not create a new store.')
  lines.push('')

  const entityTypes: string[] = []
  const valueTypes: Set<string> = new Set()
  const factLines: string[] = []

  // Internal columns to skip (infrastructure, not domain)
  const skipColumns = new Set(['createdAt', 'meta', 'label'])

  for (const table of tables) {
    const entityName = toNounName(table.name)
    entityTypes.push(entityName)

    // Determine primary key from ORDER BY or first column
    const pkCol = table.orderBy?.[0] ?? table.columns[0]?.name ?? 'id'

    const sectionLines: string[] = []
    sectionLines.push(`### ${entityName}`)

    for (const col of table.columns) {
      if (skipColumns.has(col.name)) continue
      if (col.name === pkCol) continue // PK is in the reference scheme

      const propName = toNounName(col.name)

      // FK reference → relationship fact type
      if (col.fk) {
        const refTable = col.fk.split('.')[0]
        const refEntity = toNounName(refTable)
        sectionLines.push(`${entityName} has ${refEntity}.`)
        sectionLines.push(`  Each ${entityName} has at most one ${refEntity}.`)
      } else if (isValueType(col.type)) {
        valueTypes.add(propName)
        sectionLines.push(`${entityName} has ${propName}.`)
        if (!col.nullable && col.default === undefined) {
          sectionLines.push(`  Each ${entityName} has exactly one ${propName}.`)
        } else {
          sectionLines.push(`  Each ${entityName} has at most one ${propName}.`)
        }
      }
    }

    factLines.push(...sectionLines, '')
  }

  // Entity Types
  if (entityTypes.length > 0) {
    lines.push('## Entity Types')
    lines.push('')
    for (const name of entityTypes) {
      lines.push(`${name}(.id) is an entity type.`)
    }
    lines.push('')
  }

  // Value Types
  if (valueTypes.size > 0) {
    lines.push('## Value Types')
    lines.push('')
    for (const name of [...valueTypes].sort()) {
      lines.push(`${name} is a value type.`)
    }
    lines.push('')
  }

  // Fact Types
  if (factLines.length > 0) {
    lines.push('## Fact Types')
    lines.push('')
    lines.push(...factLines)
  }

  // Instance Facts
  lines.push('## Instance Facts')
  lines.push('')
  lines.push(`Domain '${domainName}' has Visibility 'public'.`)
  lines.push('')

  return lines.join('\n')
}

// ── Helpers ──────────────────────────────────────────────────────────

function toNounName(name: string): string {
  return name
    .replace(/([a-z])([A-Z])/g, '$1 $2')
    .replace(/_/g, ' ')
    .replace(/\b\w/g, c => c.toUpperCase())
    .replace(/\bId\b/g, 'Id')
    .replace(/\bUrl\b/g, 'URL')
    .replace(/\bVin\b/g, 'VIN')
}

function isValueType(chType: string): boolean {
  const t = chType.toLowerCase()
  return t.startsWith('string') || t.startsWith('uint') || t.startsWith('int')
    || t.startsWith('float') || t.startsWith('bool') || t === 'date' || t === 'datetime'
}

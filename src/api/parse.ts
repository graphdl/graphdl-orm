import { json, error } from 'itty-router'
import type { Env } from '../types'
import type { ExtractedClaims } from '../claims/ingest'
import { tokenizeReading } from '../claims/tokenize'
import { parseConstraintText, parseSetComparisonBlock, isInformationalPattern } from '../hooks/parse-constraint'

interface ParseResult extends ExtractedClaims {
  warnings: string[]
}

/**
 * Pure-function FORML2 parser.
 *
 * Parses multi-line FORML2 text into structured ExtractedClaims.
 * No DB writes, no hooks — read-only.
 *
 * @param text - Multi-line FORML2 text
 * @param existingNouns - Known nouns for tokenization context (from DB, read-only)
 */
export function parseFORML2(
  text: string,
  existingNouns: Array<{ name: string; id: string; objectType?: 'entity' | 'value' }>,
): ParseResult {
  const warnings: string[] = []
  const nounMap = new Map<string, { name: string; objectType: 'entity' | 'value' }>()
  const readings: ParseResult['readings'] = []
  const constraints: ParseResult['constraints'] = []
  const subtypes: ParseResult['subtypes'] = []
  const deferred: Array<{ constraintText: string; readingText: string }> = []

  // Initialize nounMap with existing nouns (preserve their stored objectType)
  for (const n of existingNouns) {
    if (!nounMap.has(n.name)) {
      nounMap.set(n.name, { name: n.name, objectType: n.objectType || 'entity' })
    }
  }

  // Split on blank lines into blocks
  const blocks = text.split(/\n\s*\n/).filter(b => b.trim())

  for (const block of blocks) {
    const lines = block.split('\n')
    const factLine = lines[0].trim().replace(/\.$/, '')

    // Check for set-comparison block (XO/XC/OR/SS — multi-line, standalone)
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
      continue
    }

    // Skip informational patterns (not readings or constraints)
    if (isInformationalPattern(factLine)) {
      continue
    }

    // Check for subtype declaration
    const subtypeMatch = factLine.match(/^([A-Z][a-zA-Z0-9]*)\s+is a subtype of\s+([A-Z][a-zA-Z0-9]*)/i)
    if (subtypeMatch) {
      const child = subtypeMatch[1]
      const parent = subtypeMatch[2]
      subtypes.push({ child, parent })
      // Ensure both nouns exist
      if (!nounMap.has(child)) nounMap.set(child, { name: child, objectType: 'entity' })
      if (!nounMap.has(parent)) nounMap.set(parent, { name: parent, objectType: 'entity' })
      continue
    }

    // Build current noun list for tokenization (combine existing + discovered)
    const currentNouns = [
      ...existingNouns,
      ...[...nounMap.values()]
        .filter(n => !existingNouns.some(e => e.name === n.name))
        .map(n => ({ name: n.name, id: '' })),
    ]

    // Tokenize reading
    const tokenized = tokenizeReading(factLine, currentNouns)
    let nounNames = tokenized.nounRefs.map(r => r.name)

    // PascalCase fallback if tokenization found fewer than 2 nouns
    if (nounNames.length < 2) {
      const pascalWords = factLine.match(/[A-Z][a-zA-Z0-9]*/g) || []
      nounNames = pascalWords
    }

    if (nounNames.length < 2) {
      warnings.push(`Reading "${factLine}" has fewer than 2 nouns — skipped`)
      continue
    }

    // Determine predicate
    const predicate = tokenized.predicate || extractPredicate(factLine, nounNames)
    const isHasPredicate = /^has$/i.test(predicate.trim())

    // Accumulate nouns
    for (let i = 0; i < nounNames.length; i++) {
      const name = nounNames[i]
      if (!nounMap.has(name)) {
        const objectType = (isHasPredicate && i === nounNames.length - 1) ? 'value' : 'entity'
        nounMap.set(name, { name, objectType })
      }
    }

    // Accumulate reading
    const readingText = factLine
    readings.push({ text: readingText, nouns: nounNames, predicate })

    // Parse indented constraint lines
    const constraintLines = lines.slice(1)
      .filter(l => l.match(/^\s+\S/))
      .map(l => l.trim())

    for (const constraintText of constraintLines) {
      const parsed = parseConstraintText(constraintText)
      if (!parsed) {
        warnings.push(`Unrecognized constraint pattern: "${constraintText}"`)
        continue
      }

      for (const pc of parsed) {
        // For UC/MC constraints, the first noun is the subject (constraining role).
        // Only map the first noun to a role index to match ORM semantics:
        // "Each Customer has at most one Name" → UC on role 0 (Customer).
        const constraintNouns = (pc.kind === 'UC' || pc.kind === 'MC') && pc.nouns.length > 0
          ? [pc.nouns[0]]
          : pc.nouns
        const roles = constraintNouns
          .map(cn => nounNames.indexOf(cn))
          .filter(idx => idx !== -1)

        if (roles.length === 0 && pc.nouns.length > 0) {
          // Constraint nouns not in this reading — defer
          deferred.push({ constraintText, readingText })
          continue
        }

        constraints.push({
          kind: pc.kind,
          modality: pc.modality,
          reading: readingText,
          roles,
        })
      }
    }
  }

  // Retry deferred constraints against full noun/reading set
  for (const d of deferred) {
    const parsed = parseConstraintText(d.constraintText)
    if (!parsed) continue

    // Find a reading whose nouns match the constraint's nouns
    let resolved = false
    for (const reading of readings) {
      for (const pc of parsed) {
        const constraintNouns = (pc.kind === 'UC' || pc.kind === 'MC') && pc.nouns.length > 0
          ? [pc.nouns[0]]
          : pc.nouns
        const roles = constraintNouns
          .map(cn => reading.nouns.indexOf(cn))
          .filter(idx => idx !== -1)

        if (roles.length > 0) {
          constraints.push({
            kind: pc.kind,
            modality: pc.modality,
            reading: reading.text,
            roles,
          })
          resolved = true
        }
      }
      if (resolved) break
    }

    if (!resolved) {
      warnings.push(`Deferred constraint still unresolved: "${d.constraintText}"`)
    }
  }

  // Build nouns array from map
  const nouns = [...nounMap.values()]

  return {
    nouns,
    readings,
    constraints,
    subtypes,
    transitions: [],
    facts: [],
    warnings,
  }
}

/** Extract predicate between first two nouns when tokenizer didn't find it. */
function extractPredicate(text: string, nounNames: string[]): string {
  if (nounNames.length < 2) return ''
  const first = text.indexOf(nounNames[0])
  if (first === -1) return ''
  const afterFirst = first + nounNames[0].length
  const second = text.indexOf(nounNames[1], afterFirst)
  if (second === -1) return ''
  return text.slice(afterFirst, second).trim()
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
  return json(result)
}

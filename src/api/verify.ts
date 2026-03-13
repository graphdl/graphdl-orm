import { json, error } from 'itty-router'
import type { Env } from '../types'
import { tokenizeReading } from '../claims/tokenize'

interface DomainData {
  nouns: Array<{ id: string; name: string }>
  readings: Array<{ id: string; text: string }>
  roles: Array<{ id: string; reading: string; noun: string; roleIndex: number }>
  constraints: Array<{ id: string; kind: string; text?: string }>
  constraintSpans: Array<{ constraint: string; role: string }>
}

interface VerifyResult {
  matches: Array<{
    reading: string
    nouns: string[]
  }>
  unmatchedConstraints: string[]
}

/**
 * Pure-function prose verifier.
 *
 * Tokenizes prose against domain nouns to find which readings' noun types
 * are mentioned. Classifies constraints as deterministically checkable
 * (nouns present) or unmatched (nouns absent — needs semantic analysis).
 */
export function verifyProse(prose: string, data: DomainData): VerifyResult {
  const matches: VerifyResult['matches'] = []
  const matchedReadingIds = new Set<string>()

  // Tokenize prose once against all domain nouns (result is the same every iteration)
  const tokenized = tokenizeReading(prose, data.nouns)
  const foundNounNames = new Set(tokenized.nounRefs.map(r => r.name))

  // For each reading, check if its nouns appear in the prose
  for (const reading of data.readings) {
    // Get nouns for this reading via roles
    const readingRoles = data.roles
      .filter(r => r.reading === reading.id)
      .sort((a, b) => a.roleIndex - b.roleIndex)

    const readingNouns = readingRoles
      .map(r => data.nouns.find(n => n.id === r.noun))
      .filter((n): n is { id: string; name: string } => !!n)

    // Check which of this reading's nouns appear in the prose
    const matchedNouns = readingNouns.filter(n => foundNounNames.has(n.name))

    if (matchedNouns.length > 0) {
      matches.push({
        reading: reading.text,
        nouns: matchedNouns.map(n => n.name),
      })
      matchedReadingIds.add(reading.id)
    }
  }

  // Build constraint → reading mapping via spans → roles
  const unmatchedConstraints: string[] = []

  for (const constraint of data.constraints) {
    // Find which roles this constraint spans
    const spans = data.constraintSpans.filter(s => s.constraint === constraint.id)
    const roleIds = spans.map(s => s.role)

    // Find which reading(s) those roles belong to
    const readingIds = new Set(
      roleIds
        .map(rid => data.roles.find(r => r.id === rid)?.reading)
        .filter((rid): rid is string => !!rid)
    )

    // If none of the constraint's readings matched, it's unmatched
    const anyMatched = [...readingIds].some(rid => matchedReadingIds.has(rid))
    if (!anyMatched) {
      unmatchedConstraints.push(constraint.text || `${constraint.kind} constraint ${constraint.id}`)
    }
  }

  return { matches, unmatchedConstraints }
}

// ── HTTP Handler ───────────────────────────────────────────────────

function getDB(env: Env): DurableObjectStub {
  const id = env.GRAPHDL_DB.idFromName('graphdl-primary')
  return env.GRAPHDL_DB.get(id)
}

export async function handleVerify(request: Request, env: Env): Promise<Response> {
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

  const db = getDB(env) as any
  const domainId = body.domain

  // Load all domain data in parallel (read-only)
  // Note: roles and constraint-spans don't have a domain_id column — they're
  // child records of domain-scoped readings/constraints. We load all and filter
  // in-memory by matching against the domain-scoped reading IDs.
  const [nounsResult, readingsResult, rolesResult, constraintsResult, spansResult] = await Promise.all([
    db.findInCollection('nouns', { domain_id: { equals: domainId } }, { limit: 10000 }),
    db.findInCollection('readings', { domain_id: { equals: domainId } }, { limit: 10000 }),
    db.findInCollection('roles', {}, { limit: 10000 }),
    db.findInCollection('constraints', { domain_id: { equals: domainId } }, { limit: 10000 }),
    db.findInCollection('constraint-spans', {}, { limit: 10000 }),
  ])

  const domainData: DomainData = {
    nouns: nounsResult.docs.map((n: any) => ({ id: n.id, name: n.name })),
    readings: readingsResult.docs.map((r: any) => ({ id: r.id, text: r.text })),
    roles: rolesResult.docs
      .filter((r: any) => readingsResult.docs.some((rd: any) => rd.id === r.reading))
      .map((r: any) => ({ id: r.id, reading: r.reading, noun: r.noun, roleIndex: r.roleIndex })),
    constraints: constraintsResult.docs.map((c: any) => ({ id: c.id, kind: c.kind, text: c.text })),
    constraintSpans: spansResult.docs.map((s: any) => ({ constraint: s.constraint, role: s.role })),
  }

  const result = verifyProse(body.text, domainData)
  return json(result)
}

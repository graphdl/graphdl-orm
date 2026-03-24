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

/** Fetch all entities of a given type for a domain via Registry+EntityDB fan-out. */
async function fanOutEntities(env: Env, registry: any, entityType: string, domainSlug?: string): Promise<any[]> {
  const ids: string[] = await registry.getEntityIds(entityType, domainSlug)
  const entities = await Promise.all(
    ids.map(id => (env.ENTITY_DB.get(env.ENTITY_DB.idFromName(id)) as any).get())
  )
  return entities.filter(Boolean).map((e: any) => ({ id: e.id, ...e.data }))
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

  const registry = env.REGISTRY_DB.get(env.REGISTRY_DB.idFromName('global')) as any
  const domainSlug = body.domain

  // Load all domain data in parallel via Registry+EntityDB fan-out (read-only)
  const [nounDocs, readingDocs, roleDocs, constraintDocs, spanDocs] = await Promise.all([
    fanOutEntities(env, registry, 'Noun', domainSlug),
    fanOutEntities(env, registry, 'Reading', domainSlug),
    fanOutEntities(env, registry, 'Role'),
    fanOutEntities(env, registry, 'Constraint', domainSlug),
    fanOutEntities(env, registry, 'ConstraintSpan'),
  ])

  // Filter roles and spans to only those belonging to domain-scoped readings
  const readingIds = new Set(readingDocs.map((r: any) => r.id))

  const domainData: DomainData = {
    nouns: nounDocs.map((n: any) => ({ id: n.id, name: n.name })),
    readings: readingDocs.map((r: any) => ({ id: r.id, text: r.text })),
    roles: roleDocs
      .filter((r: any) => readingIds.has(r.reading || r.readingId))
      .map((r: any) => ({ id: r.id, reading: r.reading || r.readingId, noun: r.noun || r.nounId, roleIndex: r.roleIndex })),
    constraints: constraintDocs.map((c: any) => ({ id: c.id, kind: c.kind, text: c.text })),
    constraintSpans: spanDocs.map((s: any) => ({ constraint: s.constraint || s.constraintId, role: s.role || s.roleId })),
  }

  const result = verifyProse(body.text, domainData)
  return json(result)
}

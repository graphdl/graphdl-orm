import configPromise from '@payload-config'
import { getPayload } from 'payload'
import { buildMatchers, matchText } from '../../extract/matcher'
import type { DeonticConstraintGroup } from '../../seed/deontic'

interface DetMatch {
  factType: string
  instance: string
  span: [number, number]
}

interface SemanticClaim {
  factType: string
  claim: string
  confidence: number
  span: [number, number]
}

async function checkText(text: string, domain: string) {
  const payload = await getPayload({ config: configPromise })

  const domainResult = await payload.find({
    collection: 'domains',
    where: { domainSlug: { equals: domain } },
    limit: 1,
  })

  if (domainResult.docs.length === 0) {
    return Response.json({ error: `Domain "${domain}" not found` }, { status: 404 })
  }

  const domainDoc = domainResult.docs[0]

  const readingsResult = await payload.find({
    collection: 'readings',
    where: { domain: { equals: domainDoc.id } },
    pagination: false,
  })

  const readings = readingsResult.docs
  const deonticConstraints: string[] = []
  const allTexts: string[] = []

  for (const r of readings) {
    const t = typeof (r as any).text === 'string' ? (r as any).text : ''
    if (t) allTexts.push(t)
    if (/\bmust\b/i.test(t)) {
      deonticConstraints.push(t)
    }
  }

  const instanceFactTexts = new Set<string>()
  for (const a of deonticConstraints) {
    for (const b of deonticConstraints) {
      if (a !== b && b.startsWith(a) && b.length > a.length) {
        instanceFactTexts.add(b)
      }
    }
  }
  const rootConstraints = deonticConstraints.filter((c) => !instanceFactTexts.has(c))

  const groups: DeonticConstraintGroup[] = rootConstraints.map((constraintText) => {
    const instances: string[] = []
    for (const readingText of allTexts) {
      if (readingText === constraintText) continue
      if (readingText.startsWith(constraintText)) {
        let trailing = readingText.slice(constraintText.length).trim()
        if (
          trailing.length >= 2 &&
          ((trailing.startsWith("'") && trailing.endsWith("'")) ||
            (trailing.startsWith('"') && trailing.endsWith('"')) ||
            (trailing.startsWith('\u2018') && trailing.endsWith('\u2019')) ||
            (trailing.startsWith('\u201C') && trailing.endsWith('\u201D')))
        ) {
          trailing = trailing.slice(1, -1)
        }
        if (trailing) instances.push(trailing)
      }
    }
    return { constraintText, instances }
  })

  const matchers = buildMatchers(groups)
  const extracted = matchText(text, matchers)

  const deonticTexts = new Set(deonticConstraints)
  const warnings = []

  for (const match of extracted.matches || []) {
    if (deonticTexts.has(match.factType)) {
      warnings.push({
        reading: match.factType,
        instance: match.instance,
        span: match.span,
        method: 'deterministic' as const,
      })
    }
  }

  return Response.json({ text, domain, extracted, warnings })
}

async function checkMatches(matches: DetMatch[], claims: SemanticClaim[]) {
  const payload = await getPayload({ config: configPromise })

  const readingsResult = await payload.find({
    collection: 'readings',
    pagination: false,
  })

  const deonticTexts = new Set(
    readingsResult.docs
      .filter((r: any) => /\bmust\b/i.test(r.text || ''))
      .map((r: any) => r.text as string),
  )

  const warnings = []

  for (const match of matches) {
    if (deonticTexts.has(match.factType)) {
      warnings.push({
        reading: match.factType,
        instance: match.instance,
        span: match.span,
        method: 'deterministic' as const,
      })
    }
  }

  for (const claim of claims) {
    if (deonticTexts.has(claim.factType)) {
      warnings.push({
        reading: claim.factType,
        claim: claim.claim,
        span: claim.span,
        method: 'semantic' as const,
        confidence: claim.confidence,
      })
    }
  }

  return Response.json({ warnings })
}

export const GET = async (request: Request) => {
  const url = new URL(request.url)
  const text = url.searchParams.get('text')
  const domain = url.searchParams.get('domain')

  if (!text || !domain) {
    return Response.json({ error: 'Required params: ?text=...&domain=...' }, { status: 400 })
  }

  return checkText(text, domain)
}

export const POST = async (request: Request) => {
  let body: { matches?: DetMatch[]; claims?: SemanticClaim[]; text?: string; domain?: string }
  try {
    body = await request.json()
  } catch {
    return Response.json({ error: 'Invalid JSON' }, { status: 400 })
  }

  if (body.text && body.domain) {
    return checkText(body.text, body.domain)
  }

  const { matches = [], claims = [] } = body
  return checkMatches(matches, claims)
}

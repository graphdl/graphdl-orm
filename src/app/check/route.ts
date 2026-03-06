import configPromise from '@payload-config'
import { getPayload } from 'payload'

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

export const POST = async (request: Request) => {
  let body: { matches?: DetMatch[]; claims?: SemanticClaim[] }
  try {
    body = await request.json()
  } catch {
    return Response.json({ error: 'Invalid JSON' }, { status: 400 })
  }

  const { matches = [], claims = [] } = body

  const payload = await getPayload({ config: configPromise })

  // Fetch all readings to find active deontic constraints
  const readingsResult = await payload.find({
    collection: 'readings',
    pagination: false,
  })

  const deonticTexts = new Set(
    readingsResult.docs
      .filter((r: any) => /\bmust\b/i.test(r.text || ''))
      .map((r: any) => r.text as string)
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

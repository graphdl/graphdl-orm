import configPromise from '@payload-config'
import { getPayload } from 'payload'
import { nounListToRegex, toPredicate } from '../../collections/Generator'
import type { Noun, GraphSchema } from '../../payload-types'

/**
 * /parse — Parse reading text into structured predicates using known nouns.
 *
 * GET  /parse?reading=Customer+submits+SupportRequest&domain=support
 * POST /parse { readings: [...], domain: "support" }
 */
async function parse(readings: string[], domain: string) {
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

  const [domainNouns, graphSchemas] = await Promise.all([
    payload.find({
      collection: 'nouns',
      where: { domain: { equals: domainDoc.id } },
      pagination: false,
    }),
    payload.find({
      collection: 'graph-schemas',
      where: { domain: { equals: domainDoc.id } },
      pagination: false,
    }),
  ])

  const graphNouns = graphSchemas.docs.filter((g: any) => g.title === g.name)
  const allNouns = [...graphNouns, ...domainNouns.docs] as (Noun | GraphSchema)[]
  const nounRegex = nounListToRegex(allNouns as any)

  const parsed = readings.map((text) => {
    const tokens = toPredicate({ reading: text, nouns: allNouns as any, nounRegex })
    const nouns: Array<{ name: string; position: number }> = []

    tokens.forEach((token, i) => {
      const found = allNouns.find((n) => n.name === token || n.name === token.replace(/-$/, ''))
      if (found?.name) {
        nouns.push({ name: found.name, position: i })
      }
    })

    return { text, tokens, nouns }
  })

  return Response.json({ parsed })
}

export const GET = async (request: Request) => {
  const url = new URL(request.url)
  const readings = url.searchParams.getAll('reading')
  const domain = url.searchParams.get('domain')

  if (!readings.length || !domain) {
    return Response.json({ error: 'Required params: ?reading=...&domain=...' }, { status: 400 })
  }

  return parse(readings, domain)
}

export const POST = async (request: Request) => {
  let body: { readings?: string[]; domain?: string }
  try {
    body = await request.json()
  } catch {
    return Response.json({ error: 'Invalid JSON body' }, { status: 400 })
  }

  const { readings, domain } = body
  if (!readings?.length || !domain) {
    return Response.json({ error: 'Required: { readings: string[], domain: string }' }, { status: 400 })
  }

  return parse(readings, domain)
}

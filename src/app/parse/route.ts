import configPromise from '@payload-config'
import { getPayload } from 'payload'
import { nounListToRegex, toPredicate } from '../../collections/Generator'
import type { Noun, GraphSchema } from '../../payload-types'

/**
 * POST /parse — Parse reading text into structured predicates using known nouns.
 *
 * Same nounRegex algorithm used by the generator and the Readings hook,
 * exposed as an API endpoint. Takes natural language readings and returns
 * tokenized predicates with noun positions identified.
 *
 * Input:
 *   { readings: string[], domain: string }
 *
 * Output:
 *   { parsed: Array<{ text: string, tokens: string[], nouns: Array<{ name: string, position: number }> }> }
 */
export const POST = async (request: Request) => {
  let body: { readings?: string[]; domain?: string }
  try {
    body = await request.json()
  } catch {
    return Response.json({ error: 'Invalid JSON body' }, { status: 400 })
  }

  const { readings, domain } = body

  if (!readings || !Array.isArray(readings) || !readings.length) {
    return Response.json({ error: 'Missing or invalid "readings" array' }, { status: 400 })
  }
  if (!domain || typeof domain !== 'string') {
    return Response.json({ error: 'Missing or invalid "domain" field' }, { status: 400 })
  }

  const payload = await getPayload({ config: configPromise })

  // Find the domain
  const domainResult = await payload.find({
    collection: 'domains',
    where: { domainSlug: { equals: domain } },
    limit: 1,
  })

  if (domainResult.docs.length === 0) {
    return Response.json({ error: `Domain "${domain}" not found` }, { status: 404 })
  }

  const domainDoc = domainResult.docs[0]

  // Fetch all nouns in this domain + global nouns (no domain)
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

  // Combine nouns and graph schemas (graph schemas with title === name act as nouns)
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

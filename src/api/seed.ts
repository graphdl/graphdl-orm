import { json, error } from 'itty-router'
import type { Env } from '../types'
import { ingestClaims } from '../claims/ingest'
import type { ExtractedClaims } from '../claims/ingest'

export async function handleSeed(request: Request, env: Env): Promise<Response> {
  const db = getDB(env)

  if (request.method === 'GET') {
    const stats = {
      nouns: (await (db as any).findInCollection('nouns', {}, { limit: 0 })).totalDocs,
      readings: (await (db as any).findInCollection('readings', {}, { limit: 0 })).totalDocs,
      domains: (await (db as any).findInCollection('domains', {}, { limit: 0 })).totalDocs,
    }
    return json(stats)
  }

  if (request.method === 'DELETE') {
    await (db as any).wipeAllData()
    return json({ message: 'All data wiped' })
  }

  if (request.method === 'POST') {
    const body = await request.json() as {
      type: string
      claims?: ExtractedClaims
      domain?: string
      domainId?: string
      domains?: Array<{ slug: string; name?: string; claims: ExtractedClaims }>
    }

    if (body.type === 'claims') {
      // Bulk: multiple domains in one call
      if (body.domains?.length) {
        const results = []
        for (const entry of body.domains) {
          const domain = await ensureDomain(db as any, entry.slug, entry.name)
          const result = await ingestClaims(db as any, { claims: entry.claims, domainId: domain.id })
          results.push({ domain: entry.slug, domainId: domain.id, ...result })
        }
        return json({ domains: results })
      }

      // Single domain
      if (body.claims) {
        const domainId = body.domainId || body.domain
        if (!domainId) return error(400, { errors: [{ message: 'domainId or domains[] required' }] })

        const result = await ingestClaims(db as any, { claims: body.claims, domainId })
        return json(result)
      }

      return error(400, { errors: [{ message: 'Provide claims + domainId, or domains[]' }] })
    }

    return error(400, { errors: [{ message: 'Unsupported seed type. Use type: "claims"' }] })
  }

  return error(405, { errors: [{ message: 'Method not allowed' }] })
}

function getDB(env: Env) {
  const id = env.GRAPHDL_DB.idFromName('graphdl-primary')
  return env.GRAPHDL_DB.get(id)
}

async function ensureDomain(db: any, slug: string, name?: string): Promise<Record<string, any>> {
  const existing = await db.findInCollection('domains', {
    domainSlug: { equals: slug },
  }, { limit: 1 })

  if (existing.docs.length) return existing.docs[0]

  return db.createInCollection('domains', {
    domainSlug: slug,
    name: name || slug,
    visibility: 'private',
  })
}

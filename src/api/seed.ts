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
    const body = await request.json() as { type: string; claims?: ExtractedClaims; domain?: string; domainId?: string }

    if (body.type === 'claims' && body.claims) {
      const domainId = body.domainId || body.domain
      if (!domainId) return error(400, { errors: [{ message: 'domainId required for claims ingestion' }] })

      const result = await ingestClaims(db as any, { claims: body.claims, domainId })
      return json(result)
    }

    return error(400, { errors: [{ message: 'Unsupported seed type. Use type: "claims"' }] })
  }

  return error(405, { errors: [{ message: 'Method not allowed' }] })
}

function getDB(env: Env) {
  const id = env.GRAPHDL_DB.idFromName('graphdl-primary')
  return env.GRAPHDL_DB.get(id)
}

import { json, error } from 'itty-router'
import type { Env } from '../types'

const VALID_FORMATS = ['openapi', 'sqlite', 'xstate', 'ilayer', 'readings', 'business-rules', 'mdxui', 'readme', 'schema'] as const

export async function handleGenerate(request: Request, env: Env): Promise<Response> {
  const body = await request.json() as Record<string, any>
  const { domainId, outputFormat = 'openapi' } = body

  if (!domainId) return error(400, { errors: [{ message: 'domainId is required' }] })
  if (!VALID_FORMATS.includes(outputFormat)) {
    return error(400, { errors: [{ message: `Invalid outputFormat. Valid: ${VALID_FORMATS.join(', ')}` }] })
  }

  const id = env.GRAPHDL_DB.idFromName('graphdl-primary')
  const db = env.GRAPHDL_DB.get(id) as any

  // Delegate to DO's generate() RPC method
  const output = await db.generate(domainId, outputFormat)

  // Persist the generator output so ui.do can fetch it via /graphdl/raw/generators
  try {
    const existing = await db.findInCollection('generators', {
      domain: { equals: domainId },
      outputFormat: { equals: outputFormat },
    }, { limit: 1 })

    const outputStr = typeof output === 'string' ? output : JSON.stringify(output)

    if (existing?.docs?.[0]) {
      await db.updateInCollection('generators', existing.docs[0].id, { output: outputStr })
    } else {
      await db.createInCollection('generators', {
        domain: domainId,
        outputFormat,
        output: outputStr,
      })
    }
  } catch {
    // Don't fail the response if persistence fails
  }

  return json({ output, format: outputFormat, domainId })
}

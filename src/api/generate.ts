import { json, error } from 'itty-router'
import type { Env } from '../types'
import { generateOpenAPI } from '../generate/openapi'
import { generateSQLite } from '../generate/sqlite'
import { generateXState } from '../generate/xstate'
import { generateILayer } from '../generate/ilayer'
import { generateReadings } from '../generate/readings'
import { generateConstraintIR } from '../generate/constraint-ir'

const VALID_FORMATS = ['openapi', 'sqlite', 'xstate', 'ilayer', 'readings', 'constraint-ir'] as const

export async function handleGenerate(request: Request, env: Env): Promise<Response> {
  const body = await request.json() as Record<string, any>
  const { domainId, outputFormat = 'openapi' } = body

  if (!domainId) return error(400, { errors: [{ message: 'domainId is required' }] })
  if (!VALID_FORMATS.includes(outputFormat)) {
    return error(400, { errors: [{ message: `Invalid outputFormat. Valid: ${VALID_FORMATS.join(', ')}` }] })
  }

  const id = env.GRAPHDL_DB.idFromName('graphdl-primary')
  const db = env.GRAPHDL_DB.get(id) as any

  let output: any
  switch (outputFormat) {
    case 'openapi':
      output = await generateOpenAPI(db, domainId)
      break
    case 'sqlite': {
      const openapi = await generateOpenAPI(db, domainId)
      output = generateSQLite(openapi)
      break
    }
    case 'xstate':
      output = await generateXState(db, domainId)
      break
    case 'ilayer':
      output = await generateILayer(db, domainId)
      break
    case 'readings':
      output = await generateReadings(db, domainId)
      break
    case 'constraint-ir':
      output = await generateConstraintIR(db, domainId)
      break
  }

  // Persist the generator output so ui.do can fetch it via /graphdl/raw/generators
  try {
    // Find existing generator for this domain+format to update, or create new
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

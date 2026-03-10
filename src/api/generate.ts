import { json, error } from 'itty-router'
import type { Env } from '../types'
import { generateOpenAPI } from '../generate/openapi'
import { generateSQLite } from '../generate/sqlite'
import { generateXState } from '../generate/xstate'
import { generateILayer } from '../generate/ilayer'
import { generateReadings } from '../generate/readings'

const VALID_FORMATS = ['openapi', 'sqlite', 'xstate', 'ilayer', 'readings'] as const

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
  }

  return json({ output, format: outputFormat, domainId })
}

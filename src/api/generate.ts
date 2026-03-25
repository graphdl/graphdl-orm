import { json, error } from 'itty-router'
import type { Env } from '../types'
import { DomainModel } from '../model/domain-model'
import { EntityDataLoader } from '../model/entity-data-loader'
import { generateOpenAPI } from '../generate/openapi'
import { generateSQLite } from '../generate/sqlite'
import { generateXState } from '../generate/xstate'
import { generateILayer } from '../generate/ilayer'
import { generateReadings } from '../generate/readings'
import { generateSchema } from '../generate/schema'
import { generateMdxui } from '../generate/mdxui'
import { generateReadme } from '../generate/readme'

const VALID_FORMATS = ['openapi', 'sqlite', 'xstate', 'ilayer', 'readings', 'schema', 'mdxui', 'readme', 'json-schema'] as const

/**
 * Build a DomainModel backed by EntityDataLoader (Registry+EntityDB fan-out).
 * This replaces the SqlDataLoader path that required going through DomainDB.generate().
 */
function buildEntityModel(env: Env, domainId: string): DomainModel {
  const registry = env.REGISTRY_DB.get(env.REGISTRY_DB.idFromName('global')) as any
  const getStub = (id: string) => env.ENTITY_DB.get(env.ENTITY_DB.idFromName(id)) as any
  const loader = new EntityDataLoader(registry, getStub)
  return new DomainModel(loader, domainId)
}

/**
 * Run the appropriate generator for the given format using the EntityDataLoader-backed model.
 */
async function generateOutput(model: DomainModel, format: string): Promise<any> {
  switch (format) {
    case 'openapi':
      return generateOpenAPI(model)
    case 'sqlite':
      return generateSQLite(await generateOpenAPI(model))
    case 'xstate':
      return generateXState(model)
    case 'ilayer':
      return generateILayer(model)
    case 'readings':
      return generateReadings(model)
    case 'schema':
      return generateSchema(model)
    case 'mdxui':
      return generateMdxui(model)
    case 'readme':
      return generateReadme(model)
    case 'json-schema': {
      // json-schema returns the OpenAPI+SQLite table/field map (same as applySchema)
      const openapi = await generateOpenAPI(model)
      const { tableMap, fieldMap } = generateSQLite(openapi)
      return { tableMap, fieldMap }
    }
    default:
      throw new Error(`Unknown format: ${format}`)
  }
}

export async function handleGenerate(request: Request, env: Env): Promise<Response> {
  const body = await request.json() as Record<string, any>
  const { domainId, outputFormat = 'openapi' } = body

  if (!domainId) return error(400, { errors: [{ message: 'domainId is required' }] })
  if (!VALID_FORMATS.includes(outputFormat)) {
    return error(400, { errors: [{ message: `Invalid outputFormat. Valid: ${VALID_FORMATS.join(', ')}` }] })
  }

  // Build model from EntityDataLoader (Registry+EntityDB fan-out)
  const model = buildEntityModel(env, domainId)
  const output = await generateOutput(model, outputFormat)

  // Persist the generator output in DomainDB's generators table (cache)
  try {
    const db = env.DOMAIN_DB.get(env.DOMAIN_DB.idFromName('graphdl-primary')) as any
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

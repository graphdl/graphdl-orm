import { json, error } from 'itty-router'
import type { Env } from '../types'
import { DomainModel } from '../model/domain-model'
import { AppModel } from '../model/app-model'
import { BatchDataLoader } from '../model/batch-data-loader'
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
 * Build a DomainModel backed by BatchDataLoader (DomainDB batch WAL).
 * Reads metamodel entities directly from the DomainDB's committed batches.
 * No EntityDB fan-out, no subrequest limits.
 */
function buildDomainModel(env: Env, domainSlug: string): DomainModel {
  const domainDO = env.DOMAIN_DB.get(env.DOMAIN_DB.idFromName(domainSlug)) as any
  const loader = new BatchDataLoader(domainDO)
  return new DomainModel(loader, domainSlug)
}

/**
 * Build an AppModel from an App's navigable domains.
 * Resolves the App entity to find its domains, then merges all domain models
 * into one composite. The App is the RMAP compilation unit.
 */
async function buildAppModel(env: Env, appSlug: string): Promise<AppModel> {
  const registry = env.REGISTRY_DB.get(env.REGISTRY_DB.idFromName('global')) as any

  // Find the App entity to get its navigable domains
  const appIds: string[] = await registry.getEntityIds('App')
  let domainSlugs: string[] = []

  for (const appId of appIds) {
    const stub = env.ENTITY_DB.get(env.ENTITY_DB.idFromName(appId)) as any
    try {
      const entity = await stub.get()
      if (entity && (entity.id === appSlug || entity.data?.name === appSlug || entity.data?.appSlug === appSlug)) {
        // Found the App — collect navigable domain slugs from its data
        const domains = entity.data?.navigableDomain || entity.data?.navigableDomains || entity.data?.domains
        if (Array.isArray(domains)) {
          domainSlugs = domains
        } else if (typeof domains === 'string') {
          domainSlugs = domains.split(',').map((s: string) => s.trim()).filter(Boolean)
        }
        break
      }
    } catch { /* skip unreachable DOs */ }
  }

  // If no App entity found, try to resolve domain slugs from Registry
  // (Apps may have domain associations stored as separate facts)
  if (domainSlugs.length === 0) {
    const domainIds: string[] = await registry.getEntityIds('Domain')
    for (const did of domainIds) {
      const stub = env.ENTITY_DB.get(env.ENTITY_DB.idFromName(did)) as any
      try {
        const entity = await stub.get()
        if (entity?.data?.app === appSlug || entity?.data?.appId === appSlug) {
          domainSlugs.push(entity.id)
        }
      } catch { /* skip */ }
    }
  }

  if (domainSlugs.length === 0) {
    throw new Error(`No navigable domains found for app '${appSlug}'`)
  }

  const models = domainSlugs.map(slug => buildDomainModel(env, slug))
  return new AppModel(appSlug, models)
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
  const { domainId, appId, outputFormat = 'openapi' } = body

  if (!domainId && !appId) return error(400, { errors: [{ message: 'domainId or appId is required' }] })
  if (!VALID_FORMATS.includes(outputFormat)) {
    return error(400, { errors: [{ message: `Invalid outputFormat. Valid: ${VALID_FORMATS.join(', ')}` }] })
  }

  // Build model: AppModel (combined domains) or single DomainModel.
  // The App is the compilation unit — RMAP generates one spec from all navigable domains.
  let model: DomainModel | AppModel
  const targetId = appId || domainId
  if (appId) {
    model = await buildAppModel(env, appId)
  } else {
    model = buildDomainModel(env, domainId)
  }
  const output = await generateOutput(model, outputFormat)

  // Persist the generator output in DomainDB's generators table (cache)
  try {
    const db = env.DOMAIN_DB.get(env.DOMAIN_DB.idFromName('graphdl-primary')) as any
    const existing = await db.findInCollection('generators', {
      domain: { equals: targetId },
      outputFormat: { equals: outputFormat },
    }, { limit: 1 })

    const outputStr = typeof output === 'string' ? output : JSON.stringify(output)

    if (existing?.docs?.[0]) {
      await db.updateInCollection('generators', existing.docs[0].id, { output: outputStr })
    } else {
      await db.createInCollection('generators', {
        domain: targetId,
        outputFormat,
        output: outputStr,
      })
    }
  } catch {
    // Don't fail the response if persistence fails
  }

  return json({ output, format: outputFormat, ...(appId ? { appId } : { domainId }) })
}

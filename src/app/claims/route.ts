/**
 * Claims ingestion endpoint — the open-source deterministic entry point.
 *
 * POST /claims — accepts structured ExtractedClaims + domainId, creates
 * nouns/readings/constraints/transitions/facts via the claims module.
 *
 * GET /claims — returns database entity counts (diagnostic).
 */

import configPromise from '@payload-config'
import { getPayload } from 'payload'
import { ingestClaims } from '../../claims'

export const POST = async (request: Request) => {
  const payload = await getPayload({ config: configPromise })
  const body = await request.json()

  const { claims, domainId, domain } = body as {
    claims?: any
    domainId?: string
    domain?: string // domain slug fallback
  }

  if (!claims) {
    return Response.json({ error: 'claims is required' }, { status: 400 })
  }

  // Resolve domain: accept domainId directly or look up by slug
  let resolvedDomainId: string
  if (domainId) {
    resolvedDomainId = domainId
  } else if (domain) {
    const domainResult = await payload.find({
      collection: 'domains',
      where: { domainSlug: { equals: domain } },
      limit: 1,
    })
    if (domainResult.docs.length) {
      resolvedDomainId = domainResult.docs[0].id
    } else {
      const newDomain = await payload.create({
        collection: 'domains',
        data: { domainSlug: domain, name: domain },
      })
      resolvedDomainId = newDomain.id
    }
  } else {
    return Response.json({ error: 'domainId or domain slug is required' }, { status: 400 })
  }

  const result = await ingestClaims(payload, { claims, domainId: resolvedDomainId })

  return Response.json({
    totalNouns: result.nouns,
    totalReadings: result.readings,
    totalStateMachines: result.stateMachines,
    totalSkipped: result.skipped,
    totalErrors: result.errors.length,
    errors: result.errors,
  })
}

export const GET = async () => {
  const payload = await getPayload({ config: configPromise })

  const [nouns, readings, domains, stateMachines, eventTypes, transitions, verbs, functions, graphs, resources, resourceRoles] = await Promise.all([
    payload.count({ collection: 'nouns' }),
    payload.count({ collection: 'readings' }),
    payload.count({ collection: 'domains' }),
    payload.count({ collection: 'state-machine-definitions' }),
    payload.count({ collection: 'event-types' }),
    payload.count({ collection: 'transitions' }),
    payload.count({ collection: 'verbs' }),
    payload.count({ collection: 'functions' }),
    payload.count({ collection: 'graphs' }),
    payload.count({ collection: 'resources' }),
    payload.count({ collection: 'resource-roles' }),
  ])

  return Response.json({
    counts: {
      nouns: nouns.totalDocs,
      readings: readings.totalDocs,
      domains: domains.totalDocs,
      graphs: graphs.totalDocs,
      resources: resources.totalDocs,
      resourceRoles: resourceRoles.totalDocs,
      stateMachineDefinitions: stateMachines.totalDocs,
      eventTypes: eventTypes.totalDocs,
      transitions: transitions.totalDocs,
      verbs: verbs.totalDocs,
      functions: functions.totalDocs,
    },
    actions: {
      ingest: 'POST /claims with { claims: ExtractedClaims, domainId: string }',
    },
  })
}

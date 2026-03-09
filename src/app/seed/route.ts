import configPromise from '@payload-config'
import { getPayload } from 'payload'
import { parseDomainMarkdown, parseFORML2, parseStateMachineMarkdown } from '../../seed/parser'
import {
  type ExtractedClaims,
  type IngestClaimsResult,
  ingestClaims,
  domainParseToClaims,
  stateMachineParseToClaims,
  readingDefsToClaims,
} from '../../claims'

/** @deprecated Use POST /claims for structured claim ingestion instead. */
interface SeedFileInput {
  markdown: string
  text?: string // plain text input for unified parser
  type: 'domain' | 'state-machine' | 'forml2' | 'text' | 'claims'
  domain?: string // domain slug (looked up without tenant scoping — prefer domainId)
  domainId?: string // domain ID (bypasses slug lookup — use when caller already resolved the domain)
  entityNoun?: string // required for state-machine type
  claims?: ExtractedClaims // pre-parsed structured claims from LLM extraction
}

interface SeedResult {
  domain?: string
  nouns: number
  readings: number
  stateMachines: number
  skipped: number
  errors: string[]
}

/** Resolve domain ID from file input: accepts domainId directly, or looks up/creates by slug. */
async function resolveDomainId(payload: any, file: SeedFileInput): Promise<string | null> {
  if (file.domainId) return file.domainId
  if (!file.domain) return null
  const domainResult = await payload.find({
    collection: 'domains',
    where: { domainSlug: { equals: file.domain } },
    limit: 1,
  })
  if (domainResult.docs.length) return domainResult.docs[0].id
  const newDomain = await payload.create({
    collection: 'domains',
    data: { domainSlug: file.domain, name: file.domain },
  })
  return newDomain.id
}

/** Create a SeedResult with a single error message. */
function errorResult(domain: string | undefined, message: string): SeedResult {
  return { domain, nouns: 0, readings: 0, stateMachines: 0, skipped: 0, errors: [message] }
}

/** Convert IngestClaimsResult to SeedResult. */
function toSeedResult(domain: string | undefined, r: IngestClaimsResult): SeedResult {
  return {
    domain,
    nouns: r.nouns,
    readings: r.readings,
    stateMachines: r.stateMachines,
    skipped: r.skipped,
    errors: r.errors,
  }
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
    seed: {
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
      seed: 'POST /seed with { claims, type: "claims", domainId } or { text, type: "text", domain } or { markdown, type: "domain"|"state-machine"|"forml2" }',
      wipe: 'DELETE /seed — drops all collections except users',
    },
  })
}

export const POST = async (request: Request) => {
  const payload = await getPayload({ config: configPromise })
  const body = await request.json()

  // Normalize: accept single file or batch
  const files: SeedFileInput[] = body.files || [body]

  const results: SeedResult[] = []

  for (const file of files) {
    if (file.type === 'claims' && file.claims) {
      // Structured claims from LLM extraction — bypass the regex parser entirely
      const domainId = await resolveDomainId(payload, file)
      if (!domainId) {
        results.push(errorResult(file.domain, 'domain or domainId is required for type "claims"'))
        continue
      }

      const claimsResult = await ingestClaims(payload, { claims: file.claims, domainId })
      results.push(toSeedResult(file.domain, claimsResult))
    } else if (file.type === 'text' || file.text) {
      const { seedReadingsFromText } = await import('../../seed/unified')
      const inputText = file.text || file.markdown
      if (!inputText) {
        results.push(errorResult(file.domain, 'text field is required for type "text"'))
        continue
      }

      const domainId = await resolveDomainId(payload, file)
      if (!domainId) {
        results.push(errorResult(file.domain, 'domain is required for type "text"'))
        continue
      }

      const unifiedResult = await seedReadingsFromText(payload, {
        text: inputText,
        domainId,
      })
      results.push({
        domain: file.domain,
        nouns: unifiedResult.nounsCreated,
        readings: unifiedResult.readingsCreated,
        stateMachines: 0,
        skipped: 0,
        errors: unifiedResult.errors,
      })
    } else if (file.type === 'domain') {
      // Parse markdown -> convert to ExtractedClaims -> ingestClaims
      const domainId = await resolveDomainId(payload, file)
      if (!domainId) {
        results.push(errorResult(file.domain, 'domain or domainId is required for type "domain"'))
        continue
      }
      const parsed = parseDomainMarkdown(file.markdown)
      const claims = domainParseToClaims(parsed)
      const claimsResult = await ingestClaims(payload, { claims, domainId })
      results.push(toSeedResult(file.domain, claimsResult))
    } else if (file.type === 'state-machine') {
      if (!file.entityNoun) {
        results.push(errorResult(file.domain, 'entityNoun is required for state-machine type'))
        continue
      }
      const domainId = await resolveDomainId(payload, file)
      if (!domainId) {
        results.push(errorResult(file.domain, 'domain or domainId is required for type "state-machine"'))
        continue
      }
      const parsed = parseStateMachineMarkdown(file.markdown)
      const claims = stateMachineParseToClaims(parsed, file.entityNoun)
      const claimsResult = await ingestClaims(payload, { claims, domainId })
      results.push(toSeedResult(file.domain, claimsResult))
    } else if (file.type === 'forml2') {
      const domainId = await resolveDomainId(payload, file)
      if (!domainId) {
        results.push(errorResult(file.domain, 'domain or domainId is required for type "forml2"'))
        continue
      }
      const readingDefs = parseFORML2(file.markdown)
      const claims = readingDefsToClaims(readingDefs)
      const claimsResult = await ingestClaims(payload, { claims, domainId })
      results.push(toSeedResult(file.domain, claimsResult))
    } else {
      results.push({
        nouns: 0,
        readings: 0,
        stateMachines: 0,
        skipped: 0,
        errors: [`unknown type "${(file as any).type}" — use domain, state-machine, or forml2`],
      })
    }
  }

  const summary = {
    totalNouns: results.reduce((s, r) => s + r.nouns, 0),
    totalReadings: results.reduce((s, r) => s + r.readings, 0),
    totalStateMachines: results.reduce((s, r) => s + r.stateMachines, 0),
    totalSkipped: results.reduce((s, r) => s + r.skipped, 0),
    totalErrors: results.reduce((s, r) => s + r.errors.length, 0),
    files: results,
  }

  return Response.json(summary)
}

const WIPE_ORDER = [
  'readings', 'graph-schemas', 'roles', 'constraint-spans', 'constraints',
  'guards', 'guard-runs', 'transitions', 'verbs', 'functions', 'statuses', 'event-types', 'state-machine-definitions',
  'generators', 'graphs', 'json-examples', 'resources', 'resource-roles', 'nouns',
]

export const DELETE = async () => {
  const payload = await getPayload({ config: configPromise })

  // Drop all collections except users (preserves auth)
  const db = (payload.db as any).connection
  const collections = await db.db.listCollections().toArray()
  const dropped: string[] = []
  for (const col of collections) {
    if (col.name === 'users') continue
    await db.db.dropCollection(col.name)
    dropped.push(col.name)
  }

  return Response.json({ dropped })
}

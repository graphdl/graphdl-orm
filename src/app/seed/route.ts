import configPromise from '@payload-config'
import type { Payload } from 'payload'
import { getPayload } from 'payload'
import { parseDomainMarkdown, parseFORML2, parseStateMachineMarkdown } from '../../seed/parser'
import { seedDomain, seedReadings, seedStateMachine, seedInstanceFacts, type SeedResult } from '../../seed/handler'

interface SeedFileInput {
  markdown: string
  text?: string // plain text input for unified parser
  type: 'domain' | 'state-machine' | 'forml2' | 'text' | 'claims'
  domain?: string // domain slug (looked up without tenant scoping — prefer domainId)
  domainId?: string // domain ID (bypasses slug lookup — use when caller already resolved the domain)
  entityNoun?: string // required for state-machine type
  claims?: ExtractedClaims // pre-parsed structured claims from LLM extraction
}

/** Structured claims from LLM extraction — mirrors apis/graphdl/extract-claims.ts */
interface ExtractedClaims {
  nouns: Array<{
    name: string
    objectType: 'entity' | 'value'
    plural?: string
    valueType?: string
    format?: string
    enum?: string[]
    minimum?: number
    maximum?: number
    pattern?: string
  }>
  readings: Array<{
    text: string
    nouns: string[]
    predicate: string
    multiplicity?: string
  }>
  constraints: Array<{
    kind: 'UC' | 'MC'
    modality: 'Alethic' | 'Deontic'
    reading: string
    roles: number[]
  }>
  subtypes: Array<{ child: string; parent: string }>
  transitions: Array<{ entity: string; from: string; to: string; event: string }>
  facts: Array<{
    reading: string // references a reading text (fact type)
    values: Array<{ noun: string; value: string }> // concrete instance values
  }>
}

/** Ensure a noun exists for this domain; return the doc. */
async function ensureNoun(payload: Payload, name: string, data: Record<string, any>, domainId: string): Promise<any> {
  const existing = await payload.find({
    collection: 'nouns',
    where: { name: { equals: name }, domain: { equals: domainId } },
    limit: 1,
  })
  if (existing.docs.length) return existing.docs[0]
  return payload.create({ collection: 'nouns', data: { name, domain: domainId, ...data } })
}

/** Seed structured claims from LLM extraction into the domain model. */
async function seedFromClaims(payload: Payload, claims: ExtractedClaims, domainId: string): Promise<SeedResult> {
  const result: SeedResult = { nouns: 0, readings: 0, stateMachines: 0, skipped: 0, errors: [] }
  const nounMap = new Map<string, any>()

  // 1. Create all nouns with proper objectType, valueType, plural
  for (const noun of claims.nouns) {
    try {
      const data: Record<string, any> = { objectType: noun.objectType }
      if (noun.plural) data.plural = noun.plural
      if (noun.valueType) data.valueType = noun.valueType
      if (noun.format) data.format = noun.format
      if (noun.enum) data.enum = noun.enum
      if (noun.minimum !== undefined) data.minimum = noun.minimum
      if (noun.maximum !== undefined) data.maximum = noun.maximum
      if (noun.pattern) data.pattern = noun.pattern
      const doc = await ensureNoun(payload, noun.name, data, domainId)
      nounMap.set(noun.name, doc)
      result.nouns++
    } catch (err: any) {
      result.errors.push(`noun "${noun.name}": ${err.message}`)
    }
  }

  // 2. Apply subtypes
  for (const sub of claims.subtypes || []) {
    try {
      const child = nounMap.get(sub.child)
      const parent = nounMap.get(sub.parent)
      if (child && parent) {
        await payload.update({ collection: 'nouns', id: child.id, data: { superType: parent.id } })
      } else {
        result.errors.push(`subtype: "${sub.child}" or "${sub.parent}" not found`)
      }
    } catch (err: any) {
      result.errors.push(`subtype "${sub.child} → ${sub.parent}": ${err.message}`)
    }
  }

  // 3. Create graph schemas + readings, then apply constraints
  const schemaMap = new Map<string, any>() // reading text → schema doc

  for (const reading of claims.readings) {
    try {
      // Ensure all referenced nouns exist (LLM may reference nouns not in the nouns array)
      for (const nounName of reading.nouns) {
        if (!nounMap.has(nounName)) {
          const doc = await ensureNoun(payload, nounName, { objectType: 'entity' }, domainId)
          nounMap.set(nounName, doc)
          result.nouns++
        }
      }

      // Check if reading already exists
      const existingReading = await payload.find({
        collection: 'readings',
        where: { text: { equals: reading.text }, domain: { equals: domainId } },
        limit: 1,
      })
      if (existingReading.docs.length) {
        schemaMap.set(reading.text, { id: (existingReading.docs[0] as any).graphSchema })
        result.skipped++
        continue
      }

      const schemaName = reading.nouns.join('')
      const schema = await payload.create({
        collection: 'graph-schemas',
        data: { name: schemaName, title: schemaName, domain: domainId },
      })
      schemaMap.set(reading.text, schema)

      // Reading afterChange hook auto-creates roles by tokenizing text against known nouns
      await payload.create({
        collection: 'readings',
        data: { text: reading.text, graphSchema: schema.id, domain: domainId },
      } as any)
      result.readings++
    } catch (err: any) {
      result.errors.push(`reading "${reading.text}": ${err.message}`)
    }
  }

  // 4. Apply constraints — match by reading text, then apply to roles by index
  for (const constraint of claims.constraints || []) {
    try {
      const schema = schemaMap.get(constraint.reading)
      if (!schema) {
        result.errors.push(`constraint: reading "${constraint.reading}" not found in claims`)
        continue
      }

      const roles = await payload.find({
        collection: 'roles',
        where: { graphSchema: { equals: schema.id } },
        sort: 'createdAt',
      })

      const c = await payload.create({
        collection: 'constraints',
        data: { kind: constraint.kind, modality: constraint.modality },
      })

      const roleIds = constraint.roles
        .map((idx) => roles.docs[idx]?.id)
        .filter(Boolean)

      if (roleIds.length) {
        await payload.create({
          collection: 'constraint-spans',
          data: { roles: roleIds, constraint: c.id },
        } as any)
      }
    } catch (err: any) {
      result.errors.push(`constraint on "${constraint.reading}": ${err.message}`)
    }
  }

  // 5. Seed state machine transitions
  if (claims.transitions?.length) {
    // Group transitions by entity
    const byEntity = new Map<string, typeof claims.transitions>()
    for (const t of claims.transitions) {
      const group = byEntity.get(t.entity) || []
      group.push(t)
      byEntity.set(t.entity, group)
    }

    for (const [entityName, transitions] of byEntity) {
      try {
        const noun = nounMap.get(entityName)
        if (!noun) {
          result.errors.push(`transition entity "${entityName}" not found`)
          continue
        }

        // Ensure state machine definition
        const existingDef = await payload.find({
          collection: 'state-machine-definitions',
          where: { 'noun.value': { equals: noun.id } },
          limit: 1,
        })
        const definition = existingDef.docs.length
          ? existingDef.docs[0]
          : await payload.create({
              collection: 'state-machine-definitions',
              data: { noun: { relationTo: 'nouns', value: noun.id }, domain: domainId },
            })

        // Collect unique states and events
        const stateNames = new Set<string>()
        const eventNames = new Set<string>()
        for (const t of transitions) {
          stateNames.add(t.from)
          stateNames.add(t.to)
          eventNames.add(t.event)
        }

        // Ensure statuses
        const statusMap = new Map<string, string>()
        for (const name of stateNames) {
          const existing = await payload.find({
            collection: 'statuses',
            where: { name: { equals: name }, stateMachineDefinition: { equals: definition.id } },
            limit: 1,
          })
          const status = existing.docs.length
            ? existing.docs[0]
            : await payload.create({
                collection: 'statuses',
                data: { name, stateMachineDefinition: definition.id },
              })
          statusMap.set(name, status.id)
        }

        // Ensure event types
        const eventMap = new Map<string, string>()
        for (const name of eventNames) {
          const existing = await payload.find({
            collection: 'event-types',
            where: { name: { equals: name } },
            limit: 1,
          })
          const et = existing.docs.length
            ? existing.docs[0]
            : await payload.create({ collection: 'event-types', data: { name } })
          eventMap.set(name, et.id)
        }

        // Create transitions
        for (const t of transitions) {
          const fromId = statusMap.get(t.from)!
          const toId = statusMap.get(t.to)!
          const eventId = eventMap.get(t.event)!

          const existingT = await payload.find({
            collection: 'transitions',
            where: { from: { equals: fromId }, to: { equals: toId }, eventType: { equals: eventId } },
            limit: 1,
          })
          if (!existingT.docs.length) {
            await payload.create({
              collection: 'transitions',
              data: { from: fromId, to: toId, eventType: eventId },
            })
          }
        }

        result.stateMachines++
      } catch (err: any) {
        result.errors.push(`transitions for "${entityName}": ${err.message}`)
      }
    }
  }

  // 6. Seed instance facts (concrete instances of fact types)
  if (claims.facts?.length) {
    for (const fact of claims.facts) {
      try {
        // Build instance fact text in the expected format: "Customer 'John' has Email 'john@example.com'"
        let instanceText = fact.reading
        for (const v of fact.values) {
          // Replace the first occurrence of the noun name with noun + quoted value
          instanceText = instanceText.replace(v.noun, `${v.noun} '${v.value}'`)
        }

        await seedInstanceFacts(payload, [instanceText], { domain: domainId }, result)
      } catch (err: any) {
        result.errors.push(`fact "${fact.reading}": ${err.message}`)
      }
    }
  }

  return result
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
      let domainId: string
      if (file.domainId) {
        domainId = file.domainId
      } else if (file.domain) {
        const domainResult = await payload.find({
          collection: 'domains',
          where: { domainSlug: { equals: file.domain } },
          limit: 1,
        })
        if (domainResult.docs.length) {
          domainId = domainResult.docs[0].id
        } else {
          const newDomain = await payload.create({
            collection: 'domains',
            data: { domainSlug: file.domain, name: file.domain },
          })
          domainId = newDomain.id
        }
      } else {
        results.push({
          domain: file.domain,
          nouns: 0,
          readings: 0,
          stateMachines: 0,
          skipped: 0,
          errors: ['domain or domainId is required for type "claims"'],
        })
        continue
      }

      const claimsResult = await seedFromClaims(payload, file.claims, domainId)
      results.push({ domain: file.domain, ...claimsResult })
    } else if (file.type === 'text' || file.text) {
      const { seedReadingsFromText } = await import('../../seed/unified')
      const inputText = file.text || file.markdown
      if (!inputText) {
        results.push({
          domain: file.domain,
          nouns: 0,
          readings: 0,
          stateMachines: 0,
          skipped: 0,
          errors: ['text field is required for type "text"'],
        })
        continue
      }

      // Find or create domain — accept domainId directly or look up by slug
      let domainId: string
      if (file.domainId) {
        domainId = file.domainId
      } else if (file.domain) {
        const domainResult = await payload.find({
          collection: 'domains',
          where: { domainSlug: { equals: file.domain } },
          limit: 1,
        })
        if (domainResult.docs.length) {
          domainId = domainResult.docs[0].id
        } else {
          const newDomain = await payload.create({
            collection: 'domains',
            data: { domainSlug: file.domain, name: file.domain },
          })
          domainId = newDomain.id
        }
      } else {
        results.push({
          domain: file.domain,
          nouns: 0,
          readings: 0,
          stateMachines: 0,
          skipped: 0,
          errors: ['domain is required for type "text"'],
        })
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
      const parsed = parseDomainMarkdown(file.markdown)
      const result = await seedDomain(payload, parsed, file.domain)
      results.push(result)
    } else if (file.type === 'state-machine') {
      if (!file.entityNoun) {
        results.push({
          domain: file.domain,
          nouns: 0,
          readings: 0,
          stateMachines: 0,
          skipped: 0,
          errors: ['entityNoun is required for state-machine type'],
        })
        continue
      }
      const parsed = parseStateMachineMarkdown(file.markdown)
      const result = await seedStateMachine(payload, file.entityNoun, parsed, file.domain)
      results.push(result)
    } else if (file.type === 'forml2') {
      const readings = parseFORML2(file.markdown)
      const result: SeedResult = {
        domain: file.domain,
        nouns: 0,
        readings: 0,
        stateMachines: 0,
        skipped: 0,
        errors: [],
      }
      await seedReadings(payload, readings, file.domain ? { domain: file.domain } : {}, result)
      results.push(result)
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

/**
 * Core Ingestion Functions — consolidates:
 * - src/seed/unified.ts:seedReadingsFromText
 * - src/app/seed/route.ts:seedFromClaims
 * - src/seed/handler.ts:seedReadings
 *
 * Two entry points:
 * - ingestReading()  — single reading (creates schema, reading, applies multiplicity)
 * - ingestClaims()   — bulk structured claims (nouns, readings, constraints, transitions, facts)
 *
 * Key architectural decision: the Readings collection afterChange hook auto-creates
 * roles when a reading is created. So ingestReading() does NOT create roles directly.
 */

import type { Payload } from 'payload'
import { tokenizeReading } from './tokenize'
import { parseMultiplicity, applyConstraints } from './constraints'

// ── Types ──────────────────────────────────────────────────────────────────────

export interface ExtractedClaims {
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
  subtypes?: Array<{ child: string; parent: string }>
  transitions?: Array<{ entity: string; from: string; to: string; event: string }>
  facts?: Array<{
    reading: string
    values: Array<{ noun: string; value: string }>
  }>
}

export interface IngestReadingResult {
  graphSchemaId: string
  readingId: string
  errors: string[]
}

export interface IngestClaimsResult {
  nouns: number
  readings: number
  stateMachines: number
  skipped: number
  errors: string[]
}

// ── Helpers ────────────────────────────────────────────────────────────────────

/** Ensure a noun exists for this domain; return the doc. Updates objectType if it differs. */
async function ensureNoun(
  payload: Payload,
  name: string,
  data: Record<string, any>,
  domainId: string,
): Promise<any> {
  const existing = await payload.find({
    collection: 'nouns',
    where: { name: { equals: name }, domain: { equals: domainId } },
    limit: 1,
  })
  if (existing.docs.length) {
    const doc = existing.docs[0] as any
    // Update objectType if the caller knows better (e.g. LLM-classified value vs default entity)
    if (data.objectType && doc.objectType !== data.objectType) {
      return payload.update({ collection: 'nouns', id: doc.id, data: { objectType: data.objectType } })
    }
    return doc
  }
  return payload.create({ collection: 'nouns', data: { name, domain: domainId, ...data } })
}

// ── ingestReading ──────────────────────────────────────────────────────────────

/**
 * Ingest a single reading — creates graph schema if needed, creates reading
 * (hook auto-creates roles), applies constraints from multiplicity.
 */
export async function ingestReading(
  payload: Payload,
  opts: {
    text: string
    graphSchemaId?: string
    domainId?: string
    multiplicity?: string
  },
): Promise<IngestReadingResult> {
  const { text, domainId, multiplicity } = opts
  const errors: string[] = []

  // 1. Fetch all nouns in the domain to build the noun list
  const nounQuery: Record<string, any> = domainId ? { domain: { equals: domainId } } : {}
  const allNouns = await payload.find({
    collection: 'nouns',
    where: nounQuery,
    pagination: false,
  })
  const nounList = allNouns.docs.map((n: any) => ({ name: n.name as string, id: n.id as string }))

  // 2. Tokenize the reading text to find noun references
  const tokenized = tokenizeReading(text, nounList)

  // 3. If no graphSchemaId provided, create a graph schema named by joining noun names
  let graphSchemaId = opts.graphSchemaId || ''
  if (!graphSchemaId) {
    const schemaName = tokenized.nounRefs.length > 0
      ? tokenized.nounRefs.map((n) => n.name).join('')
      : text.split(' ').map((w) => w.charAt(0).toUpperCase() + w.slice(1)).join('')
    try {
      const schema = await payload.create({
        collection: 'graph-schemas',
        data: {
          name: schemaName,
          title: schemaName,
          ...(domainId ? { domain: domainId } : {}),
        },
      })
      graphSchemaId = schema.id
    } catch (err: any) {
      errors.push(`graph schema: ${err.message}`)
      return { graphSchemaId: '', readingId: '', errors }
    }
  }

  // 4. Check if reading already exists (same text + domain) — skip if so
  const existingWhere: Record<string, any> = { text: { equals: text } }
  if (domainId) existingWhere.domain = { equals: domainId }
  const existingReading = await payload.find({
    collection: 'readings',
    where: existingWhere,
    limit: 1,
  })
  if (existingReading.docs.length) {
    // Return the existing schema ID but empty readingId to signal skip
    const existingSchemaId = (existingReading.docs[0] as any).graphSchema
    return {
      graphSchemaId: typeof existingSchemaId === 'string' ? existingSchemaId : existingSchemaId?.id || graphSchemaId,
      readingId: '',
      errors,
    }
  }

  // 5. Create the reading (the afterChange hook auto-creates roles)
  let readingId = ''
  try {
    const reading = await payload.create({
      collection: 'readings',
      data: {
        text,
        graphSchema: graphSchemaId,
        ...(domainId ? { domain: domainId } : {}),
      },
    } as any)
    readingId = reading.id
  } catch (err: any) {
    errors.push(`reading: ${err.message}`)
    return { graphSchemaId, readingId: '', errors }
  }

  // 6. If multiplicity provided, fetch the roles, then apply constraints
  if (multiplicity) {
    try {
      const constraintDefs = parseMultiplicity(multiplicity)
      if (constraintDefs.length > 0) {
        const roles = await payload.find({
          collection: 'roles',
          where: { graphSchema: { equals: graphSchemaId } },
          sort: 'createdAt',
        })
        const roleIds = roles.docs.map((r: any) => r.id as string)
        await applyConstraints(payload, {
          constraints: constraintDefs,
          roleIds,
          domainId,
        })
      }
    } catch (err: any) {
      errors.push(`constraints: ${err.message}`)
    }
  }

  return { graphSchemaId, readingId, errors }
}

// ── ingestClaims ───────────────────────────────────────────────────────────────

/**
 * Ingest bulk structured claims — the open-source deterministic entry point.
 *
 * Follows the same 6-step flow as the original seedFromClaims:
 * 1. Create all nouns (idempotent via ensureNoun)
 * 2. Apply subtypes (set superType on child noun)
 * 3. Create graph schemas + readings (reuses ingestReading for each)
 * 4. Apply explicit constraints from claims.constraints
 * 5. Seed state machine transitions
 * 6. Seed instance facts
 */
export async function ingestClaims(
  payload: Payload,
  opts: {
    claims: ExtractedClaims
    domainId: string
  },
): Promise<IngestClaimsResult> {
  const { claims, domainId } = opts
  const result: IngestClaimsResult = { nouns: 0, readings: 0, stateMachines: 0, skipped: 0, errors: [] }
  const nounMap = new Map<string, any>()

  // ── Step 1: Create all nouns with proper objectType, valueType, plural ──
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

  // ── Step 2: Apply subtypes ──
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
      result.errors.push(`subtype "${sub.child} -> ${sub.parent}": ${err.message}`)
    }
  }

  // ── Step 3: Create graph schemas + readings ──
  // We create the schema ourselves and pass the schemaId to ingestReading,
  // because we need the schema map for step 4 (explicit constraints).
  const schemaMap = new Map<string, any>() // reading text -> schema doc

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

      // Create graph schema
      const schemaName = reading.nouns.join('')
      const schema = await payload.create({
        collection: 'graph-schemas',
        data: { name: schemaName, title: schemaName, domain: domainId },
      })
      schemaMap.set(reading.text, schema)

      // Create reading (afterChange hook auto-creates roles)
      await payload.create({
        collection: 'readings',
        data: { text: reading.text, graphSchema: schema.id, domain: domainId },
      } as any)
      result.readings++

      // Apply multiplicity constraints from the reading definition
      if (reading.multiplicity) {
        const constraintDefs = parseMultiplicity(reading.multiplicity)
        if (constraintDefs.length > 0) {
          const roles = await payload.find({
            collection: 'roles',
            where: { graphSchema: { equals: schema.id } },
            sort: 'createdAt',
          })
          const roleIds = roles.docs.map((r: any) => r.id as string)
          await applyConstraints(payload, {
            constraints: constraintDefs,
            roleIds,
            domainId,
          })
        }
      }
    } catch (err: any) {
      result.errors.push(`reading "${reading.text}": ${err.message}`)
    }
  }

  // ── Step 4: Apply explicit constraints from claims.constraints ──
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
        data: { kind: constraint.kind, modality: constraint.modality, domain: domainId } as any,
      })

      const roleIds = constraint.roles
        .map((idx) => roles.docs[idx]?.id)
        .filter(Boolean)

      if (roleIds.length) {
        await payload.create({
          collection: 'constraint-spans',
          data: { roles: roleIds, constraint: c.id, domain: domainId },
        } as any)
      }
    } catch (err: any) {
      result.errors.push(`constraint on "${constraint.reading}": ${err.message}`)
    }
  }

  // ── Step 5: Seed state machine transitions ──
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

  // ── Step 6: Seed instance facts ──
  if (claims.facts?.length) {
    for (const fact of claims.facts) {
      try {
        // Build instance fact text: "Customer 'John' has Email 'john@example.com'"
        let instanceText = fact.reading
        for (const v of fact.values) {
          instanceText = instanceText.replace(v.noun, `${v.noun} '${v.value}'`)
        }

        // Parse the instance fact into base reading + quoted values
        const instances: { entityType: string; value: string }[] = []
        const quotedPattern = /(\b[A-Z]\w*)\s+'([^']+)'/g
        let match
        while ((match = quotedPattern.exec(instanceText)) !== null) {
          instances.push({ entityType: match[1], value: match[2] })
        }
        if (!instances.length) continue

        // Build base reading text by removing quoted values
        let baseReading = instanceText
        for (const inst of instances) {
          baseReading = baseReading.replace(` '${inst.value}'`, '')
        }
        baseReading = baseReading.replace(/\s+/g, ' ').trim()

        // Find the reading for this fact type
        const readingResult = await payload.find({
          collection: 'readings',
          where: { text: { equals: baseReading } },
          limit: 1,
          depth: 1,
        })
        const reading = readingResult.docs[0]
        if (!reading) {
          result.errors.push(`fact: no reading found for base fact type "${baseReading}"`)
          continue
        }

        const graphSchemaId = typeof reading.graphSchema === 'string'
          ? reading.graphSchema
          : (reading.graphSchema as any)?.id
        if (!graphSchemaId) {
          result.errors.push(`fact: reading "${baseReading}" has no graph schema`)
          continue
        }

        // Ensure resources for each instance value
        const resources: any[] = []
        for (const inst of instances) {
          const resource = await ensureResource(payload, inst.entityType, inst.value, domainId)
          if (resource) {
            resources.push(resource)
          } else {
            result.errors.push(`fact: could not create resource for ${inst.entityType} '${inst.value}'`)
          }
        }

        // Create the graph (instance of the fact type)
        const graph = await payload.create({
          collection: 'graphs',
          data: { type: graphSchemaId, domain: domainId },
        })

        // Link resources via resource-roles
        const roles: any[] = Array.isArray(reading.roles) ? reading.roles : []
        for (let i = 0; i < resources.length && i < roles.length; i++) {
          const roleId = typeof roles[i] === 'string' ? roles[i] : roles[i]?.id
          if (!roleId) continue
          await payload.create({
            collection: 'resource-roles',
            data: {
              graph: graph.id,
              resource: { relationTo: 'resources', value: resources[i].id },
              role: roleId,
              domain: domainId,
            },
          })
        }
      } catch (err: any) {
        result.errors.push(`fact "${fact.reading}": ${err.message}`)
      }
    }
  }

  return result
}

// ── Instance fact helpers ──────────────────────────────────────────────────────

/** Ensure a resource exists for the given noun type and value. */
async function ensureResource(
  payload: Payload,
  nounName: string,
  value: string,
  domainId: string,
): Promise<any> {
  const nounResult = await payload.find({
    collection: 'nouns',
    where: { name: { equals: nounName }, domain: { equals: domainId } },
    limit: 1,
  })
  const noun = nounResult.docs[0]
  if (!noun) return null

  const existing = await payload.find({
    collection: 'resources',
    where: { type: { equals: noun.id }, value: { equals: value } },
    limit: 1,
  })
  if (existing.docs.length) return existing.docs[0]

  return payload.create({
    collection: 'resources',
    data: { type: noun.id, value, domain: domainId },
  })
}

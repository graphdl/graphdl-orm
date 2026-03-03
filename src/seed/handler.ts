/**
 * Seed handler: accepts parsed domain/state-machine definitions and creates
 * ORM entities via the Payload local API with idempotent upserts.
 */

import type { Payload } from 'payload'
import type { DomainParseResult, ReadingDef, StateMachineParseResult } from './parser'

const MULT_MAP: Record<string, string> = {
  '*:1': 'many-to-one',
  '1:*': 'one-to-many',
  '*:*': 'many-to-many',
  '1:1': 'one-to-one',
}

async function ensureNoun(payload: Payload, data: Record<string, any>): Promise<any> {
  const existing = await payload.find({
    collection: 'nouns',
    where: { name: { equals: data.name } },
    limit: 1,
  })
  if (existing.docs.length) return existing.docs[0]
  return payload.create({ collection: 'nouns', data })
}

async function ensureEventType(payload: Payload, name: string): Promise<any> {
  const existing = await payload.find({
    collection: 'event-types',
    where: { name: { equals: name } },
    limit: 1,
  })
  if (existing.docs.length) return existing.docs[0]
  return payload.create({ collection: 'event-types', data: { name } })
}

export interface SeedResult {
  domain?: string
  nouns: number
  readings: number
  stateMachines: number
  skipped: number
  errors: string[]
}

export async function seedDomain(
  payload: Payload,
  parsed: DomainParseResult,
  domain?: string,
): Promise<SeedResult> {
  const result: SeedResult = { domain, nouns: 0, readings: 0, stateMachines: 0, skipped: 0, errors: [] }
  const domainData = domain ? { domain } : {}

  // 1. Create value nouns
  for (const v of parsed.valueTypes) {
    try {
      const data: Record<string, any> = { name: v.name, objectType: 'value', valueType: v.valueType, ...domainData }
      if (v.format) data.format = v.format
      if (v.enum) data.enum = v.enum
      if (v.pattern) data.pattern = v.pattern
      if (v.minimum !== undefined) data.minimum = v.minimum
      if (v.maximum !== undefined) data.maximum = v.maximum
      await ensureNoun(payload, data)
      result.nouns++
    } catch (e: any) {
      result.errors.push(`value noun ${v.name}: ${e.message}`)
    }
  }

  // 2. Create entity nouns
  for (const e of parsed.entityTypes) {
    try {
      await ensureNoun(payload, {
        name: e.name,
        objectType: 'entity',
        plural: e.name.toLowerCase().replace(/([A-Z])/g, '-$1').replace(/^-/, '') + 's',
        permissions: ['create', 'read', 'update', 'list'],
        ...domainData,
      })
      result.nouns++
    } catch (err: any) {
      result.errors.push(`entity noun ${e.name}: ${err.message}`)
    }
  }

  // 3. Set reference schemes
  for (const e of parsed.entityTypes) {
    try {
      const noun = await payload.find({ collection: 'nouns', where: { name: { equals: e.name } }, limit: 1 })
      if (!noun.docs.length) continue
      const refIds = []
      for (const refName of e.referenceScheme) {
        const refNoun = await payload.find({ collection: 'nouns', where: { name: { equals: refName } }, limit: 1 })
        if (refNoun.docs.length) refIds.push(refNoun.docs[0].id)
      }
      if (refIds.length) {
        await payload.update({ collection: 'nouns', id: noun.docs[0].id, data: { referenceScheme: refIds } })
      }
    } catch (err: any) {
      result.errors.push(`ref scheme ${e.name}: ${err.message}`)
    }
  }

  // 4. Create readings
  await seedReadings(payload, parsed.readings, domainData, result)

  return result
}

export async function seedReadings(
  payload: Payload,
  readings: ReadingDef[],
  domainData: Record<string, any>,
  result: SeedResult,
): Promise<void> {
  for (const r of readings) {
    const rel = MULT_MAP[r.multiplicity]
    if (!rel) {
      result.errors.push(`unknown multiplicity "${r.multiplicity}": ${r.text}`)
      result.skipped++
      continue
    }
    try {
      // Check if reading already exists
      const existing = await payload.find({
        collection: 'readings',
        where: { text: { equals: r.text } },
        limit: 1,
      })
      if (existing.docs.length) {
        result.skipped++
        continue
      }

      const name = r.text.split(' ').map((w) => w.charAt(0).toUpperCase() + w.slice(1)).join('')
      const schema = await payload.create({ collection: 'graph-schemas', data: { name, title: name, ...domainData } as any })
      await payload.create({ collection: 'readings', data: { text: r.text, graphSchema: schema.id, ...domainData } as any })
      await payload.update({ collection: 'graph-schemas', id: schema.id, data: { roleRelationship: rel } as any })
      result.readings++
    } catch (err: any) {
      result.errors.push(`reading "${r.text}": ${err.message}`)
    }
  }
}

export async function seedStateMachine(
  payload: Payload,
  entityNounName: string,
  parsed: StateMachineParseResult,
  domain?: string,
): Promise<SeedResult> {
  const result: SeedResult = { domain, nouns: 0, readings: 0, stateMachines: 0, skipped: 0, errors: [] }
  const domainData = domain ? { domain } : {}

  try {
    const noun = await payload.find({ collection: 'nouns', where: { name: { equals: entityNounName } }, limit: 1 })
    if (!noun.docs.length) {
      result.errors.push(`entity noun "${entityNounName}" not found — create domain readings first`)
      return result
    }

    const definition = await payload.create({
      collection: 'state-machine-definitions',
      data: { noun: { relationTo: 'nouns', value: noun.docs[0].id }, ...domainData },
    })

    const statusMap = new Map<string, string>()
    for (const s of parsed.states) {
      const status = await payload.create({
        collection: 'statuses',
        data: { name: s, stateMachineDefinition: definition.id },
      })
      statusMap.set(s, status.id)
    }

    const eventTypeCache = new Map<string, string>()
    for (const t of parsed.transitions) {
      if (!eventTypeCache.has(t.event)) {
        const et = await ensureEventType(payload, t.event)
        eventTypeCache.set(t.event, et.id)
      }
      await payload.create({
        collection: 'transitions',
        data: {
          from: statusMap.get(t.from)!,
          to: statusMap.get(t.to)!,
          eventType: eventTypeCache.get(t.event)!,
        },
      })
    }

    result.stateMachines++
  } catch (err: any) {
    result.errors.push(`state machine for ${entityNounName}: ${err.message}`)
  }

  return result
}

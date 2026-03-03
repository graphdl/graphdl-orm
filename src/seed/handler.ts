/**
 * Seed handler: accepts parsed domain/state-machine definitions and creates
 * ORM entities via the Payload local API with idempotent upserts.
 *
 * Creates independent entities in parallel batches:
 * 1. All nouns (value + entity) — no dependencies
 * 2. Reference schemes — depends on noun IDs
 * 3. Graph schemas + readings — depends on nouns (for role auto-creation)
 * 4. Constraints — depends on roles (created by reading hooks)
 */

import type { Payload } from 'payload'
import type { DomainParseResult, ReadingDef, StateMachineParseResult } from './parser'

async function batch<T>(
  items: T[],
  fn: (item: T) => Promise<void>,
  concurrency = 5,
): Promise<void> {
  for (let i = 0; i < items.length; i += concurrency) {
    await Promise.all(items.slice(i, i + concurrency).map(fn))
  }
}

const MULT_MAP: Record<string, string> = {
  '*:1': 'many-to-one',
  '1:*': 'one-to-many',
  '*:*': 'many-to-many',
  '1:1': 'one-to-one',
  ternary: 'many-to-many',
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
  const result: SeedResult = {
    domain,
    nouns: 0,
    readings: 0,
    stateMachines: 0,
    skipped: 0,
    errors: [],
  }
  const domainData = domain ? { domain } : {}

  // ── Batch 1: All nouns (no dependencies) ──
  const allNounDefs = [
    ...parsed.valueTypes.map((v) => {
      const data: Record<string, any> = {
        name: v.name,
        objectType: 'value',
        valueType: v.valueType,
        ...domainData,
      }
      if (v.format) data.format = v.format
      if (v.enum) data.enum = v.enum
      if (v.pattern) data.pattern = v.pattern
      if (v.minimum !== undefined) data.minimum = v.minimum
      if (v.maximum !== undefined) data.maximum = v.maximum
      return { data, label: `value noun ${v.name}` }
    }),
    ...parsed.entityTypes.map((e) => ({
      data: {
        name: e.name,
        objectType: 'entity',
        plural:
          e.name
            .toLowerCase()
            .replace(/([A-Z])/g, '-$1')
            .replace(/^-/, '') + 's',
        permissions: ['create', 'read', 'update', 'list'],
        ...domainData,
      },
      label: `entity noun ${e.name}`,
    })),
  ]
  await batch(allNounDefs, async (n) => {
    try {
      await ensureNoun(payload, n.data)
      result.nouns++
    } catch (e: any) {
      result.errors.push(`${n.label}: ${e.message}`)
    }
  })

  // ── Batch 2: Reference schemes in parallel (depends on noun IDs) ──
  const nounCache = new Map<string, any>()
  const allNouns = await payload.find({
    collection: 'nouns',
    pagination: false,
    where: domain ? { domain: { equals: domain } } : {},
  })
  for (const n of allNouns.docs) nounCache.set((n as any).name, n)

  await batch(parsed.entityTypes, async (e) => {
    try {
      const noun = nounCache.get(e.name)
      if (!noun) return
      const refIds = e.referenceScheme.map((name) => nounCache.get(name)?.id).filter(Boolean)
      if (refIds.length) {
        await payload.update({
          collection: 'nouns',
          id: noun.id,
          data: { referenceScheme: refIds },
        })
      }
    } catch (err: any) {
      result.errors.push(`ref scheme ${e.name}: ${err.message}`)
    }
  })

  // ── Batch 3: Readings ──
  await seedReadings(payload, parsed.readings, domainData, result)

  return result
}

export async function seedReadings(
  payload: Payload,
  readings: ReadingDef[],
  domainData: Record<string, any>,
  result: SeedResult,
): Promise<void> {
  // Check which readings already exist in one query
  const existingReadings = await payload.find({
    collection: 'readings',
    where: { text: { in: readings.map((r) => r.text) } },
    pagination: false,
  })
  const existingTexts = new Set(existingReadings.docs.map((d: any) => d.text))

  // Separate new readings from existing
  const newReadings: ReadingDef[] = []
  for (const r of readings) {
    if (existingTexts.has(r.text)) {
      result.skipped++
    } else {
      newReadings.push(r)
    }
  }

  // ── Batch 3a: Create all graph schemas ──
  const schemaMap = new Map<string, any>()
  await batch(newReadings, async (r) => {
    try {
      const name = r.text
        .split(' ')
        .map((w) => w.charAt(0).toUpperCase() + w.slice(1))
        .join('')
      const schema = await payload.create({
        collection: 'graph-schemas',
        data: { name, title: name, ...domainData } as any,
      })
      schemaMap.set(r.text, schema)
    } catch (err: any) {
      result.errors.push(`schema for "${r.text}": ${err.message}`)
    }
  })

  // ── Batch 3b: Create all readings (triggers role hooks) ──
  await batch(newReadings, async (r) => {
    const schema = schemaMap.get(r.text)
    if (!schema) return
    try {
      await payload.create({
        collection: 'readings',
        data: { text: r.text, graphSchema: schema.id, ...domainData } as any,
      })
    } catch (err: any) {
      result.errors.push(`reading "${r.text}": ${err.message}`)
    }
  })

  // ── Batch 3c: Set roleRelationships + UC constraints ──
  await batch(newReadings, async (r) => {
    const schema = schemaMap.get(r.text)
    if (!schema) return
    try {
      if (r.ucs?.length) {
        const roles = await payload.find({
          collection: 'roles',
          where: { graphSchema: { equals: schema.id } },
          depth: 2,
          pagination: false,
        })
        for (const ucRoleNames of r.ucs) {
          const ucRoleIds = ucRoleNames
            .map((roleName) => {
              const role = roles.docs.find((role: any) => {
                const noun = role.noun
                const nounName = typeof noun === 'string' ? null : noun?.value?.name || noun?.name
                return nounName === roleName
              })
              return role?.id
            })
            .filter((id): id is string => !!id)

          if (ucRoleIds.length) {
            const constraint = await payload.create({
              collection: 'constraints',
              data: { kind: 'UC', modality: 'Alethic' },
            })
            await payload.create({
              collection: 'constraint-spans',
              data: { constraint: constraint.id, roles: ucRoleIds },
            } as any)
          }
        }
      } else {
        const rel = MULT_MAP[r.multiplicity]
        if (!rel) {
          result.errors.push(`unknown multiplicity "${r.multiplicity}": ${r.text}`)
          return
        }
        await payload.update({
          collection: 'graph-schemas',
          id: schema.id,
          data: { roleRelationship: rel } as any,
        })
      }
      result.readings++
    } catch (err: any) {
      result.errors.push(`constraint "${r.text}": ${err.message}`)
    }
  })
}

export async function seedStateMachine(
  payload: Payload,
  entityNounName: string,
  parsed: StateMachineParseResult,
  domain?: string,
): Promise<SeedResult> {
  const result: SeedResult = {
    domain,
    nouns: 0,
    readings: 0,
    stateMachines: 0,
    skipped: 0,
    errors: [],
  }
  const domainData = domain ? { domain } : {}

  try {
    const noun = await payload.find({
      collection: 'nouns',
      where: { name: { equals: entityNounName } },
      limit: 1,
    })
    if (!noun.docs.length) {
      result.errors.push(`entity noun "${entityNounName}" not found — create domain readings first`)
      return result
    }

    const definition = await payload.create({
      collection: 'state-machine-definitions',
      data: { noun: { relationTo: 'nouns', value: noun.docs[0].id }, ...domainData },
    })

    // ── Batch: All statuses ──
    const statusMap = new Map<string, string>()
    await batch(parsed.states, async (s) => {
      const status = await payload.create({
        collection: 'statuses',
        data: { name: s, stateMachineDefinition: definition.id },
      })
      statusMap.set(s, status.id)
    })

    // ── Batch: All event types ──
    const uniqueEvents = [...new Set(parsed.transitions.map((t) => t.event))]
    const eventTypeCache = new Map<string, string>()
    await batch(uniqueEvents, async (event) => {
      const et = await ensureEventType(payload, event)
      eventTypeCache.set(event, et.id)
    })

    // ── Batch: All transitions ──
    await batch(parsed.transitions, async (t) => {
      await payload.create({
        collection: 'transitions',
        data: {
          from: statusMap.get(t.from)!,
          to: statusMap.get(t.to)!,
          eventType: eventTypeCache.get(t.event)!,
        },
      })
    })

    result.stateMachines++
  } catch (err: any) {
    result.errors.push(`state machine for ${entityNounName}: ${err.message}`)
  }

  return result
}

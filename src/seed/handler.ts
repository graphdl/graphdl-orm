/**
 * Seed handler: accepts parsed domain/state-machine definitions and creates
 * ORM entities via the Payload local API with idempotent upserts.
 *
 * Readings are the source of truth. The handler:
 * 1. Creates nouns (entity + value types)
 * 2. Creates graph schemas + readings (hooks auto-detect nouns, create roles)
 * 3. Applies uniqueness constraints from multiplicity notation
 * 4. Handles subtype declarations by setting noun.superType
 *
 * Multiplicity notation is shorthand for uniqueness constraints:
 *   *:1  → UC on role 0 (subject)
 *   1:*  → UC on role 1 (object)
 *   1:1  → UC on role 0 AND UC on role 1 (two separate constraints)
 *   *:*  → UC spanning both roles (pair uniqueness)
 *   UC(A,B) → explicit UC spanning named roles
 *   unary → no constraint (single-role boolean fact)
 *   subtype → sets noun.superType relationship
 */

import type { Payload } from 'payload'
import type { DomainParseResult, ReadingDef, StateMachineParseResult } from './parser'

async function batch<T>(
  items: T[],
  fn: (item: T) => Promise<void>,
  concurrency = 5,
): Promise<void> {
  if (!items.length) return
  // First item runs alone to initialize MongoDB collection + indexes
  await fn(items[0])
  // Rest run in parallel batches
  for (let i = 1; i < items.length; i += concurrency) {
    await Promise.all(items.slice(i, i + concurrency).map(fn))
  }
}

async function ensureDomain(payload: Payload, domainSlug: string): Promise<string> {
  const existing = await payload.find({
    collection: 'domains',
    where: { domainSlug: { equals: domainSlug } },
    limit: 1,
  })
  if (existing.docs.length) return existing.docs[0].id
  const created = await payload.create({
    collection: 'domains',
    data: { domainSlug, name: domainSlug },
  })
  return created.id
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

/**
 * Parse a compound constraint spec into its parts.
 *
 * The multiplicity column is a constraint specification. Alethic modality is
 * implicit; prefix with D for deontic.
 *
 *   UC shorthand:   *:1, 1:*, *:*, 1:1       (Alethic)
 *   Deontic UC:     D*:1, D1:*, D*:*, D1:1
 *   Explicit UC:    UC(A,B), DUC(A,B)
 *   Mandatory:      MC (Alethic), DMC (Deontic)
 *   Subset:         SS (Alethic), DSS (Deontic)
 *   Standalone:     subtype, unary
 *
 * Compound: "*:1 MC" = Alethic UC + Alethic MC
 *           "D*:1 DMC" = Deontic UC + Deontic MC
 */
interface ParsedConstraint {
  kind: 'UC' | 'MC'
  modality: 'Alethic' | 'Deontic'
  uc?: string           // *:1, 1:*, *:*, 1:1
  ucs?: string[][]      // UC(A,B) explicit notation
}

interface ParsedConstraints {
  constraints: ParsedConstraint[]
  skip?: boolean        // unary, subtype, SS — handled elsewhere
}

function parseConstraintSpec(reading: ReadingDef): ParsedConstraints {
  const mult = reading.multiplicity
  if (mult === 'unary' || mult === 'subtype') return { constraints: [], skip: true }
  if (/^D?SS$/i.test(mult.split(/\s+/)[0])) return { constraints: [], skip: true }

  if (reading.ucs?.length) {
    const deontic = /^D/i.test(mult)
    return {
      constraints: [{ kind: 'UC', modality: deontic ? 'Deontic' : 'Alethic', ucs: reading.ucs }],
    }
  }

  const parts = mult.split(/\s+/)
  const constraints: ParsedConstraint[] = []

  for (const part of parts) {
    // Deontic UC: D*:1, D1:*, etc.
    const ducMatch = part.match(/^D([*1]:[*1])$/i)
    if (ducMatch) {
      constraints.push({ kind: 'UC', modality: 'Deontic', uc: ducMatch[1] })
      continue
    }
    // Alethic UC: *:1, 1:*, etc.
    if (/^[*1]:[*1]$/.test(part)) {
      constraints.push({ kind: 'UC', modality: 'Alethic', uc: part })
      continue
    }
    // Deontic MC
    if (/^DMC$/i.test(part)) {
      constraints.push({ kind: 'MC', modality: 'Deontic' })
      continue
    }
    // Alethic MC (AMC or just MC)
    if (/^A?MC$/i.test(part)) {
      constraints.push({ kind: 'MC', modality: 'Alethic' })
      continue
    }
  }

  return { constraints }
}

/**
 * Apply all constraints from a reading's constraint spec.
 *
 * Every constraint is a record with a kind and modality. Alethic is implicit;
 * D prefix makes it deontic. Multiple constraints can be combined per reading.
 */
async function applyConstraints(
  payload: Payload,
  schemaId: string,
  reading: ReadingDef,
  result: SeedResult,
): Promise<void> {
  const spec = parseConstraintSpec(reading)
  if (spec.skip) return

  if (!spec.constraints.length) {
    result.errors.push(`unknown constraint notation "${reading.multiplicity}": ${reading.text}`)
    return
  }

  // Fetch roles for this schema
  const roles = await payload.find({
    collection: 'roles',
    where: { graphSchema: { equals: schemaId } },
    depth: 2,
    sort: 'createdAt',
    pagination: false,
  })

  for (const constraint of spec.constraints) {
    // Explicit UC notation: UC(Noun1,Noun2)
    if (constraint.ucs?.length) {
      for (const ucRoleNames of constraint.ucs) {
        const ucRoleIds = ucRoleNames
          .map((roleName) => {
            const role = roles.docs.find((r: any) => {
              const noun = r.noun
              const nounName = typeof noun === 'string' ? null : noun?.value?.name || noun?.name
              return nounName === roleName
            })
            return role?.id
          })
          .filter((id): id is string => !!id)

        if (ucRoleIds.length) {
          const c = await payload.create({
            collection: 'constraints',
            data: { kind: 'UC', modality: constraint.modality },
          })
          await payload.create({
            collection: 'constraint-spans',
            data: { constraint: c.id, roles: ucRoleIds },
          } as any)
        }
      }
      continue
    }

    // Binary UC shorthand
    if (constraint.kind === 'UC' && constraint.uc && roles.docs.length >= 2) {
      const role0 = roles.docs[0]
      const role1 = roles.docs[1]

      if (constraint.uc === '*:1') {
        const c = await payload.create({ collection: 'constraints', data: { kind: 'UC', modality: constraint.modality } })
        await payload.create({ collection: 'constraint-spans', data: { constraint: c.id, roles: [role0.id] } } as any)
      } else if (constraint.uc === '1:*') {
        const c = await payload.create({ collection: 'constraints', data: { kind: 'UC', modality: constraint.modality } })
        await payload.create({ collection: 'constraint-spans', data: { constraint: c.id, roles: [role1.id] } } as any)
      } else if (constraint.uc === '*:*') {
        const c = await payload.create({ collection: 'constraints', data: { kind: 'UC', modality: constraint.modality } })
        await payload.create({ collection: 'constraint-spans', data: { constraint: c.id, roles: [role0.id, role1.id] } } as any)
      } else if (constraint.uc === '1:1') {
        const c0 = await payload.create({ collection: 'constraints', data: { kind: 'UC', modality: constraint.modality } })
        const c1 = await payload.create({ collection: 'constraints', data: { kind: 'UC', modality: constraint.modality } })
        await Promise.all([
          payload.create({ collection: 'constraint-spans', data: { constraint: c0.id, roles: [role0.id] } } as any),
          payload.create({ collection: 'constraint-spans', data: { constraint: c1.id, roles: [role1.id] } } as any),
        ])
      }
      continue
    }

    // Mandatory constraint — applies to object role (last role)
    if (constraint.kind === 'MC' && roles.docs.length >= 1) {
      const objectRole = roles.docs[roles.docs.length - 1]
      const c = await payload.create({
        collection: 'constraints',
        data: { kind: 'MC', modality: constraint.modality },
      })
      await payload.create({
        collection: 'constraint-spans',
        data: { constraint: c.id, roles: [objectRole.id] },
      } as any)
      continue
    }
  }
}

/**
 * Apply a subset constraint from a verbalization like:
 * "If some StateMachine is currently in some Status then that Status is defined in
 *  some StateMachineDefinition where that StateMachine is instance of that StateMachineDefinition"
 *
 * The verbalization uses "some" to introduce entity bindings and "that" to refer back.
 * The subset constraint says: the roles in the "if" fact must be a subset of the
 * corresponding roles in the "then"/"where" facts, matched by entity type.
 *
 * Creates Constraint(kind: 'SS') with two ConstraintSpans:
 *   - Subset span: roles from the "if" clause
 *   - Superset span: matching roles from the "then"/"where" clauses
 */
async function applySubsetConstraint(
  payload: Payload,
  text: string,
  modality: 'Alethic' | 'Deontic',
  result: SeedResult,
): Promise<void> {
  // Parse "If ... then ... where ..." clauses
  const clauseMatch = text.match(
    /^If\s+(.+?)\s+then\s+(.+?)(?:\s+where\s+(.+))?$/i,
  )
  if (!clauseMatch) {
    result.errors.push(`could not parse subset constraint: ${text}`)
    return
  }

  const [, ifClause, thenClause, whereClause] = clauseMatch
  const allClauses = [ifClause, thenClause, ...(whereClause ? [whereClause] : [])]

  // Parse each clause into entity bindings and a fact pattern.
  // "some X verb some Y" or "that X verb that Y" or "that X verb some Y"
  // We extract the noun names and the reading text (with nouns substituted back in).
  interface ClauseBinding {
    readingText: string
    nouns: string[] // noun names in order of appearance
  }

  // Fetch all nouns to build a regex for detection
  const allNouns = await payload.find({ collection: 'nouns', pagination: false })
  const nounNames = allNouns.docs.map((n: any) => n.name).sort((a: string, b: string) => b.length - a.length)
  const nounRegex = new RegExp(`\\b(some|that)\\s+(${nounNames.join('|')})\\b`, 'g')

  const parsedClauses: ClauseBinding[] = allClauses.map((clause) => {
    const nouns: string[] = []
    // Replace "some X" / "that X" with just "X" to get the reading text
    const readingText = clause.replace(nounRegex, (_match, _quantifier, nounName) => {
      nouns.push(nounName)
      return nounName
    }).trim()
    return { readingText, nouns }
  })

  // Find the graph schemas + roles for each clause by matching reading text
  const subsetClause = parsedClauses[0]
  const supersetClauses = parsedClauses.slice(1)

  // Find subset reading's roles
  const subsetReading = await payload.find({
    collection: 'readings',
    where: { text: { equals: subsetClause.readingText } },
    limit: 1,
    depth: 0,
  })
  if (!subsetReading.docs.length) {
    result.errors.push(`subset constraint: reading "${subsetClause.readingText}" not found`)
    return
  }
  const subsetSchemaId = (subsetReading.docs[0] as any).graphSchema
  const subsetRoles = await payload.find({
    collection: 'roles',
    where: { graphSchema: { equals: subsetSchemaId } },
    depth: 2,
    sort: 'createdAt',
    pagination: false,
  })

  // Map noun names to role IDs in the subset fact
  const subsetRoleIds: string[] = []
  for (const nounName of subsetClause.nouns) {
    const role = subsetRoles.docs.find((r: any) => {
      const noun = r.noun
      const name = typeof noun === 'string' ? null : noun?.value?.name || noun?.name
      return name === nounName
    })
    if (role) subsetRoleIds.push(role.id)
  }

  // Find superset roles: for each noun in the subset, find the matching role
  // in the superset clauses (the "then" and "where" facts)
  const supersetRoleIds: string[] = []
  for (const nounName of subsetClause.nouns) {
    // Find which superset clause contains this noun
    for (const superClause of supersetClauses) {
      if (!superClause.nouns.includes(nounName)) continue

      const superReading = await payload.find({
        collection: 'readings',
        where: { text: { equals: superClause.readingText } },
        limit: 1,
        depth: 0,
      })
      if (!superReading.docs.length) continue

      const superSchemaId = (superReading.docs[0] as any).graphSchema
      const superRoles = await payload.find({
        collection: 'roles',
        where: { graphSchema: { equals: superSchemaId } },
        depth: 2,
        sort: 'createdAt',
        pagination: false,
      })

      const matchingRole = superRoles.docs.find((r: any) => {
        const noun = r.noun
        const name = typeof noun === 'string' ? null : noun?.value?.name || noun?.name
        return name === nounName
      })
      if (matchingRole) {
        supersetRoleIds.push(matchingRole.id)
        break
      }
    }
  }

  if (!subsetRoleIds.length || !supersetRoleIds.length) {
    result.errors.push(`subset constraint: could not resolve roles for "${text}"`)
    return
  }

  // Create the SS constraint with two spans
  const constraint = await payload.create({
    collection: 'constraints',
    data: { kind: 'SS', modality },
  })

  // Subset span (the "if" roles)
  await payload.create({
    collection: 'constraint-spans',
    data: { constraint: constraint.id, roles: subsetRoleIds },
  } as any)

  // Superset span (the "then"/"where" roles)
  await payload.create({
    collection: 'constraint-spans',
    data: { constraint: constraint.id, roles: supersetRoleIds },
  } as any)
}

/**
 * Handle subtype declarations: "X is a subtype of Y" → set noun.superType.
 */
async function applySubtype(
  payload: Payload,
  text: string,
  result: SeedResult,
): Promise<void> {
  const match = text.match(/^(\S+)\s+is\s+a\s+subtype\s+of\s+(\S+)$/i)
  if (!match) {
    result.errors.push(`could not parse subtype declaration: ${text}`)
    return
  }
  const [, childName, parentName] = match
  const [childResult, parentResult] = await Promise.all([
    payload.find({ collection: 'nouns', where: { name: { equals: childName } }, limit: 1 }),
    payload.find({ collection: 'nouns', where: { name: { equals: parentName } }, limit: 1 }),
  ])
  if (!childResult.docs.length) {
    result.errors.push(`subtype child noun "${childName}" not found`)
    return
  }
  if (!parentResult.docs.length) {
    result.errors.push(`subtype parent noun "${parentName}" not found`)
    return
  }
  await payload.update({
    collection: 'nouns',
    id: childResult.docs[0].id,
    data: { superType: parentResult.docs[0].id },
  })
}

export async function seedDomain(
  payload: Payload,
  parsed: DomainParseResult,
  domain?: string,
): Promise<SeedResult> {
  const domainId = domain ? await ensureDomain(payload, domain) : undefined
  const result: SeedResult = {
    domain,
    nouns: 0,
    readings: 0,
    stateMachines: 0,
    skipped: 0,
    errors: [],
  }
  const domainData = domainId ? { domain: domainId } : {}

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

  // ── Batch 2: Reference schemes (depends on noun IDs) ──
  const nounCache = new Map<string, any>()
  const allNouns = await payload.find({
    collection: 'nouns',
    pagination: false,
    where: domainId ? { domain: { equals: domainId } } : {},
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

  // ── Batch 3: Readings (hooks create roles, then we apply constraints) ──
  await seedReadings(payload, parsed.readings, domainData, result)

  // ── Batch 4: Instance facts as readings ──
  if (parsed.instanceFacts.length) {
    const instanceReadings: ReadingDef[] = parsed.instanceFacts.map((text) => ({
      text,
      multiplicity: '*:1',
    }))
    await seedReadings(payload, instanceReadings, domainData, result)
  }

  // ── Batch 5: Deontic constraints as readings ──
  if (parsed.deonticConstraints.length) {
    const deonticReadings: ReadingDef[] = parsed.deonticConstraints.map((text) => ({
      text,
      multiplicity: '*:1',
    }))
    await seedReadings(payload, deonticReadings, domainData, result)
  }

  // ── Batch 6: Deontic constraint instance facts as readings ──
  if (parsed.deonticConstraintInstances.length) {
    const instanceReadings: ReadingDef[] = parsed.deonticConstraintInstances.map((d) => {
      let inst = d.instance
      if (
        inst.length >= 2 &&
        ((inst.startsWith('"') && inst.endsWith('"')) ||
          (inst.startsWith("'") && inst.endsWith("'")) ||
          (inst.startsWith('\u201C') && inst.endsWith('\u201D')) ||
          (inst.startsWith('\u2018') && inst.endsWith('\u2019')))
      ) {
        inst = inst.slice(1, -1)
      }
      return {
        text: `${d.constraint} '${inst}'`,
        multiplicity: '*:1',
      }
    })
    await seedReadings(payload, instanceReadings, domainData, result)
  }

  return result
}

export async function seedReadings(
  payload: Payload,
  readings: ReadingDef[],
  domainData: Record<string, any>,
  result: SeedResult,
): Promise<void> {
  // Check which readings already exist
  const existingReadings = await payload.find({
    collection: 'readings',
    where: { text: { in: readings.map((r) => r.text) } },
    pagination: false,
  })
  const existingTexts = new Set(existingReadings.docs.map((d: any) => d.text))

  const newReadings: ReadingDef[] = []
  for (const r of readings) {
    if (existingTexts.has(r.text)) {
      result.skipped++
    } else {
      newReadings.push(r)
    }
  }

  // Separate special readings from regular fact types
  const subtypeReadings = newReadings.filter((r) => r.multiplicity === 'subtype')
  const subsetReadings = newReadings.filter((r) => /^D?SS$/i.test(r.multiplicity))
  const regularReadings = newReadings.filter((r) => r.multiplicity !== 'subtype' && !/^D?SS$/i.test(r.multiplicity))

  // ── Handle subtypes: set noun.superType ──
  for (const r of subtypeReadings) {
    try {
      await applySubtype(payload, r.text, result)
      result.readings++
    } catch (err: any) {
      result.errors.push(`subtype "${r.text}": ${err.message}`)
    }
  }

  // ── Create graph schemas for regular readings ──
  const schemaMap = new Map<string, any>()
  await batch(regularReadings, async (r) => {
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

  // ── Create readings (hooks auto-create roles from text) ──
  await batch(regularReadings, async (r) => {
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

  // ── Apply uniqueness constraints from notation ──
  await batch(regularReadings, async (r) => {
    const schema = schemaMap.get(r.text)
    if (!schema) return
    try {
      await applyConstraints(payload, schema.id, r, result)
      result.readings++
    } catch (err: any) {
      result.errors.push(`constraint "${r.text}": ${err.message}`)
    }
  })

  // ── Apply subset constraints (SS) — must run after all readings exist ──
  for (const r of subsetReadings) {
    try {
      const ssModality = /^D/i.test(r.multiplicity) ? 'Deontic' as const : 'Alethic' as const
      await applySubsetConstraint(payload, r.text, ssModality, result)
      result.readings++
    } catch (err: any) {
      result.errors.push(`subset constraint "${r.text}": ${err.message}`)
    }
  }
}

export async function seedStateMachine(
  payload: Payload,
  entityNounName: string,
  parsed: StateMachineParseResult,
  domain?: string,
): Promise<SeedResult> {
  const domainId = domain ? await ensureDomain(payload, domain) : undefined
  const result: SeedResult = {
    domain,
    nouns: 0,
    readings: 0,
    stateMachines: 0,
    skipped: 0,
    errors: [],
  }
  const domainData = domainId ? { domain: domainId } : {}

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

    const statusMap = new Map<string, string>()
    await batch(parsed.states, async (s) => {
      const status = await payload.create({
        collection: 'statuses',
        data: { name: s, stateMachineDefinition: definition.id },
      })
      statusMap.set(s, status.id)
    })

    const uniqueEvents = [...new Set(parsed.transitions.map((t) => t.event))]
    const eventTypeCache = new Map<string, string>()
    await batch(uniqueEvents, async (event) => {
      const et = await ensureEventType(payload, event)
      eventTypeCache.set(event, et.id)
    })

    const transitionsByEvent = new Map<string, string[]>()
    await batch(parsed.transitions, async (t) => {
      const transition = await payload.create({
        collection: 'transitions',
        data: {
          from: statusMap.get(t.from)!,
          to: statusMap.get(t.to)!,
          eventType: eventTypeCache.get(t.event)!,
        },
      })
      const existing = transitionsByEvent.get(t.event) || []
      existing.push(transition.id)
      transitionsByEvent.set(t.event, existing)
    })

    const transitionsWithGuards = parsed.transitions.filter((t) => t.guard)
    if (transitionsWithGuards.length) {
      await batch(transitionsWithGuards, async (t) => {
        const fromId = statusMap.get(t.from)
        const toId = statusMap.get(t.to)
        const eventId = eventTypeCache.get(t.event)

        const matchingTransitions = await payload.find({
          collection: 'transitions',
          where: {
            from: { equals: fromId },
            to: { equals: toId },
            eventType: { equals: eventId },
          },
          limit: 1,
        })

        if (matchingTransitions.docs.length) {
          const guardTexts = t.guard!.split(';').map((g: string) => g.trim()).filter(Boolean)
          for (const guardText of guardTexts) {
            await payload.create({
              collection: 'guards',
              data: {
                name: guardText,
                transition: matchingTransitions.docs[0].id,
              },
            })
          }
        }
      })
    }

    await wireVerbsAndFunctions(payload, uniqueEvents, transitionsByEvent, result)

    result.stateMachines++
  } catch (err: any) {
    result.errors.push(`state machine for ${entityNounName}: ${err.message}`)
  }

  return result
}

/**
 * For each event name, search readings for instance facts that wire verbs to functions.
 */
async function wireVerbsAndFunctions(
  payload: Payload,
  uniqueEvents: string[],
  transitionsByEvent: Map<string, string[]>,
  result: SeedResult,
): Promise<void> {
  for (const eventName of uniqueEvents) {
    try {
      const runsReadings = await payload.find({
        collection: 'readings',
        where: { text: { like: `${eventName} runs` } },
        limit: 1,
      })
      if (!runsReadings.docs.length) continue

      const runsText = (runsReadings.docs[0] as any).text as string
      const runsMatch = runsText.match(/runs\s+(\S+)/)
      if (!runsMatch) continue
      const functionName = runsMatch[1]

      const [typeReadings, urlReadings, methodReadings] = await Promise.all([
        payload.find({ collection: 'readings', where: { text: { like: `${functionName} has FunctionType` } }, limit: 1 }),
        payload.find({ collection: 'readings', where: { text: { like: `${functionName} has CallbackUrl` } }, limit: 1 }),
        payload.find({ collection: 'readings', where: { text: { like: `${functionName} has HttpMethod` } }, limit: 1 }),
      ])

      const extractValue = (docs: any[], property: string): string | undefined => {
        if (!docs.length) return undefined
        const text = docs[0].text as string
        const match = text.match(new RegExp(`has\\s+${property}\\s+(.+)$`))
        return match?.[1]?.trim()
      }

      const functionType = extractValue(typeReadings.docs, 'FunctionType')
      if (!functionType) continue

      const callbackUrl = extractValue(urlReadings.docs, 'CallbackUrl')
      const httpMethod = extractValue(methodReadings.docs, 'HttpMethod')

      const fn = await payload.find({ collection: 'functions', where: { name: { equals: functionName } }, limit: 1 })
      let functionId: string
      if (fn.docs.length) {
        functionId = fn.docs[0].id
      } else {
        const created = await payload.create({
          collection: 'functions',
          data: {
            name: functionName,
            functionType: functionType as 'httpCallback' | 'query' | 'agentInvocation' | 'transform',
            ...(callbackUrl && { callbackUrl }),
            ...(httpMethod && { httpMethod: httpMethod as 'GET' | 'POST' | 'PUT' | 'PATCH' | 'DELETE' }),
          },
        })
        functionId = created.id
      }

      const verb = await payload.find({ collection: 'verbs', where: { name: { equals: eventName } }, limit: 1 })
      let verbId: string
      if (verb.docs.length) {
        verbId = verb.docs[0].id
      } else {
        const created = await payload.create({
          collection: 'verbs',
          data: { name: eventName, function: functionId },
        })
        verbId = created.id
      }

      const transitionIds = transitionsByEvent.get(eventName) || []
      for (const transitionId of transitionIds) {
        await payload.update({
          collection: 'transitions',
          id: transitionId,
          data: { verb: verbId },
        })
      }
    } catch (err: any) {
      result.errors.push(`verb/function wiring for "${eventName}": ${err.message}`)
    }
  }
}

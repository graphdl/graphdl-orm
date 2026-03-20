/**
 * Claims ingestion — ported from Payload to GraphDLDB.
 *
 * Two entry points:
 * - ingestClaims()   — bulk structured claims
 */
import type { GraphDLDB } from '../do'
import { tokenizeReading } from './tokenize'
import { parseMultiplicity, applyConstraints } from './constraints'

export interface ExtractedClaims {
  nouns: Array<{
    name: string
    objectType: 'entity' | 'value'
    plural?: string
    valueType?: string
    format?: string
    enum?: string[]
    enumValues?: string[]
    minimum?: number
    maximum?: number
    pattern?: string
    worldAssumption?: 'closed' | 'open'
  }>
  readings: Array<{
    text: string
    nouns: string[]
    predicate: string
    multiplicity?: string
  }>
  constraints: Array<{
    kind: 'UC' | 'MC' | 'RC' | 'SS' | 'XC' | 'EQ' | 'OR' | 'XO'
    modality: 'Alethic' | 'Deontic'
    reading: string
    roles: number[]
    /** Full verbalized text (set-comparison constraints) */
    text?: string
    /** For XO/XC/OR: the individual clause texts */
    clauses?: string[]
    /** For set-comparison: the constrained entity name */
    entity?: string
    /** For set-comparison: role spans across multiple readings */
    spans?: Array<{ reading: string; roles: number[] }>
  }>
  subtypes?: Array<{ child: string; parent: string }>
  transitions?: Array<{ entity: string; from: string; to: string; event: string }>
  facts?: Array<{
    reading?: string
    values?: Array<{ noun: string; value: string }>
    /** Entity-centric format from FORML2 parser */
    entity?: string
    entityValue?: string
    predicate?: string
    valueType?: string
    value?: string
  }>
}

export interface IngestClaimsResult {
  nouns: number
  readings: number
  stateMachines: number
  skipped: number
  errors: string[]
}

/** Ensure a noun exists for this domain; return the doc. */
async function ensureNoun(
  db: GraphDLDB,
  name: string,
  data: Record<string, any>,
  domainId: string,
): Promise<Record<string, any>> {
  const existing = await db.findInCollection('nouns', {
    name: { equals: name },
    domain: { equals: domainId },
  }, { limit: 1 })

  if (existing.docs.length) {
    const doc = existing.docs[0]
    const updates: Record<string, any> = {}
    if (data.objectType && doc.objectType !== data.objectType) updates.objectType = data.objectType
    if (data.enumValues && !doc.enumValues) updates.enumValues = data.enumValues
    if (data.valueType && !doc.valueType) updates.valueType = data.valueType
    if (Object.keys(updates).length) {
      return (await db.updateInCollection('nouns', doc.id as string, updates))!
    }
    return doc
  }

  return db.createInCollection('nouns', { name, domain: domainId, ...data })
}

/**
 * Resolve a noun or reading across domain boundaries.
 *
 * Search order:
 * 1. Target domain (domain-local)
 * 2. Other domains in the same Organization (org-shared)
 * 3. Public domains (visibility: 'public')
 */
async function resolveNounAcrossDomains(
  db: GraphDLDB,
  name: string,
  domainId: string,
): Promise<Record<string, any> | null> {
  // 1. Domain-local
  const local = await db.findInCollection('nouns', {
    name: { equals: name },
    domain: { equals: domainId },
  }, { limit: 1 })
  if (local.docs.length) return local.docs[0]

  // 2. Search all nouns with this name — check org, app, or public
  const allNouns = await db.findInCollection('nouns', {
    name: { equals: name },
  }, { limit: 20 })

  if (allNouns.docs.length) {
    const domain = await db.findInCollection('domains', { id: { equals: domainId } }, { limit: 1 })
    const orgId = domain.docs[0]?.organization
    const appId = domain.docs[0]?.app

    for (const doc of allNouns.docs) {
      if (doc.domain === domainId) continue
      const nounDomain = await db.findInCollection('domains', { id: { equals: doc.domain } }, { limit: 1 })
      const nd = nounDomain.docs[0]
      if (!nd) continue
      // Same org, same app, or public
      if ((orgId && nd.organization === orgId) ||
          (appId && nd.app === appId) ||
          nd.visibility === 'public' ||
          // Same app prefix (e.g., support-auto-dev-*)
          (nd.domainSlug?.split('-').slice(0, -1).join('-') ===
           domain.docs[0]?.domainSlug?.split('-').slice(0, -1).join('-'))) {
        return doc
      }
    }

    // Last resort: return any match (within the same system)
    if (allNouns.docs.length === 1) return allNouns.docs[0]
    // If multiple, prefer the first non-local one
    const nonLocal = allNouns.docs.find((d: any) => d.domain !== domainId)
    if (nonLocal) return nonLocal
  }

  return null
}

async function resolveReadingAcrossDomains(
  db: GraphDLDB,
  readingText: string,
  domainId: string,
): Promise<{ schema: Record<string, any>; reading: Record<string, any> } | null> {
  // 1. Domain-local
  const local = await db.findInCollection('readings', {
    text: { equals: readingText },
    domain: { equals: domainId },
  }, { limit: 1 })
  if (local.docs.length) {
    const r = local.docs[0]
    return { schema: { id: r.graphSchema }, reading: r }
  }

  // 2. Search all readings with this text
  const all = await db.findInCollection('readings', {
    text: { equals: readingText },
  }, { limit: 10 })

  if (all.docs.length) {
    // Return the first match — cross-domain readings are always shareable
    const r = all.docs[0]
    return { schema: { id: r.graphSchema }, reading: r }
  }

  return null
}

/**
 * Ingest bulk structured claims.
 */
export async function ingestClaims(
  db: GraphDLDB,
  opts: { claims: ExtractedClaims; domainId: string },
): Promise<IngestClaimsResult> {
  const { claims, domainId } = opts
  const result: IngestClaimsResult = { nouns: 0, readings: 0, stateMachines: 0, skipped: 0, errors: [] }
  const nounMap = new Map<string, Record<string, any>>()

  // Noun names (or supertypes) that imply open-world assumption
  const OPEN_WORLD_NOUNS = ['Right', 'Freedom', 'Liberty', 'Protection', 'Privilege']

  // Step 1: Create all nouns
  for (const noun of claims.nouns) {
    try {
      const data: Record<string, any> = { objectType: noun.objectType }
      if (noun.plural) data.plural = noun.plural
      if (noun.valueType) data.valueType = noun.valueType
      if (noun.format) data.format = noun.format
      const enumVals = noun.enumValues || noun.enum
      if (enumVals) data.enumValues = Array.isArray(enumVals) ? enumVals.join(', ') : enumVals
      if (noun.minimum !== undefined) data.minimum = noun.minimum
      if (noun.maximum !== undefined) data.maximum = noun.maximum
      if (noun.pattern) data.pattern = noun.pattern

      // World assumption: explicit value or auto-detect from noun name
      if (noun.worldAssumption) {
        data.worldAssumption = noun.worldAssumption
      } else if (OPEN_WORLD_NOUNS.some(ow => noun.name === ow || noun.name.endsWith(` ${ow}`))) {
        data.worldAssumption = 'open'
      }

      const doc = await ensureNoun(db, noun.name, data, domainId)
      nounMap.set(noun.name, doc)
      result.nouns++
    } catch (err: any) {
      result.errors.push(`noun "${noun.name}": ${err.message}`)
    }
  }

  // Step 2: Apply subtypes (resolve parent across domains if needed)
  for (const sub of claims.subtypes || []) {
    try {
      const child = nounMap.get(sub.child)
      let parent = nounMap.get(sub.parent)
      if (!parent) {
        parent = await resolveNounAcrossDomains(db, sub.parent, domainId) || undefined
        if (parent) nounMap.set(sub.parent, parent)
      }
      if (child && parent) {
        const updates: Record<string, any> = { superType: parent.id as string }
        // Inherit open-world assumption from parent if applicable
        if (OPEN_WORLD_NOUNS.some(ow => sub.parent === ow || sub.parent.endsWith(` ${ow}`))) {
          updates.worldAssumption = 'open'
        }
        await db.updateInCollection('nouns', child.id as string, updates)
      }
    } catch (err: any) {
      result.errors.push(`subtype "${sub.child} -> ${sub.parent}": ${err.message}`)
    }
  }

  // Step 3: Create graph schemas + readings
  const schemaMap = new Map<string, Record<string, any>>()

  for (const reading of claims.readings) {
    try {
      // Derivation rules (predicate ':=') — store as reading with full text, no graph schema
      if (reading.predicate === ':=' || reading.text.includes(':=')) {
        const existingDeriv = await db.findInCollection('readings', {
          text: { equals: reading.text },
          domain: { equals: domainId },
        }, { limit: 1 })
        if (!existingDeriv.docs.length) {
          // Create a graph schema to hold the derivation reading
          const schema = await db.createInCollection('graph-schemas', {
            name: 'derivation',
            title: reading.text.split(':=')[0].trim(),
            domain: domainId,
          })
          await db.createInCollection('readings', {
            text: reading.text,
            graphSchema: schema.id,
            domain: domainId,
          })
          schemaMap.set(reading.text, schema)
          result.readings++
        } else {
          result.skipped++
        }
        continue
      }

      // Ensure referenced nouns exist
      for (const nounName of reading.nouns) {
        if (!nounMap.has(nounName)) {
          const doc = await ensureNoun(db, nounName, { objectType: 'entity' }, domainId)
          nounMap.set(nounName, doc)
          result.nouns++
        }
      }

      // Check for existing reading
      const existingReading = await db.findInCollection('readings', {
        text: { equals: reading.text },
        domain: { equals: domainId },
      }, { limit: 1 })

      if (existingReading.docs.length) {
        schemaMap.set(reading.text, { id: existingReading.docs[0].graphSchema })
        result.skipped++
        continue
      }

      // Create graph schema
      const schemaName = reading.nouns.join('')
      const schema = await db.createInCollection('graph-schemas', {
        name: schemaName,
        title: schemaName,
        domain: domainId,
      })
      schemaMap.set(reading.text, schema)

      // Create reading
      const readingDoc = await db.createInCollection('readings', {
        text: reading.text,
        graphSchema: schema.id,
        domain: domainId,
      })

      // Auto-create roles
      const allNouns = await db.findInCollection('nouns', {
        domain: { equals: domainId },
      }, { limit: 1000 })
      const nounList = allNouns.docs.map((n: any) => ({ name: n.name, id: n.id }))
      const tokenized = tokenizeReading(reading.text, nounList)

      for (const nounRef of tokenized.nounRefs) {
        await db.createInCollection('roles', {
          reading: readingDoc.id,
          noun: nounRef.id,
          graphSchema: schema.id,
          roleIndex: nounRef.index,
        })
      }

      result.readings++

      // Apply multiplicity constraints
      if (reading.multiplicity) {
        const constraintDefs = parseMultiplicity(reading.multiplicity)
        if (constraintDefs.length > 0) {
          const roles = await db.findInCollection('roles', {
            graphSchema: { equals: schema.id },
          }, { sort: 'createdAt' })
          const roleIds = roles.docs.map((r: any) => r.id)
          await applyConstraints(db, { constraints: constraintDefs, roleIds, domainId })
        }
      }
    } catch (err: any) {
      result.errors.push(`reading "${reading.text}": ${err.message}`)
    }
  }

  // Step 4: Apply explicit constraints
  for (const constraint of claims.constraints || []) {
    try {
      // Set-comparison constraints (SS/XC/EQ/OR/XO) have no single host reading
      if (!constraint.reading && ['SS', 'XC', 'EQ', 'OR', 'XO'].includes(constraint.kind)) {
        const c = await db.createInCollection('constraints', {
          kind: constraint.kind,
          modality: constraint.modality,
          domain: domainId,
        })

        // If cross-reading spans are provided, create constraint-spans
        if (constraint.spans?.length) {
          for (const span of constraint.spans) {
            const schema = schemaMap.get(span.reading)
            if (!schema) continue
            const roles = await db.findInCollection('roles', {
              graphSchema: { equals: schema.id },
            }, { sort: 'createdAt' })
            for (const idx of span.roles) {
              const roleId = roles.docs[idx]?.id
              if (roleId) {
                await db.createInCollection('constraint-spans', {
                  constraint: c.id,
                  role: roleId,
                })
              }
            }
          }
        }
        continue
      }

      let schema = schemaMap.get(constraint.reading)
      if (!schema) {
        // Reading may have been created in a prior ingestion — look it up from DB
        const existingReading = await db.findInCollection('readings', {
          text: { equals: constraint.reading },
          domain: { equals: domainId },
        }, { limit: 1 })
        if (existingReading.docs.length) {
          schema = { id: existingReading.docs[0].graphSchema }
        }
      }
      if (!schema) { result.errors.push(`constraint: reading "${constraint.reading}" not found`); continue }

      const roles = await db.findInCollection('roles', {
        graphSchema: { equals: schema.id },
      }, { sort: 'createdAt' })

      // Idempotent: check if this constraint already exists by text + kind + modality
      let c: any
      if (constraint.text) {
        const existing = await db.findInCollection('constraints', {
          text: { equals: constraint.text },
          domain: { equals: domainId },
          kind: { equals: constraint.kind },
        }, { limit: 1 })
        if (existing.docs.length) {
          result.skipped++
          continue
        }
      }

      c = await db.createInCollection('constraints', {
        kind: constraint.kind,
        modality: constraint.modality,
        text: constraint.text || '',
        domain: domainId,
      })

      const roleIds = constraint.roles
        .map(idx => roles.docs[idx]?.id)
        .filter(Boolean)

      for (const roleId of roleIds) {
        await db.createInCollection('constraint-spans', {
          constraint: c.id,
          role: roleId,
        })
      }
    } catch (err: any) {
      result.errors.push(`constraint on "${constraint.reading}": ${err.message}`)
    }
  }

  // Step 5: Seed state machine transitions
  if (claims.transitions?.length) {
    const byEntity = new Map<string, typeof claims.transitions>()
    for (const t of claims.transitions) {
      const group = byEntity.get(t.entity) || []
      group.push(t)
      byEntity.set(t.entity, group)
    }

    for (const [entityName, transitions] of byEntity) {
      try {
        let noun = nounMap.get(entityName)
        if (!noun) {
          noun = await resolveNounAcrossDomains(db, entityName, domainId) || undefined
          if (noun) nounMap.set(entityName, noun)
        }
        if (!noun) { result.errors.push(`transition entity "${entityName}" not found`); continue }

        // Ensure state machine definition (find by noun + domain, or title + domain)
        const existingDef = await db.findInCollection('state-machine-definitions', {
          noun: { equals: noun.id },
          domain: { equals: domainId },
        }, { limit: 1 })

        const definition = existingDef.docs.length
          ? existingDef.docs[0]
          : await db.createInCollection('state-machine-definitions', {
              title: entityName,
              noun: noun.id,
              domain: domainId,
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
          const existing = await db.findInCollection('statuses', {
            name: { equals: name },
            stateMachineDefinition: { equals: definition.id },
          }, { limit: 1 })
          const status = existing.docs.length
            ? existing.docs[0]
            : await db.createInCollection('statuses', {
                name,
                stateMachineDefinition: definition.id,
              })
          statusMap.set(name, status.id as string)
        }

        // Ensure event types
        const eventMap = new Map<string, string>()
        for (const name of eventNames) {
          const existing = await db.findInCollection('event-types', {
            name: { equals: name },
          }, { limit: 1 })
          const et = existing.docs.length
            ? existing.docs[0]
            : await db.createInCollection('event-types', { name })
          eventMap.set(name, et.id as string)
        }

        // Create transitions
        for (const t of transitions) {
          const fromId = statusMap.get(t.from)!
          const toId = statusMap.get(t.to)!
          const eventId = eventMap.get(t.event)!

          const existingT = await db.findInCollection('transitions', {
            from: { equals: fromId },
            to: { equals: toId },
            eventType: { equals: eventId },
          }, { limit: 1 })

          if (!existingT.docs.length) {
            await db.createInCollection('transitions', {
              from: fromId,
              to: toId,
              eventType: eventId,
              stateMachineDefinition: definition.id,
              domain: domainId,
            })
          }
        }

        result.stateMachines++
      } catch (err: any) {
        result.errors.push(`transitions for "${entityName}": ${err.message}`)
      }
    }
  }

  // Step 6: Create instance facts as 3NF entity rows
  let schemaApplied = false
  if (claims.facts?.length) {
    for (const fact of claims.facts) {
      try {
        // Normalize: convert entity-centric format to reading-centric
        let reading = fact.reading || ''
        let values = fact.values || []

        if (!reading && fact.entity && fact.valueType) {
          // Entity-centric format: { entity, entityValue, predicate, valueType, value }
          const predicate = fact.predicate || 'has'
          reading = `${fact.entity} ${predicate} ${fact.valueType}`
          values = [
            { noun: fact.entity, value: fact.entityValue || '' },
            { noun: fact.valueType, value: fact.value || '' },
          ]
        }

        if (!reading) {
          result.errors.push(`fact: no reading or entity/valueType`)
          continue
        }

        // Instance facts go to 3NF tables via createEntity.
        // The entity is identified by entityValue (reference), and the
        // value type field is set from the fact's value.
        //
        // Example: Status 'Received' has Display Color 'blue'
        //   → createEntity(domain, 'Status', { displayColor: 'blue' }, 'Received')

        const entityName = fact.entity || values[0]?.noun || ''
        const entityRef = fact.entityValue || values[0]?.value || ''
        const fieldValues: Record<string, string> = {}

        // For binary facts: the second value is the field
        if (values.length >= 2) {
          // Convert value type name to camelCase field name
          const fieldName = values[1].noun
            .split(' ')
            .map((w, i) => i === 0 ? w.toLowerCase() : w.charAt(0).toUpperCase() + w.slice(1).toLowerCase())
            .join('')
          fieldValues[fieldName] = values[1].value
        }

        if (!entityName) {
          result.errors.push(`fact: no entity name`)
          continue
        }

        // Ensure schema is applied before first entity creation
        if (!schemaApplied) {
          try {
            await (db as any).applySchema(domainId)
          } catch { /* may fail if no readings yet */ }
          schemaApplied = true
        }

        try {
          await (db as any).createEntity(domainId, entityName, fieldValues, entityRef)
        } catch (err: any) {
          result.errors.push(`fact "${entityName} '${entityRef}': ${err.message}`)
          continue
        }
      } catch (err: any) {
        result.errors.push(`fact "${fact.reading || fact.entity}": ${err.message}`)
      }
    }
  }

  return result
}

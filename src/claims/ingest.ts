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
    reading: string
    values: Array<{ noun: string; value: string }>
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
 * Ingest bulk structured claims.
 */
export async function ingestClaims(
  db: GraphDLDB,
  opts: { claims: ExtractedClaims; domainId: string },
): Promise<IngestClaimsResult> {
  const { claims, domainId } = opts
  const result: IngestClaimsResult = { nouns: 0, readings: 0, stateMachines: 0, skipped: 0, errors: [] }
  const nounMap = new Map<string, Record<string, any>>()

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

      const doc = await ensureNoun(db, noun.name, data, domainId)
      nounMap.set(noun.name, doc)
      result.nouns++
    } catch (err: any) {
      result.errors.push(`noun "${noun.name}": ${err.message}`)
    }
  }

  // Step 2: Apply subtypes
  for (const sub of claims.subtypes || []) {
    try {
      const child = nounMap.get(sub.child)
      const parent = nounMap.get(sub.parent)
      if (child && parent) {
        await db.updateInCollection('nouns', child.id as string, { superType: parent.id as string })
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
        const noun = nounMap.get(entityName)
        if (!noun) { result.errors.push(`transition entity "${entityName}" not found`); continue }

        // Ensure state machine definition
        const existingDef = await db.findInCollection('state-machine-definitions', {
          noun: { equals: noun.id },
        }, { limit: 1 })

        const definition = existingDef.docs.length
          ? existingDef.docs[0]
          : await db.createInCollection('state-machine-definitions', {
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

  // Step 6: Create instance facts (Graphs with ResourceRoles)
  if (claims.facts?.length) {
    for (const fact of claims.facts) {
      try {
        // Find the graph schema by matching the reading text
        const reading = fact.reading
        const schema = schemaMap.get(reading)
        if (!schema) {
          result.errors.push(`fact: reading "${reading}" not found`)
          continue
        }

        // Build bindings from the fact values
        const bindings = fact.values.map(v => {
          const noun = nounMap.get(v.noun)
          if (!noun) return null
          return { nounId: noun.id as string, value: v.value }
        }).filter(Boolean) as Array<{ nounId: string; value: string }>

        if (bindings.length < 2) {
          result.errors.push(`fact: "${reading}" needs at least 2 bindings`)
          continue
        }

        await (db as any).createFact(domainId, schema.id as string, bindings)
      } catch (err: any) {
        result.errors.push(`fact "${fact.reading}": ${err.message}`)
      }
    }
  }

  return result
}

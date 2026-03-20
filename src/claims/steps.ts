/**
 * Extracted step functions for claims ingestion.
 *
 * Each step operates on a shared Scope object and a GraphDLDB instance.
 * Extracted from the monolithic ingestClaims() in ingest.ts.
 */
import type { GraphDLDB } from '../do'
import type { ExtractedClaims } from './ingest'
import type { Scope } from './scope'
import { addNoun, resolveNoun, addSchema, resolveSchema } from './scope'
import { tokenizeReading } from './tokenize'
import { parseMultiplicity, applyConstraints } from './constraints'

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/** Noun names (or supertypes) that imply open-world assumption */
export const OPEN_WORLD_NOUNS = ['Right', 'Freedom', 'Liberty', 'Protection', 'Privilege']

/** Ensure a noun exists for this domain; return the doc. */
export async function ensureNoun(
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

// ---------------------------------------------------------------------------
// Step 1: Create nouns
// ---------------------------------------------------------------------------

export async function ingestNouns(
  db: GraphDLDB,
  nouns: ExtractedClaims['nouns'],
  domainId: string,
  scope: Scope,
): Promise<number> {
  let count = 0

  for (const noun of nouns) {
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
      addNoun(scope, { id: doc.id as string, name: noun.name, domainId })
      count++
    } catch (err: any) {
      scope.errors.push(`[${domainId}] noun "${noun.name}": ${err.message}`)
    }
  }

  return count
}

// ---------------------------------------------------------------------------
// Step 2: Apply subtypes
// ---------------------------------------------------------------------------

export async function ingestSubtypes(
  db: GraphDLDB,
  subtypes: NonNullable<ExtractedClaims['subtypes']>,
  domainId: string,
  scope: Scope,
): Promise<void> {
  for (const sub of subtypes) {
    try {
      const child = resolveNoun(scope, sub.child, domainId)
      let parent = resolveNoun(scope, sub.parent, domainId)
      if (!parent) {
        // Fallback: create the parent noun if it doesn't exist in scope
        const doc = await ensureNoun(db, sub.parent, { objectType: 'entity' }, domainId)
        addNoun(scope, { id: doc.id as string, name: sub.parent, domainId })
        parent = { id: doc.id as string, name: sub.parent, domainId }
      }
      if (child && parent) {
        const updates: Record<string, any> = { superType: parent.id }
        // Inherit open-world assumption from parent if applicable
        if (OPEN_WORLD_NOUNS.some(ow => sub.parent === ow || sub.parent.endsWith(` ${ow}`))) {
          updates.worldAssumption = 'open'
        }
        await db.updateInCollection('nouns', child.id, updates)
      }
    } catch (err: any) {
      scope.errors.push(`[${domainId}] subtype "${sub.child} -> ${sub.parent}": ${err.message}`)
    }
  }
}

// ---------------------------------------------------------------------------
// Step 3: Create graph schemas + readings
// ---------------------------------------------------------------------------

export async function ingestReadings(
  db: GraphDLDB,
  readings: ExtractedClaims['readings'],
  domainId: string,
  scope: Scope,
): Promise<number> {
  let count = 0

  for (const reading of readings) {
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
          addSchema(scope, reading.text, schema)
          count++
        } else {
          scope.skipped++
        }
        continue
      }

      // Ensure referenced nouns exist
      for (const nounName of reading.nouns) {
        if (!resolveNoun(scope, nounName, domainId)) {
          const doc = await ensureNoun(db, nounName, { objectType: 'entity' }, domainId)
          addNoun(scope, { id: doc.id as string, name: nounName, domainId })
        }
      }

      // Check for existing reading
      const existingReading = await db.findInCollection('readings', {
        text: { equals: reading.text },
        domain: { equals: domainId },
      }, { limit: 1 })

      if (existingReading.docs.length) {
        addSchema(scope, reading.text, { id: existingReading.docs[0].graphSchema })
        scope.skipped++
        continue
      }

      // Create graph schema
      const schemaName = reading.nouns.join('')
      const schema = await db.createInCollection('graph-schemas', {
        name: schemaName,
        title: schemaName,
        domain: domainId,
      })
      addSchema(scope, reading.text, schema)

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

      count++

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
      scope.errors.push(`[${domainId}] reading "${reading.text}": ${err.message}`)
    }
  }

  return count
}

// ---------------------------------------------------------------------------
// Step 4: Apply explicit constraints
// ---------------------------------------------------------------------------

export async function ingestConstraints(
  db: GraphDLDB,
  constraints: NonNullable<ExtractedClaims['constraints']>,
  domainId: string,
  scope: Scope,
): Promise<void> {
  for (const constraint of constraints) {
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
            const schema = resolveSchema(scope, span.reading)
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

      let schema = resolveSchema(scope, constraint.reading)
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
      if (!schema) {
        scope.errors.push(`[${domainId}] constraint: reading "${constraint.reading}" not found`)
        continue
      }

      const roles = await db.findInCollection('roles', {
        graphSchema: { equals: schema.id },
      }, { sort: 'createdAt' })

      // Idempotent: check if this constraint already exists by text + kind + modality
      if (constraint.text) {
        const existing = await db.findInCollection('constraints', {
          text: { equals: constraint.text },
          domain: { equals: domainId },
          kind: { equals: constraint.kind },
        }, { limit: 1 })
        if (existing.docs.length) {
          scope.skipped++
          continue
        }
      }

      const c = await db.createInCollection('constraints', {
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
      scope.errors.push(`[${domainId}] constraint on "${constraint.reading}": ${err.message}`)
    }
  }
}

// ---------------------------------------------------------------------------
// Step 5: Seed state machine transitions
// ---------------------------------------------------------------------------

export async function ingestTransitions(
  db: GraphDLDB,
  transitions: NonNullable<ExtractedClaims['transitions']>,
  domainId: string,
  scope: Scope,
): Promise<number> {
  let count = 0

  // Group transitions by entity
  const byEntity = new Map<string, typeof transitions>()
  for (const t of transitions) {
    const group = byEntity.get(t.entity) || []
    group.push(t)
    byEntity.set(t.entity, group)
  }

  for (const [entityName, entityTransitions] of byEntity) {
    try {
      let noun = resolveNoun(scope, entityName, domainId)
      if (!noun) {
        // Fallback: create the noun if it doesn't exist
        const doc = await ensureNoun(db, entityName, { objectType: 'entity' }, domainId)
        addNoun(scope, { id: doc.id as string, name: entityName, domainId })
        noun = { id: doc.id as string, name: entityName, domainId }
      }

      // Ensure state machine definition (find by noun + domain)
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
      for (const t of entityTransitions) {
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
      for (const t of entityTransitions) {
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

      count++
    } catch (err: any) {
      scope.errors.push(`[${domainId}] transitions for "${entityName}": ${err.message}`)
    }
  }

  return count
}

// ---------------------------------------------------------------------------
// Step 6: Create instance facts
// ---------------------------------------------------------------------------

export async function ingestFacts(
  db: GraphDLDB,
  facts: NonNullable<ExtractedClaims['facts']>,
  domainId: string,
  scope: Scope,
): Promise<void> {
  for (const fact of facts) {
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
        scope.errors.push(`[${domainId}] fact: no reading or entity/valueType`)
        continue
      }

      // Instance facts go to 3NF tables via createEntity.
      const entityName = fact.entity || values[0]?.noun || ''
      const entityRef = fact.entityValue || values[0]?.value || ''
      const fieldValues: Record<string, string> = {}

      // For binary facts: the second value is the field
      if (values.length >= 2 && values[1].noun) {
        // Convert value type name to camelCase field name
        const fieldName = values[1].noun
          .split(' ')
          .map((w, i) => i === 0 ? w.toLowerCase() : w.charAt(0).toUpperCase() + w.slice(1).toLowerCase())
          .join('')
        fieldValues[fieldName] = values[1].value
      }

      if (!entityName) {
        scope.errors.push(`[${domainId}] fact: no entity name`)
        continue
      }

      try {
        await (db as any).createEntity(domainId, entityName, fieldValues, entityRef)
      } catch (err: any) {
        scope.errors.push(`[${domainId}] fact "${entityName} '${entityRef}': ${err.message}`)
        continue
      }
    } catch (err: any) {
      scope.errors.push(`[${domainId}] fact "${fact.reading || fact.entity}": ${err.message}`)
    }
  }
}

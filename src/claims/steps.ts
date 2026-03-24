/**
 * Extracted step functions for claims ingestion.
 *
 * Each step operates on a shared Scope object and a BatchBuilder instance.
 * Metamodel entities are accumulated in the batch (committed by the caller).
 * Instance facts (ingestFacts) still use the DB interface directly.
 */
import type { BatchBuilder } from './batch-builder'
import type { GraphDLDBLike } from './ingest'
import type { ExtractedClaims } from './ingest'
import type { Scope } from './scope'
import { addNoun, resolveNoun, addSchema, resolveSchema } from './scope'
import { tokenizeReading } from './tokenize'
import { parseMultiplicity, applyConstraintsBatch } from './constraints'

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/** Noun names (or supertypes) that imply open-world assumption */
export const OPEN_WORLD_NOUNS = ['Right', 'Freedom', 'Liberty', 'Protection', 'Privilege']

/** Ensure a noun exists in the batch for this domain; return its id. */
export function ensureNoun(
  builder: BatchBuilder,
  name: string,
  data: Record<string, any>,
  domainId: string,
): string {
  return builder.ensureEntity('Noun', 'name', name, {
    name,
    domain: domainId,
    ...data,
  })
}

// ---------------------------------------------------------------------------
// Step 1: Create nouns
// ---------------------------------------------------------------------------

export function ingestNouns(
  builder: BatchBuilder,
  nouns: ExtractedClaims['nouns'],
  domainId: string,
  scope: Scope,
): number {
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

      // Reference scheme: store as JSON array of value type noun names
      if (noun.refScheme?.length) {
        data.referenceScheme = JSON.stringify(noun.refScheme)
      }

      const id = ensureNoun(builder, noun.name, data, domainId)
      addNoun(scope, { id, name: noun.name, domainId })
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

export function ingestSubtypes(
  builder: BatchBuilder,
  subtypes: NonNullable<ExtractedClaims['subtypes']>,
  domainId: string,
  scope: Scope,
): void {
  for (const sub of subtypes) {
    try {
      const child = resolveNoun(scope, sub.child, domainId)
      let parent = resolveNoun(scope, sub.parent, domainId)
      if (!parent) {
        // Fallback: create the parent noun if it doesn't exist in scope
        const id = ensureNoun(builder, sub.parent, { objectType: 'entity' }, domainId)
        addNoun(scope, { id, name: sub.parent, domainId })
        parent = { id, name: sub.parent, domainId }
      }
      if (child && parent) {
        const updates: Record<string, any> = { superType: parent.id }
        // Inherit open-world assumption from parent if applicable
        if (OPEN_WORLD_NOUNS.some(ow => sub.parent === ow || sub.parent.endsWith(` ${ow}`))) {
          updates.worldAssumption = 'open'
        }
        builder.updateEntity(child.id, updates)
      }
    } catch (err: any) {
      scope.errors.push(`[${domainId}] subtype "${sub.child} -> ${sub.parent}": ${err.message}`)
    }
  }
}

// ---------------------------------------------------------------------------
// Step 3: Create graph schemas + readings
// ---------------------------------------------------------------------------

export function ingestReadings(
  builder: BatchBuilder,
  readings: ExtractedClaims['readings'],
  domainId: string,
  scope: Scope,
  objectificationMap?: Map<string, string>,
): number {
  let count = 0

  for (const reading of readings) {
    try {
      // Derivation rules (predicate ':=') — store as reading with full text, no graph schema
      if (reading.predicate === ':=' || reading.text.includes(':=')) {
        // Check if this derivation already exists in the batch
        const existingDerivs = builder.findEntities('Reading', { text: reading.text, domain: domainId })
        if (existingDerivs.length) {
          scope.skipped++
          continue
        }

        // Create a graph schema to hold the derivation reading
        const schemaId = builder.addEntity('GraphSchema', {
          name: 'derivation',
          title: reading.text.split(':=')[0].trim(),
          domain: domainId,
        })
        builder.addEntity('Reading', {
          text: reading.text,
          graphSchema: schemaId,
          domain: domainId,
        })
        addSchema(scope, reading.text, { id: schemaId })
        count++
        continue
      }

      // Ensure referenced nouns exist
      for (const nounName of reading.nouns) {
        if (!resolveNoun(scope, nounName, domainId)) {
          const id = ensureNoun(builder, nounName, { objectType: 'entity' }, domainId)
          addNoun(scope, { id, name: nounName, domainId })
        }
      }

      // Check for existing reading in the batch
      const existingReadings = builder.findEntities('Reading', { text: reading.text, domain: domainId })
      if (existingReadings.length) {
        const existingReading = existingReadings[0]
        addSchema(scope, reading.text, { id: existingReading.data.graphSchema as string })
        scope.skipped++
        continue
      }

      // Create graph schema — if a noun objectifies this reading, share the noun's ID
      const schemaName = reading.nouns.join('')
      const objectifyingNounId = objectificationMap?.get(reading.text)
      const schemaId = builder.addEntity('GraphSchema', {
        name: schemaName,
        title: schemaName,
        domain: domainId,
      }, objectifyingNounId)
      addSchema(scope, reading.text, { id: schemaId })

      // Create reading
      const readingId = builder.addEntity('Reading', {
        text: reading.text,
        graphSchema: schemaId,
        domain: domainId,
      })

      // Auto-create roles — collect all nouns from the batch for tokenization
      const allNounEntities = builder.findEntities('Noun')
      const nounList = allNounEntities.map(e => ({
        name: e.data.name as string,
        id: e.id,
      }))
      const tokenized = tokenizeReading(reading.text, nounList)

      const roleIds: string[] = []
      for (const nounRef of tokenized.nounRefs) {
        const roleId = builder.addEntity('Role', {
          reading: readingId,
          noun: nounRef.id,
          graphSchema: schemaId,
          roleIndex: nounRef.index,
        })
        roleIds.push(roleId)
      }

      count++

      // Apply multiplicity constraints
      if (reading.multiplicity) {
        const constraintDefs = parseMultiplicity(reading.multiplicity)
        if (constraintDefs.length > 0) {
          applyConstraintsBatch(builder, { constraints: constraintDefs, roleIds, domainId })
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

export function ingestConstraints(
  builder: BatchBuilder,
  constraints: NonNullable<ExtractedClaims['constraints']>,
  domainId: string,
  scope: Scope,
): void {
  for (const constraint of constraints) {
    try {
      // Set-comparison constraints (SS/XC/EQ/OR/XO) have no single host reading
      if (!constraint.reading && ['SS', 'XC', 'EQ', 'OR', 'XO'].includes(constraint.kind)) {
        const constraintId = builder.addEntity('Constraint', {
          kind: constraint.kind,
          modality: constraint.modality,
          domain: domainId,
        })

        // If cross-reading spans are provided, create constraint-spans
        if (constraint.spans?.length) {
          for (const span of constraint.spans) {
            const schema = resolveSchema(scope, span.reading)
            if (!schema) continue
            const roles = builder.findEntities('Role', { graphSchema: schema.id })
            // Sort by roleIndex to maintain consistent ordering
            roles.sort((a, b) => (a.data.roleIndex as number) - (b.data.roleIndex as number))
            for (const idx of span.roles) {
              const role = roles[idx]
              if (role) {
                builder.addEntity('ConstraintSpan', {
                  constraint: constraintId,
                  role: role.id,
                })
              }
            }
          }
        }
        continue
      }

      let schema = resolveSchema(scope, constraint.reading)
      if (!schema) {
        // Reading may have been created in a prior ingestion — look it up from batch
        const existingReadings = builder.findEntities('Reading', {
          text: constraint.reading,
          domain: domainId,
        })
        if (existingReadings.length) {
          schema = { id: existingReadings[0].data.graphSchema as string }
        }
      }
      if (!schema) {
        scope.errors.push(`[${domainId}] constraint: reading "${constraint.reading}" not found`)
        continue
      }

      const roles = builder.findEntities('Role', { graphSchema: schema.id })
      // Sort by roleIndex to maintain consistent ordering
      roles.sort((a, b) => (a.data.roleIndex as number) - (b.data.roleIndex as number))

      // Idempotent: check if this constraint already exists by text + kind + modality
      if (constraint.text) {
        const existingConstraints = builder.findEntities('Constraint', {
          text: constraint.text,
          domain: domainId,
          kind: constraint.kind,
        })
        if (existingConstraints.length) {
          scope.skipped++
          continue
        }
      }

      const constraintId = builder.addEntity('Constraint', {
        kind: constraint.kind,
        modality: constraint.modality,
        text: constraint.text || '',
        domain: domainId,
      })

      const roleIds = constraint.roles
        .map(idx => roles[idx]?.id)
        .filter(Boolean)

      for (const roleId of roleIds) {
        builder.addEntity('ConstraintSpan', {
          constraint: constraintId,
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

export function ingestTransitions(
  builder: BatchBuilder,
  transitions: NonNullable<ExtractedClaims['transitions']>,
  domainId: string,
  scope: Scope,
): number {
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
        const id = ensureNoun(builder, entityName, { objectType: 'entity' }, domainId)
        addNoun(scope, { id, name: entityName, domainId })
        noun = { id, name: entityName, domainId }
      }

      // Ensure state machine definition (find by noun + domain in batch)
      const existingDefs = builder.findEntities('StateMachineDefinition', {
        noun: noun.id,
        domain: domainId,
      })

      const definitionId = existingDefs.length
        ? existingDefs[0].id
        : builder.addEntity('StateMachineDefinition', {
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
        const statusId = builder.ensureEntity('Status', 'name', `${definitionId}:${name}`, {
          name,
          stateMachineDefinition: definitionId,
        })
        statusMap.set(name, statusId)
      }

      // Ensure event types
      const eventMap = new Map<string, string>()
      for (const name of eventNames) {
        const eventId = builder.ensureEntity('EventType', 'name', name, {
          name,
        })
        eventMap.set(name, eventId)
      }

      // Create transitions
      for (const t of entityTransitions) {
        const fromId = statusMap.get(t.from)!
        const toId = statusMap.get(t.to)!
        const eventId = eventMap.get(t.event)!

        // Idempotent: check batch for existing transition
        const existingTransitions = builder.findEntities('Transition', {
          from: fromId,
          to: toId,
          eventType: eventId,
        })

        if (!existingTransitions.length) {
          builder.addEntity('Transition', {
            from: fromId,
            to: toId,
            eventType: eventId,
            stateMachineDefinition: definitionId,
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

/**
 * Instance facts write to EntityDB DOs (not metamodel), so they still
 * use the GraphDLDBLike interface directly. This step is NOT batched.
 */
export async function ingestFacts(
  db: GraphDLDBLike,
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

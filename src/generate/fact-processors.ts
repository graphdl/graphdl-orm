/**
 * Fact type processors — binary, array, and unary schema generation.
 *
 * Ported from Generator.ts.bak (commit ddb8880) lines 2148-2295.
 * Processes constraint spans and graph schemas into JSON Schema properties.
 */

import type { NounDef } from '../model/types'
import {
  toPredicate,
  findPredicateObject,
  extractPropertyName,
  transformPropertyName,
  nameToKey,
} from './rmap'
import {
  ensureTableExists,
  createProperty,
  setTableProperty,
  type Schema,
  type JSONSchemaType,
} from './schema-builder'

// ---------------------------------------------------------------------------
// Loose Payload shapes — minimal interfaces matching the nested API data
// ---------------------------------------------------------------------------
interface Role {
  id: string
  noun?: { value: NounDef | string }
  graphSchema?: { id: string }
  required?: boolean
}

interface Reading {
  text: string
}

interface GraphSchema {
  id: string
  name?: string
  roles?: { docs?: Role[] }
  readings?: { docs?: Reading[] }
}

interface ConstraintSpan {
  roles: Role[]
}

interface ResourceRole {
  role?: { id: string } | Role
  resource?: { value?: { value?: string } }
}

interface Graph {
  type?: { id: string } | GraphSchema
  resourceRoles?: { docs?: ResourceRole[] }
}

// ---------------------------------------------------------------------------
// resolveNoun — resolve a noun value that might be a string id
// ---------------------------------------------------------------------------
function resolveNoun(raw: NounDef | string | undefined, nouns: NounDef[]): NounDef | undefined {
  if (!raw) return undefined
  if (typeof raw === 'string') return nouns.find((n) => n.id === raw)
  if (raw.id) return nouns.find((n) => n.id === raw.id) || raw
  return raw
}

// ---------------------------------------------------------------------------
// processBinarySchemas
// ---------------------------------------------------------------------------
/**
 * Process single-role uniqueness constraints into typed properties.
 *
 * For each constraint span with exactly 1 role:
 * - The constrained role's graphSchema = the fact type
 * - The constrained role = subject noun
 * - The OTHER role in the graphSchema = object noun
 * - Tokenizes the reading, finds object position, extracts property name
 * - Calls setTableProperty to add the property to the subject's schema
 */
export function processBinarySchemas(
  constraintSpans: ConstraintSpan[],
  schemas: Record<string, Schema>,
  nouns: NounDef[],
  jsonExamples: Record<string, JSONSchemaType>,
  nounRegex: RegExp,
  examples: Graph[],
  graphSchemas: GraphSchema[],
): void {
  // Per Halpin p.416-420: for 1:1 fact types, only one side gets the FK.
  // Detect 1:1 by finding graph schemas with single-role UC on BOTH roles.
  // The preferred side is: mandatory > more other roles > first role.
  const singleRoleSpans = constraintSpans.filter((cs) => cs.roles?.length === 1)
  const ucBySchema = new Map<string, string[]>() // graphSchemaId → [constrainedRoleId, ...]
  for (const cs of singleRoleSpans) {
    const role = cs.roles[0]
    const gsId = typeof role.graphSchema === 'string' ? role.graphSchema : role.graphSchema?.id
    if (gsId) {
      const roles = ucBySchema.get(gsId) || []
      roles.push(role.id)
      ucBySchema.set(gsId, roles)
    }
  }
  // For 1:1 schemas (two single-role UCs on same schema), pick the preferred side
  const skipRoles = new Set<string>()
  for (const [gsId, roleIds] of ucBySchema) {
    if (roleIds.length === 2) {
      // 1:1 detected — skip the second role (prefer the first/mandatory side)
      const gs = graphSchemas.find((g) => g.id === gsId)
      if (gs?.roles?.docs?.length === 2) {
        const role0 = gs.roles.docs[0]
        const role1 = gs.roles.docs[1]
        // Prefer the mandatory side; if both or neither mandatory, prefer role 0
        if (role1.required && !role0.required) {
          skipRoles.add(role0.id) // role1 is mandatory, put FK there
        } else {
          skipRoles.add(role1.id) // role0 is mandatory or default, put FK there
        }
      }
    }
  }

  for (const { propertySchema, subjectRole } of constraintSpans
    .filter((cs) => cs.roles?.length === 1)
    .map((cs) => {
      const constrainedRole = cs.roles[0]
      const nestedGs = constrainedRole.graphSchema as { id: string }
      // Look up the top-level graphSchema (which has join fields populated) instead of the nested one
      const propertySchema = graphSchemas.find((gs) => gs.id === nestedGs?.id)
      // The single role from the constraint span is fully populated (depth 6)
      return { propertySchema, subjectRole: propertySchema ? constrainedRole : undefined }
    })) {
    if (!subjectRole || !propertySchema) continue

    // Skip the non-preferred side of 1:1 fact types (Halpin RMAP)
    if (skipRoles.has(subjectRole.id)) continue

    const subject = resolveNoun(subjectRole.noun?.value, nouns)
    if (!subject) continue
    ensureTableExists({ tables: schemas, subject, nouns, jsonExamples })

    const objectRole = propertySchema.roles?.docs?.find((r) => r.id !== subjectRole.id)
    // Use noun from constraint span data (fully populated) if available, otherwise from join field
    const objectNounValue = objectRole?.noun?.value
    const object = resolveNoun(objectNounValue, nouns)
    if (!object) continue

    const reading = propertySchema.readings?.docs?.[0]
    if (!reading) continue
    const predicate = toPredicate({ reading: reading.text, nouns, nounRegex })
    const { objectBegin, objectEnd } = findPredicateObject({ predicate, subject, object })

    const objectReading = predicate
      .slice(objectBegin, objectEnd)
      .map((n) => n[0].toUpperCase() + n.slice(1).replace(/-$/, ''))
    predicate.splice(objectBegin, objectReading.length, ...objectReading)

    let example: string | undefined = undefined
    const exampleProperty = examples.find(
      (g) => (g.type as GraphSchema)?.id === propertySchema.id,
    )
    if (exampleProperty) {
      example = (
        exampleProperty?.resourceRoles?.docs?.find(
          (role) => objectRole!.id === (role.role as Role)?.id,
        )?.resource?.value as { value?: string }
      )?.value
    }

    setTableProperty({
      tables: schemas,
      subject,
      object: object as NounDef,
      nouns,
      propertyName: extractPropertyName(objectReading),
      description: predicate.join(' '),
      required: subjectRole.required || false,
      property: createProperty({
        object: object as NounDef,
        nouns,
        tables: schemas,
        jsonExamples,
      }),
      example,
      jsonExamples,
    })
  }
}

// ---------------------------------------------------------------------------
// processArraySchemas
// ---------------------------------------------------------------------------
/**
 * Process compound uniqueness constraints that have no parent reference (array types).
 *
 * Each becomes an array property on the subject entity. The items type is derived
 * from the object noun via createProperty.
 */
export function processArraySchemas(
  arrayTypes: { gs: GraphSchema; cs: ConstraintSpan }[],
  nouns: NounDef[],
  nounRegex: RegExp,
  schemas: Record<string, Schema>,
  jsonExamples: Record<string, JSONSchemaType>,
): void {
  for (const { gs: schema } of arrayTypes) {
    const reading = schema.readings?.docs?.[0]
    if (!reading) continue
    const predicate = toPredicate({ reading: reading.text, nouns, nounRegex })

    const subjectRaw = (schema.roles?.docs?.[0] as Role)?.noun?.value
    const subject = resolveNoun(subjectRaw, nouns)
    const objectRaw = (schema.roles?.docs?.[1] as Role)?.noun?.value
    const object = resolveNoun(objectRaw, nouns)
    if (!subject?.name || !object?.name) continue // Skip readings with unresolved nouns
    const plural = object.plural

    const { objectBegin, objectEnd } = findPredicateObject({ predicate, subject, object, plural })
    const objectReading = predicate
      .slice(objectBegin, objectEnd)
      .map((n) => n[0].toUpperCase() + n.slice(1).replace(/-$/, ''))
    predicate.splice(objectBegin, objectReading.length, ...objectReading)
    let propertyName = schema.name || extractPropertyName(objectReading) + (plural ? '' : 's')
    propertyName = transformPropertyName(propertyName)

    ensureTableExists({ tables: schemas, subject, nouns, jsonExamples })
    const key = nameToKey('Update' + (subject.name || ''))
    const properties = schemas[key].properties ?? {}

    const property: Schema = {
      type: 'array',
      items: createProperty({ object, nouns, tables: schemas, jsonExamples }),
    }
    property.description = predicate.join(' ')
    properties[propertyName] = property
    schemas[key].properties = properties
  }
}

// ---------------------------------------------------------------------------
// processUnarySchemas
// ---------------------------------------------------------------------------
/**
 * Process graph schemas with exactly 1 role (unary facts).
 *
 * Each becomes a boolean property on the entity.
 * E.g., "Customer is active" -> { isActive: { type: 'boolean' } }
 */
export function processUnarySchemas(
  graphSchemas: GraphSchema[],
  nouns: NounDef[],
  nounRegex: RegExp,
  schemas: Record<string, Schema>,
  jsonExamples: Record<string, JSONSchemaType>,
  examples: Graph[],
): void {
  for (const unarySchema of graphSchemas.filter((s) => s.roles?.docs?.length === 1)) {
    const unaryRole = unarySchema.roles?.docs?.[0] as Role
    const subject = unaryRole?.noun?.value as NounDef
    if (!subject) continue
    const reading = unarySchema.readings?.docs?.[0]
    if (!reading) continue
    const predicate = toPredicate({ reading: reading.text, nouns, nounRegex })
    const { objectBegin } = findPredicateObject({ predicate, subject })
    const objectReading = predicate.slice(objectBegin)

    ensureTableExists({ tables: schemas, subject, nouns, jsonExamples })

    let example: string | undefined = undefined
    const exampleProperty = examples.find(
      (g) => (g.type as GraphSchema)?.id === unarySchema.id,
    )
    if (exampleProperty) {
      example = (
        exampleProperty?.resourceRoles?.docs?.find(
          (role) => unaryRole.id === (role.role as Role)?.id,
        )?.resource?.value as { value?: string }
      )?.value
    }

    // CWA unary = boolean (absence means false, e.g. "Person smokes")
    // OWA unary = nullable boolean (absence means unknown, e.g. "Person has right to X")
    const isOWA = (subject as any).worldAssumption === 'open'
    const property: Schema = isOWA
      ? { type: ['boolean', 'null'] as any, description: 'Open world: null = unknown' }
      : { type: 'boolean' }

    setTableProperty({
      tables: schemas,
      subject,
      object: subject as NounDef,
      nouns,
      propertyName: extractPropertyName(objectReading),
      description: predicate.join(' '),
      required: unaryRole.required || false,
      property,
      example,
      jsonExamples,
    })
  }
}

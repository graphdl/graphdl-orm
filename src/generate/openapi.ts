/**
 * generateOpenAPI — main orchestrator that queries the DO for domain model data
 * and produces an OpenAPI-style `{ components: { schemas } }` object.
 *
 * Ported from Generator.ts.bak lines 299-452 (data fetching, constraint grouping,
 * fact processing, allOf flattening). Adapted for DO findInCollection API.
 */

import { nameToKey, nounListToRegex, type NounRef } from './rmap'
import {
  ensureTableExists,
  createProperty,
  setTableProperty,
  type Schema,
  type JSONSchemaType,
} from './schema-builder'
import { processBinarySchemas, processArraySchemas, processUnarySchemas } from './fact-processors'

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/** Fetch all docs from a collection, handling pagination by using a large limit. */
async function fetchAll(db: any, slug: string, where?: any): Promise<any[]> {
  const result = await db.findInCollection(slug, where, { limit: 10000 })
  return result?.docs || []
}

// Minimal shape for a constraint row from the SQLite constraint_spans table
interface ConstraintSpanRow {
  id: string
  constraint_id: string
  role_id: string
}

// Minimal constraint row
interface ConstraintRow {
  id: string
  kind: string
}

// ---------------------------------------------------------------------------
// generateOpenAPI
// ---------------------------------------------------------------------------

/**
 * Query the DO for domain model data and produce an OpenAPI-style schema object.
 *
 * @param db  - Durable Object stub with `findInCollection(slug, where?, options?)`
 * @param domainId - The domain ID to scope graph-schemas and domain nouns
 * @returns OpenAPI 3.0.0 object with `components.schemas`
 */
export async function generateOpenAPI(db: any, domainId: string): Promise<any> {
  const schemas: Record<string, Schema> = {}
  const domainFilter = { domain: { equals: domainId } }

  // ------ 1. Fetch all domain model data ------
  const [graphSchemas, allNouns, domainNouns, constraintSpanRows, constraints] = await Promise.all([
    fetchAll(db, 'graph-schemas', domainFilter),
    fetchAll(db, 'nouns'),
    fetchAll(db, 'nouns', domainFilter),
    fetchAll(db, 'constraint-spans'),
    fetchAll(db, 'constraints'),
  ])

  // ------ 2. Populate graph schemas with their roles and readings ------
  // In the SQLite model, roles/readings are separate tables linked by graphSchema id.
  // We need to manually join them since there's no depth parameter.
  for (const gs of graphSchemas) {
    const gsRoles = await fetchAll(db, 'roles', { graphSchema: { equals: gs.id } })
    // Populate each role's noun by looking it up in allNouns
    for (const role of gsRoles) {
      const nounId = typeof role.noun === 'string' ? role.noun : role.noun?.value || role.noun?.id || role.noun_id
      const noun = allNouns.find((n: any) => n.id === nounId)
      role.noun = { value: noun || nounId }
      // Normalize graphSchema to { id } for compatibility with fact processors
      role.graphSchema = { id: gs.id }
    }
    gs.roles = { docs: gsRoles }

    const gsReadings = await fetchAll(db, 'readings', { graphSchema: { equals: gs.id } })
    gs.readings = { docs: gsReadings }
  }

  // ------ 3. Group constraint spans by constraint_id to reconstruct multi-role arrays ------
  // In Payload, a ConstraintSpan had a `roles` array (many roles).
  // In SQLite, each constraint_spans row has one role_id. Group by constraint_id.
  const ucConstraintIds = new Set(
    constraints.filter((c: ConstraintRow) => c.kind === 'UC').map((c: ConstraintRow) => c.id),
  )

  const spansByConstraint: Record<string, ConstraintSpanRow[]> = {}
  for (const row of constraintSpanRows as ConstraintSpanRow[]) {
    if (!ucConstraintIds.has(row.constraint_id)) continue
    if (!spansByConstraint[row.constraint_id]) spansByConstraint[row.constraint_id] = []
    spansByConstraint[row.constraint_id].push(row)
  }

  // Build constraint spans in the shape the fact processors expect: { roles: Role[] }
  // Each group becomes one constraint span with multiple roles.
  const constraintSpans: { roles: any[] }[] = []
  for (const [, rows] of Object.entries(spansByConstraint)) {
    const roles: any[] = []
    for (const row of rows) {
      // Find the role object from the populated graphSchemas
      let foundRole: any = undefined
      for (const gs of graphSchemas) {
        const r = gs.roles?.docs?.find((role: any) => role.id === row.role_id)
        if (r) {
          foundRole = r
          break
        }
      }
      if (foundRole) roles.push(foundRole)
    }
    if (roles.length > 0) {
      constraintSpans.push({ roles })
    }
  }

  // ------ 4. Identify compound uniqueness constraints ------
  // compoundUniqueSchemas: constraint spans with >1 role, all roles in the same graphSchema
  const compoundUniqueSchemas = constraintSpans
    .filter((cs) => {
      if (!cs.roles?.length || cs.roles.length <= 1) return false
      const firstGsId =
        typeof cs.roles[0].graphSchema === 'string'
          ? cs.roles[0].graphSchema
          : cs.roles[0].graphSchema?.id
      return cs.roles.every((r: any) => {
        const gsId = typeof r.graphSchema === 'string' ? r.graphSchema : r.graphSchema?.id
        return gsId === firstGsId
      })
    })
    .map((cs) => {
      const nestedGs = cs.roles[0].graphSchema as { id: string }
      const gs = graphSchemas.find((s: any) => s.id === nestedGs.id)
      return gs ? { gs, cs } : undefined
    })
    .filter((entry): entry is { gs: any; cs: { roles: any[] } } => !!entry)

  // arrayTypes: compound UC where the graphSchema is NOT referenced by another schema's role
  const arrayTypes = compoundUniqueSchemas.filter(
    ({ gs: compoundGs }) =>
      !graphSchemas.find((s: any) =>
        s.roles?.docs?.find((r: any) => {
          const nounValue = r.noun?.value
          return nounValue?.id === compoundGs.id
        }),
      ),
  )

  // associationSchemas: compound UC where the graphSchema IS referenced by another schema's role
  const associationSchemas = compoundUniqueSchemas.filter((cs) => !arrayTypes.includes(cs))

  // ------ 5. Process association schemas (objectified entities) ------
  // Use allNouns as the mutable nouns list (association schemas will be added to it)
  const nouns: NounRef[] = [...allNouns]
  nouns.push(...associationSchemas.map(({ gs }) => gs))

  // No json-examples or graphs (example) collections in the DO model — pass empty
  const jsonExamples: Record<string, JSONSchemaType> = {}

  for (const { gs: associationSchema, cs } of associationSchemas) {
    const key = (associationSchema.name || '').replace(/ /g, '')
    const jsonExample = jsonExamples[key]
    schemas['Update' + key] = {
      $id: 'Update' + key,
      title: associationSchema.name || '',
      type: 'object',
      description:
        associationSchema.description ||
        associationSchema.readings?.docs?.[0]?.text?.replace(/- /, ' '),
    }
    schemas['New' + key] = {
      $id: 'New' + key,
      allOf: [{ $ref: '#/components/schemas/Update' + key }],
    }
    schemas[key] = {
      $id: key,
      allOf: [{ $ref: '#/components/schemas/New' + key }],
    }
    if (jsonExample) {
      schemas['Update' + key].examples = [jsonExample]
      schemas['New' + key].examples = [jsonExample]
      schemas[key].examples = [jsonExample]
    }
    for (const role of associationSchema.roles?.docs || []) {
      const idNoun = role.noun?.value as NounRef
      setTableProperty({
        tables: schemas,
        subject: associationSchema,
        object: idNoun,
        nouns,
        required: cs.roles.find((r: any) => r.id === role.id) ? true : false,
        description: `${associationSchema.name} is uniquely identified by ${idNoun.name}`,
        property: createProperty({ object: idNoun, tables: schemas, nouns, jsonExamples }),
        jsonExamples,
      })
    }
  }

  const nounRegex = nounListToRegex(nouns)

  // Ensure domain-scoped entity nouns with permissions have schemas
  for (const noun of domainNouns.filter(
    (n: any) => n.permissions?.length && n.objectType === 'entity',
  )) {
    ensureTableExists({ tables: schemas, subject: noun, nouns, jsonExamples })
  }

  // ------ 6. Run the three fact type processors ------
  const examples: any[] = [] // No example graphs in DO model
  processBinarySchemas(constraintSpans, schemas, nouns, jsonExamples, nounRegex, examples, graphSchemas)
  processArraySchemas(arrayTypes, nouns, nounRegex, schemas, jsonExamples)
  processUnarySchemas(graphSchemas, nouns, nounRegex, schemas, jsonExamples, examples)

  // ------ 7. Flatten allOf chains ------
  const componentSchemas: [string, Schema][] = Object.entries(schemas)
  for (const [key, schema] of componentSchemas) {
    while (schema.allOf) {
      const mergedRequired: string[] = [...(schema.required || [])]
      let mergedProperties = schema.properties || {}
      const mergedAllOf: Schema[] = []
      schema.allOf.forEach((s) => {
        const dependency = schemas[(s as Schema).$ref?.split('/').pop() || '']
        if (!dependency) return // Guard against missing refs
        if (dependency.required?.length)
          mergedRequired.push(
            ...dependency.required.filter((f: string) => !mergedRequired.includes(f)),
          )
        if (Object.keys(dependency.properties || {}).length)
          mergedProperties = { ...dependency.properties, ...mergedProperties }
        if (dependency.allOf?.length) mergedAllOf.push(...dependency.allOf.map((a) => a))
        if (!schema.title && dependency.title) schema.title = dependency.title
        if (!schema.description && dependency.description)
          schema.description = dependency.description
        if (!schema.type && dependency.type) schema.type = dependency.type
        if (!schema.examples && Object.keys(dependency.examples || {}).length)
          schema.examples = dependency.examples
      })
      delete schema.allOf
      if (Object.keys(mergedProperties).length) schema.properties = mergedProperties
      if (mergedRequired.length) schema.required = mergedRequired
      if (mergedAllOf.length) schema.allOf = mergedAllOf
    }
    schemas[key] = schema
  }

  // ------ 8. Return OpenAPI-style output ------
  return {
    openapi: '3.0.0',
    info: {
      title: `Domain ${domainId} Schema`,
      version: '1.0.0',
    },
    components: {
      schemas,
    },
  }
}

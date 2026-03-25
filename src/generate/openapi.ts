/**
 * generateOpenAPI — main orchestrator that takes a DomainModel and produces
 * an OpenAPI-style `{ components: { schemas } }` object.
 *
 * Adapts DomainModel data (NounDef, FactTypeDef, ConstraintDef, SpanDef)
 * into the nested shapes that fact-processors.ts and schema-builder.ts expect.
 */

import type { NounDef, FactTypeDef, ConstraintDef, SpanDef } from '../model/types'
import type { TableDef } from '../rmap/procedure'
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
// generateOpenAPI
// ---------------------------------------------------------------------------

/**
 * Consume a DomainModel and produce an OpenAPI-style schema object.
 *
 * @param model - Structural DomainModel with async accessors
 * @returns OpenAPI 3.0.0 object with `components.schemas`
 */
export async function generateOpenAPI(model: {
  domainId: string
  nouns(): Promise<Map<string, NounDef>>
  factTypes(): Promise<Map<string, FactTypeDef>>
  constraints(): Promise<ConstraintDef[]>
  constraintSpans(): Promise<Map<string, SpanDef[]>>
}): Promise<any> {
  const schemas: Record<string, Schema> = {}

  // ------ Step A: Fetch data from DomainModel ------
  const nounMap = await model.nouns()
  const ftMap = await model.factTypes()
  const allConstraints = await model.constraints()
  const spanMap = await model.constraintSpans()

  // ------ Step B: Build nouns array ------
  const nouns: NounRef[] = [...nounMap.values()]
  const allNouns: NounRef[] = [...nouns] // mutable copy for association schemas

  // ------ Step C: Adapt FactTypeDef → GraphSchema shape ------
  // fact-processors expect { id, name, roles: { docs: Role[] }, readings: { docs: Reading[] } }
  const graphSchemas = [...ftMap.values()].map((ft) => ({
    id: ft.id,
    name: ft.name || ft.reading,
    roles: {
      docs: ft.roles.map((r) => ({
        id: r.id,
        noun: { value: r.nounDef },
        graphSchema: { id: ft.id },
        required: false,
      })),
    },
    readings: {
      docs: [{ text: ft.reading }],
    },
  }))

  // ------ Step D: Build constraint spans in the shape fact-processors expect ------
  const ucConstraintIds = new Set(
    allConstraints.filter((c) => c.kind === 'UC').map((c) => c.id),
  )

  // Step D.1: Mark roles as required based on MC (mandatory) constraints.
  // Per Halpin Ch 10 / Table 9.2: "Simple mandatory role" maps to NOT NULL / required.
  const mcConstraintIds = new Set(
    allConstraints.filter((c) => c.kind === 'MC').map((c) => c.id),
  )
  for (const [constraintId, spans] of spanMap) {
    if (!mcConstraintIds.has(constraintId)) continue
    for (const span of spans) {
      const gs = graphSchemas.find((g) => g.id === span.factTypeId)
      if (!gs) continue
      const ft = ftMap.get(span.factTypeId)
      if (!ft) continue
      // MC spans the role that MUST be played — the OTHER role becomes required
      // on the subject entity. For "Each Person was born in some Country",
      // MC spans Person's role → Country column is required on Person's table.
      // Find the constrained role and mark the opposite role as required.
      const ftRole = ft.roles.find((fr) => fr.roleIndex === span.roleIndex)
      if (!ftRole) continue
      // In a binary, the opposite role's property becomes required
      const oppositeRole = gs.roles?.docs?.find((r) => r.id !== ftRole.id)
      if (oppositeRole) oppositeRole.required = true
    }
  }

  // Step D.2: Build UC constraint spans: { roles: Role[] }[]
  const constraintSpans: { roles: any[] }[] = []
  for (const [constraintId, spans] of spanMap) {
    if (!ucConstraintIds.has(constraintId)) continue
    const roles: any[] = []
    for (const span of spans) {
      // Find the adapted graphSchema (fact type) and the role within it
      const gs = graphSchemas.find((g) => g.id === span.factTypeId)
      const role = gs?.roles?.docs?.find((r) => {
        // Match by roleIndex within the fact type
        const ft = ftMap.get(span.factTypeId)
        const ftRole = ft?.roles.find((fr) => fr.roleIndex === span.roleIndex)
        return ftRole && r.id === ftRole.id
      })
      if (role) roles.push(role)
    }
    if (roles.length > 0) constraintSpans.push({ roles })
  }

  // ------ Step E: Identify compound uniqueness constraints ------
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

  // ------ Step F: Process association schemas (objectified entities) ------
  nouns.push(...associationSchemas.map(({ gs }) => gs as any))

  // No json-examples in the DomainModel — pass empty
  const jsonExamples: Record<string, JSONSchemaType> = {}

  for (const { gs: associationSchema, cs } of associationSchemas) {
    const key = (associationSchema.name || '').replace(/ /g, '')
    const jsonExample = jsonExamples[key]
    schemas['Update' + key] = {
      $id: 'Update' + key,
      title: associationSchema.name || '',
      type: 'object',
      description:
        (associationSchema as any).description ||
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
        subject: associationSchema as any,
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

  // RMAP Step 3: every entity type gets a table
  for (const noun of [...nounMap.values()].filter((n) => n.objectType === 'entity')) {
    ensureTableExists({ tables: schemas, subject: noun, nouns, jsonExamples })
  }

  // ------ Step G: Run the three fact type processors ------
  const examples: any[] = [] // No example graphs in DomainModel
  processBinarySchemas(constraintSpans, schemas, nouns, jsonExamples, nounRegex, examples, graphSchemas)
  processArraySchemas(arrayTypes, nouns, nounRegex, schemas, jsonExamples)
  processUnarySchemas(graphSchemas, nouns, nounRegex, schemas, jsonExamples, examples)

  // ------ Step G.5: Propagate supertype properties to subtypes ------
  // If Resource has a property from "StateMachine is for Resource",
  // then SupportRequest (subtype of Resource) should inherit that property.
  const subtypeMap = new Map<string, string[]>() // parent name → child names
  for (const noun of nouns) {
    if (noun.superType) {
      const parentName = typeof noun.superType === 'string' ? noun.superType : noun.superType.name
      if (parentName) {
        const children = subtypeMap.get(parentName) || []
        children.push(noun.name)
        subtypeMap.set(parentName, children)
      }
    }
  }

  // For each parent entity with subtypes, copy properties to child Update schemas
  for (const [parentName, childNames] of subtypeMap) {
    const parentKey = nameToKey('Update' + parentName)
    const parentSchema = schemas[parentKey]
    if (!parentSchema?.properties) continue

    for (const childName of childNames) {
      const childKey = nameToKey('Update' + childName)
      if (!schemas[childKey]) continue
      // Merge parent properties into child (child's own properties take precedence)
      const childProps = schemas[childKey].properties || {}
      schemas[childKey].properties = { ...parentSchema.properties, ...childProps }
      // Also merge required arrays
      if (parentSchema.required) {
        const childRequired = new Set(schemas[childKey].required || [])
        for (const r of parentSchema.required) childRequired.add(r)
        schemas[childKey].required = [...childRequired]
      }
    }
  }

  // ------ Step H: Flatten allOf chains ------
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

  // ------ Step I: Return OpenAPI-style output ------
  return {
    openapi: '3.0.0',
    info: {
      title: `Domain ${model.domainId} Schema`,
      version: '1.0.0',
    },
    components: {
      schemas,
    },
  }
}

// ---------------------------------------------------------------------------
// generateOpenAPIFromRmap — RMAP-driven OpenAPI generation
// ---------------------------------------------------------------------------

/** Map RMAP column types to JSON Schema / OpenAPI types */
const SQLITE_TO_JSON_SCHEMA: Record<string, { type: string; format?: string }> = {
  TEXT: { type: 'string' },
  INTEGER: { type: 'integer' },
  REAL: { type: 'number' },
  BLOB: { type: 'string', format: 'binary' },
}

function toJsonSchemaType(sqlType: string): { type: string; format?: string } {
  return SQLITE_TO_JSON_SCHEMA[sqlType.toUpperCase()] ?? { type: 'string' }
}

/** Convert snake_case table name to PascalCase schema name */
function toPascalCase(snake: string): string {
  return snake
    .split('_')
    .map(w => w.charAt(0).toUpperCase() + w.slice(1))
    .join('')
}

/**
 * Generate an OpenAPI 3.0 document directly from RMAP `TableDef[]` output.
 *
 * This is an alternative code path that bypasses the DomainModel-driven
 * `generateOpenAPI()`, consuming pre-computed relational table definitions
 * from `rmap()` in `src/rmap/procedure.ts`.
 *
 * Each table becomes a component schema. Columns become properties.
 * FK references become `$ref` links. NOT NULL columns become required.
 *
 * @param tables - RMAP output table definitions
 * @param domainName - Human-readable domain name for the info title
 */
export function generateOpenAPIFromRmap(tables: TableDef[], domainName: string): object {
  const schemas: Record<string, any> = {}

  for (const table of tables) {
    const schemaName = toPascalCase(table.name)
    const properties: Record<string, any> = {}
    const required: string[] = []

    for (const col of table.columns) {
      if (col.references) {
        // FK reference → $ref to the referenced table's schema
        const refSchemaName = toPascalCase(col.references)
        properties[col.name] = {
          oneOf: [
            toJsonSchemaType(col.type),
            { $ref: `#/components/schemas/${refSchemaName}` },
          ],
          description: `FK reference to ${col.references}`,
        }
      } else {
        const jsonType = toJsonSchemaType(col.type)
        properties[col.name] = { ...jsonType }
      }

      if (!col.nullable) {
        required.push(col.name)
      }
    }

    // Add enum constraints from CHECK (column IN (...)) patterns
    if (table.checks) {
      for (const check of table.checks) {
        const match = check.match(/^(\w+)\s+IN\s*\((.+)\)$/i)
        if (match) {
          const colName = match[1]
          const values = match[2]
            .split(',')
            .map(v => v.trim().replace(/^'(.*)'$/, '$1'))
          if (properties[colName]) {
            properties[colName].enum = values
          }
        }
      }
    }

    const schema: any = {
      type: 'object',
      properties,
    }
    if (required.length > 0) {
      schema.required = required
    }

    schemas[schemaName] = schema
  }

  return {
    openapi: '3.0.0',
    info: {
      title: `${domainName} Schema (RMAP)`,
      version: '1.0.0',
    },
    components: {
      schemas,
    },
  }
}

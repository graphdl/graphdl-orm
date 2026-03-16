/**
 * Schema builder functions — JSON Schema generation from ORM noun definitions.
 *
 * Ported from Generator.ts.bak (commit ddb8880) lines 2480-2812.
 * Depends on rmap.ts for nameToKey, transformPropertyName.
 */

import { nameToKey, transformPropertyName } from './rmap'
import type { NounDef } from '../model/types'

// ---------------------------------------------------------------------------
// Loose JSON Schema types (enough for the builder, not a full spec)
// ---------------------------------------------------------------------------
export type JSONSchemaType = string | number | boolean | null | JSONSchemaObject | JSONSchemaArray
export type JSONSchemaObject = { [key: string]: JSONSchemaType }
export type JSONSchemaArray = JSONSchemaType[]

export interface Schema {
  $id?: string
  $ref?: string
  type?: string
  title?: string
  description?: string
  format?: string
  pattern?: string
  enum?: (string | null)[]
  nullable?: boolean
  minLength?: number
  maxLength?: number
  minimum?: number
  exclusiveMinimum?: number
  maximum?: number
  exclusiveMaximum?: number
  multipleOf?: number
  oneOf?: Schema[]
  allOf?: Schema[]
  properties?: Record<string, Schema>
  required?: string[]
  examples?: JSONSchemaType[]
  [key: string]: unknown
}

// ---------------------------------------------------------------------------
// createProperty
// ---------------------------------------------------------------------------
/**
 * Create a JSON Schema property definition from a noun.
 *
 * Value types become primitive properties (string, number, boolean, with
 * format/pattern/enum/min/max). Entity types become `oneOf` with a `$ref`
 * plus an inline reference scheme.
 */
export function createProperty({
  description,
  object,
  nouns,
  tables,
  jsonExamples,
}: {
  description?: string
  object: NounDef
  nouns: NounDef[]
  tables: Record<string, Schema>
  jsonExamples: Record<string, JSONSchemaType>
}): Schema {
  if (!object) return {}

  // Resolve string id → NounDef
  if (typeof (object as any) === 'string') {
    object = nouns.find((n) => n.id === (object as any)) || ({ id: object, name: object } as any)
  } else if (object.id) {
    object = nouns.find((n) => n.id === object.id) || object
  }

  const property: Schema = {}
  let { referenceScheme, superType, valueType } = object

  // Traverse supertype chain to resolve valueType or referenceScheme
  while (!referenceScheme?.length && !valueType && superType) {
    if (typeof superType === 'string') superType = nouns.find((n) => n.id === superType || n.name === superType) as NounDef
    referenceScheme = superType?.referenceScheme
    valueType = superType?.valueType
    superType = superType?.superType
  }

  // Value type nouns default to string when valueType isn't explicitly set
  if (!valueType && object.objectType === 'value') {
    valueType = 'string'
  }

  if (valueType) {
    // ---- Value type → primitive property ----
    property.type = valueType
    if (object.format) property.format = String(object.format)
    if (object.pattern) property.pattern = String(object.pattern)
    if (object.enumValues)
      property.enum = object.enumValues.map((e) => {
        if (e === 'null') {
          property.nullable = true
          return null
        }
        return e
      })
    if (typeof object.minLength === 'number') property.minLength = object.minLength
    if (typeof object.maxLength === 'number') property.maxLength = object.maxLength
    if (typeof object.minimum === 'number') property.minimum = object.minimum
    if (typeof object.exclusiveMinimum === 'number') property.exclusiveMinimum = object.exclusiveMinimum
    if (typeof object.exclusiveMaximum === 'number') property.exclusiveMaximum = object.exclusiveMaximum
    if (typeof object.maximum === 'number') property.maximum = object.maximum
    if (typeof object.multipleOf === 'number') property.multipleOf = object.multipleOf
    if (description) property.description = description
  } else {
    // ---- Entity type → oneOf with $ref + inline reference scheme ----
    const required: string[] = []
    const propertyKey = nameToKey(object.name || '')

    property.oneOf = [
      (referenceScheme?.length || 0) > 1
        ? {
            type: 'object',
            properties: Object.fromEntries(
              referenceScheme?.map((role) => {
                const propertyName = transformPropertyName(role.name || '')
                required.push(propertyName)
                return [
                  propertyName,
                  createProperty({ object: role, tables, nouns, description, jsonExamples }),
                ]
              }) || [],
            ),
            required,
          }
        : referenceScheme
          ? createProperty({
              object: referenceScheme[0],
              tables,
              nouns,
              description,
              jsonExamples,
            })
          : {},
      { $ref: '#/components/schemas/' + propertyKey },
    ]
    ensureTableExists({ tables, subject: object, nouns, jsonExamples })
  }

  return property
}

// ---------------------------------------------------------------------------
// ensureTableExists
// ---------------------------------------------------------------------------
/**
 * Idempotently create the UpdateX / NewX / X schema triplet for a noun.
 *
 * Unpacks the reference scheme into properties and wires the supertype chain
 * via allOf references.
 */
export function ensureTableExists({
  tables,
  subject,
  nouns,
  jsonExamples,
}: {
  tables: Record<string, Schema>
  subject: NounDef
  nouns: NounDef[]
  jsonExamples: Record<string, JSONSchemaType>
}): void {
  // Value types don't get their own tables — they become inline columns
  if (subject.objectType === 'value') return

  const title = subject.name || ''
  const key = nameToKey(title)
  if (tables[key]) return

  tables['Update' + key] = {
    $id: 'Update' + key,
    title: subject.name || '',
  }
  tables['New' + key] = {
    $id: 'New' + key,
    allOf: [{ $ref: '#/components/schemas/Update' + key }],
  }
  tables[key] = {
    $id: key,
    allOf: [{ $ref: '#/components/schemas/New' + key }],
  }

  if (subject.description) tables['Update' + key].description = subject.description

  const json = jsonExamples[key]
  if (json) {
    tables['Update' + key].examples = [json]
    tables['New' + key].examples = [json]
    tables[key].examples = [json]
  }

  // Unpack reference scheme into properties
  if (subject.referenceScheme) {
    for (const idRole of subject.referenceScheme) {
      const property = createProperty({ object: idRole, nouns, tables, jsonExamples })
      setTableProperty({
        tables,
        subject,
        object: idRole,
        nouns,
        required: true,
        property,
        description: `${title} is uniquely identified by ${idRole.name}`,
        jsonExamples,
      })
    }
  }

  // Wire supertype chain
  let superType: NounDef | string | undefined = subject.superType
  if (typeof superType === 'string') superType = nouns?.find((n) => n.id === superType || n.name === superType)
  if ((superType as NounDef)?.name) {
    superType = (superType as NounDef) || nouns?.find((n) => n.id === (superType as NounDef).id)
    const superTypeKey = nameToKey((superType as NounDef).name || '')
    tables['Update' + key].allOf = [{ $ref: '#/components/schemas/Update' + superTypeKey }]
    tables['New' + key].allOf?.push({ $ref: '#/components/schemas/New' + superTypeKey })
    tables[key].allOf?.push({ $ref: '#/components/schemas/' + superTypeKey })
    ensureTableExists({ tables, subject: superType as NounDef, nouns, jsonExamples })
  } else {
    tables['Update' + key].type = 'object'
  }
}

// ---------------------------------------------------------------------------
// setTableProperty
// ---------------------------------------------------------------------------
/**
 * Set a property on the UpdateX schema.
 *
 * Strips subject name prefix from property name (e.g. CustomerName on
 * Customer becomes "name"). Adds to required array on NewX if required.
 * Handles examples with type coercion.
 */
export function setTableProperty({
  tables,
  nouns,
  subject,
  object,
  propertyName,
  description,
  required,
  property,
  example,
  jsonExamples,
}: {
  tables: Record<string, Schema>
  nouns: NounDef[]
  subject: NounDef
  object: NounDef
  propertyName?: string
  description?: string
  required?: boolean
  property?: Schema
  example?: string
  jsonExamples: Record<string, JSONSchemaType>
}): void {
  if (!property) property = createProperty({ object, tables, nouns, jsonExamples })
  if (description) property.description = description

  propertyName ||= transformPropertyName(object.name || '')

  // Strip subject name prefix from property name
  const compareName = subject.name?.replace(/ /g, '')?.toUpperCase() || ''
  if (
    subject.name &&
    propertyName.toUpperCase().startsWith(compareName) &&
    propertyName.length > compareName.length &&
    propertyName[compareName.length] === propertyName[compareName.length].toUpperCase()
  ) {
    propertyName = transformPropertyName(propertyName.slice(compareName.length))
  }

  const key = nameToKey('Update' + (subject.name || ''))
  const properties = tables[key].properties ?? {}
  properties[propertyName] = property
  tables[key].properties = properties

  if (required) {
    const reqKey = nameToKey((propertyName === 'id' ? '' : 'New') + (subject.name || ''))
    if (!tables[reqKey].required) tables[reqKey].required = []
    tables[reqKey].required?.push(propertyName)
  }

  if (example) {
    const examples = (tables[key].examples as JSONSchemaArray) || [{}]
    switch (property.type) {
      case 'integer':
        ;(examples[0] as JSONSchemaObject)[propertyName] = parseInt(example)
        break
      case 'number':
        ;(examples[0] as JSONSchemaObject)[propertyName] = parseFloat(example)
        break
      case 'boolean':
        ;(examples[0] as JSONSchemaObject)[propertyName] = example === 'true'
        break
      default:
        ;(examples[0] as JSONSchemaObject)[propertyName] = example
        break
    }
    tables[key].examples = examples
  }
}

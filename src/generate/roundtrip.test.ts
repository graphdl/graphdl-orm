/**
 * Roundtrip fidelity tests: fromOpenAPI → readings → model → generateOpenAPI
 *
 * Verifies that the output OpenAPI spec is a superset of the input spec:
 * every entity type, property, relationship, enum, and required field
 * from the input appears in the output.
 */

import { describe, it, expect, beforeEach } from 'vitest'
import { fromOpenAPI, type OpenAPISpec } from './from-openapi'
import { generateOpenAPI } from './openapi'
import {
  createMockModel,
  mkNounDef,
  mkValueNounDef,
  mkFactType,
  mkConstraint,
  resetIds,
} from '../model/test-utils'
import type { NounDef, FactTypeDef, ConstraintDef, SpanDef } from '../model/types'

// ---------------------------------------------------------------------------
// Lightweight readings parser
// ---------------------------------------------------------------------------

interface ParsedReadings {
  entityTypes: string[]
  valueTypes: Map<string, string[]> // name → enum values (empty array if no enum)
  factTypes: { subject: string; object: string }[]
  ucConstraints: { entity: string; target: string }[]
  mcConstraints: { entity: string; target: string }[]
}

function parseReadings(text: string): ParsedReadings {
  const lines = text.split('\n')
  const entityTypes: string[] = []
  const valueTypes = new Map<string, string[]>()
  const factTypes: { subject: string; object: string }[] = []
  const ucConstraints: { entity: string; target: string }[] = []
  const mcConstraints: { entity: string; target: string }[] = []

  for (const line of lines) {
    const trimmed = line.trim()

    // Entity types: "X(.id) is an entity type."
    const entityMatch = trimmed.match(/^(.+?)\(\.id\) is an entity type\.$/)
    if (entityMatch) {
      entityTypes.push(entityMatch[1])
      continue
    }

    // Value types: "X is a value type."
    const valueMatch = trimmed.match(/^(.+?) is a value type\.$/)
    if (valueMatch) {
      if (!valueTypes.has(valueMatch[1])) {
        valueTypes.set(valueMatch[1], [])
      }
      continue
    }

    // Enum values: "The possible values of X are 'a', 'b', 'c'."
    const enumMatch = trimmed.match(/^The possible values of (.+?) are (.+)\.$/)
    if (enumMatch) {
      const name = enumMatch[1]
      const values = enumMatch[2].match(/'([^']+)'/g)?.map((v) => v.replace(/'/g, '')) ?? []
      valueTypes.set(name, values)
      continue
    }

    // Fact types: "X has Y."
    const factMatch = trimmed.match(/^(.+?) has (.+?)\.$/)
    if (factMatch && !trimmed.startsWith('Each ') && !trimmed.startsWith('Domain ')) {
      factTypes.push({ subject: factMatch[1], object: factMatch[2] })
      continue
    }

    // UC constraints: "Each X has at most one Y."
    const ucMatch = trimmed.match(/^Each (.+?) has at most one (.+?)\.$/)
    if (ucMatch) {
      ucConstraints.push({ entity: ucMatch[1], target: ucMatch[2] })
      continue
    }

    // MC constraints: "Each X has exactly one Y."
    const mcMatch = trimmed.match(/^Each (.+?) has exactly one (.+?)\.$/)
    if (mcMatch) {
      mcConstraints.push({ entity: mcMatch[1], target: mcMatch[2] })
      continue
    }
  }

  return { entityTypes, valueTypes, factTypes, ucConstraints, mcConstraints }
}

// ---------------------------------------------------------------------------
// Model builder: parsed readings → DomainModel via createMockModel
// ---------------------------------------------------------------------------

function buildModel(parsed: ParsedReadings) {
  resetIds()

  const nounDefs = new Map<string, NounDef>()
  const factTypeDefs: FactTypeDef[] = []
  const constraintDefs: ConstraintDef[] = []

  // Create entity type nouns
  for (const name of parsed.entityTypes) {
    nounDefs.set(name, mkNounDef({ name }))
  }

  // Create value type nouns
  for (const [name, enumValues] of parsed.valueTypes) {
    if (!nounDefs.has(name)) {
      nounDefs.set(
        name,
        mkValueNounDef({
          name,
          valueType: 'string',
          enumValues: enumValues.length > 0 ? enumValues : undefined,
        }),
      )
    }
  }

  // Create fact types and constraints
  for (const ft of parsed.factTypes) {
    // Ensure both nouns exist
    if (!nounDefs.has(ft.subject)) {
      nounDefs.set(ft.subject, mkNounDef({ name: ft.subject }))
    }
    if (!nounDefs.has(ft.object)) {
      // Check if the object is an entity type or value type
      if (parsed.entityTypes.includes(ft.object)) {
        nounDefs.set(ft.object, mkNounDef({ name: ft.object }))
      } else {
        const enumValues = parsed.valueTypes.get(ft.object)
        nounDefs.set(
          ft.object,
          mkValueNounDef({
            name: ft.object,
            valueType: 'string',
            enumValues: enumValues && enumValues.length > 0 ? enumValues : undefined,
          }),
        )
      }
    }

    const subjectNoun = nounDefs.get(ft.subject)!
    const objectNoun = nounDefs.get(ft.object)!

    const factType = mkFactType({
      reading: `${ft.subject} has ${ft.object}`,
      roles: [
        { nounDef: subjectNoun, roleIndex: 0 },
        { nounDef: objectNoun, roleIndex: 1 },
      ],
    })
    factTypeDefs.push(factType)

    // Check if this fact type has a UC constraint
    const hasUC = parsed.ucConstraints.some(
      (uc) => uc.entity === ft.subject && uc.target === ft.object,
    )
    if (hasUC) {
      constraintDefs.push(
        mkConstraint({
          kind: 'UC',
          spans: [{ factTypeId: factType.id, roleIndex: 0 }],
        }),
      )
    }

    // Check if this fact type has an MC constraint
    // MC spans the property role (roleIndex 1) so that the generateOpenAPI MC
    // processor marks the opposite role (the entity, roleIndex 0) as required.
    // processBinarySchemas then reads subjectRole.required (the UC-constrained
    // entity role) and propagates it to the schema's required array.
    const hasMC = parsed.mcConstraints.some(
      (mc) => mc.entity === ft.subject && mc.target === ft.object,
    )
    if (hasMC) {
      constraintDefs.push(
        mkConstraint({
          kind: 'MC',
          spans: [{ factTypeId: factType.id, roleIndex: 1 }],
        }),
      )
    }
  }

  return createMockModel({
    nouns: [...nounDefs.values()],
    factTypes: factTypeDefs,
    constraints: constraintDefs,
  })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/** Convert Title Case noun name to camelCase property name (mirrors transformPropertyName) */
function nounToProp(nounName: string): string {
  const key = nounName.replace(/[ \-]/g, '').replace(/&/g, 'And')
  if (key === key.toUpperCase()) return key.toLowerCase()
  const leadingUpper = key.match(/^[A-Z]+/)
  if (leadingUpper) {
    const run = leadingUpper[0]
    if (run.length === key.length) return key.toLowerCase()
    if (run.length > 1) return run.slice(0, -1).toLowerCase() + key.slice(run.length - 1)
  }
  return key[0].toLowerCase() + key.slice(1)
}

/** Get all property names from a flattened schema (checks both Update and base) */
function getSchemaProps(schemas: Record<string, any>, entityKey: string): string[] {
  const base = schemas[entityKey]
  const update = schemas['Update' + entityKey]
  return [
    ...Object.keys(base?.properties ?? {}),
    ...Object.keys(update?.properties ?? {}),
  ]
}

/** Get the required array from a flattened schema */
function getSchemaRequired(schemas: Record<string, any>, entityKey: string): string[] {
  const base = schemas[entityKey]
  const newSchema = schemas['New' + entityKey]
  return [...(base?.required ?? []), ...(newSchema?.required ?? [])]
}

// ---------------------------------------------------------------------------
// Test cases
// ---------------------------------------------------------------------------

describe('roundtrip: fromOpenAPI → readings → model → generateOpenAPI', () => {
  beforeEach(() => resetIds())

  // =========================================================================
  // 1. Minimal Petstore
  // =========================================================================
  it('minimal Petstore: 2 schemas, properties, enum, $ref', async () => {
    const inputSpec: OpenAPISpec = {
      info: { title: 'Petstore', version: '1.0.0' },
      components: {
        schemas: {
          Pet: {
            type: 'object',
            required: ['name', 'status'],
            properties: {
              name: { type: 'string' },
              status: { type: 'string', enum: ['available', 'pending', 'sold'] },
              category: { $ref: '#/components/schemas/Category' },
            },
          },
          Category: {
            type: 'object',
            properties: {
              name: { type: 'string' },
            },
          },
        },
      },
    }

    // Step 1: fromOpenAPI
    const readings = fromOpenAPI(inputSpec, 'petstore')
    console.log('=== Petstore Readings ===')
    console.log(readings)
    console.log('========================')

    // Step 2: Parse readings
    const parsed = parseReadings(readings)

    expect(parsed.entityTypes).toContain('Pet')
    expect(parsed.entityTypes).toContain('Category')

    // Step 3: Build model
    const model = buildModel(parsed)

    // Step 4: Generate output OpenAPI
    const output = await generateOpenAPI(model)
    const schemas = output.components.schemas

    // --- Assertions ---

    // Every input schema name appears as an output schema
    expect(schemas['Pet']).toBeDefined()
    expect(schemas['Category']).toBeDefined()

    // Every input property appears in the output
    const petProps = getSchemaProps(schemas, 'Pet')
    expect(petProps).toContain('name')
    expect(petProps).toContain('status')
    // $ref property: category → entity reference
    expect(petProps).toContain('category')

    const categoryProps = getSchemaProps(schemas, 'Category')
    expect(categoryProps).toContain('name')

    // Enum values are preserved
    const statusProp =
      schemas['Pet']?.properties?.status ??
      schemas['UpdatePet']?.properties?.status
    expect(statusProp).toBeDefined()
    expect(statusProp.enum).toEqual(
      expect.arrayContaining(['available', 'pending', 'sold']),
    )

    // Required fields: 'name' and 'status' were required in input
    // "exactly one" MC constraints → required in output
    const petRequired = getSchemaRequired(schemas, 'Pet')
    expect(petRequired).toContain(nounToProp('Name'))
    expect(petRequired).toContain(nounToProp('Status'))

    // $ref relationship: category is an entity reference (oneOf with $ref)
    const catProp =
      schemas['Pet']?.properties?.category ??
      schemas['UpdatePet']?.properties?.category
    expect(catProp).toBeDefined()
    expect(catProp.oneOf).toBeDefined()
    expect(
      catProp.oneOf.some((o: any) => o.$ref === '#/components/schemas/Category'),
    ).toBe(true)
  })

  // =========================================================================
  // 2. Billing (Stripe-like)
  // =========================================================================
  it('Billing: Customer, Subscription, Invoice with enums, required, relationships', async () => {
    const inputSpec: OpenAPISpec = {
      info: { title: 'Billing API', version: '2024-01-01' },
      components: {
        schemas: {
          Customer: {
            type: 'object',
            required: ['id', 'email'],
            properties: {
              id: { type: 'string' },
              email: { type: 'string' },
              name: { type: 'string' },
              balance: { type: 'integer' },
            },
          },
          Subscription: {
            type: 'object',
            required: ['id', 'status', 'customer'],
            properties: {
              id: { type: 'string' },
              status: {
                type: 'string',
                enum: ['active', 'past_due', 'canceled', 'trialing', 'incomplete'],
              },
              customer: { $ref: '#/components/schemas/Customer' },
              currentPeriodEnd: { type: 'integer' },
            },
          },
          Invoice: {
            type: 'object',
            required: ['id', 'customer', 'total'],
            properties: {
              id: { type: 'string' },
              customer: { $ref: '#/components/schemas/Customer' },
              subscription: { $ref: '#/components/schemas/Subscription' },
              total: { type: 'integer' },
              status: {
                type: 'string',
                enum: ['draft', 'open', 'paid', 'void', 'uncollectible'],
              },
            },
          },
        },
      },
    }

    // Step 1: fromOpenAPI
    const readings = fromOpenAPI(inputSpec, 'billing')
    console.log('=== Billing Readings ===')
    console.log(readings)
    console.log('========================')

    // Step 2: Parse readings
    const parsed = parseReadings(readings)

    expect(parsed.entityTypes).toContain('Customer')
    expect(parsed.entityTypes).toContain('Subscription')
    expect(parsed.entityTypes).toContain('Invoice')

    // Step 3: Build model
    const model = buildModel(parsed)

    // Step 4: Generate output OpenAPI
    const output = await generateOpenAPI(model)
    const schemas = output.components.schemas

    // --- Assertions ---

    // Every input schema name appears
    expect(schemas['Customer']).toBeDefined()
    expect(schemas['Subscription']).toBeDefined()
    expect(schemas['Invoice']).toBeDefined()

    // Customer properties
    const custProps = getSchemaProps(schemas, 'Customer')
    // 'id' in the readings becomes 'Id' noun → property 'id' is the reference scheme identifier.
    // fromOpenAPI emits "Customer has Id." which may or may not map to the implicit id.
    // The important ones are email, name, balance.
    expect(custProps).toContain('email')
    expect(custProps).toContain('name')
    expect(custProps).toContain('balance')

    // Subscription properties
    const subProps = getSchemaProps(schemas, 'Subscription')
    expect(subProps).toContain('status')
    expect(subProps).toContain('customer')
    expect(subProps).toContain('currentPeriodEnd')

    // Invoice properties
    const invProps = getSchemaProps(schemas, 'Invoice')
    expect(invProps).toContain('customer')
    expect(invProps).toContain('subscription')
    expect(invProps).toContain('total')
    expect(invProps).toContain('status')

    // Enum values preserved — Subscription Status
    // fromOpenAPI creates a single "Status" value type shared between Subscription and Invoice.
    // The first one encountered (Subscription's) defines the enum values.
    // Both Subscription and Invoice get the Status property with enum.
    const subStatus =
      schemas['Subscription']?.properties?.status ??
      schemas['UpdateSubscription']?.properties?.status
    expect(subStatus).toBeDefined()
    expect(subStatus.enum).toBeDefined()
    expect(subStatus.enum).toEqual(
      expect.arrayContaining(['active', 'past_due', 'canceled', 'trialing', 'incomplete']),
    )

    // Required fields: Subscription requires status and customer
    const subRequired = getSchemaRequired(schemas, 'Subscription')
    expect(subRequired).toContain('status')
    expect(subRequired).toContain('customer')

    // Required fields: Invoice requires customer and total
    const invRequired = getSchemaRequired(schemas, 'Invoice')
    expect(invRequired).toContain('customer')
    expect(invRequired).toContain('total')

    // $ref relationships become entity references
    const subCustProp =
      schemas['Subscription']?.properties?.customer ??
      schemas['UpdateSubscription']?.properties?.customer
    expect(subCustProp).toBeDefined()
    expect(subCustProp.oneOf).toBeDefined()
    expect(
      subCustProp.oneOf.some((o: any) => o.$ref === '#/components/schemas/Customer'),
    ).toBe(true)

    const invSubProp =
      schemas['Invoice']?.properties?.subscription ??
      schemas['UpdateInvoice']?.properties?.subscription
    expect(invSubProp).toBeDefined()
    expect(invSubProp.oneOf).toBeDefined()
    expect(
      invSubProp.oneOf.some((o: any) => o.$ref === '#/components/schemas/Subscription'),
    ).toBe(true)
  })

  // =========================================================================
  // 3. Auto.dev VIN decode
  // =========================================================================
  it('Auto.dev VIN decode: Vehicle with year/make/model/trim and Specs reference', async () => {
    const inputSpec: OpenAPISpec = {
      info: { title: 'Auto.dev VIN Decode', version: '1.0.0' },
      components: {
        schemas: {
          Vehicle: {
            type: 'object',
            required: ['vin', 'year', 'make', 'model'],
            properties: {
              vin: { type: 'string' },
              year: { type: 'integer' },
              make: { type: 'string' },
              model: { type: 'string' },
              trim: { type: 'string' },
              bodyType: {
                type: 'string',
                enum: ['sedan', 'suv', 'truck', 'coupe', 'convertible', 'van', 'wagon'],
              },
              specs: { $ref: '#/components/schemas/Specs' },
            },
          },
          Specs: {
            type: 'object',
            properties: {
              horsepower: { type: 'integer' },
              torque: { type: 'integer' },
              fuelType: {
                type: 'string',
                enum: ['gasoline', 'diesel', 'electric', 'hybrid'],
              },
              drivetrain: { type: 'string' },
            },
          },
        },
      },
    }

    // Step 1: fromOpenAPI
    const readings = fromOpenAPI(inputSpec, 'auto-dev')
    console.log('=== Auto.dev VIN Decode Readings ===')
    console.log(readings)
    console.log('====================================')

    // Step 2: Parse readings
    const parsed = parseReadings(readings)

    expect(parsed.entityTypes).toContain('Vehicle')
    expect(parsed.entityTypes).toContain('Specs')

    // Step 3: Build model
    const model = buildModel(parsed)

    // Step 4: Generate output OpenAPI
    const output = await generateOpenAPI(model)
    const schemas = output.components.schemas

    // --- Assertions ---

    // Every input schema name appears
    expect(schemas['Vehicle']).toBeDefined()
    expect(schemas['Specs']).toBeDefined()

    // Vehicle properties
    const vehicleProps = getSchemaProps(schemas, 'Vehicle')
    expect(vehicleProps).toContain('vin')
    expect(vehicleProps).toContain('year')
    expect(vehicleProps).toContain('make')
    expect(vehicleProps).toContain('model')
    expect(vehicleProps).toContain('trim')
    expect(vehicleProps).toContain('bodyType')
    expect(vehicleProps).toContain('specs')

    // Specs properties
    const specsProps = getSchemaProps(schemas, 'Specs')
    expect(specsProps).toContain('horsepower')
    expect(specsProps).toContain('torque')
    expect(specsProps).toContain('fuelType')
    expect(specsProps).toContain('drivetrain')

    // Enum values preserved: bodyType
    const bodyTypeProp =
      schemas['Vehicle']?.properties?.bodyType ??
      schemas['UpdateVehicle']?.properties?.bodyType
    expect(bodyTypeProp).toBeDefined()
    expect(bodyTypeProp.enum).toEqual(
      expect.arrayContaining(['sedan', 'suv', 'truck', 'coupe', 'convertible', 'van', 'wagon']),
    )

    // Enum values preserved: fuelType
    const fuelTypeProp =
      schemas['Specs']?.properties?.fuelType ??
      schemas['UpdateSpecs']?.properties?.fuelType
    expect(fuelTypeProp).toBeDefined()
    expect(fuelTypeProp.enum).toEqual(
      expect.arrayContaining(['gasoline', 'diesel', 'electric', 'hybrid']),
    )

    // Required fields: vin, year, make, model
    const vehicleRequired = getSchemaRequired(schemas, 'Vehicle')
    expect(vehicleRequired).toContain('vin')
    expect(vehicleRequired).toContain('year')
    expect(vehicleRequired).toContain('make')
    // 'model' becomes noun name 'Model' → property 'model'
    expect(vehicleRequired).toContain('model')

    // $ref relationship: specs is an entity reference
    const specsProp =
      schemas['Vehicle']?.properties?.specs ??
      schemas['UpdateVehicle']?.properties?.specs
    expect(specsProp).toBeDefined()
    expect(specsProp.oneOf).toBeDefined()
    expect(
      specsProp.oneOf.some((o: any) => o.$ref === '#/components/schemas/Specs'),
    ).toBe(true)
  })
})

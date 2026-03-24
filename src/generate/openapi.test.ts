import { describe, it, expect, beforeEach } from 'vitest'
import { generateOpenAPI, generateOpenAPIFromRmap } from './openapi'
import type { TableDef } from '../rmap/procedure'
import {
  createMockModel,
  mkNounDef,
  mkValueNounDef,
  mkFactType,
  mkConstraint,
  resetIds,
} from '../model/test-utils'

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

describe('generateOpenAPI', () => {
  beforeEach(() => resetIds())

  it('returns valid OpenAPI structure for empty domain', async () => {
    const model = createMockModel({
      nouns: [],
      factTypes: [],
      constraints: [],
    })

    const result = await generateOpenAPI(model)

    expect(result.openapi).toBe('3.0.0')
    expect(result.info).toBeDefined()
    expect(result.info.version).toBe('1.0.0')
    expect(result.components).toBeDefined()
    expect(result.components.schemas).toEqual({})
  })

  it('creates schema with value-type property from a binary reading with UC', async () => {
    // Domain: Customer has Name
    // Single-role UC on Customer role → Name becomes a property of Customer
    const customerNoun = mkNounDef({ name: 'Customer' })
    const nameNoun = mkValueNounDef({ name: 'Name', valueType: 'string' })

    const ft = mkFactType({
      id: 'gs1',
      reading: 'Customer has Name',
      roles: [
        { nounDef: customerNoun, roleIndex: 0 },
        { nounDef: nameNoun, roleIndex: 1 },
      ],
    })

    const model = createMockModel({
      nouns: [customerNoun, nameNoun],
      factTypes: [ft],
      constraints: [
        mkConstraint({
          kind: 'UC',
          spans: [{ factTypeId: ft.id, roleIndex: 0 }],
        }),
      ],
    })

    const result = await generateOpenAPI(model)
    const s = result.components.schemas

    // Should have created Customer schema triplet (Update, New, base)
    expect(s['UpdateCustomer']).toBeDefined()
    expect(s['NewCustomer']).toBeDefined()
    expect(s['Customer']).toBeDefined()

    // After allOf flattening, Customer should have the "name" property
    expect(s['Customer'].properties?.name).toBeDefined()
    expect(s['Customer'].properties?.name.type).toBe('string')
  })

  it('creates entity reference (oneOf with $ref) for entity-type object noun', async () => {
    // Domain: Order has Customer (entity-to-entity)
    const orderNoun = mkNounDef({ name: 'Order' })
    const customerNoun = mkNounDef({ name: 'Customer' })

    const ft = mkFactType({
      id: 'gs1',
      reading: 'Order has Customer',
      roles: [
        { nounDef: orderNoun, roleIndex: 0 },
        { nounDef: customerNoun, roleIndex: 1 },
      ],
    })

    const model = createMockModel({
      nouns: [orderNoun, customerNoun],
      factTypes: [ft],
      constraints: [
        mkConstraint({
          kind: 'UC',
          spans: [{ factTypeId: ft.id, roleIndex: 0 }],
        }),
      ],
    })

    const result = await generateOpenAPI(model)
    const s = result.components.schemas

    // Order should exist with a customer property
    expect(s['Order']).toBeDefined()
    expect(s['Order'].properties?.customer).toBeDefined()

    // Entity references produce oneOf with a $ref
    const prop = s['Order'].properties?.customer
    expect(prop.oneOf).toBeDefined()
    expect(prop.oneOf.some((o: any) => o.$ref === '#/components/schemas/Customer')).toBe(true)
  })

  it('creates boolean property from unary reading', async () => {
    // Domain: Customer is active (1 role)
    const customerNoun = mkNounDef({ name: 'Customer' })

    const ft = mkFactType({
      id: 'gs1',
      reading: 'Customer is active',
      roles: [{ nounDef: customerNoun, roleIndex: 0 }],
    })

    const model = createMockModel({
      nouns: [customerNoun],
      factTypes: [ft],
      constraints: [],
    })

    const result = await generateOpenAPI(model)
    const s = result.components.schemas

    // Unary fact → boolean property
    expect(s['Customer']).toBeDefined()
    expect(s['Customer'].properties?.isActive).toBeDefined()
    expect(s['Customer'].properties?.isActive.type).toBe('boolean')
  })

  it('creates array property for compound UC not referenced elsewhere', async () => {
    // Domain: Customer has Skill (compound UC on both roles, not referenced by another schema)
    // This is an "array type" — Skill becomes an array property on Customer
    const customerNoun = mkNounDef({ name: 'Customer' })
    const skillNoun = mkValueNounDef({ name: 'Skill', valueType: 'string' })

    const ft = mkFactType({
      id: 'gs1',
      reading: 'Customer has Skill',
      roles: [
        { nounDef: customerNoun, roleIndex: 0 },
        { nounDef: skillNoun, roleIndex: 1 },
      ],
    })

    const model = createMockModel({
      nouns: [customerNoun, skillNoun],
      factTypes: [ft],
      constraints: [
        mkConstraint({
          kind: 'UC',
          // Compound UC: both roles constrained under same constraint
          spans: [
            { factTypeId: ft.id, roleIndex: 0 },
            { factTypeId: ft.id, roleIndex: 1 },
          ],
        }),
      ],
    })

    const result = await generateOpenAPI(model)
    const s = result.components.schemas

    // Customer should have an array property
    expect(s['Customer']).toBeDefined()
    // Either the flattened Customer or UpdateCustomer should have the array
    const allProps = { ...s['UpdateCustomer']?.properties, ...s['Customer']?.properties }
    const arrayProp = Object.values(allProps).find((p: any) => p.type === 'array')
    expect(arrayProp).toBeDefined()
    expect((arrayProp as any).type).toBe('array')
  })

  it('creates association/objectified entity for compound UC referenced by another schema', async () => {
    // Setup:
    //   gs1: "Enrollment" fact type with roles Student, Course (compound UC on both roles)
    //   gs2: "Enrollment has Grade" — references gs1 schema (Enrollment) as a noun
    // gs1 IS referenced by gs2's role → it's an association schema, not an array type
    const studentNoun = mkNounDef({ name: 'Student' })
    const courseNoun = mkNounDef({ name: 'Course' })
    // The enrollment schema itself is treated as an entity noun (objectified)
    // Its id must match the fact type id so the cross-reference works
    const enrollmentNoun = mkNounDef({ id: 'gs1', name: 'Enrollment' })
    const gradeNoun = mkValueNounDef({ name: 'Grade', valueType: 'string' })

    const ft1 = mkFactType({
      id: 'gs1',
      name: 'Enrollment',
      reading: 'Student enrolls in Course',
      roles: [
        { nounDef: studentNoun, roleIndex: 0 },
        { nounDef: courseNoun, roleIndex: 1 },
      ],
    })

    const ft2 = mkFactType({
      id: 'gs2',
      reading: 'Enrollment has Grade',
      roles: [
        { nounDef: enrollmentNoun, roleIndex: 0 },
        { nounDef: gradeNoun, roleIndex: 1 },
      ],
    })

    const model = createMockModel({
      nouns: [studentNoun, courseNoun, enrollmentNoun, gradeNoun],
      factTypes: [ft1, ft2],
      constraints: [
        mkConstraint({
          kind: 'UC', // compound UC on gs1
          spans: [
            { factTypeId: ft1.id, roleIndex: 0 },
            { factTypeId: ft1.id, roleIndex: 1 },
          ],
        }),
        mkConstraint({
          kind: 'UC', // single-role UC on gs2
          spans: [{ factTypeId: ft2.id, roleIndex: 0 }],
        }),
      ],
    })

    const result = await generateOpenAPI(model)
    const s = result.components.schemas

    // The association schema should exist as its own entity with the triplet
    expect(s['UpdateEnrollment']).toBeDefined()
    expect(s['UpdateEnrollment'].type).toBe('object')
    expect(s['NewEnrollment']).toBeDefined()
    expect(s['Enrollment']).toBeDefined()

    // After flattening, Enrollment should have properties from the association roles
    // (Student and Course as ID properties)
    const enrollProps = s['Enrollment'].properties || {}
    expect(Object.keys(enrollProps).length).toBeGreaterThan(0)

    // And Enrollment should also have the Grade property from the binary reading
    expect(enrollProps.grade).toBeDefined()
    expect(enrollProps.grade.type).toBe('string')
  })

  it('flattens allOf chains so properties propagate to derived schemas', async () => {
    // A simple entity with a value-type property should have properties
    // on the base schema (after allOf flattening)
    const productNoun = mkNounDef({ name: 'Product' })
    const priceNoun = mkValueNounDef({ name: 'Price', valueType: 'number' })

    const ft = mkFactType({
      id: 'gs1',
      reading: 'Product has Price',
      roles: [
        { nounDef: productNoun, roleIndex: 0 },
        { nounDef: priceNoun, roleIndex: 1 },
      ],
    })

    const model = createMockModel({
      nouns: [productNoun, priceNoun],
      factTypes: [ft],
      constraints: [
        mkConstraint({
          kind: 'UC',
          spans: [{ factTypeId: ft.id, roleIndex: 0 }],
        }),
      ],
    })

    const result = await generateOpenAPI(model)
    const s = result.components.schemas

    // After flattening, Product (not just UpdateProduct) should have properties
    expect(s['Product']).toBeDefined()
    expect(s['Product'].properties).toBeDefined()
    expect(s['Product'].properties?.price).toBeDefined()
    expect(s['Product'].properties?.price.type).toBe('number')

    // allOf should be removed after flattening
    expect(s['Product'].allOf).toBeUndefined()

    // type should propagate from UpdateProduct
    expect(s['Product'].type).toBe('object')
  })

  it('handles multiple binary readings on the same entity', async () => {
    // Customer has Name, Customer has Email
    const customerNoun = mkNounDef({ name: 'Customer' })
    const nameNoun = mkValueNounDef({ name: 'Name', valueType: 'string' })
    const emailNoun = mkValueNounDef({ name: 'Email', valueType: 'string', format: 'email' })

    const ft1 = mkFactType({
      id: 'gs1',
      reading: 'Customer has Name',
      roles: [
        { nounDef: customerNoun, roleIndex: 0 },
        { nounDef: nameNoun, roleIndex: 1 },
      ],
    })

    const ft2 = mkFactType({
      id: 'gs2',
      reading: 'Customer has Email',
      roles: [
        { nounDef: customerNoun, roleIndex: 0 },
        { nounDef: emailNoun, roleIndex: 1 },
      ],
    })

    const model = createMockModel({
      nouns: [customerNoun, nameNoun, emailNoun],
      factTypes: [ft1, ft2],
      constraints: [
        mkConstraint({
          kind: 'UC',
          spans: [{ factTypeId: ft1.id, roleIndex: 0 }],
        }),
        mkConstraint({
          kind: 'UC',
          spans: [{ factTypeId: ft2.id, roleIndex: 0 }],
        }),
      ],
    })

    const result = await generateOpenAPI(model)
    const s = result.components.schemas

    expect(s['Customer']).toBeDefined()
    expect(s['Customer'].properties?.name).toBeDefined()
    expect(s['Customer'].properties?.name.type).toBe('string')
    expect(s['Customer'].properties?.email).toBeDefined()
    expect(s['Customer'].properties?.email.type).toBe('string')
    expect(s['Customer'].properties?.email.format).toBe('email')
  })

  it('ignores non-UC constraints', async () => {
    // A constraint with kind='MC' should not produce compound schemas
    const customerNoun = mkNounDef({ name: 'Customer' })
    const nameNoun = mkValueNounDef({ name: 'Name', valueType: 'string' })

    const ft = mkFactType({
      id: 'gs1',
      reading: 'Customer has Name',
      roles: [
        { nounDef: customerNoun, roleIndex: 0 },
        { nounDef: nameNoun, roleIndex: 1 },
      ],
    })

    const model = createMockModel({
      nouns: [customerNoun, nameNoun],
      factTypes: [ft],
      constraints: [
        mkConstraint({
          kind: 'MC', // Mandatory constraint, not UC
          spans: [
            { factTypeId: ft.id, roleIndex: 0 },
            { factTypeId: ft.id, roleIndex: 1 },
          ],
        }),
      ],
    })

    const result = await generateOpenAPI(model)
    const s = result.components.schemas

    // Entity nouns always get schemas (RMAP Step 3).
    // Without UC constraints, no properties are added — just the empty schema.
    expect(s['UpdateCustomer']).toBeDefined()
    expect(Object.keys(s['UpdateCustomer'].properties || {}).length).toBe(0)
  })

  it('ensures domain-scoped entity nouns with permissions get schemas even without readings', async () => {
    const widgetNoun = mkNounDef({
      name: 'Widget',
      objectType: 'entity',
      permissions: ['create', 'read'],
    })

    const model = createMockModel({
      nouns: [widgetNoun],
      factTypes: [],
      constraints: [],
    })

    const result = await generateOpenAPI(model)
    const s = result.components.schemas

    // Entity with permissions should get a schema even without readings
    expect(s['Widget']).toBeDefined()
    expect(s['UpdateWidget']).toBeDefined()
    expect(s['NewWidget']).toBeDefined()
  })

  it('propagates supertype properties to subtypes (Step G.5)', async () => {
    // Resource has Status (core fact type)
    // SupportRequest is a subtype of Request, Request is a subtype of Resource
    // SupportRequest should inherit the "status" property
    const resourceNoun = mkNounDef({ name: 'Resource' })
    const requestNoun = mkNounDef({ name: 'Request', superType: 'Resource' })
    const supportRequestNoun = mkNounDef({ name: 'SupportRequest', superType: 'Request' })
    const statusNoun = mkValueNounDef({ name: 'Status', valueType: 'string' })
    const priorityNoun = mkValueNounDef({ name: 'Priority', valueType: 'string' })

    const ft1 = mkFactType({
      id: 'gs1',
      reading: 'Resource has Status',
      roles: [
        { nounDef: resourceNoun, roleIndex: 0 },
        { nounDef: statusNoun, roleIndex: 1 },
      ],
    })

    const ft2 = mkFactType({
      id: 'gs2',
      reading: 'SupportRequest has Priority',
      roles: [
        { nounDef: supportRequestNoun, roleIndex: 0 },
        { nounDef: priorityNoun, roleIndex: 1 },
      ],
    })

    const model = createMockModel({
      nouns: [resourceNoun, requestNoun, supportRequestNoun, statusNoun, priorityNoun],
      factTypes: [ft1, ft2],
      constraints: [
        mkConstraint({ kind: 'UC', spans: [{ factTypeId: ft1.id, roleIndex: 0 }] }),
        mkConstraint({ kind: 'UC', spans: [{ factTypeId: ft2.id, roleIndex: 0 }] }),
      ],
    })

    const result = await generateOpenAPI(model)
    const s = result.components.schemas

    // Resource has status property directly
    expect(s['Resource']?.properties?.status).toBeDefined()

    // Request inherits status from Resource (Step G.5)
    expect(s['Request']?.properties?.status).toBeDefined()

    // SupportRequest inherits status from Request (which got it from Resource)
    // AND has its own priority property
    expect(s['SupportRequest']?.properties?.status).toBeDefined()
    expect(s['SupportRequest']?.properties?.priority).toBeDefined()
  })

  it('produces enum array in the schema for value types with enumValues', async () => {
    // Domain: Task has Priority (enum: Low, Medium, High)
    const taskNoun = mkNounDef({ name: 'Task' })
    const priorityNoun = mkValueNounDef({
      name: 'Priority',
      valueType: 'string',
      enumValues: ['Low', 'Medium', 'High'],
    })

    const ft = mkFactType({
      id: 'gs1',
      reading: 'Task has Priority',
      roles: [
        { nounDef: taskNoun, roleIndex: 0 },
        { nounDef: priorityNoun, roleIndex: 1 },
      ],
    })

    const model = createMockModel({
      nouns: [taskNoun, priorityNoun],
      factTypes: [ft],
      constraints: [
        mkConstraint({
          kind: 'UC',
          spans: [{ factTypeId: ft.id, roleIndex: 0 }],
        }),
      ],
    })

    const result = await generateOpenAPI(model)
    const s = result.components.schemas

    expect(s['Task']).toBeDefined()
    const prop = s['Task'].properties?.priority
    expect(prop).toBeDefined()
    expect(prop.enum).toEqual(['Low', 'Medium', 'High'])
  })

  it('includes deontic constraints in the model output without affecting schema structure', async () => {
    // Deontic constraints should not crash the pipeline; they are non-UC
    // constraints that describe obligations, and should be accepted silently
    const orderNoun = mkNounDef({ name: 'Order' })
    const itemNoun = mkValueNounDef({ name: 'Item', valueType: 'string' })

    const ft = mkFactType({
      id: 'gs1',
      reading: 'Order has Item',
      roles: [
        { nounDef: orderNoun, roleIndex: 0 },
        { nounDef: itemNoun, roleIndex: 1 },
      ],
    })

    const model = createMockModel({
      nouns: [orderNoun, itemNoun],
      factTypes: [ft],
      constraints: [
        mkConstraint({
          kind: 'UC',
          spans: [{ factTypeId: ft.id, roleIndex: 0 }],
        }),
        mkConstraint({
          kind: 'MC',
          modality: 'Deontic',
          deonticOperator: 'obligatory',
          text: 'It is obligatory that each Order has at least one Item',
          spans: [{ factTypeId: ft.id, roleIndex: 0 }],
        }),
      ],
    })

    const result = await generateOpenAPI(model)
    const s = result.components.schemas

    // The schema should be generated correctly despite the deontic constraint
    expect(s['Order']).toBeDefined()
    expect(s['Order'].properties?.item).toBeDefined()
    expect(s['Order'].properties?.item.type).toBe('string')
  })

  it('multiple subtypes of the same parent all inherit parent properties', async () => {
    // Vehicle has Color; Car and Truck both extend Vehicle
    const vehicleNoun = mkNounDef({ name: 'Vehicle' })
    const carNoun = mkNounDef({ name: 'Car', superType: 'Vehicle' })
    const truckNoun = mkNounDef({ name: 'Truck', superType: 'Vehicle' })
    const colorNoun = mkValueNounDef({ name: 'Color', valueType: 'string' })
    const payloadNoun = mkValueNounDef({ name: 'Payload', valueType: 'number' })
    const doorsNoun = mkValueNounDef({ name: 'Doors', valueType: 'integer' })

    const ftColor = mkFactType({
      id: 'gs1',
      reading: 'Vehicle has Color',
      roles: [
        { nounDef: vehicleNoun, roleIndex: 0 },
        { nounDef: colorNoun, roleIndex: 1 },
      ],
    })

    const ftPayload = mkFactType({
      id: 'gs2',
      reading: 'Truck has Payload',
      roles: [
        { nounDef: truckNoun, roleIndex: 0 },
        { nounDef: payloadNoun, roleIndex: 1 },
      ],
    })

    const ftDoors = mkFactType({
      id: 'gs3',
      reading: 'Car has Doors',
      roles: [
        { nounDef: carNoun, roleIndex: 0 },
        { nounDef: doorsNoun, roleIndex: 1 },
      ],
    })

    const model = createMockModel({
      nouns: [vehicleNoun, carNoun, truckNoun, colorNoun, payloadNoun, doorsNoun],
      factTypes: [ftColor, ftPayload, ftDoors],
      constraints: [
        mkConstraint({ kind: 'UC', spans: [{ factTypeId: ftColor.id, roleIndex: 0 }] }),
        mkConstraint({ kind: 'UC', spans: [{ factTypeId: ftPayload.id, roleIndex: 0 }] }),
        mkConstraint({ kind: 'UC', spans: [{ factTypeId: ftDoors.id, roleIndex: 0 }] }),
      ],
    })

    const result = await generateOpenAPI(model)
    const s = result.components.schemas

    // Vehicle has its own property
    expect(s['Vehicle']?.properties?.color).toBeDefined()
    expect(s['Vehicle']?.properties?.color?.type).toBe('string')

    // Car inherits color from Vehicle AND has its own doors property
    expect(s['Car']?.properties?.color).toBeDefined()
    expect(s['Car']?.properties?.color?.type).toBe('string')
    expect(s['Car']?.properties?.doors).toBeDefined()
    expect(s['Car']?.properties?.doors?.type).toBe('integer')

    // Truck inherits color from Vehicle AND has its own payload property
    expect(s['Truck']?.properties?.color).toBeDefined()
    expect(s['Truck']?.properties?.color?.type).toBe('string')
    expect(s['Truck']?.properties?.payload).toBeDefined()
    expect(s['Truck']?.properties?.payload?.type).toBe('number')
  })

  it('entity with no readings gets a schema (RMAP Step 3)', async () => {
    // An entity noun with NO fact types at all should still get a schema triplet
    const widgetNoun = mkNounDef({ name: 'Widget', objectType: 'entity' })

    const model = createMockModel({
      nouns: [widgetNoun],
      factTypes: [],
      constraints: [],
    })

    const result = await generateOpenAPI(model)
    const s = result.components.schemas

    // Even without any readings, RMAP Step 3 ensures the table exists
    expect(s['Widget']).toBeDefined()
    expect(s['UpdateWidget']).toBeDefined()
    expect(s['NewWidget']).toBeDefined()

    // It should have type 'object' (from ensureTableExists)
    expect(s['Widget'].type).toBe('object')

    // No properties beyond what ensureTableExists provides
    // (empty properties or no user-defined properties)
    const userProps = Object.keys(s['UpdateWidget'].properties || {})
    expect(userProps.length).toBe(0)
  })

  it('wires supertype allOf references for string-name supertypes', async () => {
    const parentNoun = mkNounDef({ name: 'Vehicle' })
    const childNoun = mkNounDef({ name: 'Car', superType: 'Vehicle' })
    const mileageNoun = mkValueNounDef({ name: 'Mileage', valueType: 'integer' })

    const ft = mkFactType({
      id: 'gs1',
      reading: 'Vehicle has Mileage',
      roles: [
        { nounDef: parentNoun, roleIndex: 0 },
        { nounDef: mileageNoun, roleIndex: 1 },
      ],
    })

    const model = createMockModel({
      nouns: [parentNoun, childNoun, mileageNoun],
      factTypes: [ft],
      constraints: [
        mkConstraint({ kind: 'UC', spans: [{ factTypeId: ft.id, roleIndex: 0 }] }),
      ],
    })

    const result = await generateOpenAPI(model)
    const s = result.components.schemas

    // Car should inherit mileage from Vehicle
    expect(s['Car']?.properties?.mileage).toBeDefined()
    expect(s['Car']?.properties?.mileage?.type).toBe('integer')
  })
})

// ---------------------------------------------------------------------------
// generateOpenAPIFromRmap tests
// ---------------------------------------------------------------------------

describe('generateOpenAPIFromRmap', () => {
  it('returns valid OpenAPI 3.0 structure', () => {
    const result = generateOpenAPIFromRmap([], 'Test') as any
    expect(result.openapi).toBe('3.0.0')
    expect(result.info.title).toBe('Test Schema (RMAP)')
    expect(result.info.version).toBe('1.0.0')
    expect(result.components.schemas).toEqual({})
  })

  it('converts table to component schema with properties', () => {
    const tables: TableDef[] = [{
      name: 'person',
      primaryKey: ['id'],
      columns: [
        { name: 'id', type: 'TEXT', nullable: false },
        { name: 'name', type: 'TEXT', nullable: false },
        { name: 'age', type: 'INTEGER', nullable: true },
      ],
    }]
    const result = generateOpenAPIFromRmap(tables, 'University') as any
    const schema = result.components.schemas['Person']

    expect(schema).toBeDefined()
    expect(schema.type).toBe('object')
    expect(schema.properties.id.type).toBe('string')
    expect(schema.properties.name.type).toBe('string')
    expect(schema.properties.age.type).toBe('integer')
  })

  it('marks NOT NULL columns as required', () => {
    const tables: TableDef[] = [{
      name: 'person',
      primaryKey: ['id'],
      columns: [
        { name: 'id', type: 'TEXT', nullable: false },
        { name: 'name', type: 'TEXT', nullable: false },
        { name: 'nickname', type: 'TEXT', nullable: true },
      ],
    }]
    const result = generateOpenAPIFromRmap(tables, 'Test') as any
    const schema = result.components.schemas['Person']

    expect(schema.required).toContain('id')
    expect(schema.required).toContain('name')
    expect(schema.required).not.toContain('nickname')
  })

  it('generates $ref for FK references', () => {
    const tables: TableDef[] = [{
      name: 'person',
      primaryKey: ['id'],
      columns: [
        { name: 'id', type: 'TEXT', nullable: false },
        { name: 'country_id', type: 'TEXT', nullable: true, references: 'country' },
      ],
    }]
    const result = generateOpenAPIFromRmap(tables, 'Test') as any
    const schema = result.components.schemas['Person']

    expect(schema.properties.country_id.oneOf).toBeDefined()
    expect(schema.properties.country_id.oneOf).toEqual(
      expect.arrayContaining([
        { $ref: '#/components/schemas/Country' },
      ]),
    )
  })

  it('converts CHECK IN constraints to enum values', () => {
    const tables: TableDef[] = [{
      name: 'person',
      primaryKey: ['id'],
      columns: [
        { name: 'id', type: 'TEXT', nullable: false },
        { name: 'sex', type: 'TEXT', nullable: false },
      ],
      checks: ["sex IN ('male', 'female')"],
    }]
    const result = generateOpenAPIFromRmap(tables, 'Test') as any
    const schema = result.components.schemas['Person']

    expect(schema.properties.sex.enum).toEqual(['male', 'female'])
  })

  it('handles multiple tables', () => {
    const tables: TableDef[] = [
      {
        name: 'person',
        primaryKey: ['id'],
        columns: [
          { name: 'id', type: 'TEXT', nullable: false },
          { name: 'name', type: 'TEXT', nullable: false },
        ],
      },
      {
        name: 'country',
        primaryKey: ['id'],
        columns: [
          { name: 'id', type: 'TEXT', nullable: false },
        ],
      },
    ]
    const result = generateOpenAPIFromRmap(tables, 'Test') as any

    expect(result.components.schemas['Person']).toBeDefined()
    expect(result.components.schemas['Country']).toBeDefined()
  })

  it('converts snake_case table names to PascalCase schema names', () => {
    const tables: TableDef[] = [{
      name: 'person_teaches_course',
      primaryKey: ['person_id', 'course_id'],
      columns: [
        { name: 'person_id', type: 'TEXT', nullable: false, references: 'person' },
        { name: 'course_id', type: 'TEXT', nullable: false, references: 'course' },
      ],
    }]
    const result = generateOpenAPIFromRmap(tables, 'Test') as any

    expect(result.components.schemas['PersonTeachesCourse']).toBeDefined()
  })

  it('maps REAL type to number', () => {
    const tables: TableDef[] = [{
      name: 'product',
      primaryKey: ['id'],
      columns: [
        { name: 'id', type: 'TEXT', nullable: false },
        { name: 'price', type: 'REAL', nullable: false },
      ],
    }]
    const result = generateOpenAPIFromRmap(tables, 'Test') as any
    expect(result.components.schemas['Product'].properties.price.type).toBe('number')
  })

  it('does not set required array when all columns are nullable', () => {
    const tables: TableDef[] = [{
      name: 'optional_data',
      primaryKey: [],
      columns: [
        { name: 'value', type: 'TEXT', nullable: true },
      ],
    }]
    const result = generateOpenAPIFromRmap(tables, 'Test') as any
    expect(result.components.schemas['OptionalData'].required).toBeUndefined()
  })
})

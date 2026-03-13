import { describe, it, expect, beforeEach } from 'vitest'
import { generateOpenAPI } from './openapi'
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

    // With no UC constraints, no schemas should be created
    // (no binary, no array, no compound processing)
    expect(Object.keys(s).length).toBe(0)
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
})

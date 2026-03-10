import { describe, it, expect } from 'vitest'
import { generateOpenAPI } from './openapi'

// ---------------------------------------------------------------------------
// Mock DB helper
// ---------------------------------------------------------------------------

function mockDB(data: Record<string, any[]>) {
  return {
    findInCollection: async (slug: string, where?: any, _opts?: any) => {
      let docs = data[slug] || []
      // Simple filtering: match on first where clause key
      if (where) {
        docs = docs.filter((doc) => {
          for (const [key, condition] of Object.entries(where)) {
            const cond = condition as any
            if (cond?.equals !== undefined) {
              // Support nested dot paths like "graphSchema"
              const value = doc[key]
              if (value !== cond.equals) return false
            }
          }
          return true
        })
      }
      return { docs, totalDocs: docs.length, hasNextPage: false, page: 1, limit: 100 }
    },
  }
}

// ---------------------------------------------------------------------------
// Test data factories
// ---------------------------------------------------------------------------

function mkNoun(overrides: { id: string; name: string } & Record<string, any>) {
  return { objectType: 'entity', ...overrides }
}

function mkValueNoun(overrides: { id: string; name: string; valueType: string } & Record<string, any>) {
  return { objectType: 'value', ...overrides }
}

function mkRole(id: string, nounId: string, graphSchemaId: string, extra?: Record<string, any>) {
  return { id, noun: nounId, graphSchema: graphSchemaId, ...extra }
}

function mkReading(text: string, graphSchemaId: string) {
  return { id: `reading-${text.replace(/\s/g, '-')}`, text, graphSchema: graphSchemaId }
}

function mkGraphSchema(id: string, name: string, domainId: string) {
  return { id, name, domain: domainId }
}

function mkConstraintSpan(id: string, constraintId: string, roleId: string) {
  return { id, constraint_id: constraintId, role_id: roleId }
}

function mkConstraint(id: string, kind: string) {
  return { id, kind }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

describe('generateOpenAPI', () => {
  it('returns valid OpenAPI structure for empty domain', async () => {
    const db = mockDB({
      'graph-schemas': [],
      nouns: [],
      'constraint-spans': [],
      constraints: [],
      roles: [],
      readings: [],
    })

    const result = await generateOpenAPI(db, 'domain-1')

    expect(result.openapi).toBe('3.0.0')
    expect(result.info).toBeDefined()
    expect(result.info.version).toBe('1.0.0')
    expect(result.components).toBeDefined()
    expect(result.components.schemas).toEqual({})
  })

  it('creates schema with value-type property from a binary reading with UC', async () => {
    // Domain: Customer has Name
    // Single-role UC on Customer role → Name becomes a property of Customer
    const domainId = 'd1'
    const db = mockDB({
      'graph-schemas': [mkGraphSchema('gs1', 'Customer has Name', domainId)],
      nouns: [
        mkNoun({ id: 'n1', name: 'Customer' }),
        mkValueNoun({ id: 'n2', name: 'Name', valueType: 'string' }),
      ],
      roles: [
        mkRole('r1', 'n1', 'gs1'),
        mkRole('r2', 'n2', 'gs1'),
      ],
      readings: [mkReading('Customer has Name', 'gs1')],
      constraints: [mkConstraint('c1', 'UC')],
      'constraint-spans': [mkConstraintSpan('cs1', 'c1', 'r1')],
    })

    const result = await generateOpenAPI(db, domainId)
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
    const domainId = 'd1'
    const db = mockDB({
      'graph-schemas': [mkGraphSchema('gs1', 'Order has Customer', domainId)],
      nouns: [
        mkNoun({ id: 'n1', name: 'Order' }),
        mkNoun({ id: 'n2', name: 'Customer' }),
      ],
      roles: [
        mkRole('r1', 'n1', 'gs1'),
        mkRole('r2', 'n2', 'gs1'),
      ],
      readings: [mkReading('Order has Customer', 'gs1')],
      constraints: [mkConstraint('c1', 'UC')],
      'constraint-spans': [mkConstraintSpan('cs1', 'c1', 'r1')],
    })

    const result = await generateOpenAPI(db, domainId)
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
    const domainId = 'd1'
    const db = mockDB({
      'graph-schemas': [mkGraphSchema('gs1', 'Customer is active', domainId)],
      nouns: [mkNoun({ id: 'n1', name: 'Customer' })],
      roles: [mkRole('r1', 'n1', 'gs1')],
      readings: [mkReading('Customer is active', 'gs1')],
      constraints: [],
      'constraint-spans': [],
    })

    const result = await generateOpenAPI(db, domainId)
    const s = result.components.schemas

    // Unary fact → boolean property
    expect(s['Customer']).toBeDefined()
    expect(s['Customer'].properties?.isActive).toBeDefined()
    expect(s['Customer'].properties?.isActive.type).toBe('boolean')
  })

  it('creates array property for compound UC not referenced elsewhere', async () => {
    // Domain: Customer has Skill (compound UC on both roles, not referenced by another schema)
    // This is an "array type" — Skill becomes an array property on Customer
    const domainId = 'd1'
    const db = mockDB({
      'graph-schemas': [mkGraphSchema('gs1', 'Customer has Skill', domainId)],
      nouns: [
        mkNoun({ id: 'n1', name: 'Customer' }),
        mkValueNoun({ id: 'n2', name: 'Skill', valueType: 'string' }),
      ],
      roles: [
        mkRole('r1', 'n1', 'gs1'),
        mkRole('r2', 'n2', 'gs1'),
      ],
      readings: [mkReading('Customer has Skill', 'gs1')],
      constraints: [mkConstraint('c1', 'UC')],
      // Compound UC: both roles constrained under same constraint
      'constraint-spans': [
        mkConstraintSpan('cs1', 'c1', 'r1'),
        mkConstraintSpan('cs2', 'c1', 'r2'),
      ],
    })

    const result = await generateOpenAPI(db, domainId)
    const s = result.components.schemas

    // Customer should have an array property
    expect(s['Customer']).toBeDefined()
    // The processArraySchemas function uses the schema name as the property name
    const customerProps = s['Customer'].properties || s['UpdateCustomer']?.properties || {}
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
    const domainId = 'd1'
    const db = mockDB({
      'graph-schemas': [
        mkGraphSchema('gs1', 'Enrollment', domainId),
        mkGraphSchema('gs2', 'Enrollment has Grade', domainId),
      ],
      nouns: [
        mkNoun({ id: 'n1', name: 'Student' }),
        mkNoun({ id: 'n2', name: 'Course' }),
        // The enrollment schema itself is treated as an entity noun (objectified)
        mkNoun({ id: 'gs1', name: 'Enrollment' }),
        mkValueNoun({ id: 'n3', name: 'Grade', valueType: 'string' }),
      ],
      roles: [
        // gs1 roles
        mkRole('r1', 'n1', 'gs1'),
        mkRole('r2', 'n2', 'gs1'),
        // gs2 roles — r3 references gs1 (the Enrollment schema) as its noun
        mkRole('r3', 'gs1', 'gs2'),
        mkRole('r4', 'n3', 'gs2'),
      ],
      readings: [
        mkReading('Student enrolls in Course', 'gs1'),
        mkReading('Enrollment has Grade', 'gs2'),
      ],
      constraints: [
        mkConstraint('c1', 'UC'), // compound UC on gs1
        mkConstraint('c2', 'UC'), // single-role UC on gs2
      ],
      'constraint-spans': [
        mkConstraintSpan('cs1', 'c1', 'r1'),
        mkConstraintSpan('cs2', 'c1', 'r2'),
        mkConstraintSpan('cs3', 'c2', 'r3'),
      ],
    })

    const result = await generateOpenAPI(db, domainId)
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
    const domainId = 'd1'
    const db = mockDB({
      'graph-schemas': [mkGraphSchema('gs1', 'Product has Price', domainId)],
      nouns: [
        mkNoun({ id: 'n1', name: 'Product' }),
        mkValueNoun({ id: 'n2', name: 'Price', valueType: 'number' }),
      ],
      roles: [
        mkRole('r1', 'n1', 'gs1'),
        mkRole('r2', 'n2', 'gs1'),
      ],
      readings: [mkReading('Product has Price', 'gs1')],
      constraints: [mkConstraint('c1', 'UC')],
      'constraint-spans': [mkConstraintSpan('cs1', 'c1', 'r1')],
    })

    const result = await generateOpenAPI(db, domainId)
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
    const domainId = 'd1'
    const db = mockDB({
      'graph-schemas': [
        mkGraphSchema('gs1', 'Customer has Name', domainId),
        mkGraphSchema('gs2', 'Customer has Email', domainId),
      ],
      nouns: [
        mkNoun({ id: 'n1', name: 'Customer' }),
        mkValueNoun({ id: 'n2', name: 'Name', valueType: 'string' }),
        mkValueNoun({ id: 'n3', name: 'Email', valueType: 'string', format: 'email' }),
      ],
      roles: [
        mkRole('r1', 'n1', 'gs1'),
        mkRole('r2', 'n2', 'gs1'),
        mkRole('r3', 'n1', 'gs2'),
        mkRole('r4', 'n3', 'gs2'),
      ],
      readings: [
        mkReading('Customer has Name', 'gs1'),
        mkReading('Customer has Email', 'gs2'),
      ],
      constraints: [
        mkConstraint('c1', 'UC'),
        mkConstraint('c2', 'UC'),
      ],
      'constraint-spans': [
        mkConstraintSpan('cs1', 'c1', 'r1'),
        mkConstraintSpan('cs2', 'c2', 'r3'),
      ],
    })

    const result = await generateOpenAPI(db, domainId)
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
    const domainId = 'd1'
    const db = mockDB({
      'graph-schemas': [mkGraphSchema('gs1', 'Customer has Name', domainId)],
      nouns: [
        mkNoun({ id: 'n1', name: 'Customer' }),
        mkValueNoun({ id: 'n2', name: 'Name', valueType: 'string' }),
      ],
      roles: [
        mkRole('r1', 'n1', 'gs1'),
        mkRole('r2', 'n2', 'gs1'),
      ],
      readings: [mkReading('Customer has Name', 'gs1')],
      constraints: [mkConstraint('c1', 'MC')], // Mandatory constraint, not UC
      'constraint-spans': [
        mkConstraintSpan('cs1', 'c1', 'r1'),
        mkConstraintSpan('cs2', 'c1', 'r2'),
      ],
    })

    const result = await generateOpenAPI(db, domainId)
    const s = result.components.schemas

    // With no UC constraints, no schemas should be created
    // (no binary, no array, no compound processing)
    expect(Object.keys(s).length).toBe(0)
  })

  it('ensures domain-scoped entity nouns with permissions get schemas even without readings', async () => {
    const domainId = 'd1'
    const db = mockDB({
      'graph-schemas': [],
      nouns: [
        mkNoun({ id: 'n1', name: 'Widget', objectType: 'entity', permissions: ['create', 'read'], domain: domainId }),
      ],
      'constraint-spans': [],
      constraints: [],
      roles: [],
      readings: [],
    })

    const result = await generateOpenAPI(db, domainId)
    const s = result.components.schemas

    // Entity with permissions should get a schema even without readings
    expect(s['Widget']).toBeDefined()
    expect(s['UpdateWidget']).toBeDefined()
    expect(s['NewWidget']).toBeDefined()
  })
})

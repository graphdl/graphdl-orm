import { describe, it, expect } from 'vitest'
import { createProperty, ensureTableExists, setTableProperty } from './schema-builder'
import type { NounDef } from '../model/types'

// ---------------------------------------------------------------------------
// Helper — quick NounDef factory
// ---------------------------------------------------------------------------
function mkNoun(overrides: Partial<NounDef> & { id: string; name: string }): NounDef {
  return { objectType: 'value', domainId: 'd1', ...overrides }
}

// ---------------------------------------------------------------------------
// ensureTableExists
// ---------------------------------------------------------------------------
describe('ensureTableExists', () => {
  it('creates UpdateX, NewX, and X schema triplet', () => {
    const tables: Record<string, any> = {}
    const subject = mkNoun({ id: '1', name: 'Customer', valueType: 'string' })
    ensureTableExists({ tables, subject, nouns: [subject], jsonExamples: {} })

    expect(tables['UpdateCustomer']).toBeDefined()
    expect(tables['NewCustomer']).toBeDefined()
    expect(tables['Customer']).toBeDefined()

    expect(tables['UpdateCustomer'].$id).toBe('UpdateCustomer')
    expect(tables['UpdateCustomer'].title).toBe('Customer')
    expect(tables['NewCustomer'].$id).toBe('NewCustomer')
    expect(tables['NewCustomer'].allOf).toContainEqual({ $ref: '#/components/schemas/UpdateCustomer' })
    expect(tables['Customer'].$id).toBe('Customer')
    expect(tables['Customer'].allOf).toContainEqual({ $ref: '#/components/schemas/NewCustomer' })
  })

  it('is idempotent — does not overwrite existing tables', () => {
    const tables: Record<string, any> = {}
    const subject = mkNoun({ id: '1', name: 'Customer', valueType: 'string' })
    ensureTableExists({ tables, subject, nouns: [subject], jsonExamples: {} })

    // Modify a table to verify it's not overwritten
    tables['UpdateCustomer'].custom = true
    ensureTableExists({ tables, subject, nouns: [subject], jsonExamples: {} })
    expect(tables['UpdateCustomer'].custom).toBe(true)
  })

  it('sets type: object on UpdateX when no superType', () => {
    const tables: Record<string, any> = {}
    const subject = mkNoun({ id: '1', name: 'Order', valueType: 'string' })
    ensureTableExists({ tables, subject, nouns: [subject], jsonExamples: {} })

    expect(tables['UpdateOrder'].type).toBe('object')
  })

  it('wires superType chain via allOf references', () => {
    const tables: Record<string, any> = {}
    const parent = mkNoun({ id: '1', name: 'Person', valueType: 'string' })
    const child = mkNoun({ id: '2', name: 'Employee', superType: parent })

    ensureTableExists({ tables, subject: child, nouns: [parent, child], jsonExamples: {} })

    // UpdateEmployee should reference UpdatePerson
    expect(tables['UpdateEmployee'].allOf).toContainEqual({
      $ref: '#/components/schemas/UpdatePerson',
    })
    // NewEmployee should reference both UpdateEmployee and NewPerson
    expect(tables['NewEmployee'].allOf).toContainEqual({
      $ref: '#/components/schemas/UpdateEmployee',
    })
    expect(tables['NewEmployee'].allOf).toContainEqual({
      $ref: '#/components/schemas/NewPerson',
    })
    // Employee should reference both NewEmployee and Person
    expect(tables['Employee'].allOf).toContainEqual({
      $ref: '#/components/schemas/NewEmployee',
    })
    expect(tables['Employee'].allOf).toContainEqual({
      $ref: '#/components/schemas/Person',
    })
    // Parent should also be ensured
    expect(tables['UpdatePerson']).toBeDefined()
    expect(tables['Person']).toBeDefined()
  })

  it('handles superType as string id', () => {
    const tables: Record<string, any> = {}
    const parent = mkNoun({ id: 'p1', name: 'Person', valueType: 'string' })
    const child = mkNoun({ id: 'c1', name: 'Employee', superType: 'p1' })

    ensureTableExists({ tables, subject: child, nouns: [parent, child], jsonExamples: {} })

    expect(tables['UpdateEmployee'].allOf).toContainEqual({
      $ref: '#/components/schemas/UpdatePerson',
    })
  })

  it('unpacks referenceScheme into properties via setTableProperty', () => {
    const tables: Record<string, any> = {}
    const idNoun = mkNoun({ id: 'id1', name: 'Customer Id', valueType: 'integer' })
    const subject = mkNoun({
      id: '1',
      name: 'Customer',
      referenceScheme: [idNoun],
    })

    ensureTableExists({ tables, subject, nouns: [subject, idNoun], jsonExamples: {} })

    // The referenceScheme noun should become a property on UpdateCustomer
    expect(tables['UpdateCustomer'].properties).toBeDefined()
    // "Customer Id" on "Customer" strips the "Customer" prefix → "id"
    expect(tables['UpdateCustomer'].properties['id']).toBeDefined()
    expect(tables['UpdateCustomer'].properties['id'].type).toBe('integer')
  })

  it('unpacks referenceScheme given as NounDef objects', () => {
    const tables: Record<string, any> = {}
    const idNoun = mkNoun({ id: 'id1', name: 'Customer Nr', valueType: 'integer' })
    const subject = mkNoun({
      id: '1',
      name: 'Customer',
      objectType: 'entity',
      referenceScheme: [idNoun],
    })

    ensureTableExists({ tables, subject, nouns: [subject, idNoun], jsonExamples: {} })

    expect(tables['UpdateCustomer'].properties).toBeDefined()
    expect(tables['UpdateCustomer'].properties['nr']).toBeDefined()
  })

  it('adds json examples when available', () => {
    const tables: Record<string, any> = {}
    const subject = mkNoun({ id: '1', name: 'Order', valueType: 'string' })
    const jsonExamples = { Order: { status: 'pending' } }

    ensureTableExists({ tables, subject, nouns: [subject], jsonExamples })

    expect(tables['UpdateOrder'].examples).toEqual([{ status: 'pending' }])
    expect(tables['NewOrder'].examples).toEqual([{ status: 'pending' }])
    expect(tables['Order'].examples).toEqual([{ status: 'pending' }])
  })

  it('sets description on UpdateX when subject has description', () => {
    const tables: Record<string, any> = {}
    const subject = mkNoun({ id: '1', name: 'Order', description: 'A customer order' })

    ensureTableExists({ tables, subject, nouns: [subject], jsonExamples: {} })

    expect(tables['UpdateOrder'].description).toBe('A customer order')
  })

  it('handles name with spaces', () => {
    const tables: Record<string, any> = {}
    const subject = mkNoun({ id: '1', name: 'Support Request', valueType: 'string' })

    ensureTableExists({ tables, subject, nouns: [subject], jsonExamples: {} })

    expect(tables['UpdateSupportRequest']).toBeDefined()
    expect(tables['NewSupportRequest']).toBeDefined()
    expect(tables['SupportRequest']).toBeDefined()
  })
})

// ---------------------------------------------------------------------------
// createProperty
// ---------------------------------------------------------------------------
describe('createProperty', () => {
  it('returns empty object for null/undefined object', () => {
    const result = createProperty({
      object: null as any,
      nouns: [],
      tables: {},
      jsonExamples: {},
    })
    expect(result).toEqual({})
  })

  it('creates string value type property', () => {
    const noun = mkNoun({ id: '1', name: 'Name', valueType: 'string' })
    const result = createProperty({ object: noun, nouns: [noun], tables: {}, jsonExamples: {} })
    expect(result.type).toBe('string')
  })

  it('creates number value type property', () => {
    const noun = mkNoun({ id: '1', name: 'Price', valueType: 'number' })
    const result = createProperty({ object: noun, nouns: [noun], tables: {}, jsonExamples: {} })
    expect(result.type).toBe('number')
  })

  it('creates integer value type property', () => {
    const noun = mkNoun({ id: '1', name: 'Count', valueType: 'integer' })
    const result = createProperty({ object: noun, nouns: [noun], tables: {}, jsonExamples: {} })
    expect(result.type).toBe('integer')
  })

  it('creates boolean value type property', () => {
    const noun = mkNoun({ id: '1', name: 'Active', valueType: 'boolean' })
    const result = createProperty({ object: noun, nouns: [noun], tables: {}, jsonExamples: {} })
    expect(result.type).toBe('boolean')
  })

  it('includes format when present', () => {
    const noun = mkNoun({ id: '1', name: 'Email', valueType: 'string', format: 'email' })
    const result = createProperty({ object: noun, nouns: [noun], tables: {}, jsonExamples: {} })
    expect(result.format).toBe('email')
  })

  it('includes pattern when present', () => {
    const noun = mkNoun({ id: '1', name: 'ZipCode', valueType: 'string', pattern: '^\\d{5}$' })
    const result = createProperty({ object: noun, nouns: [noun], tables: {}, jsonExamples: {} })
    expect(result.pattern).toBe('^\\d{5}$')
  })

  it('includes description when provided', () => {
    const noun = mkNoun({ id: '1', name: 'Name', valueType: 'string' })
    const result = createProperty({
      description: 'A human-readable name',
      object: noun,
      nouns: [noun],
      tables: {},
      jsonExamples: {},
    })
    expect(result.description).toBe('A human-readable name')
  })

  it('handles enum values from enumValues field', () => {
    const noun = mkNoun({
      id: '1',
      name: 'Status',
      valueType: 'string',
      enumValues: ['active', 'inactive', 'pending'],
    })
    const result = createProperty({ object: noun, nouns: [noun], tables: {}, jsonExamples: {} })
    expect(result.enum).toEqual(['active', 'inactive', 'pending'])
  })

  it('handles null in enum values', () => {
    const noun = mkNoun({
      id: '1',
      name: 'Status',
      valueType: 'string',
      enumValues: ['active', 'null', 'pending'],
    })
    const result = createProperty({ object: noun, nouns: [noun], tables: {}, jsonExamples: {} })
    expect(result.enum).toContain(null)
    expect(result.nullable).toBe(true)
  })

  it('includes minLength and maxLength', () => {
    const noun = mkNoun({
      id: '1',
      name: 'Code',
      valueType: 'string',
      minLength: 3,
      maxLength: 10,
    })
    const result = createProperty({ object: noun, nouns: [noun], tables: {}, jsonExamples: {} })
    expect(result.minLength).toBe(3)
    expect(result.maxLength).toBe(10)
  })

  it('includes minimum and maximum', () => {
    const noun = mkNoun({
      id: '1',
      name: 'Age',
      valueType: 'integer',
      minimum: 0,
      maximum: 150,
    })
    const result = createProperty({ object: noun, nouns: [noun], tables: {}, jsonExamples: {} })
    expect(result.minimum).toBe(0)
    expect(result.maximum).toBe(150)
  })

  it('includes exclusiveMinimum and exclusiveMaximum', () => {
    const noun = mkNoun({
      id: '1',
      name: 'Temperature',
      valueType: 'number',
      exclusiveMinimum: -273.15,
      exclusiveMaximum: 1000,
    })
    const result = createProperty({ object: noun, nouns: [noun], tables: {}, jsonExamples: {} })
    expect(result.exclusiveMinimum).toBe(-273.15)
    expect(result.exclusiveMaximum).toBe(1000)
  })

  it('includes multipleOf', () => {
    const noun = mkNoun({
      id: '1',
      name: 'Quantity',
      valueType: 'integer',
      multipleOf: 5,
    })
    const result = createProperty({ object: noun, nouns: [noun], tables: {}, jsonExamples: {} })
    expect(result.multipleOf).toBe(5)
  })

  it('does not include constraints when value is null/undefined', () => {
    const noun = mkNoun({ id: '1', name: 'Name', valueType: 'string' })
    const result = createProperty({ object: noun, nouns: [noun], tables: {}, jsonExamples: {} })
    expect(result).not.toHaveProperty('format')
    expect(result).not.toHaveProperty('pattern')
    expect(result).not.toHaveProperty('enum')
    expect(result).not.toHaveProperty('minLength')
    expect(result).not.toHaveProperty('maxLength')
    expect(result).not.toHaveProperty('minimum')
    expect(result).not.toHaveProperty('maximum')
  })

  it('creates oneOf with $ref for entity type (no valueType)', () => {
    const entity = mkNoun({ id: '1', name: 'Customer', referenceScheme: [] })
    const idNoun = mkNoun({ id: '2', name: 'Customer Id', valueType: 'integer' })
    entity.referenceScheme = [idNoun]

    const tables: Record<string, any> = {}
    const nouns = [entity, idNoun]
    const result = createProperty({ object: entity, nouns, tables, jsonExamples: {} })

    expect(result.oneOf).toBeDefined()
    expect(result.oneOf).toHaveLength(2)
    // Second element is always the $ref
    expect(result.oneOf![1]).toEqual({ $ref: '#/components/schemas/Customer' })
  })

  it('creates inline property for single referenceScheme item', () => {
    const idNoun = mkNoun({ id: '2', name: 'Customer Nr', valueType: 'integer' })
    const entity = mkNoun({ id: '1', name: 'Customer', referenceScheme: [idNoun] })

    const tables: Record<string, any> = {}
    const nouns = [entity, idNoun]
    const result = createProperty({ object: entity, nouns, tables, jsonExamples: {} })

    // For single referenceScheme, first oneOf is the inline property
    expect(result.oneOf![0]).toEqual({ type: 'integer' })
  })

  it('creates object with properties for multi-item referenceScheme', () => {
    const firstName = mkNoun({ id: '2', name: 'First Name', valueType: 'string' })
    const lastName = mkNoun({ id: '3', name: 'Last Name', valueType: 'string' })
    const entity = mkNoun({ id: '1', name: 'Person', referenceScheme: [firstName, lastName] })

    const tables: Record<string, any> = {}
    const nouns = [entity, firstName, lastName]
    const result = createProperty({ object: entity, nouns, tables, jsonExamples: {} })

    // For multi referenceScheme, first oneOf is an object with properties
    const inline = result.oneOf![0]
    expect(inline.type).toBe('object')
    expect(inline.properties).toBeDefined()
    expect(inline.properties['firstName']).toBeDefined()
    expect(inline.properties['lastName']).toBeDefined()
    expect(inline.required).toContain('firstName')
    expect(inline.required).toContain('lastName')
  })

  it('ensures table exists for entity types', () => {
    const idNoun = mkNoun({ id: '2', name: 'Customer Id', valueType: 'integer' })
    const entity = mkNoun({ id: '1', name: 'Customer', referenceScheme: [idNoun] })

    const tables: Record<string, any> = {}
    const nouns = [entity, idNoun]
    createProperty({ object: entity, nouns, tables, jsonExamples: {} })

    expect(tables['Customer']).toBeDefined()
    expect(tables['UpdateCustomer']).toBeDefined()
    expect(tables['NewCustomer']).toBeDefined()
  })

  it('traverses superType chain to find valueType', () => {
    const grandparent = mkNoun({ id: '1', name: 'Text', valueType: 'string' })
    const parent = mkNoun({ id: '2', name: 'ShortText', superType: grandparent })
    const child = mkNoun({ id: '3', name: 'Name', superType: parent })

    const nouns = [grandparent, parent, child]
    const result = createProperty({ object: child, nouns, tables: {}, jsonExamples: {} })

    expect(result.type).toBe('string')
  })

  it('traverses superType chain to find referenceScheme', () => {
    const idNoun = mkNoun({ id: 'id1', name: 'Party Id', valueType: 'integer' })
    const grandparent = mkNoun({ id: '1', name: 'Party', referenceScheme: [idNoun] })
    const parent = mkNoun({ id: '2', name: 'Person', superType: grandparent })
    const child = mkNoun({ id: '3', name: 'Employee', superType: parent })

    const tables: Record<string, any> = {}
    const nouns = [grandparent, parent, child, idNoun]
    const result = createProperty({ object: child, nouns, tables, jsonExamples: {} })

    // Should resolve to entity type with oneOf
    expect(result.oneOf).toBeDefined()
  })

  it('resolves string object id to noun from nouns array', () => {
    const noun = mkNoun({ id: 'n1', name: 'Name', valueType: 'string' })
    const result = createProperty({
      object: 'n1' as any,
      nouns: [noun],
      tables: {},
      jsonExamples: {},
    })
    expect(result.type).toBe('string')
  })

  it('resolves object by id from nouns array', () => {
    const noun = mkNoun({ id: 'n1', name: 'Name', valueType: 'string' })
    const result = createProperty({
      object: { id: 'n1' } as any,
      nouns: [noun],
      tables: {},
      jsonExamples: {},
    })
    expect(result.type).toBe('string')
  })

  it('handles superType chain as string ids', () => {
    const grandparent = mkNoun({ id: 'g1', name: 'Text', valueType: 'string' })
    const parent = mkNoun({ id: 'p1', name: 'ShortText', superType: 'g1' })
    const child = mkNoun({ id: 'c1', name: 'Name', superType: 'p1' })

    const nouns = [grandparent, parent, child]
    const result = createProperty({ object: child, nouns, tables: {}, jsonExamples: {} })

    expect(result.type).toBe('string')
  })
})

// ---------------------------------------------------------------------------
// setTableProperty
// ---------------------------------------------------------------------------
describe('setTableProperty', () => {
  function setupTables(): Record<string, any> {
    return {
      UpdateCustomer: { $id: 'UpdateCustomer', title: 'Customer', type: 'object' },
      NewCustomer: {
        $id: 'NewCustomer',
        allOf: [{ $ref: '#/components/schemas/UpdateCustomer' }],
      },
      Customer: { $id: 'Customer', allOf: [{ $ref: '#/components/schemas/NewCustomer' }] },
    }
  }

  it('sets a property on UpdateX schema', () => {
    const tables = setupTables()
    const subject = mkNoun({ id: '1', name: 'Customer' })
    const object = mkNoun({ id: '2', name: 'Email', valueType: 'string' })

    setTableProperty({
      tables,
      nouns: [subject, object],
      subject,
      object,
      jsonExamples: {},
    })

    expect(tables['UpdateCustomer'].properties).toBeDefined()
    expect(tables['UpdateCustomer'].properties['email']).toBeDefined()
    expect(tables['UpdateCustomer'].properties['email'].type).toBe('string')
  })

  it('strips subject name prefix from property name', () => {
    const tables = setupTables()
    const subject = mkNoun({ id: '1', name: 'Customer' })
    const object = mkNoun({ id: '2', name: 'CustomerName', valueType: 'string' })

    setTableProperty({
      tables,
      nouns: [subject, object],
      subject,
      object,
      jsonExamples: {},
    })

    // "CustomerName" on "Customer" → strips prefix → "name"
    expect(tables['UpdateCustomer'].properties['name']).toBeDefined()
  })

  it('strips subject name prefix with spaces', () => {
    const tables: Record<string, any> = {
      'UpdateSupportRequest': { $id: 'UpdateSupportRequest', title: 'Support Request', type: 'object' },
      'NewSupportRequest': {
        $id: 'NewSupportRequest',
        allOf: [{ $ref: '#/components/schemas/UpdateSupportRequest' }],
      },
      'SupportRequest': {
        $id: 'SupportRequest',
        allOf: [{ $ref: '#/components/schemas/NewSupportRequest' }],
      },
    }
    const subject = mkNoun({ id: '1', name: 'Support Request' })
    const object = mkNoun({ id: '2', name: 'Support Request Priority', valueType: 'string' })

    setTableProperty({
      tables,
      nouns: [subject, object],
      subject,
      object,
      jsonExamples: {},
    })

    // "SupportRequestPriority" on "Support Request" (compareName "SUPPORTREQUEST") → "priority"
    expect(tables['UpdateSupportRequest'].properties['priority']).toBeDefined()
  })

  it('does not strip prefix when property name equals subject name', () => {
    const tables = setupTables()
    const subject = mkNoun({ id: '1', name: 'Customer' })
    const object = mkNoun({ id: '2', name: 'Customer', valueType: 'string' })

    setTableProperty({
      tables,
      nouns: [subject, object],
      subject,
      object,
      jsonExamples: {},
    })

    // "Customer" on "Customer" — same length, should NOT strip
    expect(tables['UpdateCustomer'].properties['customer']).toBeDefined()
  })

  it('adds to required array on NewX when required is true', () => {
    const tables = setupTables()
    const subject = mkNoun({ id: '1', name: 'Customer' })
    const object = mkNoun({ id: '2', name: 'Email', valueType: 'string' })

    setTableProperty({
      tables,
      nouns: [subject, object],
      subject,
      object,
      required: true,
      jsonExamples: {},
    })

    expect(tables['NewCustomer'].required).toContain('email')
  })

  it('adds id to base schema required (not NewX) when propertyName is id', () => {
    const tables = setupTables()
    const subject = mkNoun({ id: '1', name: 'Customer' })
    const object = mkNoun({ id: '2', name: 'Customer Id', valueType: 'integer' })

    setTableProperty({
      tables,
      nouns: [subject, object],
      subject,
      object,
      required: true,
      jsonExamples: {},
    })

    // Property name is "id" (after stripping "Customer" prefix from "CustomerId")
    // When propertyName is 'id', required goes on base schema (Customer), not NewCustomer
    expect(tables['Customer'].required).toContain('id')
  })

  it('uses provided propertyName when specified', () => {
    const tables = setupTables()
    const subject = mkNoun({ id: '1', name: 'Customer' })
    const object = mkNoun({ id: '2', name: 'Email', valueType: 'string' })

    setTableProperty({
      tables,
      nouns: [subject, object],
      subject,
      object,
      propertyName: 'contactEmail',
      jsonExamples: {},
    })

    expect(tables['UpdateCustomer'].properties['contactEmail']).toBeDefined()
  })

  it('includes description on property', () => {
    const tables = setupTables()
    const subject = mkNoun({ id: '1', name: 'Customer' })
    const object = mkNoun({ id: '2', name: 'Email', valueType: 'string' })

    setTableProperty({
      tables,
      nouns: [subject, object],
      subject,
      object,
      description: 'Primary contact email',
      jsonExamples: {},
    })

    expect(tables['UpdateCustomer'].properties['email'].description).toBe('Primary contact email')
  })

  it('uses provided property when specified', () => {
    const tables = setupTables()
    const subject = mkNoun({ id: '1', name: 'Customer' })
    const object = mkNoun({ id: '2', name: 'Status' })

    setTableProperty({
      tables,
      nouns: [subject, object],
      subject,
      object,
      property: { type: 'string', enum: ['active', 'inactive'] },
      jsonExamples: {},
    })

    expect(tables['UpdateCustomer'].properties['status'].enum).toEqual(['active', 'inactive'])
  })

  it('handles integer example with type coercion', () => {
    const tables = setupTables()
    const subject = mkNoun({ id: '1', name: 'Customer' })
    const object = mkNoun({ id: '2', name: 'Age', valueType: 'integer' })

    setTableProperty({
      tables,
      nouns: [subject, object],
      subject,
      object,
      property: { type: 'integer' },
      example: '25',
      jsonExamples: {},
    })

    const examples = tables['UpdateCustomer'].examples
    expect(examples).toBeDefined()
    expect(examples[0].age).toBe(25)
    expect(typeof examples[0].age).toBe('number')
  })

  it('handles number example with type coercion', () => {
    const tables = setupTables()
    const subject = mkNoun({ id: '1', name: 'Customer' })
    const object = mkNoun({ id: '2', name: 'Balance', valueType: 'number' })

    setTableProperty({
      tables,
      nouns: [subject, object],
      subject,
      object,
      property: { type: 'number' },
      example: '99.95',
      jsonExamples: {},
    })

    const examples = tables['UpdateCustomer'].examples
    expect(examples[0].balance).toBe(99.95)
  })

  it('handles boolean example with type coercion', () => {
    const tables = setupTables()
    const subject = mkNoun({ id: '1', name: 'Customer' })
    const object = mkNoun({ id: '2', name: 'Active', valueType: 'boolean' })

    setTableProperty({
      tables,
      nouns: [subject, object],
      subject,
      object,
      property: { type: 'boolean' },
      example: 'true',
      jsonExamples: {},
    })

    const examples = tables['UpdateCustomer'].examples
    expect(examples[0].active).toBe(true)
  })

  it('handles string example without coercion', () => {
    const tables = setupTables()
    const subject = mkNoun({ id: '1', name: 'Customer' })
    const object = mkNoun({ id: '2', name: 'Name', valueType: 'string' })

    setTableProperty({
      tables,
      nouns: [subject, object],
      subject,
      object,
      property: { type: 'string' },
      example: 'John Doe',
      jsonExamples: {},
    })

    const examples = tables['UpdateCustomer'].examples
    expect(examples[0].name).toBe('John Doe')
  })
})

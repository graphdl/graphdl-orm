import { describe, it, expect } from 'vitest'
import { processBinarySchemas, processArraySchemas, processUnarySchemas } from './fact-processors'
import type { NounRef } from './rmap'
import { nounListToRegex } from './rmap'
import type { Schema, JSONSchemaType } from './schema-builder'

// ---------------------------------------------------------------------------
// Helper factories
// ---------------------------------------------------------------------------
function mkNoun(overrides: Partial<NounRef> & { id: string; name: string }): NounRef {
  return { ...overrides }
}

function mkRole(id: string, noun: NounRef, graphSchemaId: string) {
  return { id, noun: { value: noun }, graphSchema: { id: graphSchemaId } }
}

function mkGraphSchema(opts: {
  id: string
  name: string
  roles: ReturnType<typeof mkRole>[]
  readingText: string
}) {
  return {
    id: opts.id,
    name: opts.name,
    roles: { docs: opts.roles },
    readings: { docs: [{ text: opts.readingText }] },
  }
}

function mkConstraintSpan(roles: ReturnType<typeof mkRole>[]) {
  return { roles }
}

function setupTables(name: string): Record<string, Schema> {
  const key = name.replace(/[ \-]/g, '').replace(/&/g, 'And')
  return {
    ['Update' + key]: { $id: 'Update' + key, title: name, type: 'object' },
    ['New' + key]: {
      $id: 'New' + key,
      allOf: [{ $ref: '#/components/schemas/Update' + key }],
    },
    [key]: { $id: key, allOf: [{ $ref: '#/components/schemas/New' + key }] },
  }
}

// ---------------------------------------------------------------------------
// processBinarySchemas
// ---------------------------------------------------------------------------
describe('processBinarySchemas', () => {
  it('adds value-type property to subject schema for single-role UC', () => {
    const customer = mkNoun({ id: 'n1', name: 'Customer', objectType: 'entity' })
    const name = mkNoun({ id: 'n2', name: 'Name', objectType: 'value', valueType: 'string' })
    const nouns = [customer, name]
    const nounRegex = nounListToRegex(nouns)

    const role1 = mkRole('r1', customer, 'gs1')
    const role2 = mkRole('r2', name, 'gs1')
    const gs = mkGraphSchema({
      id: 'gs1',
      name: 'CustomerName',
      roles: [role1, role2],
      readingText: 'Customer has Name',
    })

    // Single-role UC constraining the Customer role → property on Customer
    const cs = mkConstraintSpan([role1])
    const schemas = setupTables('Customer')

    processBinarySchemas(
      [cs],
      schemas,
      nouns,
      {},
      nounRegex,
      [],
      [gs],
    )

    expect(schemas['UpdateCustomer'].properties).toBeDefined()
    expect(schemas['UpdateCustomer'].properties!['name']).toBeDefined()
    expect(schemas['UpdateCustomer'].properties!['name'].type).toBe('string')
    expect(schemas['UpdateCustomer'].properties!['name'].description).toBe('Customer has Name')
  })

  it('adds entity-type property as oneOf with $ref for single-role UC', () => {
    const order = mkNoun({ id: 'n1', name: 'Order', objectType: 'entity' })
    const customer = mkNoun({ id: 'n2', name: 'Customer', objectType: 'entity', referenceScheme: [] })
    const customerId = mkNoun({ id: 'n3', name: 'Customer Id', valueType: 'integer' })
    customer.referenceScheme = [customerId]
    const nouns = [order, customer, customerId]
    const nounRegex = nounListToRegex(nouns)

    const role1 = mkRole('r1', order, 'gs1')
    const role2 = mkRole('r2', customer, 'gs1')
    const gs = mkGraphSchema({
      id: 'gs1',
      name: 'OrderCustomer',
      roles: [role1, role2],
      readingText: 'Order has Customer',
    })

    const cs = mkConstraintSpan([role1])
    const schemas = setupTables('Order')

    processBinarySchemas([cs], schemas, nouns, {}, nounRegex, [], [gs])

    const prop = schemas['UpdateOrder'].properties!['customer']
    expect(prop).toBeDefined()
    expect(prop.oneOf).toBeDefined()
    expect(prop.oneOf).toContainEqual({ $ref: '#/components/schemas/Customer' })
  })

  it('skips constraint spans with more than 1 role', () => {
    const customer = mkNoun({ id: 'n1', name: 'Customer', objectType: 'entity' })
    const name = mkNoun({ id: 'n2', name: 'Name', objectType: 'value', valueType: 'string' })
    const nouns = [customer, name]
    const nounRegex = nounListToRegex(nouns)

    const role1 = mkRole('r1', customer, 'gs1')
    const role2 = mkRole('r2', name, 'gs1')
    const gs = mkGraphSchema({
      id: 'gs1',
      name: 'CustomerName',
      roles: [role1, role2],
      readingText: 'Customer has Name',
    })

    // Two roles in constraint span → skip (compound UC, not binary)
    const cs = mkConstraintSpan([role1, role2])
    const schemas = setupTables('Customer')

    processBinarySchemas([cs], schemas, nouns, {}, nounRegex, [], [gs])

    // No properties should be added
    expect(schemas['UpdateCustomer'].properties).toBeUndefined()
  })

  it('skips when graph schema not found in graphSchemas array', () => {
    const customer = mkNoun({ id: 'n1', name: 'Customer', objectType: 'entity' })
    const name = mkNoun({ id: 'n2', name: 'Name', objectType: 'value', valueType: 'string' })
    const nouns = [customer, name]
    const nounRegex = nounListToRegex(nouns)

    const role1 = mkRole('r1', customer, 'gs-missing')
    const cs = mkConstraintSpan([role1])
    const schemas = setupTables('Customer')

    // graphSchemas is empty → no matching schema found
    processBinarySchemas([cs], schemas, nouns, {}, nounRegex, [], [])

    expect(schemas['UpdateCustomer'].properties).toBeUndefined()
  })

  it('handles empty constraint spans array', () => {
    const schemas = setupTables('Customer')
    processBinarySchemas([], schemas, [], {}, /(?:)/, [], [])
    expect(schemas['UpdateCustomer'].properties).toBeUndefined()
  })

  it('marks required property when role is required', () => {
    const customer = mkNoun({ id: 'n1', name: 'Customer', objectType: 'entity' })
    const name = mkNoun({ id: 'n2', name: 'Name', objectType: 'value', valueType: 'string' })
    const nouns = [customer, name]
    const nounRegex = nounListToRegex(nouns)

    const role1 = { ...mkRole('r1', customer, 'gs1'), required: true }
    const role2 = mkRole('r2', name, 'gs1')
    const gs = mkGraphSchema({
      id: 'gs1',
      name: 'CustomerName',
      roles: [role1, role2],
      readingText: 'Customer has Name',
    })

    const cs = mkConstraintSpan([role1])
    const schemas = setupTables('Customer')

    processBinarySchemas([cs], schemas, nouns, {}, nounRegex, [], [gs])

    expect(schemas['NewCustomer'].required).toContain('name')
  })

  it('extracts example value from examples data', () => {
    const customer = mkNoun({ id: 'n1', name: 'Customer', objectType: 'entity' })
    const name = mkNoun({ id: 'n2', name: 'Name', objectType: 'value', valueType: 'string' })
    const nouns = [customer, name]
    const nounRegex = nounListToRegex(nouns)

    const role1 = mkRole('r1', customer, 'gs1')
    const role2 = mkRole('r2', name, 'gs1')
    const gs = mkGraphSchema({
      id: 'gs1',
      name: 'CustomerName',
      roles: [role1, role2],
      readingText: 'Customer has Name',
    })

    const cs = mkConstraintSpan([role1])
    const schemas = setupTables('Customer')

    const examples = [
      {
        type: { id: 'gs1' },
        resourceRoles: {
          docs: [
            {
              role: { id: 'r2' },
              resource: { value: { value: 'Acme Corp' } },
            },
          ],
        },
      },
    ]

    processBinarySchemas([cs], schemas, nouns, {}, nounRegex, examples as any, [gs])

    const ex = schemas['UpdateCustomer'].examples as any[]
    expect(ex).toBeDefined()
    expect(ex[0].name).toBe('Acme Corp')
  })
})

// ---------------------------------------------------------------------------
// processArraySchemas
// ---------------------------------------------------------------------------
describe('processArraySchemas', () => {
  it('adds array property on subject schema', () => {
    const customer = mkNoun({ id: 'n1', name: 'Customer', objectType: 'entity' })
    const email = mkNoun({ id: 'n2', name: 'Email', objectType: 'value', valueType: 'string' })
    const nouns = [customer, email]
    const nounRegex = nounListToRegex(nouns)

    const role1 = mkRole('r1', customer, 'gs1')
    const role2 = mkRole('r2', email, 'gs1')
    const gs = mkGraphSchema({
      id: 'gs1',
      name: '',
      roles: [role1, role2],
      readingText: 'Customer has Email',
    })

    const cs = mkConstraintSpan([role1, role2])
    const schemas = setupTables('Customer')

    processArraySchemas([{ gs, cs }], nouns, nounRegex, schemas, {})

    const prop = schemas['UpdateCustomer'].properties!['emails']
    expect(prop).toBeDefined()
    expect(prop.type).toBe('array')
    expect(prop.items).toBeDefined()
    expect((prop.items as Schema).type).toBe('string')
    expect(prop.description).toBe('Customer has Email')
  })

  it('uses schema name as property name when available', () => {
    const customer = mkNoun({ id: 'n1', name: 'Customer', objectType: 'entity' })
    const address = mkNoun({ id: 'n2', name: 'Address', objectType: 'value', valueType: 'string' })
    const nouns = [customer, address]
    const nounRegex = nounListToRegex(nouns)

    const role1 = mkRole('r1', customer, 'gs1')
    const role2 = mkRole('r2', address, 'gs1')
    const gs = mkGraphSchema({
      id: 'gs1',
      name: 'MailingAddresses',
      roles: [role1, role2],
      readingText: 'Customer has Address',
    })

    const cs = mkConstraintSpan([role1, role2])
    const schemas = setupTables('Customer')

    processArraySchemas([{ gs, cs }], nouns, nounRegex, schemas, {})

    expect(schemas['UpdateCustomer'].properties!['mailingAddresses']).toBeDefined()
  })

  it('uses plural form for property name when available', () => {
    const customer = mkNoun({ id: 'n1', name: 'Customer', objectType: 'entity' })
    const domain = mkNoun({ id: 'n2', name: 'Domain', objectType: 'entity', plural: 'domains', referenceScheme: [] })
    const domainId = mkNoun({ id: 'n3', name: 'Domain Id', valueType: 'string' })
    domain.referenceScheme = [domainId]
    const nouns = [customer, domain, domainId]
    const nounRegex = nounListToRegex(nouns)

    const role1 = mkRole('r1', customer, 'gs1')
    const role2 = mkRole('r2', domain, 'gs1')
    const gs = mkGraphSchema({
      id: 'gs1',
      name: '',
      roles: [role1, role2],
      readingText: 'Customer has Domain',
    })

    const cs = mkConstraintSpan([role1, role2])
    const schemas = setupTables('Customer')

    processArraySchemas([{ gs, cs }], nouns, nounRegex, schemas, {})

    // When plural is provided, don't add extra 's'
    const props = schemas['UpdateCustomer'].properties!
    // The property name should use the plural form (via findPredicateObject replacing the noun)
    expect(props['domains']).toBeDefined()
  })

  it('skips array types with unresolved nouns', () => {
    const customer = mkNoun({ id: 'n1', name: 'Customer', objectType: 'entity' })
    const missingNoun: NounRef = { id: 'n2', name: undefined as any }
    const nouns = [customer, missingNoun]
    const nounRegex = nounListToRegex(nouns)

    const role1 = mkRole('r1', customer, 'gs1')
    const role2 = mkRole('r2', missingNoun, 'gs1')
    const gs = mkGraphSchema({
      id: 'gs1',
      name: '',
      roles: [role1, role2],
      readingText: 'Customer has Something',
    })

    const cs = mkConstraintSpan([role1, role2])
    const schemas = setupTables('Customer')

    processArraySchemas([{ gs, cs }], nouns, nounRegex, schemas, {})

    // Should skip because object noun has no name
    expect(schemas['UpdateCustomer'].properties).toBeUndefined()
  })

  it('handles empty arrayTypes array', () => {
    const schemas = setupTables('Customer')
    processArraySchemas([], [], /(?:)/, schemas, {})
    expect(schemas['UpdateCustomer'].properties).toBeUndefined()
  })

  it('resolves string noun refs from nouns array', () => {
    const customer = mkNoun({ id: 'n1', name: 'Customer', objectType: 'entity' })
    const tag = mkNoun({ id: 'n2', name: 'Tag', objectType: 'value', valueType: 'string' })
    const nouns = [customer, tag]
    const nounRegex = nounListToRegex(nouns)

    // Roles with string noun values instead of full objects
    const role1 = { id: 'r1', noun: { value: 'n1' }, graphSchema: { id: 'gs1' } }
    const role2 = { id: 'r2', noun: { value: 'n2' }, graphSchema: { id: 'gs1' } }
    const gs = {
      id: 'gs1',
      name: '',
      roles: { docs: [role1, role2] },
      readings: { docs: [{ text: 'Customer has Tag' }] },
    }

    const cs = { roles: [role1, role2] }
    const schemas = setupTables('Customer')

    processArraySchemas([{ gs: gs as any, cs: cs as any }], nouns, nounRegex, schemas, {})

    expect(schemas['UpdateCustomer'].properties!['tags']).toBeDefined()
    expect(schemas['UpdateCustomer'].properties!['tags'].type).toBe('array')
  })
})

// ---------------------------------------------------------------------------
// processUnarySchemas
// ---------------------------------------------------------------------------
describe('processUnarySchemas', () => {
  it('adds boolean property for single-role schema', () => {
    const customer = mkNoun({ id: 'n1', name: 'Customer', objectType: 'entity' })
    const nouns = [customer]
    const nounRegex = nounListToRegex(nouns)

    const role = mkRole('r1', customer, 'gs1')
    const gs = mkGraphSchema({
      id: 'gs1',
      name: 'CustomerIsActive',
      roles: [role],
      readingText: 'Customer is active',
    })

    const schemas = setupTables('Customer')

    processUnarySchemas([gs], nouns, nounRegex, schemas, {}, [])

    const prop = schemas['UpdateCustomer'].properties!['isActive']
    expect(prop).toBeDefined()
    expect(prop.type).toBe('boolean')
  })

  it('sets description from predicate', () => {
    const customer = mkNoun({ id: 'n1', name: 'Customer', objectType: 'entity' })
    const nouns = [customer]
    const nounRegex = nounListToRegex(nouns)

    const role = mkRole('r1', customer, 'gs1')
    const gs = mkGraphSchema({
      id: 'gs1',
      name: 'CustomerIsVerified',
      roles: [role],
      readingText: 'Customer is verified',
    })

    const schemas = setupTables('Customer')

    processUnarySchemas([gs], nouns, nounRegex, schemas, {}, [])

    const prop = schemas['UpdateCustomer'].properties!['isVerified']
    expect(prop).toBeDefined()
    expect(prop.description).toContain('Customer')
    expect(prop.description).toContain('verified')
  })

  it('skips graph schemas with more than 1 role', () => {
    const customer = mkNoun({ id: 'n1', name: 'Customer', objectType: 'entity' })
    const name = mkNoun({ id: 'n2', name: 'Name', valueType: 'string' })
    const nouns = [customer, name]
    const nounRegex = nounListToRegex(nouns)

    const role1 = mkRole('r1', customer, 'gs1')
    const role2 = mkRole('r2', name, 'gs1')
    const gs = mkGraphSchema({
      id: 'gs1',
      name: 'CustomerName',
      roles: [role1, role2],
      readingText: 'Customer has Name',
    })

    const schemas = setupTables('Customer')

    processUnarySchemas([gs], nouns, nounRegex, schemas, {}, [])

    expect(schemas['UpdateCustomer'].properties).toBeUndefined()
  })

  it('marks required property when role is required', () => {
    const customer = mkNoun({ id: 'n1', name: 'Customer', objectType: 'entity' })
    const nouns = [customer]
    const nounRegex = nounListToRegex(nouns)

    const role = { ...mkRole('r1', customer, 'gs1'), required: true }
    const gs = mkGraphSchema({
      id: 'gs1',
      name: 'CustomerIsActive',
      roles: [role],
      readingText: 'Customer is active',
    })

    const schemas = setupTables('Customer')

    processUnarySchemas([gs], nouns, nounRegex, schemas, {}, [])

    expect(schemas['NewCustomer'].required).toContain('isActive')
  })

  it('extracts example value from examples data', () => {
    const customer = mkNoun({ id: 'n1', name: 'Customer', objectType: 'entity' })
    const nouns = [customer]
    const nounRegex = nounListToRegex(nouns)

    const role = mkRole('r1', customer, 'gs1')
    const gs = mkGraphSchema({
      id: 'gs1',
      name: 'CustomerIsActive',
      roles: [role],
      readingText: 'Customer is active',
    })

    const schemas = setupTables('Customer')

    const examples = [
      {
        type: { id: 'gs1' },
        resourceRoles: {
          docs: [
            {
              role: { id: 'r1' },
              resource: { value: { value: 'true' } },
            },
          ],
        },
      },
    ]

    processUnarySchemas([gs], nouns, nounRegex, schemas, {}, examples as any)

    const ex = schemas['UpdateCustomer'].examples as any[]
    expect(ex).toBeDefined()
    expect(ex[0].isActive).toBe(true)
  })

  it('handles empty graph schemas array', () => {
    const schemas = setupTables('Customer')
    processUnarySchemas([], [], /(?:)/, schemas, {}, [])
    expect(schemas['UpdateCustomer'].properties).toBeUndefined()
  })
})

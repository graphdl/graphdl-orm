import { describe, it, expect } from 'vitest'
import { generateSQLite } from './sqlite'

// ---------------------------------------------------------------------------
// Helpers — build minimal OpenAPI-style schema objects
// ---------------------------------------------------------------------------

function openapi(schemas: Record<string, any>) {
  return { openapi: '3.0.0', components: { schemas } }
}

/** Convenience: creates Update+New+base triplet for an entity. */
function entityTriplet(
  name: string,
  properties: Record<string, any>,
  required?: string[],
) {
  const update: any = {
    $id: `Update${name}`,
    type: 'object',
    title: name,
    properties,
  }
  if (required) update.required = required
  return {
    [`Update${name}`]: update,
    [`New${name}`]: {
      $id: `New${name}`,
      type: 'object',
      title: name,
      properties,
      ...(required ? { required } : {}),
    },
    [name]: {
      $id: name,
      type: 'object',
      title: name,
      properties,
      ...(required ? { required } : {}),
    },
  }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

describe('generateSQLite', () => {
  it('returns empty results for empty schemas', () => {
    const result = generateSQLite(openapi({}))
    expect(result.ddl).toEqual([])
    expect(result.tableMap).toEqual({})
    expect(result.fieldMap).toEqual({})
  })

  it('generates table with value-type string column', () => {
    const api = openapi(entityTriplet('Customer', { name: { type: 'string' } }))
    const result = generateSQLite(api)

    expect(result.ddl.length).toBeGreaterThan(0)
    expect(result.tableMap['Customer']).toBe('customers')

    // The DDL should contain a CREATE TABLE for customers
    const createTable = result.ddl.find((s) => s.startsWith('CREATE TABLE'))
    expect(createTable).toBeDefined()
    expect(createTable).toContain('customers')
    expect(createTable).toContain('name TEXT')
    expect(createTable).toContain('id TEXT PRIMARY KEY')
    expect(createTable).toContain('domain_id TEXT REFERENCES domains(id)')
    expect(createTable).toContain("created_at TEXT NOT NULL DEFAULT (datetime('now'))")
    expect(createTable).toContain("updated_at TEXT NOT NULL DEFAULT (datetime('now'))")
    expect(createTable).toContain('version INTEGER NOT NULL DEFAULT 1')
  })

  it('maps integer type to INTEGER', () => {
    const api = openapi(entityTriplet('Product', { quantity: { type: 'integer' } }))
    const result = generateSQLite(api)

    const createTable = result.ddl.find((s) => s.startsWith('CREATE TABLE'))
    expect(createTable).toContain('quantity INTEGER')
  })

  it('maps number type to REAL', () => {
    const api = openapi(entityTriplet('Product', { price: { type: 'number' } }))
    const result = generateSQLite(api)

    const createTable = result.ddl.find((s) => s.startsWith('CREATE TABLE'))
    expect(createTable).toContain('price REAL')
  })

  it('maps boolean type to INTEGER', () => {
    const api = openapi(entityTriplet('Customer', { isActive: { type: 'boolean' } }))
    const result = generateSQLite(api)

    const createTable = result.ddl.find((s) => s.startsWith('CREATE TABLE'))
    expect(createTable).toContain('is_active INTEGER')
  })

  it('maps array type to TEXT (JSON)', () => {
    const api = openapi(
      entityTriplet('Customer', {
        skills: { type: 'array', items: { type: 'string' } },
      }),
    )
    const result = generateSQLite(api)

    const createTable = result.ddl.find((s) => s.startsWith('CREATE TABLE'))
    expect(createTable).toContain('skills TEXT')
  })

  it('maps object type to TEXT (JSON)', () => {
    const api = openapi(
      entityTriplet('Customer', {
        metadata: { type: 'object' },
      }),
    )
    const result = generateSQLite(api)

    const createTable = result.ddl.find((s) => s.startsWith('CREATE TABLE'))
    expect(createTable).toContain('metadata TEXT')
  })

  it('generates FK column from oneOf with $ref', () => {
    const api = openapi({
      ...entityTriplet('Customer', { name: { type: 'string' } }),
      ...entityTriplet('Order', {
        customer: {
          oneOf: [{ type: 'string' }, { $ref: '#/components/schemas/Customer' }],
        },
        total: { type: 'number' },
      }),
    })
    const result = generateSQLite(api)

    const orderTable = result.ddl.find(
      (s) => s.startsWith('CREATE TABLE') && s.includes('orders'),
    )
    expect(orderTable).toBeDefined()
    expect(orderTable).toContain('customer_id TEXT REFERENCES customers(id)')
    // Should NOT have a plain "customer" column
    expect(orderTable).not.toMatch(/\bcustomer TEXT\b/)
  })

  it('generates FK column from direct $ref', () => {
    const api = openapi({
      ...entityTriplet('Customer', { name: { type: 'string' } }),
      ...entityTriplet('Order', {
        customer: { $ref: '#/components/schemas/Customer' },
        total: { type: 'number' },
      }),
    })
    const result = generateSQLite(api)

    const orderTable = result.ddl.find(
      (s) => s.startsWith('CREATE TABLE') && s.includes('orders'),
    )
    expect(orderTable).toBeDefined()
    expect(orderTable).toContain('customer_id TEXT REFERENCES customers(id)')
  })

  it('generates CREATE INDEX for FK columns', () => {
    const api = openapi({
      ...entityTriplet('Customer', { name: { type: 'string' } }),
      ...entityTriplet('Order', {
        customer: {
          oneOf: [{ type: 'string' }, { $ref: '#/components/schemas/Customer' }],
        },
      }),
    })
    const result = generateSQLite(api)

    // Index on customer_id
    const fkIndex = result.ddl.find(
      (s) => s.startsWith('CREATE INDEX') && s.includes('customer_id'),
    )
    expect(fkIndex).toBeDefined()
    expect(fkIndex).toContain('orders')

    // Index on domain_id (always present)
    const domainIndex = result.ddl.find(
      (s) => s.startsWith('CREATE INDEX') && s.includes('domain_id') && s.includes('orders'),
    )
    expect(domainIndex).toBeDefined()
  })

  it('always generates domain_id index', () => {
    const api = openapi(entityTriplet('Customer', { name: { type: 'string' } }))
    const result = generateSQLite(api)

    const domainIndex = result.ddl.find(
      (s) => s.startsWith('CREATE INDEX') && s.includes('domain_id'),
    )
    expect(domainIndex).toBeDefined()
    expect(domainIndex).toContain('customers')
  })

  it('builds correct tableMap', () => {
    const api = openapi({
      ...entityTriplet('Customer', { name: { type: 'string' } }),
      ...entityTriplet('SupportRequest', { subject: { type: 'string' } }),
    })
    const result = generateSQLite(api)

    expect(result.tableMap['Customer']).toBe('customers')
    expect(result.tableMap['SupportRequest']).toBe('support_requests')
  })

  it('builds fieldMap only where names differ', () => {
    const api = openapi({
      ...entityTriplet('Customer', {
        name: { type: 'string' },
        isActive: { type: 'boolean' },
        graphSchema: {
          oneOf: [{ type: 'string' }, { $ref: '#/components/schemas/GraphSchema' }],
        },
      }),
      ...entityTriplet('GraphSchema', { name: { type: 'string' } }),
    })
    const result = generateSQLite(api)

    const fm = result.fieldMap['customers']
    expect(fm).toBeDefined()

    // name → name (same, should NOT be in fieldMap)
    expect(fm['name']).toBeUndefined()

    // isActive → is_active (different)
    expect(fm['isActive']).toBe('is_active')

    // graphSchema → graph_schema_id (FK to existing entity table)
    expect(fm['graphSchema']).toBe('graph_schema_id')
  })

  it('handles PascalCase to snake_case table name conversion correctly', () => {
    const api = openapi(entityTriplet('GraphSchema', { name: { type: 'string' } }))
    const result = generateSQLite(api)

    expect(result.tableMap['GraphSchema']).toBe('graph_schemas')
  })

  it('produces DDL for multiple entities', () => {
    const api = openapi({
      ...entityTriplet('Customer', { name: { type: 'string' } }),
      ...entityTriplet('Product', { price: { type: 'number' } }),
    })
    const result = generateSQLite(api)

    const createTables = result.ddl.filter((s) => s.startsWith('CREATE TABLE'))
    expect(createTables.length).toBe(2)

    const tableNames = createTables.map((ct) => {
      const match = ct.match(/CREATE TABLE (\w+)/)
      return match?.[1]
    })
    expect(tableNames).toContain('customers')
    expect(tableNames).toContain('products')
  })

  it('skips schemas without Update prefix or without type object', () => {
    // Only UpdateX schemas with type: 'object' should be processed
    const api = openapi({
      // This has no UpdateFoo counterpart — should be ignored
      Foo: { $id: 'Foo', type: 'object', properties: { x: { type: 'string' } } },
      // This has an Update but no base — should also be ignored
      UpdateBar: { $id: 'UpdateBar', type: 'object', properties: { y: { type: 'string' } } },
    })
    const result = generateSQLite(api)

    // Bar has Update but no base schema named "Bar", so it should be skipped
    expect(result.ddl).toEqual([])
    expect(result.tableMap).toEqual({})
  })

  it('FK columns get _id suffix and REFERENCES clause', () => {
    // Verify that an entity reference property produces a column named
    // "propertyName_id" with a REFERENCES clause to the target table.
    const api = openapi({
      ...entityTriplet('Author', { name: { type: 'string' } }),
      ...entityTriplet('Book', {
        title: { type: 'string' },
        author: {
          oneOf: [{ type: 'string' }, { $ref: '#/components/schemas/Author' }],
        },
      }),
    })
    const result = generateSQLite(api)

    const bookTable = result.ddl.find(
      (s) => s.startsWith('CREATE TABLE') && s.includes('books'),
    )
    expect(bookTable).toBeDefined()
    // Must have author_id (with _id suffix), not just author
    expect(bookTable).toContain('author_id TEXT REFERENCES authors(id)')
    // Should NOT have a bare "author TEXT" column
    expect(bookTable).not.toMatch(/\bauthor TEXT\b/)
  })

  it('system columns (id, domain_id, created_at, updated_at, version) are on every table', () => {
    // Verify that EVERY generated table includes all 5 system columns
    const api = openapi({
      ...entityTriplet('Customer', { email: { type: 'string' } }),
      ...entityTriplet('Order', { total: { type: 'number' } }),
    })
    const result = generateSQLite(api)

    const createTables = result.ddl.filter((s) => s.startsWith('CREATE TABLE'))
    expect(createTables.length).toBe(2)

    for (const ct of createTables) {
      expect(ct).toContain('id TEXT PRIMARY KEY')
      expect(ct).toContain('domain_id TEXT REFERENCES domains(id)')
      expect(ct).toContain("created_at TEXT NOT NULL DEFAULT (datetime('now'))")
      expect(ct).toContain("updated_at TEXT NOT NULL DEFAULT (datetime('now'))")
      expect(ct).toContain('version INTEGER NOT NULL DEFAULT 1')
    }
  })

  it('creates index for every FK column', () => {
    // Verify indexes are created for each FK column on a table with multiple FKs
    const api = openapi({
      ...entityTriplet('Customer', { name: { type: 'string' } }),
      ...entityTriplet('Product', { name: { type: 'string' } }),
      ...entityTriplet('Order', {
        customer: {
          oneOf: [{ type: 'string' }, { $ref: '#/components/schemas/Customer' }],
        },
        product: {
          oneOf: [{ type: 'string' }, { $ref: '#/components/schemas/Product' }],
        },
        quantity: { type: 'integer' },
      }),
    })
    const result = generateSQLite(api)

    // Should have indexes for customer_id, product_id, and domain_id
    const orderIndexes = result.ddl.filter(
      (s) => s.startsWith('CREATE INDEX') && s.includes('orders'),
    )

    const hasCustomerIdx = orderIndexes.some((s) => s.includes('customer_id'))
    const hasProductIdx = orderIndexes.some((s) => s.includes('product_id'))
    const hasDomainIdx = orderIndexes.some((s) => s.includes('domain_id'))

    expect(hasCustomerIdx).toBe(true)
    expect(hasProductIdx).toBe(true)
    expect(hasDomainIdx).toBe(true)
  })

  it('FK column from direct $ref also gets _id suffix and index', () => {
    // Same as oneOf but with direct $ref syntax
    const api = openapi({
      ...entityTriplet('Department', { name: { type: 'string' } }),
      ...entityTriplet('Employee', {
        name: { type: 'string' },
        department: { $ref: '#/components/schemas/Department' },
      }),
    })
    const result = generateSQLite(api)

    const empTable = result.ddl.find(
      (s) => s.startsWith('CREATE TABLE') && s.includes('employees'),
    )
    expect(empTable).toContain('department_id TEXT REFERENCES departments(id)')

    const deptIndex = result.ddl.find(
      (s) => s.startsWith('CREATE INDEX') && s.includes('department_id'),
    )
    expect(deptIndex).toBeDefined()
    expect(deptIndex).toContain('employees')
  })

  it('defaults unknown types to TEXT', () => {
    const api = openapi(
      entityTriplet('Widget', {
        data: { type: 'unknown-weird-type' },
      }),
    )
    const result = generateSQLite(api)

    const createTable = result.ddl.find((s) => s.startsWith('CREATE TABLE'))
    expect(createTable).toContain('data TEXT')
  })

  it('handles entity with no user-defined properties (only system columns)', () => {
    const api = openapi(entityTriplet('Widget', {}))
    const result = generateSQLite(api)

    const createTable = result.ddl.find((s) => s.startsWith('CREATE TABLE'))
    expect(createTable).toBeDefined()
    expect(createTable).toContain('id TEXT PRIMARY KEY')
    expect(createTable).toContain('domain_id TEXT REFERENCES domains(id)')
    expect(createTable).toContain("created_at TEXT NOT NULL DEFAULT (datetime('now'))")
    expect(createTable).toContain("updated_at TEXT NOT NULL DEFAULT (datetime('now'))")
    expect(createTable).toContain('version INTEGER NOT NULL DEFAULT 1')
    expect(result.tableMap['Widget']).toBe('widgets')
  })
})

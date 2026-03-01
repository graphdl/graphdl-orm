import { describe, it, expect, beforeAll } from 'vitest'
import { initPayload } from '../helpers/initPayload'
import { seedSupportDomain } from '../helpers/seed'

let payload: any
let output: any
let collections: any

describe('Payload Collection Generator', () => {
  beforeAll(async () => {
    payload = await initPayload()

    // Drop database for clean slate
    await payload.db.connection.dropDatabase()

    await seedSupportDomain(payload)

    const generator = await payload.create({
      collection: 'generators',
      data: {
        title: 'auto.dev Support API',
        version: '1.0.0',
        databaseEngine: 'Payload',
      },
    })

    output = generator.output
    collections = output.payloadCollections
  }, 120_000)

  it('should generate payloadCollections in output', () => {
    expect(collections).toBeDefined()
    expect(typeof collections).toBe('object')
  })

  it('should generate collections for seeded entity nouns', () => {
    expect(collections['support-requests']).toBeDefined()
    expect(collections['feature-requests']).toBeDefined()
    expect(collections['customers']).toBeDefined()
    expect(collections['api-products']).toBeDefined()
  })

  it('should set correct slugs and labels', () => {
    expect(collections['support-requests'].slug).toBe('support-requests')
    expect(collections['support-requests'].labels.singular).toBe('SupportRequest')
    expect(collections['customers'].slug).toBe('customers')
  })

  it('should produce all expected Payload field types', () => {
    // Collect all field types across all generated collections
    const allFields = Object.values(collections).flatMap((c: any) => c.fields)
    const fieldTypes = new Set(allFields.map((f: any) => f.type))

    // text (string value types), select (enums), email (format: email),
    // number (integer), relationship (entity references)
    expect(fieldTypes.has('text')).toBe(true)
    expect(fieldTypes.has('select')).toBe(true)
    expect(fieldTypes.has('email')).toBe(true)
    expect(fieldTypes.has('number')).toBe(true)
    expect(fieldTypes.has('relationship')).toBe(true)

    // Select fields should have options arrays
    const selectFields = allFields.filter((f: any) => f.type === 'select')
    for (const f of selectFields) {
      expect(Array.isArray((f as any).options)).toBe(true)
      expect((f as any).options.length).toBeGreaterThan(0)
    }
  })

  it('should include access control based on permissions', () => {
    const sr = collections['support-requests']
    expect(sr.access).toBeDefined()
    expect(sr.access.create).toBeDefined()
    expect(sr.access.read).toBeDefined()
    expect(sr.access.update).toBeDefined()
    // SupportRequest has no delete permission
    expect(sr.access.delete).toBeUndefined()
  })

  it('should include auth config when login permission is set', () => {
    expect(collections['customers'].auth).toBe(true)
    expect(collections['support-requests'].auth).toBeUndefined()
  })

  it('should set admin useAsTitle on all collections', () => {
    for (const c of Object.values(collections) as any[]) {
      expect(c.admin).toBeDefined()
      expect(c.admin.useAsTitle).toBeDefined()
    }
  })

  it('should generate valid OpenAPI output alongside Payload collections', () => {
    expect(output.openapi).toBe('3.1.0')
    expect(output.components?.schemas).toBeDefined()
    expect(output.paths).toBeDefined()
  })
}, 120_000)

import { describe, it, expect, beforeAll } from 'vitest'
import { initPayload } from '../helpers/initPayload'
import { seedSupportDomain } from '../helpers/seed'

let payload: any
let output: any

describe('Payload Collection Generator', () => {
  beforeAll(async () => {
    payload = await initPayload()

    // Clear any data from prior test files sharing this fork
    for (const slug of ['readings', 'roles', 'constraint-spans', 'constraints', 'graph-schemas', 'nouns', 'generators'] as const) {
      await payload.delete({ collection: slug, where: { id: { exists: true } } })
    }

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
  }, 120_000)

  it('should generate payloadCollections in output', () => {
    expect(output.payloadCollections).toBeDefined()
    expect(typeof output.payloadCollections).toBe('object')
  })

  it('should generate all four entity collections', () => {
    const c = output.payloadCollections
    expect(c['support-requests']).toBeDefined()
    expect(c['feature-requests']).toBeDefined()
    expect(c['customers']).toBeDefined()
    expect(c['api-products']).toBeDefined()
  })

  it('should set correct slugs and labels', () => {
    const sr = output.payloadCollections['support-requests']
    expect(sr.slug).toBe('support-requests')
    expect(sr.labels.singular).toBe('SupportRequest')
  })

  it('should map string value types to text fields', () => {
    const sr = output.payloadCollections['support-requests']
    const textFields = sr.fields.filter((f: any) => f.type === 'text')
    // At minimum, requestId should be a text field
    expect(textFields.length).toBeGreaterThan(0)
    // Check total field count — should have properties from all readings
    expect(sr.fields.length).toBeGreaterThanOrEqual(3)
  })

  it('should map enum value types to select fields', () => {
    const sr = output.payloadCollections['support-requests']
    const selectFields = sr.fields.filter((f: any) => f.type === 'select')
    expect(selectFields.length).toBeGreaterThan(0)
    const hasEnumField = selectFields.some((f: any) =>
      f.options?.includes('urgent') || f.options?.includes('Slack')
    )
    expect(hasEnumField).toBe(true)
  })

  it('should map integer value types to number fields', () => {
    const fr = output.payloadCollections['feature-requests']
    const voteField = fr.fields.find((f: any) => f.type === 'number')
    expect(voteField).toBeDefined()
  })

  it('should map email format to email field type', () => {
    const customer = output.payloadCollections['customers']
    const emailField = customer.fields.find((f: any) => f.type === 'email')
    expect(emailField).toBeDefined()
  })

  it('should generate relationship fields for entity references', () => {
    const sr = output.payloadCollections['support-requests']
    const relFields = sr.fields.filter((f: any) => f.type === 'relationship')
    expect(relFields.length).toBeGreaterThan(0)
  })

  it('should include access control based on permissions', () => {
    const sr = output.payloadCollections['support-requests']
    expect(sr.access).toBeDefined()
    expect(sr.access.create).toBeDefined()
    expect(sr.access.read).toBeDefined()
    // SupportRequest has no delete permission
    expect(sr.access.delete).toBeUndefined()
  })

  it('should include auth config when login permission is set', () => {
    const customer = output.payloadCollections['customers']
    expect(customer.auth).toBe(true)
  })

  it('should set useAsTitle from reference scheme', () => {
    const sr = output.payloadCollections['support-requests']
    expect(sr.admin).toBeDefined()
    expect(sr.admin.useAsTitle).toBeDefined()
  })

  it('should still generate valid OpenAPI output alongside', () => {
    expect(output.openapi).toBe('3.1.0')
    expect(output.components?.schemas).toBeDefined()
    expect(output.paths).toBeDefined()
  })
}, 120_000)

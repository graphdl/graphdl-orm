import { describe, it, expect, beforeAll } from 'vitest'
import { initPayload } from '../helpers/initPayload'
import { seedSupportDomain } from '../helpers/seed'

let payload: any
let output: any

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
  }, 120_000)

  it('should not include intermediate payloadCollections in output', () => {
    expect(output.payloadCollections).toBeUndefined()
  })

  it('should generate .ts collection files in output.files', () => {
    expect(output.files).toBeDefined()
    expect(typeof output.files).toBe('object')
    expect(output.files['collections/support-requests.ts']).toBeDefined()
    expect(typeof output.files['collections/support-requests.ts']).toBe('string')
  })

  it('should generate valid TypeScript with proper imports and structure', () => {
    const tsContent = output.files['collections/support-requests.ts']
    expect(tsContent).toContain("import type { CollectionConfig } from 'payload'")
    expect(tsContent).toContain('export const SupportRequests: CollectionConfig')
    expect(tsContent).toContain("slug: 'support-requests'")
    expect(tsContent).toContain("type: 'text'")
    // Verify TypeScript syntax uses unquoted keys and single-quoted strings (not JSON double-quotes)
    expect(tsContent).not.toContain('"slug"')
    expect(tsContent).not.toContain('"type"')
    expect(tsContent).not.toContain('"name"')
  })

  it('should generate .ts files for all entity nouns with permissions', () => {
    expect(output.files['collections/support-requests.ts']).toBeDefined()
    expect(output.files['collections/feature-requests.ts']).toBeDefined()
    expect(output.files['collections/customers.ts']).toBeDefined()
    expect(output.files['collections/api-products.ts']).toBeDefined()
  })

  it('should include access control in generated TypeScript', () => {
    const tsContent = output.files['collections/support-requests.ts']
    expect(tsContent).toContain('access:')
    expect(tsContent).toContain('({ req: { user } }) => Boolean(user)')
  })

  it('should include auth config for login collections', () => {
    const tsContent = output.files['collections/customers.ts']
    expect(tsContent).toContain('auth: true')
  })

  it('should generate valid OpenAPI output alongside Payload collections', () => {
    expect(output.openapi).toBe('3.1.0')
    expect(output.components?.schemas).toBeDefined()
    expect(output.paths).toBeDefined()
  })
}, 120_000)

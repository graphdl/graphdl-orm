import { describe, it, expect, beforeAll } from 'vitest'
import { initPayload } from '../helpers/initPayload'
import { seedSupportDomain } from '../helpers/seed'

let payload: any
let openapiOutput: any
let payloadOutput: any

describe('Payload Collection Generator', () => {
  beforeAll(async () => {
    payload = await initPayload()

    // Drop database for clean slate
    await payload.db.connection.dropDatabase()

    await seedSupportDomain(payload)

    // Create OpenAPI generator (source)
    const openapiGenerator = await payload.create({
      collection: 'generators',
      data: {
        title: 'auto.dev Support API',
        version: '1.0.0',
        databaseEngine: 'Payload',
      },
    })
    openapiOutput = openapiGenerator.output

    // Create Payload generator (derives from OpenAPI)
    const payloadGenerator = await payload.create({
      collection: 'generators',
      data: {
        title: 'auto.dev Support Collections',
        version: '1.0.0',
        databaseEngine: 'Payload',
        outputFormat: 'payload',
      },
    })
    payloadOutput = payloadGenerator.output
  }, 120_000)

  it('should not include intermediate payloadCollections in output', () => {
    expect(payloadOutput.payloadCollections).toBeUndefined()
  })

  it('should generate .ts collection files in output.files', () => {
    expect(payloadOutput.files).toBeDefined()
    expect(typeof payloadOutput.files).toBe('object')
    expect(payloadOutput.files['collections/support-requests.ts']).toBeDefined()
    expect(typeof payloadOutput.files['collections/support-requests.ts']).toBe('string')
  })

  it('should generate valid TypeScript with proper imports and structure', () => {
    const tsContent = payloadOutput.files['collections/support-requests.ts']
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
    expect(payloadOutput.files['collections/support-requests.ts']).toBeDefined()
    expect(payloadOutput.files['collections/feature-requests.ts']).toBeDefined()
    expect(payloadOutput.files['collections/customers.ts']).toBeDefined()
    expect(payloadOutput.files['collections/api-products.ts']).toBeDefined()
  })

  it('should include access control in generated TypeScript', () => {
    const tsContent = payloadOutput.files['collections/support-requests.ts']
    expect(tsContent).toContain('access:')
    expect(tsContent).toContain('({ req: { user } }) => Boolean(user)')
  })

  it('should include auth config for login collections', () => {
    const tsContent = payloadOutput.files['collections/customers.ts']
    expect(tsContent).toContain('auth: true')
  })

  it('should generate valid OpenAPI output (backwards compat, no outputFormat)', () => {
    expect(openapiOutput.openapi).toBe('3.1.0')
    expect(openapiOutput.components?.schemas).toBeDefined()
    expect(openapiOutput.paths).toBeDefined()
  })

  it('should not include files in OpenAPI-only output', () => {
    expect(openapiOutput.files).toBeUndefined()
  })

  it('should auto-find source OpenAPI generator for payload output', () => {
    // payloadOutput was created without sourceGenerator
    // it should have auto-found the openapi generator
    expect(payloadOutput.files).toBeDefined()
    expect(payloadOutput.files['collections/support-requests.ts']).toBeDefined()
  })

  it('should not generate collections for value nouns', () => {
    const valueNounFiles = Object.keys(payloadOutput.files).filter(f =>
      ['descriptions.ts', 'subjects.ts', 'prioritys.ts', 'bodys.ts', 'names.ts'].some(v => f.endsWith(v))
    )
    expect(valueNounFiles).toEqual([])
  })

  it('should include value-type fields as inline properties on entity collections', () => {
    const tsContent = payloadOutput.files['collections/support-requests.ts']
    expect(tsContent).toContain("name: 'subject'")
    expect(tsContent).toContain("name: 'description'")
    expect(tsContent).toContain("name: 'priority'")
  })
}, 120_000)

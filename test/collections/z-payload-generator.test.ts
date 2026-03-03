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

    const seeded = await seedSupportDomain(payload)

    // Add a description to a noun so we can test field descriptions
    await payload.update({
      collection: 'nouns',
      id: seeded.nouns.priority.id,
      data: { description: 'The urgency level of a support request' },
    })

    // Add a Category entity with a 'name' text field to test useAsTitle heuristic
    const category = await payload.create({
      collection: 'nouns',
      data: { name: 'Category', plural: 'categories', objectType: 'entity', permissions: ['create', 'read', 'update'] },
    })
    const catName = await payload.create({
      collection: 'nouns',
      data: { name: 'Name', objectType: 'value', valueType: 'string' },
    })
    const catSlug = await payload.create({
      collection: 'nouns',
      data: { name: 'Slug', objectType: 'value', valueType: 'string' },
    })
    await payload.update({ collection: 'nouns', id: category.id, data: { referenceScheme: [catSlug.id] } })
    const categoryHasName = await payload.create({ collection: 'graph-schemas', data: { name: 'CategoryHasName' } })
    await payload.create({ collection: 'readings', data: { text: 'Category has Name', graphSchema: categoryHasName.id } })
    await payload.update({ collection: 'graph-schemas', id: categoryHasName.id, data: { roleRelationship: 'many-to-one' } })
    const categoryHasSlug = await payload.create({ collection: 'graph-schemas', data: { name: 'CategoryHasSlug' } })
    await payload.create({ collection: 'readings', data: { text: 'Category has Slug', graphSchema: categoryHasSlug.id } })
    await payload.update({ collection: 'graph-schemas', id: categoryHasSlug.id, data: { roleRelationship: 'one-to-one' } })

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

  it('should render many-to-many relationships as relationship fields with hasMany', () => {
    const tsContent = payloadOutput.files['collections/api-products.ts']
    expect(tsContent).toContain("type: 'relationship'")
    expect(tsContent).toContain('hasMany: true')
  })

  it('should use correct camelCase for acronym field names', () => {
    const tsContent = payloadOutput.files['collections/customers.ts']
    expect(tsContent).toContain("name: 'apiKey'")
    expect(tsContent).not.toContain("name: 'aPIKey'")
  })

  it('should generate join fields for the one-side of 1:* relationships', () => {
    const tsContent = payloadOutput.files['collections/customers.ts']
    // The "one" side should have a join field, not a relationship
    expect(tsContent).toContain("type: 'join'")
    expect(tsContent).toContain("collection: 'support-requests'")
  })

  it('should include noun descriptions as field admin descriptions', () => {
    const tsContent = payloadOutput.files['collections/support-requests.ts']
    expect(tsContent).toContain('The urgency level of a support request')
    expect(tsContent).toContain('description:')
  })

  it('should use name field as useAsTitle when available', () => {
    const tsContent = payloadOutput.files['collections/categories.ts']
    expect(tsContent).toBeDefined()
    // Category has both 'name' (text) and 'slug' (unique text).
    // The heuristic should prefer 'name' over the unique 'slug'.
    expect(tsContent).toContain("useAsTitle: 'name'")
  })

  it('should generate defaultColumns in admin config', () => {
    const tsContent = payloadOutput.files['collections/support-requests.ts']
    expect(tsContent).toContain('defaultColumns:')
    // defaultColumns should be an array of field names, capped at 5
    expect(tsContent).toMatch(/defaultColumns:\s*\[/)
  })
}, 120_000)

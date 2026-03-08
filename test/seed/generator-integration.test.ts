import { describe, it, expect, beforeAll } from 'vitest'
import { initPayload } from '../helpers/initPayload'
import { parseDomainMarkdown } from '../../src/seed/parser'
import { seedDomain } from '../../src/seed/handler'

let payload: any

const TEST_DOMAINS: Record<string, string> = {
  'customer-auth': `# Customer & Auth

## Entity Types

| Entity | Reference Scheme | Notes |
|--------|-----------------|-------|
| Customer | EmailAddress | |
| Account | Customer + OAuthProvider | |

## Value Types

| Value | Type | Constraints |
|-------|------|------------|
| EmailAddress | string | format: email |
| Name | string | |
| OAuthProvider | string | enum: github, google |

## Readings

| Reading | Multiplicity |
|---------|-------------|
| Customer has Name | \\*:1 |
| Customer authenticates via Account | 1:\\* |
`,
  'orders': `# Orders

## Entity Types

| Entity | Reference Scheme | Notes |
|--------|-----------------|-------|
| Order | OrderNumber | |
| LineItem | Order + Product | |
| Product | ProductName | |

## Value Types

| Value | Type | Constraints |
|-------|------|------------|
| OrderNumber | string | |
| ProductName | string | |
| Quantity | integer | minimum: 1 |
| Price | number | minimum: 0 |

## Readings

| Reading | Multiplicity |
|---------|-------------|
| Customer places Order | 1:\\* |
| Order contains LineItem | 1:\\* |
| LineItem is for Product | \\*:1 |
| LineItem has Quantity | \\*:1 |
| Product has Price | \\*:1 |
`,
}

describe('Generator with real seeded domains', () => {
  beforeAll(async () => {
    payload = await initPayload()
    await payload.db.connection.dropDatabase()

    for (const [domain, md] of Object.entries(TEST_DOMAINS)) {
      const parsed = parseDomainMarkdown(md)
      const result = await seedDomain(payload, parsed, domain)
      console.log(`  ${domain}: ${result.nouns}n ${result.readings}r ${result.errors.length}err`)
    }
  }, 300_000)

  it('should generate OpenAPI for all domains', async () => {
    const gen = await payload.create({
      collection: 'generators',
      data: { title: 'Full API', version: '1.0.0', databaseEngine: 'Payload' },
    })
    const schemas = gen.output?.components?.schemas || {}
    console.log('Total schemas:', Object.keys(schemas).length)
    expect(Object.keys(schemas).length).toBeGreaterThan(0)
  }, 120_000)
})

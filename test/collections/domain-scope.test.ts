import { describe, it, expect, beforeAll } from 'vitest'
import { initPayload } from '../helpers/initPayload'

let payload: any

describe('Domain scoping', () => {
  beforeAll(async () => {
    payload = await initPayload()
    await payload.db.connection.dropDatabase()
  }, 120_000)

  it('should create a noun with a domain field', async () => {
    const noun = await payload.create({
      collection: 'nouns',
      data: { name: 'TestEntity', objectType: 'entity', domain: 'domain-123' },
    })
    expect(noun.domain).toBe('domain-123')
  })

  it('should filter nouns by domain', async () => {
    await payload.create({
      collection: 'nouns',
      data: { name: 'DomainA_Noun', objectType: 'entity', domain: 'domain-a' },
    })
    await payload.create({
      collection: 'nouns',
      data: { name: 'DomainB_Noun', objectType: 'entity', domain: 'domain-b' },
    })
    const results = await payload.find({
      collection: 'nouns',
      where: { domain: { equals: 'domain-a' } },
    })
    expect(results.docs.every((d: any) => d.domain === 'domain-a')).toBe(true)
    expect(results.docs.length).toBeGreaterThan(0)
  })

  it('should store domain on readings', async () => {
    const gs = await payload.create({
      collection: 'graph-schemas',
      data: { name: 'TestSchema', domain: 'domain-x' },
    })
    expect(gs.domain).toBe('domain-x')
  })

  it('should store domain on generators', async () => {
    const gen = await payload.create({
      collection: 'generators',
      data: { title: 'Test Gen', version: '1.0.0', databaseEngine: 'Payload', domain: 'domain-y' },
    })
    expect(gen.domain).toBe('domain-y')
  })
})

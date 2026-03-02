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

  it('should auto-create roles when reading has domain', async () => {
    const noun1 = await payload.create({
      collection: 'nouns',
      data: { name: 'Gamma', objectType: 'entity', plural: 'gammas', domain: 'test-domain' },
    })
    const noun2 = await payload.create({
      collection: 'nouns',
      data: { name: 'Delta', objectType: 'value', valueType: 'string', domain: 'test-domain' },
    })
    const gs = await payload.create({
      collection: 'graph-schemas',
      data: { name: 'GammaHasDelta', domain: 'test-domain' },
    })
    await payload.create({
      collection: 'readings',
      data: { text: 'Gamma has Delta', graphSchema: gs.id, domain: 'test-domain' },
    })
    const roles = await payload.find({
      collection: 'roles',
      where: { graphSchema: { equals: gs.id } },
    })
    expect(roles.docs.length).toBe(2)
  })

  it('should create constraints when graph schema has domain', async () => {
    const entity = await payload.create({
      collection: 'nouns',
      data: { name: 'Epsilon', objectType: 'entity', plural: 'epsilons', domain: 'test-domain-2' },
    })
    const value = await payload.create({
      collection: 'nouns',
      data: { name: 'EpsilonLabel', objectType: 'value', valueType: 'string', domain: 'test-domain-2' },
    })
    const gs = await payload.create({
      collection: 'graph-schemas',
      data: { name: 'EpsilonHasEpsilonLabel', domain: 'test-domain-2' },
    })
    await payload.create({
      collection: 'readings',
      data: { text: 'Epsilon has EpsilonLabel', graphSchema: gs.id, domain: 'test-domain-2' },
    })
    await payload.update({
      collection: 'graph-schemas',
      id: gs.id,
      data: { roleRelationship: 'many-to-one' },
    })
    const roles = await payload.find({
      collection: 'roles',
      where: { graphSchema: { equals: gs.id } },
      sort: 'createdAt',
    })
    // Should have constraint spans created
    const constraintSpans = await payload.find({
      collection: 'constraint-spans',
      pagination: false,
      depth: 4,
    })
    const relevant = constraintSpans.docs.filter((cs: any) =>
      (cs.roles as any[])?.some((r: any) => {
        const nId = typeof r.noun?.value === 'string' ? r.noun.value : r.noun?.value?.id
        return nId === entity.id
      })
    )
    expect(relevant.length).toBeGreaterThan(0)
  })
})

import { describe, it, expect, beforeAll } from 'vitest'
import { initPayload } from '../helpers/initPayload'

let payload: any

async function createDomain(slug: string) {
  const existing = await payload.find({
    collection: 'domains',
    where: { domainSlug: { equals: slug } },
    limit: 1,
  })
  if (existing.docs.length) return existing.docs[0]
  return payload.create({
    collection: 'domains',
    data: { domainSlug: slug, name: slug },
  })
}

describe('Domain scoping', () => {
  beforeAll(async () => {
    payload = await initPayload()
    await payload.db.connection.dropDatabase()
  }, 120_000)

  it('should create a noun with a domain relationship', async () => {
    const domain = await createDomain('domain-123')
    const noun = await payload.create({
      collection: 'nouns',
      data: { name: 'TestEntity', objectType: 'entity', domain: domain.id },
    })
    const domainRef = typeof noun.domain === 'object' ? noun.domain.id : noun.domain
    expect(domainRef).toBe(domain.id)
  })

  it('should filter nouns by domain', async () => {
    const domainA = await createDomain('domain-a')
    const domainB = await createDomain('domain-b')
    await payload.create({
      collection: 'nouns',
      data: { name: 'DomainA_Noun', objectType: 'entity', domain: domainA.id },
    })
    await payload.create({
      collection: 'nouns',
      data: { name: 'DomainB_Noun', objectType: 'entity', domain: domainB.id },
    })
    const results = await payload.find({
      collection: 'nouns',
      where: { domain: { equals: domainA.id } },
    })
    expect(results.docs.every((d: any) => {
      const ref = typeof d.domain === 'object' ? d.domain.id : d.domain
      return ref === domainA.id
    })).toBe(true)
    expect(results.docs.length).toBeGreaterThan(0)
  })

  it('should store domain on graph schemas', async () => {
    const domain = await createDomain('domain-x')
    const gs = await payload.create({
      collection: 'graph-schemas',
      data: { name: 'TestSchema', domain: domain.id },
    })
    const domainRef = typeof gs.domain === 'object' ? gs.domain.id : gs.domain
    expect(domainRef).toBe(domain.id)
  })

  it('should store domain on generators', async () => {
    const domain = await createDomain('domain-y')
    const gen = await payload.create({
      collection: 'generators',
      data: { title: 'Test Gen', version: '1.0.0', databaseEngine: 'Payload', domain: domain.id },
    })
    const domainRef = typeof gen.domain === 'object' ? gen.domain.id : gen.domain
    expect(domainRef).toBe(domain.id)
  })

  it('should auto-create roles when reading has domain', async () => {
    const domain = await createDomain('test-domain')
    const noun1 = await payload.create({
      collection: 'nouns',
      data: { name: 'Gamma', objectType: 'entity', plural: 'gammas', domain: domain.id },
    })
    const noun2 = await payload.create({
      collection: 'nouns',
      data: { name: 'Delta', objectType: 'value', valueType: 'string', domain: domain.id },
    })
    const gs = await payload.create({
      collection: 'graph-schemas',
      data: { name: 'GammaHasDelta', domain: domain.id },
    })
    await payload.create({
      collection: 'readings',
      data: { text: 'Gamma has Delta', graphSchema: gs.id, domain: domain.id },
    })
    const roles = await payload.find({
      collection: 'roles',
      where: { graphSchema: { equals: gs.id } },
    })
    expect(roles.docs.length).toBe(2)
  })

  it('should create constraints when graph schema has domain', async () => {
    const domain = await createDomain('test-domain-2')
    const entity = await payload.create({
      collection: 'nouns',
      data: { name: 'Epsilon', objectType: 'entity', plural: 'epsilons', domain: domain.id },
    })
    const value = await payload.create({
      collection: 'nouns',
      data: { name: 'EpsilonLabel', objectType: 'value', valueType: 'string', domain: domain.id },
    })
    const gs = await payload.create({
      collection: 'graph-schemas',
      data: { name: 'EpsilonHasEpsilonLabel', domain: domain.id },
    })
    await payload.create({
      collection: 'readings',
      data: { text: 'Epsilon has EpsilonLabel', graphSchema: gs.id, domain: domain.id },
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

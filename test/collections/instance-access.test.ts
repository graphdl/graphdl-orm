import { describe, it, expect, beforeAll } from 'vitest'
import { initPayload } from '../helpers/initPayload'
import { instanceReadAccess, instanceWriteAccess, domainReadAccess, domainWriteAccess } from '../../src/collections/shared/instanceAccess'

let payload: any

describe('Domain-level user scoping', () => {
  beforeAll(async () => {
    payload = await initPayload()
    await payload.db.connection.dropDatabase()
  }, 120_000)

  it('should store visibility on a domain', async () => {
    const domain = await payload.create({
      collection: 'domains',
      data: { domainSlug: 'vis-test', name: 'Vis Test', visibility: 'public' },
    })
    expect(domain.visibility).toBe('public')
  })

  it('should default visibility to private', async () => {
    const domain = await payload.create({
      collection: 'domains',
      data: { domainSlug: 'vis-default', name: 'Vis Default' },
    })
    expect(domain.visibility).toBe('private')
  })

  it('should store domain on resources', async () => {
    const domain = await payload.create({
      collection: 'domains',
      data: { domainSlug: 'res-domain', name: 'Res Domain' },
    })
    const noun = await payload.create({
      collection: 'nouns',
      data: { name: 'TestResNoun', objectType: 'entity', domain: domain.id },
    })
    const resource = await payload.create({
      collection: 'resources',
      data: { type: noun.id, value: 'test-value', domain: domain.id },
    })
    const domainRef = typeof resource.domain === 'object' ? resource.domain.id : resource.domain
    expect(domainRef).toBe(domain.id)
  })

  it('should store domain on state-machines', async () => {
    const domain = await payload.create({
      collection: 'domains',
      data: { domainSlug: 'sm-domain', name: 'SM Domain' },
    })
    const noun = await payload.create({
      collection: 'nouns',
      data: { name: 'SmTestNoun', objectType: 'entity', domain: domain.id },
    })
    const def = await payload.create({
      collection: 'state-machine-definitions',
      data: { noun: { relationTo: 'nouns', value: noun.id }, domain: domain.id },
    })
    const status = await payload.create({
      collection: 'statuses',
      data: { name: 'Initial', stateMachineDefinition: def.id },
    })
    const sm = await payload.create({
      collection: 'state-machines',
      data: {
        name: 'test-sm',
        stateMachineType: def.id,
        stateMachineStatus: status.id,
        domain: domain.id,
      },
    })
    const domainRef = typeof sm.domain === 'object' ? sm.domain.id : sm.domain
    expect(domainRef).toBe(domain.id)
  })

  it('should store domain on events', async () => {
    const domain = await payload.create({
      collection: 'domains',
      data: { domainSlug: 'ev-domain', name: 'Ev Domain' },
    })
    const et = await payload.create({
      collection: 'event-types',
      data: { name: 'testEvent' },
    })
    const event = await payload.create({
      collection: 'events',
      data: { type: et.id, timestamp: new Date().toISOString(), domain: domain.id },
    })
    const domainRef = typeof event.domain === 'object' ? event.domain.id : event.domain
    expect(domainRef).toBe(domain.id)
  })

  it('instanceReadAccess returns false when no user', () => {
    const result = instanceReadAccess({ req: {} } as any)
    expect(result).toBe(false)
  })

  it('instanceReadAccess returns where clause for authenticated user', () => {
    const result = instanceReadAccess({ req: { user: { email: 'test@example.com' } } } as any)
    expect(result).toHaveProperty('or')
    expect((result as any).or).toHaveLength(2)
    expect((result as any).or[0]).toEqual({ 'domain.tenant': { equals: 'test@example.com' } })
    expect((result as any).or[1]).toEqual({ 'domain.visibility': { equals: 'public' } })
  })

  it('instanceWriteAccess returns false when no user', () => {
    const result = instanceWriteAccess({ req: {} } as any)
    expect(result).toBe(false)
  })

  it('instanceWriteAccess returns where clause filtering by tenant only', () => {
    const result = instanceWriteAccess({ req: { user: { email: 'test@example.com' } } } as any)
    expect(result).toEqual({ 'domain.tenant': { equals: 'test@example.com' } })
  })

  it('domainReadAccess filters by tenant or public visibility', () => {
    const result = domainReadAccess({ req: { user: { email: 'alice@example.com' } } } as any)
    expect(result).toEqual({
      or: [
        { tenant: { equals: 'alice@example.com' } },
        { visibility: { equals: 'public' } },
      ],
    })
  })

  it('domainWriteAccess filters by tenant only', () => {
    const result = domainWriteAccess({ req: { user: { email: 'alice@example.com' } } } as any)
    expect(result).toEqual({ tenant: { equals: 'alice@example.com' } })
  })

  it('access functions produce different filters for different users', () => {
    const alice = instanceReadAccess({ req: { user: { email: 'alice@example.com' } } } as any)
    const bob = instanceReadAccess({ req: { user: { email: 'bob@example.com' } } } as any)

    // Different users get different where clauses
    expect((alice as any).or[0]['domain.tenant'].equals).toBe('alice@example.com')
    expect((bob as any).or[0]['domain.tenant'].equals).toBe('bob@example.com')

    // But both include public domain access
    expect((alice as any).or[1]).toEqual({ 'domain.visibility': { equals: 'public' } })
    expect((bob as any).or[1]).toEqual({ 'domain.visibility': { equals: 'public' } })
  })
})

import { describe, it, expect, beforeAll } from 'vitest'
import { seedReadingsFromText } from './unified'

let payload: any

describe('seedReadingsFromText', () => {
  beforeAll(async () => {
    const { initPayload } = await import('../../test/helpers/initPayload')
    payload = await initPayload()
    // Drop database for clean state
    await payload.db.connection.dropDatabase()
  }, 120_000)

  it('creates nouns and readings from plain text', async () => {
    // Seed a domain first
    const domain = await payload.create({
      collection: 'domains',
      data: { domainSlug: 'test-unified', name: 'Test Unified' },
    })

    const result = await seedReadingsFromText(payload, {
      text: 'Customer has Name\nCustomer submits SupportRequest',
      domainId: domain.id,
    })

    expect(result.errors).toHaveLength(0)
    expect(result.nounsCreated).toBeGreaterThanOrEqual(3)
    expect(result.readingsCreated).toBe(2)

    // Verify nouns exist
    const nouns = await payload.find({
      collection: 'nouns',
      where: { name: { in: ['Customer', 'Name', 'SupportRequest'] } },
    })
    expect(nouns.docs).toHaveLength(3)

    // Verify readings were created
    const readings = await payload.find({
      collection: 'readings',
      pagination: false,
    })
    expect(readings.docs.length).toBe(2)

    // Verify roles were auto-created by the afterChange hook
    const roles = await payload.find({
      collection: 'roles',
      pagination: false,
    })
    // 2 readings x 2 nouns each = 4 roles
    expect(roles.docs.length).toBe(4)
  })

  it('handles subtype declarations', async () => {
    const domain = await payload.create({
      collection: 'domains',
      data: { domainSlug: 'test-subtype', name: 'Test Subtype' },
    })

    // Pre-create the nouns so the parser can find them
    await payload.create({ collection: 'nouns', data: { name: 'Admin', objectType: 'entity', domain: domain.id } })
    await payload.create({ collection: 'nouns', data: { name: 'User', objectType: 'entity', domain: domain.id } })

    const result = await seedReadingsFromText(payload, {
      text: 'Admin is a subtype of User',
      domainId: domain.id,
    })

    expect(result.errors).toHaveLength(0)
    expect(result.readingsCreated).toBe(0) // subtypes update nouns, not create readings

    // Verify superType was set
    const admin = await payload.find({
      collection: 'nouns',
      where: { name: { equals: 'Admin' } },
      limit: 1,
      depth: 1,
    })
    const superType = admin.docs[0].superType
    const superTypeName = typeof superType === 'string' ? null : superType?.name
    expect(superTypeName).toBe('User')
  })

  it('creates constraints from verbal notation', async () => {
    const domain = await payload.create({
      collection: 'domains',
      data: { domainSlug: 'test-constraints', name: 'Test Constraints' },
    })

    // Pre-create nouns
    await payload.create({ collection: 'nouns', data: { name: 'Order', objectType: 'entity', domain: domain.id } })
    await payload.create({ collection: 'nouns', data: { name: 'Status', objectType: 'value', domain: domain.id } })

    const result = await seedReadingsFromText(payload, {
      text: 'Each Order has exactly one Status',
      domainId: domain.id,
    })

    expect(result.errors).toHaveLength(0)
    expect(result.readingsCreated).toBe(1)
    // "exactly one" produces both UC and MC constraints
    expect(result.constraintsCreated).toBeGreaterThanOrEqual(1)
  })

  it('is idempotent for nouns', async () => {
    const domain = await payload.create({
      collection: 'domains',
      data: { domainSlug: 'test-idempotent', name: 'Test Idempotent' },
    })

    // Pre-create a noun
    await payload.create({ collection: 'nouns', data: { name: 'Widget', objectType: 'entity', domain: domain.id } })

    const result = await seedReadingsFromText(payload, {
      text: 'Widget has Color',
      domainId: domain.id,
    })

    expect(result.errors).toHaveLength(0)
    // Widget already exists, so only Color is new
    expect(result.nounsCreated).toBe(1)

    // Verify no duplicate Widget nouns
    const widgets = await payload.find({
      collection: 'nouns',
      where: { name: { equals: 'Widget' } },
    })
    expect(widgets.docs).toHaveLength(1)
  })
})

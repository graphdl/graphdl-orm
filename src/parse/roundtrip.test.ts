import { describe, it, expect, beforeAll } from 'vitest'

let payload: any

describe('readings round-trip', () => {
  beforeAll(async () => {
    const { initPayload } = await import('../../test/helpers/initPayload')
    payload = await initPayload()
    await payload.db.connection.dropDatabase()
  }, 120_000)

  it('seed text → extract readings → verify DB state', async () => {
    const { seedReadingsFromText } = await import('../seed/unified')

    // Create domain
    const domain = await payload.create({
      collection: 'domains',
      data: { domainSlug: 'roundtrip-test', name: 'Round Trip Test' },
    })

    // Seed from text
    const inputText = `Customer has Name
Customer submits SupportRequest
SupportRequest has Priority
Admin is a subtype of Customer`

    const seedResult = await seedReadingsFromText(payload, {
      text: inputText,
      domainId: domain.id,
    })
    expect(seedResult.errors).toHaveLength(0)

    // Count what was created
    const nouns = await payload.find({
      collection: 'nouns',
      where: { domain: { equals: domain.id } },
      pagination: false,
    })
    const readings = await payload.find({
      collection: 'readings',
      where: { domain: { equals: domain.id } },
      pagination: false,
    })

    // Verify basic counts
    expect(nouns.docs.length).toBeGreaterThanOrEqual(4) // Customer, Name, SupportRequest, Priority, Admin
    expect(readings.docs.length).toBeGreaterThanOrEqual(3) // 3 readings (subtype doesn't create a reading)

    // Verify subtype was set
    const admin = nouns.docs.find((n: any) => n.name === 'Admin')
    expect(admin?.superType).toBeTruthy()

    // Verify all expected nouns exist
    const nounNames = nouns.docs.map((n: any) => n.name)
    expect(nounNames).toContain('Customer')
    expect(nounNames).toContain('Name')
    expect(nounNames).toContain('SupportRequest')
    expect(nounNames).toContain('Priority')
    expect(nounNames).toContain('Admin')
  })
})

import { describe, it, expect, beforeAll } from 'vitest'
import { initPayload } from '../helpers/initPayload'
import { parseDomainMarkdown, parseStateMachineMarkdown, parseFORML2 } from '../../src/seed/parser'
import { seedDomain, seedReadings, seedStateMachine, type SeedResult } from '../../src/seed/handler'

let payload: any

const SUPPORT_DOMAIN = `# Support

## Entity Types

| Entity | Reference Scheme | Notes |
|--------|-----------------|-------|
| SupportRequest | RequestId | Inbound support thread |

## Value Types

| Value | Type | Constraints |
|-------|------|------------|
| RequestId | string | format: uuid |
| Subject | string | |
| Priority | string | enum: low, medium, high, urgent |

## Readings

| Reading | Multiplicity |
|---------|-------------|
| SupportRequest has Subject | \\*:1 |
| SupportRequest has Priority | \\*:1 |
`

const SUPPORT_SM = `# Support Request Lifecycle

## States

Received, Investigating, Resolved

## Transitions

| From | To | Event |
|------|-----|-------|
| Received | Investigating | assign |
| Investigating | Resolved | resolve |
`

describe('Seed handler', () => {
  beforeAll(async () => {
    payload = await initPayload()
    await payload.db.connection.dropDatabase()
  }, 120_000)

  it('should seed a domain from parsed markdown', async () => {
    const parsed = parseDomainMarkdown(SUPPORT_DOMAIN)
    const result = await seedDomain(payload, parsed, 'support')

    expect(result.nouns).toBeGreaterThanOrEqual(4) // 1 entity + 3 values
    expect(result.readings).toBe(2)
    expect(result.errors).toHaveLength(0)

    // Verify nouns exist with domain
    const nouns = await payload.find({ collection: 'nouns', where: { domain: { equals: 'support' } }, pagination: false })
    expect(nouns.docs.length).toBeGreaterThanOrEqual(4)

    // Verify readings exist
    const readings = await payload.find({ collection: 'readings', pagination: false })
    expect(readings.docs.length).toBe(2)
  })

  it('should be idempotent on re-seed', async () => {
    const parsed = parseDomainMarkdown(SUPPORT_DOMAIN)
    const result = await seedDomain(payload, parsed, 'support')

    // Readings should be skipped (already exist)
    expect(result.skipped).toBe(2)
    expect(result.readings).toBe(0)
  })

  it('should seed a state machine', async () => {
    const parsed = parseStateMachineMarkdown(SUPPORT_SM)
    const result = await seedStateMachine(payload, 'SupportRequest', parsed, 'support')

    expect(result.stateMachines).toBe(1)
    expect(result.errors).toHaveLength(0)
  })

  it('should seed FORML2 plain text readings', async () => {
    const readings = parseFORML2('SupportRequest has Description *:1')
    const result: SeedResult = { nouns: 0, readings: 0, stateMachines: 0, skipped: 0, errors: [] }
    await seedReadings(payload, readings, { domain: 'support' }, result)

    expect(result.readings).toBe(1)
    expect(result.errors).toHaveLength(0)
  })
})

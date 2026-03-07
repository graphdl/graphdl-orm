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
    const domainDoc = await payload.find({ collection: 'domains', where: { domainSlug: { equals: 'support' } }, limit: 1 })
    const domainId = domainDoc.docs[0].id
    const nouns = await payload.find({ collection: 'nouns', where: { domain: { equals: domainId } }, pagination: false })
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

  it('should be idempotent on state machine re-seed', async () => {
    const parsed = parseStateMachineMarkdown(SUPPORT_SM)
    const result = await seedStateMachine(payload, 'SupportRequest', parsed, 'support')

    expect(result.stateMachines).toBe(1)
    expect(result.errors).toHaveLength(0)

    // Verify no duplicate statuses were created
    const allStatuses = await payload.find({
      collection: 'statuses',
      pagination: false,
    })
    // Original test created 3 (Received, Investigating, Resolved)
    // Re-seed should NOT add more — still exactly 3
    const statusNames = allStatuses.docs.map((s: any) => s.name)
    const supportStatuses = statusNames.filter((n: string) => ['Received', 'Investigating', 'Resolved'].includes(n))
    expect(supportStatuses.length).toBe(3) // not 6
  })

  it('should seed FORML2 plain text readings', async () => {
    const readings = parseFORML2('SupportRequest has Description *:1')
    const result: SeedResult = { nouns: 0, readings: 0, stateMachines: 0, skipped: 0, errors: [] }
    const domainDoc = await payload.find({ collection: 'domains', where: { domainSlug: { equals: 'support' } }, limit: 1 })
    await seedReadings(payload, readings, { domain: domainDoc.docs[0].id }, result)

    expect(result.readings).toBe(1)
    expect(result.errors).toHaveLength(0)
  })

  it('should wire verbs and functions to transitions from instance-fact readings', async () => {
    // Seed a billing domain with verb/function instance facts
    const parsed = parseDomainMarkdown(BILLING_DOMAIN)
    const domainResult = await seedDomain(payload, parsed, 'billing-test')
    expect(domainResult.errors).toHaveLength(0)

    // Instance facts should be seeded as readings
    const runsReading = await payload.find({
      collection: 'readings',
      where: { text: { equals: 'subscribe runs SubscribeCustomer' } },
      limit: 1,
    })
    expect(runsReading.docs.length).toBe(1)

    // Seed the state machine — should trigger verb/function wiring
    const smParsed = parseStateMachineMarkdown(BILLING_SM)
    const smResult = await seedStateMachine(payload, 'Subscription', smParsed, 'billing-test')
    expect(smResult.stateMachines).toBe(1)
    expect(smResult.errors).toHaveLength(0)

    // Verify Function was created with correct properties
    const functions = await payload.find({
      collection: 'functions',
      where: { name: { equals: 'SubscribeCustomer' } },
      limit: 1,
    })
    expect(functions.docs.length).toBe(1)
    const fn = functions.docs[0] as any
    expect(fn.functionType).toBe('httpCallback')
    expect(fn.callbackUrl).toBe('https://auth.vin/api/internal/billing/change-plan')
    expect(fn.httpMethod).toBe('POST')

    // Verify Verb was created linking to the Function
    const verbs = await payload.find({
      collection: 'verbs',
      where: { name: { equals: 'subscribe' } },
      limit: 1,
    })
    expect(verbs.docs.length).toBe(1)
    const verb = verbs.docs[0] as any
    const verbFunctionId = typeof verb.function === 'string' ? verb.function : verb.function?.id
    expect(verbFunctionId).toBe(fn.id)

    // Verify Transition has verb set
    const transitions = await payload.find({
      collection: 'transitions',
      where: { verb: { equals: verb.id } },
      pagination: false,
    })
    expect(transitions.docs.length).toBeGreaterThanOrEqual(1)

    // Events without instance facts should NOT have verbs (graceful skip)
    const cancelVerb = await payload.find({
      collection: 'verbs',
      where: { name: { equals: 'cancel' } },
      limit: 1,
    })
    expect(cancelVerb.docs.length).toBe(0)
  })
})

const BILLING_DOMAIN = `# Billing Test

## Entity Types

| Entity | Reference Scheme | Notes |
|--------|-----------------|-------|
| Subscription | SubscriptionId | Stripe subscription |
| Verb | VerbName | Event action |
| Function | FunctionName | Executable behavior |

## Value Types

| Value | Type | Constraints |
|-------|------|------------|
| SubscriptionId | string | pattern: sub_.+ |
| VerbName | string | |
| FunctionName | string | |
| CallbackUrl | string | format: uri |
| HttpMethod | string | enum: GET, POST, PUT, PATCH, DELETE |
| FunctionType | string | enum: httpCallback, query, agentInvocation, transform |

## Readings

| Reading | Multiplicity |
|---------|-------------|
| Verb runs Function | \\*:1 |
| Function has FunctionType | \\*:1 |
| Function has CallbackUrl | \\*:1 |
| Function has HttpMethod | \\*:1 |

## Instance Facts

| Fact |
|------|
| subscribe runs SubscribeCustomer |
| SubscribeCustomer has FunctionType httpCallback |
| SubscribeCustomer has CallbackUrl https://auth.vin/api/internal/billing/change-plan |
| SubscribeCustomer has HttpMethod POST |
`

const BILLING_SM = `# Subscription Lifecycle

## States

Free, Active, Cancelled

## Transitions

| From | To | Event |
|------|-----|-------|
| Free | Active | subscribe |
| Active | Cancelled | cancel |
`

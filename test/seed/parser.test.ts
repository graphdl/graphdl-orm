import { describe, it, expect } from 'vitest'
import { parseDomainMarkdown, parseStateMachineMarkdown, parseFORML2 } from '../../src/seed/parser'

const SUPPORT_DOMAIN = `# Support

## Entity Types

| Entity | Reference Scheme | Notes |
|--------|-----------------|-------|
| SupportRequest | RequestId | Inbound support thread |
| Message | MessageId | Individual message in a support thread |

## Value Types

| Value | Type | Constraints |
|-------|------|------------|
| RequestId | string | format: uuid |
| MessageId | string | format: uuid |
| Subject | string | |
| Priority | string | enum: low, medium, high, urgent |

## Readings

| Reading | Multiplicity |
|---------|-------------|
| Customer submits SupportRequest | 1:\\* |
| SupportRequest has Subject | \\*:1 |
| SupportRequest has Priority | \\*:1 |
| SupportRequest concerns APIProduct | \\*:\\* |

## Instance Facts

| Fact |
|------|
| SupportRequest is handled via Channel 'Email' |

## Deontic Constraints

| Constraint |
|-----------|
| Support response must not reference internal team structure |
`

const SUPPORT_SM = `# Support Request Lifecycle

## States

Received, Triaging, Investigating, WaitingOnCustomer, Resolved, Closed

## Transitions

| From | To | Event |
|------|-----|-------|
| Received | Triaging | acknowledge |
| Triaging | Investigating | assign |
| Investigating | Resolved | resolve |
`

describe('parseDomainMarkdown', () => {
  const result = parseDomainMarkdown(SUPPORT_DOMAIN)

  it('should parse entity types', () => {
    expect(result.entityTypes).toHaveLength(2)
    expect(result.entityTypes[0]).toEqual({
      name: 'SupportRequest',
      referenceScheme: ['RequestId'],
      notes: 'Inbound support thread',
    })
  })

  it('should parse value types with constraints', () => {
    expect(result.valueTypes).toHaveLength(4)
    expect(result.valueTypes[0]).toEqual({ name: 'RequestId', valueType: 'string', format: 'uuid' })
    expect(result.valueTypes[3]).toEqual({ name: 'Priority', valueType: 'string', enum: 'low, medium, high, urgent' })
  })

  it('should parse readings with escaped multiplicities', () => {
    expect(result.readings).toHaveLength(4)
    expect(result.readings[0]).toEqual({ text: 'Customer submits SupportRequest', multiplicity: '1:*' })
    expect(result.readings[1]).toEqual({ text: 'SupportRequest has Subject', multiplicity: '*:1' })
    expect(result.readings[3]).toEqual({ text: 'SupportRequest concerns APIProduct', multiplicity: '*:*' })
  })

  it('should parse instance facts', () => {
    expect(result.instanceFacts).toEqual(["SupportRequest is handled via Channel 'Email'"])
  })

  it('should parse deontic constraints', () => {
    expect(result.deonticConstraints).toEqual(['Support response must not reference internal team structure'])
  })

  it('should parse UC(Role1,Role2) ternary notation', () => {
    const ternaryMd = `# Test
## Readings
| Reading | Multiplicity |
|---------|-------------|
| Listing has Price via ListingChannel | UC(Listing,ListingChannel) |
`
    const r = parseDomainMarkdown(ternaryMd)
    expect(r.readings).toHaveLength(1)
    expect(r.readings[0]).toEqual({
      text: 'Listing has Price via ListingChannel',
      multiplicity: 'ternary',
      ucRoles: ['Listing', 'ListingChannel'],
    })
  })
})

describe('parseStateMachineMarkdown', () => {
  const result = parseStateMachineMarkdown(SUPPORT_SM)

  it('should parse states', () => {
    expect(result.states).toEqual(['Received', 'Triaging', 'Investigating', 'WaitingOnCustomer', 'Resolved', 'Closed'])
  })

  it('should parse transitions', () => {
    expect(result.transitions).toHaveLength(3)
    expect(result.transitions[0]).toEqual({ from: 'Received', to: 'Triaging', event: 'acknowledge' })
  })
})

describe('parseFORML2', () => {
  it('should parse readings with space-separated multiplicity', () => {
    const result = parseFORML2('Customer has Name *:1\nCustomer has APIKey 1:1')
    expect(result).toEqual([
      { text: 'Customer has Name', multiplicity: '*:1' },
      { text: 'Customer has APIKey', multiplicity: '1:1' },
    ])
  })

  it('should parse readings with pipe-separated multiplicity', () => {
    const result = parseFORML2('Customer has Name | *:1')
    expect(result).toEqual([{ text: 'Customer has Name', multiplicity: '*:1' }])
  })

  it('should default to *:1 when no multiplicity given', () => {
    const result = parseFORML2('Customer has Name')
    expect(result).toEqual([{ text: 'Customer has Name', multiplicity: '*:1' }])
  })

  it('should parse UC(Role1,Role2) notation', () => {
    const result = parseFORML2('Listing has Price via Channel UC(Listing,Channel)')
    expect(result).toEqual([{
      text: 'Listing has Price via Channel',
      multiplicity: 'ternary',
      ucRoles: ['Listing', 'Channel'],
    }])
  })

  it('should parse UC notation with pipe separator', () => {
    const result = parseFORML2('Listing has Price via Channel | UC(Listing,Channel)')
    expect(result).toEqual([{
      text: 'Listing has Price via Channel',
      multiplicity: 'ternary',
      ucRoles: ['Listing', 'Channel'],
    }])
  })

  it('should skip comments and blank lines', () => {
    const result = parseFORML2('# comment\n\n// another\nCustomer has Name *:1')
    expect(result).toHaveLength(1)
  })
})

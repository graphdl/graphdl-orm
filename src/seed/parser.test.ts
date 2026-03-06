import { describe, it, expect } from 'vitest'
import { parseDomainMarkdown } from './parser'

const domainMarkdown = `# Support Domain

## Entity Types

| Entity | Reference Scheme | Notes |
|--------|-----------------|-------|
| SupportRequest | RequestId | |
| SupportResponse | ResponseId | |

## Value Types

| Value | Type |
|-------|------|
| RequestId | string |
| ResponseId | string |

## Readings

| Reading | Multiplicity |
|---------|-------------|
| SupportRequest has RequestId | *:1 |

## Instance Facts

| Fact |
|------|
| SupportRequest has RequestId "REQ-001" |

## Deontic Constraints

| Constraint |
|-----------|
| SupportResponse must not contain ProhibitedPunctuation |
| SupportResponse must not name ListingSource |

## Deontic Mandatory Constraint Instance Facts

| Constraint | Instance |
|-----------|----------|
| SupportResponse must not contain ProhibitedPunctuation | — |
| SupportResponse must not name ListingSource | Edmunds |
| SupportResponse must not name ListingSource | AutoTrader |
`

describe('parseDomainMarkdown', () => {
  it('parses deontic constraint instance facts from a two-column table', () => {
    const result = parseDomainMarkdown(domainMarkdown)
    expect(result.deonticConstraintInstances).toBeDefined()
    expect(result.deonticConstraintInstances).toHaveLength(3)
  })

  it('extracts constraint text and instance values correctly', () => {
    const result = parseDomainMarkdown(domainMarkdown)
    expect(result.deonticConstraintInstances[0]).toEqual({
      constraint: 'SupportResponse must not contain ProhibitedPunctuation',
      instance: '—',
    })
    expect(result.deonticConstraintInstances[1]).toEqual({
      constraint: 'SupportResponse must not name ListingSource',
      instance: 'Edmunds',
    })
    expect(result.deonticConstraintInstances[2]).toEqual({
      constraint: 'SupportResponse must not name ListingSource',
      instance: 'AutoTrader',
    })
  })

  it('still parses existing deonticConstraints field correctly', () => {
    const result = parseDomainMarkdown(domainMarkdown)
    expect(result.deonticConstraints).toHaveLength(2)
    expect(result.deonticConstraints[0]).toBe('SupportResponse must not contain ProhibitedPunctuation')
    expect(result.deonticConstraints[1]).toBe('SupportResponse must not name ListingSource')
  })

  it('returns empty array when no deontic constraint instance facts section exists', () => {
    const minimal = `# Minimal Domain

## Entity Types

| Entity | Reference Scheme |
|--------|-----------------|
| Foo | FooId |
`
    const result = parseDomainMarkdown(minimal)
    expect(result.deonticConstraintInstances).toEqual([])
  })
})

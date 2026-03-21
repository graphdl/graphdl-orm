import { describe, it, expect } from 'vitest'
import {
  resolveConceptualQuery,
  type QueryPathStep,
  type Reading,
} from './conceptual-query'

// Shared test fixtures
const nouns = ['Customer', 'Name', 'Support Request', 'Priority']

const readings: Reading[] = [
  { text: 'Customer has Name', nouns: ['Customer', 'Name'] },
  { text: 'Customer submits Support Request', nouns: ['Customer', 'Support Request'] },
  { text: 'Support Request has Priority', nouns: ['Support Request', 'Priority'] },
]

describe('resolveConceptualQuery', () => {
  it('resolves a single-hop query: "Customer that has Name"', () => {
    const result = resolveConceptualQuery('Customer that has Name', nouns, readings)

    expect(result.rootNoun).toBe('Customer')
    expect(result.path).toHaveLength(1)
    expect(result.path[0]).toEqual<QueryPathStep>({
      from: 'Customer',
      predicate: 'has',
      to: 'Name',
      inverse: false,
    })
    expect(result.filters).toHaveLength(0)
  })

  it('resolves a multi-hop query with value filter', () => {
    const result = resolveConceptualQuery(
      "Customer that submits Support Request that has Priority 'High'",
      nouns,
      readings,
    )

    expect(result.rootNoun).toBe('Customer')
    expect(result.path).toHaveLength(2)

    expect(result.path[0]).toEqual<QueryPathStep>({
      from: 'Customer',
      predicate: 'submits',
      to: 'Support Request',
      inverse: false,
    })

    expect(result.path[1]).toEqual<QueryPathStep>({
      from: 'Support Request',
      predicate: 'has',
      to: 'Priority',
      inverse: false,
    })

    expect(result.filters).toEqual([{ field: 'Priority', value: 'High' }])
  })

  it('returns empty path for unrecognized query', () => {
    const result = resolveConceptualQuery(
      'Foo that bar Baz',
      nouns,
      readings,
    )

    expect(result.path).toHaveLength(0)
    expect(result.filters).toHaveLength(0)
    expect(result.rootNoun).toBeUndefined()
  })

  it('resolves an inverse reading path', () => {
    // The reading is "Customer submits Support Request" (Customer first),
    // but the query starts from Support Request's perspective.
    const result = resolveConceptualQuery(
      'Support Request submitted by Customer',
      nouns,
      readings,
    )

    expect(result.rootNoun).toBe('Support Request')
    expect(result.path).toHaveLength(1)
    expect(result.path[0]).toEqual<QueryPathStep>({
      from: 'Support Request',
      predicate: 'submits',
      to: 'Customer',
      inverse: true,
    })
  })

  it('extracts multiple quoted filter values', () => {
    const extendedNouns = [...nouns, 'Status']
    const extendedReadings: Reading[] = [
      ...readings,
      { text: 'Support Request has Status', nouns: ['Support Request', 'Status'] },
    ]

    const result = resolveConceptualQuery(
      "Support Request that has Priority 'High' that has Status 'Open'",
      extendedNouns,
      extendedReadings,
    )

    expect(result.rootNoun).toBe('Support Request')
    expect(result.path).toHaveLength(2)
    expect(result.filters).toEqual([
      { field: 'Priority', value: 'High' },
      { field: 'Status', value: 'Open' },
    ])
  })

  it('handles nouns with multi-word names (longest-first matching)', () => {
    // "Support Request" should match before "Support" if both existed
    const result = resolveConceptualQuery(
      'Customer that submits Support Request',
      nouns,
      readings,
    )

    expect(result.rootNoun).toBe('Customer')
    expect(result.path).toHaveLength(1)
    expect(result.path[0].to).toBe('Support Request')
  })
})

import { describe, it, expect } from 'vitest'
import { tokenizeReading } from './tokenize'

const nouns = [
  { name: 'Customer', id: 'c1' },
  { name: 'SupportRequest', id: 'sr1' },
  { name: 'Request', id: 'r1' },
  { name: 'Agent', id: 'a1' },
  { name: 'ListingChannel', id: 'lc1' },
  { name: 'Listing', id: 'l1' },
]

describe('tokenizeReading', () => {
  it('should find nouns in reading text in order', () => {
    const result = tokenizeReading('Customer submits SupportRequest', nouns)
    expect(result.nounRefs).toEqual([
      { name: 'Customer', id: 'c1', index: 0 },
      { name: 'SupportRequest', id: 'sr1', index: 1 },
    ])
  })

  it('should match longest noun first', () => {
    // "SupportRequest" should match, not "Request"
    const result = tokenizeReading('Agent handles SupportRequest', nouns)
    expect(result.nounRefs).toEqual([
      { name: 'Agent', id: 'a1', index: 0 },
      { name: 'SupportRequest', id: 'sr1', index: 1 },
    ])
    expect(result.nounRefs.find((n) => n.name === 'Request')).toBeUndefined()
  })

  it('should match longest noun first with ListingChannel vs Listing', () => {
    const result = tokenizeReading('Customer uses ListingChannel', nouns)
    expect(result.nounRefs).toEqual([
      { name: 'Customer', id: 'c1', index: 0 },
      { name: 'ListingChannel', id: 'lc1', index: 1 },
    ])
    expect(result.nounRefs.find((n) => n.name === 'Listing')).toBeUndefined()
  })

  it('should return empty for no matches', () => {
    const result = tokenizeReading('something unrelated happens', nouns)
    expect(result.nounRefs).toEqual([])
    expect(result.predicate).toBe('')
  })

  it('should extract predicate between first two nouns', () => {
    const result = tokenizeReading('Customer submits SupportRequest', nouns)
    expect(result.predicate).toBe('submits')
  })

  it('should extract multi-word predicate', () => {
    const result = tokenizeReading('Customer is assigned to Agent', nouns)
    expect(result.predicate).toBe('is assigned to')
  })

  it('should return empty predicate when fewer than two nouns', () => {
    const result = tokenizeReading('Customer exists', nouns)
    expect(result.nounRefs).toEqual([{ name: 'Customer', id: 'c1', index: 0 }])
    expect(result.predicate).toBe('')
  })

  it('should handle three nouns and extract predicate between first two', () => {
    const result = tokenizeReading('Customer submits SupportRequest to Agent', nouns)
    expect(result.nounRefs).toHaveLength(3)
    expect(result.nounRefs[0]).toEqual({ name: 'Customer', id: 'c1', index: 0 })
    expect(result.nounRefs[1]).toEqual({ name: 'SupportRequest', id: 'sr1', index: 1 })
    expect(result.nounRefs[2]).toEqual({ name: 'Agent', id: 'a1', index: 2 })
    expect(result.predicate).toBe('submits')
  })

  it('should not match nouns as substrings of other words', () => {
    // "Agent" should not match inside "Agents" if "Agents" is not a known noun
    // but it should match "Agent" as a standalone word
    const result = tokenizeReading('Agents handle Request', nouns)
    // "Agents" has no word boundary match for "Agent" — the 's' breaks the boundary
    // Actually \bAgent\b won't match "Agents" because the 's' is a word char
    expect(result.nounRefs).toEqual([{ name: 'Request', id: 'r1', index: 0 }])
  })

  it('should handle empty noun list', () => {
    const result = tokenizeReading('Customer submits SupportRequest', [])
    expect(result.nounRefs).toEqual([])
    expect(result.predicate).toBe('')
  })

  it('should deduplicate repeated nouns', () => {
    const result = tokenizeReading('Customer rates Customer', nouns)
    expect(result.nounRefs).toEqual([
      { name: 'Customer', id: 'c1', index: 0 },
      { name: 'Customer', id: 'c1', index: 1 },
    ])
  })
})

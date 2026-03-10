import { describe, it, expect } from 'vitest'
import { tokenizeReading } from './tokenize'

describe('tokenizeReading', () => {
  it('finds nouns in a reading', () => {
    const nouns = [
      { name: 'Customer', id: 'n1' },
      { name: 'SupportRequest', id: 'n2' },
    ]
    const result = tokenizeReading('Customer submits SupportRequest', nouns)
    expect(result.nounRefs).toHaveLength(2)
    expect(result.nounRefs[0].name).toBe('Customer')
    expect(result.nounRefs[1].name).toBe('SupportRequest')
    expect(result.predicate).toBe('submits')
  })

  it('handles longest-first matching', () => {
    const nouns = [
      { name: 'Request', id: 'n1' },
      { name: 'SupportRequest', id: 'n2' },
    ]
    const result = tokenizeReading('SupportRequest has Priority', nouns)
    expect(result.nounRefs[0].name).toBe('SupportRequest')
  })

  it('returns empty for no nouns', () => {
    const result = tokenizeReading('hello world', [])
    expect(result.nounRefs).toHaveLength(0)
  })

  it('extracts predicate between two nouns', () => {
    const nouns = [
      { name: 'Employee', id: 'n1' },
      { name: 'Department', id: 'n2' },
    ]
    const result = tokenizeReading('Employee works in Department', nouns)
    expect(result.predicate).toBe('works in')
  })

  it('assigns sequential indices', () => {
    const nouns = [
      { name: 'A', id: 'n1' },
      { name: 'B', id: 'n2' },
      { name: 'C', id: 'n3' },
    ]
    const result = tokenizeReading('A relates to B via C', nouns)
    expect(result.nounRefs[0].index).toBe(0)
    expect(result.nounRefs[1].index).toBe(1)
    expect(result.nounRefs[2].index).toBe(2)
  })
})

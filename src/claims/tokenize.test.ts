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

  it('multi-word nouns match as one unit', () => {
    const nouns = [
      { name: 'Support Request', id: 'n1' },
      { name: 'API Product', id: 'n2' },
    ]
    const result = tokenizeReading('Support Request concerns API Product', nouns)
    expect(result.nounRefs).toHaveLength(2)
    expect(result.nounRefs[0].name).toBe('Support Request')
    expect(result.nounRefs[1].name).toBe('API Product')
    expect(result.predicate).toBe('concerns')
  })

  it('does not match undeclared nouns', () => {
    const nouns = [
      { name: 'Support Request', id: 'n1' },
    ]
    // "Cross" and "Customer" are not declared — should not be found
    const result = tokenizeReading('Cross-domain references: Customer from auth', nouns)
    expect(result.nounRefs).toHaveLength(0)
  })

  it('hyphen binding creates property name', () => {
    const nouns = [
      { name: 'Support Request', id: 'n1' },
      { name: 'Date', id: 'n2' },
    ]
    const result = tokenizeReading('Support Request was created- at Date', nouns)
    expect(result.nounRefs).toHaveLength(2)
    expect(result.predicate).toBe('was created- at')
    expect(result.boundPropertyName).toBe('createdAtDate')
  })

  it('hyphen binding with simple predicate', () => {
    const nouns = [
      { name: 'Message', id: 'n1' },
      { name: 'Sent At', id: 'n2' },
    ]
    const result = tokenizeReading('Message has Sent At', nouns)
    expect(result.nounRefs).toHaveLength(2)
    expect(result.nounRefs[1].name).toBe('Sent At')
    // No hyphen binding here — "Sent At" is a declared noun
    expect(result.boundPropertyName).toBeUndefined()
  })

  it('longest-first prevents partial matches on multi-word nouns', () => {
    const nouns = [
      { name: 'API', id: 'n1' },
      { name: 'API Product', id: 'n2' },
      { name: 'Name', id: 'n3' },
    ]
    const result = tokenizeReading('API Product has Name', nouns)
    // Should match "API Product" not "API" separately
    expect(result.nounRefs).toHaveLength(2)
    expect(result.nounRefs[0].name).toBe('API Product')
    expect(result.nounRefs[1].name).toBe('Name')
  })
})

import { describe, it, expect } from 'vitest'
import { parseReading } from './forml2'

describe('parseReading', () => {
  it('extracts a simple binary fact type', () => {
    const result = parseReading('Customer has Name', ['Customer', 'Name'])
    expect(result.nouns).toEqual(['Customer', 'Name'])
    expect(result.predicate).toBe('has')
    expect(result.constraints).toEqual([])
  })

  it('extracts UC from "at most one"', () => {
    const result = parseReading('Each Customer has at most one Name', ['Customer', 'Name'])
    expect(result.nouns).toEqual(['Customer', 'Name'])
    expect(result.constraints).toEqual([
      { kind: 'UC', roles: [0], modality: 'Alethic' }
    ])
  })

  it('extracts MC from "some" (mandatory)', () => {
    const result = parseReading('Each Customer has some Name', ['Customer', 'Name'])
    expect(result.constraints).toEqual([
      { kind: 'MC', roles: [0], modality: 'Alethic' }
    ])
  })

  it('extracts UC + MC from "exactly one"', () => {
    const result = parseReading('Each Customer has exactly one Name', ['Customer', 'Name'])
    expect(result.constraints).toContainEqual({ kind: 'UC', roles: [0], modality: 'Alethic' })
    expect(result.constraints).toContainEqual({ kind: 'MC', roles: [0], modality: 'Alethic' })
  })

  it('extracts deontic obligation', () => {
    const result = parseReading(
      'It is obligatory that SupportResponse not contain ProhibitedPunctuation',
      ['SupportResponse', 'ProhibitedPunctuation']
    )
    expect(result.nouns).toEqual(['SupportResponse', 'ProhibitedPunctuation'])
    expect(result.constraints).toContainEqual({ kind: 'MC', roles: [0], modality: 'Deontic' })
  })

  it('extracts ternary fact type', () => {
    const result = parseReading(
      'Listing has Price via ListingChannel',
      ['Listing', 'Price', 'ListingChannel']
    )
    expect(result.nouns).toEqual(['Listing', 'Price', 'ListingChannel'])
  })

  it('extracts subtype declaration', () => {
    const result = parseReading('Admin is a subtype of Customer', ['Admin', 'Customer'])
    expect(result.isSubtype).toBe(true)
    expect(result.nouns).toEqual(['Admin', 'Customer'])
  })

  it('extracts state transition reading', () => {
    const result = parseReading(
      'SupportRequest transitions from Received to Triaging on acknowledge',
      ['SupportRequest', 'Received', 'Triaging']
    )
    expect(result.isTransition).toBe(true)
    expect(result.transition).toEqual({
      subject: 'SupportRequest',
      from: 'Received',
      to: 'Triaging',
      event: 'acknowledge',
    })
  })

  it('handles empty knownNouns without crashing', () => {
    const result = parseReading('Customer has Name', [])
    expect(result.nouns).toEqual([])
    expect(result.predicate).toBe('')
    expect(result.constraints).toEqual([])
  })

  it('handles instance fact with quoted value', () => {
    const result = parseReading(
      "Customer with EmailDomain 'driv.ly' has UserRole 'ADMIN'",
      ['Customer', 'EmailDomain', 'UserRole']
    )
    expect(result.isInstanceFact).toBe(true)
    expect(result.instanceValues).toContainEqual({ noun: 'EmailDomain', value: 'driv.ly' })
    expect(result.instanceValues).toContainEqual({ noun: 'UserRole', value: 'ADMIN' })
  })
})

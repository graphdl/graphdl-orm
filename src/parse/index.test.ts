import { describe, it, expect } from 'vitest'
import { parseText } from './index'

describe('parseText', () => {
  const knownNouns = ['Customer', 'Name', 'SupportRequest', 'Admin', 'Priority']

  it('parses a single FORML2 reading', () => {
    const result = parseText('Each Customer has at most one Name', knownNouns)
    expect(result.readings).toHaveLength(1)
    expect(result.readings[0].nouns).toEqual(['Customer', 'Name'])
    expect(result.readings[0].constraints).toContainEqual({
      kind: 'UC', roles: [0], modality: 'Alethic'
    })
  })

  it('parses multiple readings separated by newlines', () => {
    const text = `Customer has Name
Customer submits SupportRequest
SupportRequest has Priority`
    const result = parseText(text, knownNouns)
    expect(result.readings).toHaveLength(3)
  })

  it('skips blank lines and comments', () => {
    const text = `Customer has Name

# This is a comment
Customer submits SupportRequest`
    const result = parseText(text, knownNouns)
    expect(result.readings).toHaveLength(2)
  })

  it('detects subtypes', () => {
    const text = 'Admin is a subtype of Customer'
    const result = parseText(text, [...knownNouns, 'Admin'])
    expect(result.readings).toHaveLength(1)
    expect(result.readings[0].isSubtype).toBe(true)
  })

  it('collects new noun candidates from unrecognized tokens', () => {
    const text = 'Customer has EmailAddress'
    const result = parseText(text, knownNouns)
    expect(result.readings).toHaveLength(1)
    // EmailAddress should be detected as a candidate noun
    expect(result.newNounCandidates).toContain('EmailAddress')
  })
})

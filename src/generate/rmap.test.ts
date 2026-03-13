import { describe, it, expect } from 'vitest'
import {
  nameToKey,
  transformPropertyName,
  extractPropertyName,
  nounListToRegex,
  toPredicate,
  findPredicateObject,
} from './rmap'
import type { NounDef } from '../model/types'

// ---------------------------------------------------------------------------
// nameToKey
// ---------------------------------------------------------------------------
describe('nameToKey', () => {
  it('removes spaces', () => {
    expect(nameToKey('Support Request')).toBe('SupportRequest')
  })

  it('removes hyphens', () => {
    expect(nameToKey('Make-Model')).toBe('MakeModel')
  })

  it('replaces & with And', () => {
    expect(nameToKey('Terms & Conditions')).toBe('TermsAndConditions')
  })

  it('handles combined spaces, hyphens, and ampersands', () => {
    expect(nameToKey('Foo-Bar & Baz Qux')).toBe('FooBarAndBazQux')
  })

  it('returns unchanged string when no special characters', () => {
    expect(nameToKey('Customer')).toBe('Customer')
  })

  it('handles empty string', () => {
    expect(nameToKey('')).toBe('')
  })
})

// ---------------------------------------------------------------------------
// transformPropertyName
// ---------------------------------------------------------------------------
describe('transformPropertyName', () => {
  it('returns empty string for undefined', () => {
    expect(transformPropertyName(undefined)).toBe('')
  })

  it('returns empty string for empty string', () => {
    expect(transformPropertyName('')).toBe('')
  })

  it('lowercases all-caps strings', () => {
    expect(transformPropertyName('VIN')).toBe('vin')
    expect(transformPropertyName('URL')).toBe('url')
    expect(transformPropertyName('ID')).toBe('id')
  })

  it('handles leading uppercase runs', () => {
    expect(transformPropertyName('APIKey')).toBe('apiKey')
    expect(transformPropertyName('HTTPMethod')).toBe('httpMethod')
    expect(transformPropertyName('KBBId')).toBe('kbbId')
  })

  it('handles regular PascalCase', () => {
    expect(transformPropertyName('Priority')).toBe('priority')
    expect(transformPropertyName('CostCenter')).toBe('costCenter')
    expect(transformPropertyName('FirstName')).toBe('firstName')
  })

  it('removes spaces via nameToKey before transforming', () => {
    expect(transformPropertyName('Cost Center')).toBe('costCenter')
    expect(transformPropertyName('Support Request')).toBe('supportRequest')
  })

  it('removes hyphens via nameToKey before transforming', () => {
    expect(transformPropertyName('Make-Model')).toBe('makeModel')
  })

  it('handles single uppercase letter', () => {
    expect(transformPropertyName('X')).toBe('x')
  })

  it('handles single lowercase letter', () => {
    expect(transformPropertyName('x')).toBe('x')
  })

  it('handles two-letter uppercase run followed by lowercase', () => {
    expect(transformPropertyName('IPAddress')).toBe('ipAddress')
  })
})

// ---------------------------------------------------------------------------
// extractPropertyName
// ---------------------------------------------------------------------------
describe('extractPropertyName', () => {
  it('returns camelCase for single-word token', () => {
    expect(extractPropertyName(['Priority'])).toBe('priority')
  })

  it('joins multi-word tokens into camelCase', () => {
    expect(extractPropertyName(['Cost', 'Center'])).toBe('costCenter')
  })

  it('handles all-caps first token', () => {
    expect(extractPropertyName(['VIN'])).toBe('vin')
  })

  it('handles multi-word with mixed case', () => {
    expect(extractPropertyName(['API', 'Key'])).toBe('apiKey')
  })

  it('handles tokens with spaces within first element', () => {
    // If the first token contains a space, it splits it
    expect(extractPropertyName(['Cost Center'])).toBe('costCenter')
  })
})

// ---------------------------------------------------------------------------
// nounListToRegex
// ---------------------------------------------------------------------------
describe('nounListToRegex', () => {
  it('returns empty regex for undefined', () => {
    const regex = nounListToRegex(undefined)
    expect(regex.source).toBe('(?:)')
  })

  it('creates regex matching noun names', () => {
    const nouns: NounDef[] = [
      { id: '1', name: 'Customer', objectType: 'entity', domainId: 'd1' },
      { id: '2', name: 'SupportRequest', objectType: 'entity', domainId: 'd1' },
    ]
    const regex = nounListToRegex(nouns)
    expect(regex.test('Customer')).toBe(true)
    expect(regex.test('SupportRequest')).toBe(true)
    expect(regex.test('Unknown')).toBe(false)
  })

  it('sorts longest names first to avoid partial matches', () => {
    const nouns: NounDef[] = [
      { id: '1', name: 'API', objectType: 'entity', domainId: 'd1' },
      { id: '2', name: 'APIKey', objectType: 'entity', domainId: 'd1' },
    ]
    const regex = nounListToRegex(nouns)
    // The regex should match 'APIKey' before 'API'
    const match = 'APIKey'.match(regex)
    expect(match).not.toBeNull()
    expect(match![0]).toBe('APIKey')
  })

  it('handles empty noun list', () => {
    const nouns: NounDef[] = []
    const regex = nounListToRegex(nouns)
    expect(regex.source).toBe('()')
  })

  it('allows optional trailing hyphen', () => {
    const nouns: NounDef[] = [{ id: '1', name: 'Customer', objectType: 'entity', domainId: 'd1' }]
    const regex = nounListToRegex(nouns)
    expect(regex.test('Customer-')).toBe(true)
  })
})

// ---------------------------------------------------------------------------
// toPredicate
// ---------------------------------------------------------------------------
const D = 'd1'
const E = 'entity' as const

describe('toPredicate', () => {
  it('tokenizes a simple reading', () => {
    const nouns: NounDef[] = [
      { id: '1', name: 'Customer', objectType: E, domainId: D },
      { id: '2', name: 'SupportRequest', objectType: E, domainId: D },
    ]
    const result = toPredicate({
      reading: 'Customer submits SupportRequest',
      nouns,
    })
    expect(result).toEqual(['Customer', 'submits', 'SupportRequest'])
  })

  it('keeps noun names intact even with hyphens', () => {
    const nouns: NounDef[] = [
      { id: '1', name: 'Customer', objectType: E, domainId: D },
      { id: '2', name: 'Support Request', objectType: E, domainId: D },
    ]
    const result = toPredicate({
      reading: 'Customer has Support Request',
      nouns,
    })
    expect(result).toEqual(['Customer', 'has', 'Support Request'])
  })

  it('handles reading with pre-computed nounRegex', () => {
    const nouns: NounDef[] = [
      { id: '1', name: 'Customer', objectType: E, domainId: D },
      { id: '2', name: 'Domain', objectType: E, domainId: D },
    ]
    const nounRegex = nounListToRegex(nouns)
    const result = toPredicate({
      reading: 'Customer owns Domain',
      nouns,
      nounRegex,
    })
    expect(result).toEqual(['Customer', 'owns', 'Domain'])
  })

  it('filters out empty tokens', () => {
    const nouns: NounDef[] = [
      { id: '1', name: 'Customer', objectType: E, domainId: D },
      { id: '2', name: 'Name', objectType: 'value', domainId: D },
    ]
    const result = toPredicate({
      reading: 'Customer has Name',
      nouns,
    })
    expect(result).toEqual(['Customer', 'has', 'Name'])
  })

  it('handles reading starting with noun', () => {
    const nouns: NounDef[] = [{ id: '1', name: 'Vehicle', objectType: E, domainId: D }]
    const result = toPredicate({
      reading: 'Vehicle has Color',
      nouns,
    })
    expect(result).toEqual(['Vehicle', 'has', 'Color'])
  })

  it('converts hyphenated non-noun words to camelCase', () => {
    const nouns: NounDef[] = [
      { id: '1', name: 'Customer', objectType: E, domainId: D },
    ]
    const result = toPredicate({
      reading: 'Customer is-called Name',
      nouns,
    })
    expect(result).toEqual(['Customer', 'isCalled', 'Name'])
  })
})

// ---------------------------------------------------------------------------
// findPredicateObject
// ---------------------------------------------------------------------------
describe('findPredicateObject', () => {
  const mkNoun = (name: string): NounDef => ({ id: name.toLowerCase(), name, objectType: E, domainId: D })

  it('finds object after subject with verb', () => {
    const predicate = ['Customer', 'has', 'Priority']
    const result = findPredicateObject({
      predicate,
      subject: mkNoun('Customer'),
      object: mkNoun('Priority'),
    })
    expect(result.objectBegin).toBe(2)
    expect(result.objectEnd).toBe(3)
  })

  it('skips verbs and prepositions', () => {
    const predicate = ['Customer', 'submits', 'SupportRequest']
    const result = findPredicateObject({
      predicate,
      subject: mkNoun('Customer'),
      object: mkNoun('SupportRequest'),
    })
    expect(result.objectBegin).toBe(2)
    expect(result.objectEnd).toBe(3)
  })

  it('handles object before subject', () => {
    const predicate = ['Priority', 'of', 'Customer']
    const result = findPredicateObject({
      predicate,
      subject: mkNoun('Customer'),
      object: mkNoun('Priority'),
    })
    expect(result.objectBegin).toBe(0)
    expect(result.objectEnd).toBe(1)
  })

  it('returns full range when no object provided', () => {
    const predicate = ['Customer', 'has', 'priority', 'level']
    const result = findPredicateObject({
      predicate,
      subject: mkNoun('Customer'),
    })
    expect(result.objectBegin).toBe(1)
    expect(result.objectEnd).toBe(4)
  })

  it('returns zeros when subject not found', () => {
    const predicate = ['Customer', 'has', 'Priority']
    const result = findPredicateObject({
      predicate,
      subject: mkNoun('Unknown'),
    })
    expect(result).toEqual({ objectBegin: 0, objectEnd: 0 })
  })

  it('throws when object specified but not found', () => {
    const predicate = ['Customer', 'has', 'Priority']
    expect(() =>
      findPredicateObject({
        predicate,
        subject: mkNoun('Customer'),
        object: mkNoun('Missing'),
      }),
    ).toThrow('Object "Missing" not found in predicate "Customer has Priority"')
  })

  it('handles plural replacement', () => {
    const predicate = ['Customer', 'has', 'Domain']
    const result = findPredicateObject({
      predicate,
      subject: mkNoun('Customer'),
      object: mkNoun('Domain'),
      plural: 'domains',
    })
    // After plural replacement, predicate[2] becomes 'Domains'
    expect(predicate[2]).toBe('Domains')
    expect(result.objectBegin).toBe(2)
    expect(result.objectEnd).toBe(3)
  })

  it('finds subject with trailing hyphen', () => {
    const predicate = ['Customer-', 'has', 'Priority']
    const result = findPredicateObject({
      predicate,
      subject: mkNoun('Customer'),
      object: mkNoun('Priority'),
    })
    expect(result.objectBegin).toBe(2)
    expect(result.objectEnd).toBe(3)
  })

  it('finds object with trailing hyphen and extends to end', () => {
    const predicate = ['Customer', 'has', 'Priority-', 'Level']
    const result = findPredicateObject({
      predicate,
      subject: mkNoun('Customer'),
      object: mkNoun('Priority'),
    })
    // When object has trailing hyphen, objectEnd extends to predicate.length
    expect(result.objectEnd).toBe(4)
  })

  it('skips "has" as a verb/preposition', () => {
    const predicate = ['Customer', 'has', 'Name']
    const result = findPredicateObject({
      predicate,
      subject: mkNoun('Customer'),
      object: mkNoun('Name'),
    })
    expect(result.objectBegin).toBe(2)
  })

  it('skips "is" as a verb/preposition', () => {
    const predicate = ['Customer', 'is', 'Active']
    const result = findPredicateObject({
      predicate,
      subject: mkNoun('Customer'),
      object: mkNoun('Active'),
    })
    expect(result.objectBegin).toBe(2)
  })

  it('handles multi-word qualifier before object', () => {
    // e.g. ['Vehicle', 'manufactured', 'Make'] — 'manufactured' is in the verbs list
    const predicate = ['Vehicle', 'manufactured', 'Make']
    const result = findPredicateObject({
      predicate,
      subject: mkNoun('Vehicle'),
      object: mkNoun('Make'),
    })
    expect(result.objectBegin).toBe(2)
  })
})

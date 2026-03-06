import { describe, it, expect } from 'vitest'
import { buildMatchers, matchText } from './matcher'
import type { DeonticConstraintGroup } from '../seed/deontic'
import type { ConstraintMatcher, MatchResult } from './matcher'

describe('buildMatchers', () => {
  it('creates regex from groups and null for empty instances', () => {
    const groups: DeonticConstraintGroup[] = [
      { constraintText: 'It is forbidden that Response contains SourceName', instances: ['Edmunds', 'Carfax'] },
      { constraintText: 'It is obligatory that Response maintains professional tone', instances: [] },
    ]
    const matchers = buildMatchers(groups)

    expect(matchers).toHaveLength(2)
    expect(matchers[0].regex).toBeInstanceOf(RegExp)
    expect(matchers[0].instances).toEqual(['Edmunds', 'Carfax'])
    expect(matchers[1].regex).toBeNull()
    expect(matchers[1].instances).toEqual([])
  })

  it('sorts instances longest-first in the regex', () => {
    const groups: DeonticConstraintGroup[] = [
      { constraintText: 'test', instances: ['Car', 'CarMax', 'CarStory'] },
    ]
    const matchers = buildMatchers(groups)
    // CarStory (8) should come before CarMax (6) before Car (3)
    const source = matchers[0].regex!.source
    const carStoryIdx = source.indexOf('CarStory')
    const carMaxIdx = source.indexOf('CarMax')
    const carIdx = source.indexOf('\\bCar\\b')
    expect(carStoryIdx).toBeLessThan(carMaxIdx)
    expect(carMaxIdx).toBeLessThan(carIdx)
  })
})

describe('matchText', () => {
  it('finds prohibited punctuation (em dash, en dash)', () => {
    const groups: DeonticConstraintGroup[] = [
      { constraintText: 'It is forbidden that Response contains SpecialChar', instances: ['\u2014', '\u2013'] },
    ]
    const matchers = buildMatchers(groups)
    const result = matchText('This is a test\u2014with em dash and\u2013en dash', matchers)

    expect(result.matches).toHaveLength(2)
    expect(result.matches.map((m) => m.instance)).toContain('\u2014')
    expect(result.matches.map((m) => m.instance)).toContain('\u2013')
  })

  it('finds prohibited names with word boundaries', () => {
    const groups: DeonticConstraintGroup[] = [
      { constraintText: 'It is forbidden that Response contains SourceName', instances: ['Edmunds', 'Carfax'] },
    ]
    const matchers = buildMatchers(groups)
    const text = 'We checked Edmunds and Carfax for your vehicle.'
    const result = matchText(text, matchers)

    expect(result.matches).toHaveLength(2)
    const edmundsMatch = result.matches.find((m) => m.instance === 'Edmunds')!
    expect(edmundsMatch.span[0]).toBe(text.indexOf('Edmunds'))
    expect(edmundsMatch.span[1]).toBe(text.indexOf('Edmunds') + 'Edmunds'.length)
  })

  it('returns unmatchedConstraints for groups with no instances', () => {
    const groups: DeonticConstraintGroup[] = [
      { constraintText: 'It is obligatory that Response maintains professional tone', instances: [] },
      { constraintText: 'It is forbidden that Response contains SourceName', instances: ['Edmunds'] },
    ]
    const matchers = buildMatchers(groups)
    const result = matchText('Hello world', matchers)

    expect(result.unmatchedConstraints).toEqual(['It is obligatory that Response maintains professional tone'])
    expect(result.matches).toHaveLength(0)
  })

  it('matches case-insensitively', () => {
    const groups: DeonticConstraintGroup[] = [
      { constraintText: 'test', instances: ['Edmunds'] },
    ]
    const matchers = buildMatchers(groups)
    const result = matchText('Check EDMUNDS or edmunds for pricing.', matchers)

    expect(result.matches).toHaveLength(2)
    // Should resolve to original instance casing
    expect(result.matches[0].instance).toBe('Edmunds')
    expect(result.matches[1].instance).toBe('Edmunds')
  })

  it('does not match partial words (word boundary test)', () => {
    const groups: DeonticConstraintGroup[] = [
      { constraintText: 'test', instances: ['Car'] },
    ]
    const matchers = buildMatchers(groups)
    const result = matchText('We went to CarMax to buy a car today.', matchers)

    // Should NOT match "Car" inside "CarMax", but should match standalone "car"
    expect(result.matches).toHaveLength(1)
    expect(result.matches[0].instance).toBe('Car')
    expect(result.matches[0].span[0]).toBe('We went to CarMax to buy a '.length)
  })

  it('matches literal punctuation without word boundaries (hello\u2014world)', () => {
    const groups: DeonticConstraintGroup[] = [
      { constraintText: 'test', instances: ['\u2014'] },
    ]
    const matchers = buildMatchers(groups)
    const result = matchText('hello\u2014world', matchers)

    expect(result.matches).toHaveLength(1)
    expect(result.matches[0].instance).toBe('\u2014')
    expect(result.matches[0].span).toEqual([5, 6])
  })

  it('finds multiple matches of same constraint in one text', () => {
    const groups: DeonticConstraintGroup[] = [
      { constraintText: 'It is forbidden that Response contains SourceName', instances: ['Edmunds'] },
    ]
    const matchers = buildMatchers(groups)
    const text = 'Edmunds says one thing, but Edmunds also says another.'
    const result = matchText(text, matchers)

    expect(result.matches).toHaveLength(2)
    expect(result.matches[0].span[0]).toBe(0)
    expect(result.matches[1].span[0]).toBe(28)
  })
})

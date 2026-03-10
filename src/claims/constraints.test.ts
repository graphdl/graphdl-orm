import { describe, it, expect } from 'vitest'
import { parseMultiplicity } from './constraints'

describe('parseMultiplicity', () => {
  it('parses *:1', () => {
    const result = parseMultiplicity('*:1')
    expect(result).toEqual([{ kind: 'UC', modality: 'Alethic', roles: [0] }])
  })

  it('parses 1:1 into two UCs', () => {
    const result = parseMultiplicity('1:1')
    expect(result).toHaveLength(2)
    expect(result[0].roles).toEqual([0])
    expect(result[1].roles).toEqual([1])
  })

  it('parses *:* as spanning UC', () => {
    const result = parseMultiplicity('*:*')
    expect(result).toEqual([{ kind: 'UC', modality: 'Alethic', roles: [0, 1] }])
  })

  it('parses compound *:1 MC', () => {
    const result = parseMultiplicity('*:1 MC')
    expect(result).toHaveLength(2)
    expect(result[0]).toEqual({ kind: 'UC', modality: 'Alethic', roles: [0] })
    expect(result[1]).toEqual({ kind: 'MC', modality: 'Alethic', roles: [-1] })
  })

  it('parses Deontic D*:1', () => {
    const result = parseMultiplicity('D*:1')
    expect(result).toEqual([{ kind: 'UC', modality: 'Deontic', roles: [0] }])
  })

  it('returns empty for subtype', () => {
    expect(parseMultiplicity('subtype')).toEqual([])
  })

  it('returns empty for empty string', () => {
    expect(parseMultiplicity('')).toEqual([])
  })

  it('parses 1:*', () => {
    const result = parseMultiplicity('1:*')
    expect(result).toEqual([{ kind: 'UC', modality: 'Alethic', roles: [1] }])
  })

  it('returns empty for subset constraint', () => {
    expect(parseMultiplicity('SS')).toEqual([])
    expect(parseMultiplicity('DSS')).toEqual([])
  })

  it('parses unary', () => {
    const result = parseMultiplicity('unary')
    expect(result).toEqual([{ kind: 'UC', modality: 'Alethic', roles: [0] }])
  })

  it('parses DMC', () => {
    const result = parseMultiplicity('DMC')
    expect(result).toEqual([{ kind: 'MC', modality: 'Deontic', roles: [-1] }])
  })
})

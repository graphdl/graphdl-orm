import { describe, it, expect } from 'vitest'
import { parseMultiplicity } from './constraints'

describe('parseMultiplicity', () => {
  it('should parse *:1 as UC on role 0', () => {
    expect(parseMultiplicity('*:1')).toEqual([
      { kind: 'UC', modality: 'Alethic', roles: [0] },
    ])
  })

  it('should parse 1:* as UC on role 1', () => {
    expect(parseMultiplicity('1:*')).toEqual([
      { kind: 'UC', modality: 'Alethic', roles: [1] },
    ])
  })

  it('should parse 1:1 as two UCs', () => {
    expect(parseMultiplicity('1:1')).toEqual([
      { kind: 'UC', modality: 'Alethic', roles: [0] },
      { kind: 'UC', modality: 'Alethic', roles: [1] },
    ])
  })

  it('should parse *:* as UC spanning both roles', () => {
    expect(parseMultiplicity('*:*')).toEqual([
      { kind: 'UC', modality: 'Alethic', roles: [0, 1] },
    ])
  })

  it('should parse *:1 MC as UC on role 0 + MC on last role', () => {
    expect(parseMultiplicity('*:1 MC')).toEqual([
      { kind: 'UC', modality: 'Alethic', roles: [0] },
      { kind: 'MC', modality: 'Alethic', roles: [-1] },
    ])
  })

  it('should handle Deontic modality on UC', () => {
    expect(parseMultiplicity('D*:1')).toEqual([
      { kind: 'UC', modality: 'Deontic', roles: [0] },
    ])
  })

  it('should handle Deontic modality on MC', () => {
    expect(parseMultiplicity('*:1 DMC')).toEqual([
      { kind: 'UC', modality: 'Alethic', roles: [0] },
      { kind: 'MC', modality: 'Deontic', roles: [-1] },
    ])
  })

  it('should handle D1:* as Deontic UC on role 1', () => {
    expect(parseMultiplicity('D1:*')).toEqual([
      { kind: 'UC', modality: 'Deontic', roles: [1] },
    ])
  })

  it('should handle D1:1 as two Deontic UCs', () => {
    expect(parseMultiplicity('D1:1')).toEqual([
      { kind: 'UC', modality: 'Deontic', roles: [0] },
      { kind: 'UC', modality: 'Deontic', roles: [1] },
    ])
  })

  it('should handle D*:* as Deontic UC spanning both roles', () => {
    expect(parseMultiplicity('D*:*')).toEqual([
      { kind: 'UC', modality: 'Deontic', roles: [0, 1] },
    ])
  })

  it('should handle AMC as Alethic MC', () => {
    expect(parseMultiplicity('*:1 AMC')).toEqual([
      { kind: 'UC', modality: 'Alethic', roles: [0] },
      { kind: 'MC', modality: 'Alethic', roles: [-1] },
    ])
  })

  it('should handle unary as UC on role 0', () => {
    expect(parseMultiplicity('unary')).toEqual([
      { kind: 'UC', modality: 'Alethic', roles: [0] },
    ])
  })

  it('should return empty for subtype', () => {
    expect(parseMultiplicity('subtype')).toEqual([])
  })

  it('should return empty for SS', () => {
    expect(parseMultiplicity('SS')).toEqual([])
  })

  it('should return empty for DSS', () => {
    expect(parseMultiplicity('DSS')).toEqual([])
  })

  it('should handle compound Deontic specs: D*:1 DMC', () => {
    expect(parseMultiplicity('D*:1 DMC')).toEqual([
      { kind: 'UC', modality: 'Deontic', roles: [0] },
      { kind: 'MC', modality: 'Deontic', roles: [-1] },
    ])
  })

  it('should return empty for empty string', () => {
    expect(parseMultiplicity('')).toEqual([])
  })

  it('should handle mixed modalities: *:1 DMC', () => {
    expect(parseMultiplicity('*:1 DMC')).toEqual([
      { kind: 'UC', modality: 'Alethic', roles: [0] },
      { kind: 'MC', modality: 'Deontic', roles: [-1] },
    ])
  })
})

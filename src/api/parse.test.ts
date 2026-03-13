import { describe, it, expect } from 'vitest'
import { parseFORML2 } from './parse'

describe('parseFORML2', () => {
  it('parses a single reading with UC constraint', () => {
    const text = `Customer has Name.
  Each Customer has at most one Name.`

    const result = parseFORML2(text, [])

    // Nouns
    expect(result.nouns).toHaveLength(2)
    expect(result.nouns.find(n => n.name === 'Customer')).toMatchObject({
      name: 'Customer',
      objectType: 'entity',
    })
    expect(result.nouns.find(n => n.name === 'Name')).toMatchObject({
      name: 'Name',
      objectType: 'value', // object of "has" → value type
    })

    // Readings
    expect(result.readings).toHaveLength(1)
    expect(result.readings[0]).toMatchObject({
      text: 'Customer has Name',
      nouns: ['Customer', 'Name'],
      predicate: 'has',
    })

    // Constraints
    expect(result.constraints).toHaveLength(1)
    expect(result.constraints[0]).toMatchObject({
      kind: 'UC',
      modality: 'Alethic',
      reading: 'Customer has Name',
      roles: [0],
    })

    // Always empty
    expect(result.transitions).toEqual([])
    expect(result.facts).toEqual([])
    expect(result.warnings).toEqual([])
  })

  it('parses multiple readings separated by blank lines', () => {
    const text = `Customer has Name.
  Each Customer has at most one Name.

Customer submits SupportRequest.
  Each SupportRequest is submitted by at most one Customer.`

    const result = parseFORML2(text, [])

    expect(result.nouns).toHaveLength(3)
    expect(result.readings).toHaveLength(2)
    expect(result.constraints).toHaveLength(2)

    // Second reading reuses Customer noun
    expect(result.readings[1]).toMatchObject({
      text: 'Customer submits SupportRequest',
      nouns: ['Customer', 'SupportRequest'],
      predicate: 'submits',
    })
  })

  it('detects subtype declarations', () => {
    const text = `PremiumCustomer is a subtype of Customer.`

    const result = parseFORML2(text, [])

    expect(result.subtypes).toHaveLength(1)
    expect(result.subtypes![0]).toEqual({
      child: 'PremiumCustomer',
      parent: 'Customer',
    })
    // Both nouns should be in the noun list
    expect(result.nouns.find(n => n.name === 'PremiumCustomer')).toBeDefined()
    expect(result.nouns.find(n => n.name === 'Customer')).toBeDefined()
  })

  it('produces partial results with warnings for malformed input', () => {
    const text = `Customer has Name.
  Each Customer has at most one Name.

justgarbage

SupportRequest has Priority.`

    const result = parseFORML2(text, [])

    // Good blocks parsed
    expect(result.readings).toHaveLength(2)
    // Bad block produces warning
    expect(result.warnings.length).toBeGreaterThanOrEqual(1)
    expect(result.warnings.some(w => w.includes('fewer than 2 nouns'))).toBe(true)
  })

  it('handles "exactly one" producing UC + MC constraints', () => {
    const text = `Organization has Name.
  Each Organization has exactly one Name.`

    const result = parseFORML2(text, [])

    expect(result.constraints).toHaveLength(2)
    expect(result.constraints.find(c => c.kind === 'UC')).toBeDefined()
    expect(result.constraints.find(c => c.kind === 'MC')).toBeDefined()
  })

  it('uses existing nouns for tokenization context', () => {
    const existingNouns = [
      { name: 'Customer', id: 'n1' },
      { name: 'Name', id: 'n2' },
    ]
    const text = `Customer has Name.
  Each Customer has at most one Name.`

    const result = parseFORML2(text, existingNouns)

    expect(result.nouns).toHaveLength(2) // No duplicates
    expect(result.readings).toHaveLength(1)
  })

  it('returns empty arrays for transitions and facts', () => {
    const text = `Customer has Name.`

    const result = parseFORML2(text, [])

    expect(result.transitions).toEqual([])
    expect(result.facts).toEqual([])
  })

  it('warns on unrecognized constraint patterns', () => {
    const text = `Customer has Name.
  This is not a valid constraint.`

    const result = parseFORML2(text, [])

    expect(result.readings).toHaveLength(1)
    expect(result.warnings).toHaveLength(1)
    expect(result.warnings[0]).toContain('Unrecognized constraint pattern')
  })

  it('handles non-"has" predicates as entity types', () => {
    const text = `Customer submits SupportRequest.`

    const result = parseFORML2(text, [])

    expect(result.nouns.find(n => n.name === 'SupportRequest')).toMatchObject({
      objectType: 'entity', // not "has" → entity, not value
    })
  })

  it('retries deferred constraints against later-defined nouns', () => {
    // Constraint on first block references nouns from second block
    const text = `Customer has Name.
  Each SupportRequest is submitted by at most one Customer.

Customer submits SupportRequest.`

    const result = parseFORML2(text, [])

    // The deferred constraint should resolve against the second reading
    expect(result.constraints.length).toBeGreaterThanOrEqual(1)
    const deferredConstraint = result.constraints.find(
      c => c.reading === 'Customer submits SupportRequest'
    )
    expect(deferredConstraint).toBeDefined()
    expect(result.warnings.filter(w => w.includes('unresolved'))).toHaveLength(0)
  })

  it('warns on permanently unresolvable deferred constraints', () => {
    const text = `Customer has Name.
  Each Order has at most one Invoice.`

    const result = parseFORML2(text, [])

    // Order and Invoice never appear as a reading → warning
    expect(result.warnings.some(w => w.includes('unresolved'))).toBe(true)
  })
})

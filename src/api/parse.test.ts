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

  it('skips informational patterns without warnings', () => {
    const text = `Customer is an entity type.

Name is a value type.

Customer has Name.
  Each Customer has at most one Name.

Reference Mode: .Name`

    const result = parseFORML2(text, [])

    expect(result.readings).toHaveLength(1)
    expect(result.readings[0].text).toBe('Customer has Name')
    // Informational lines should not produce warnings
    expect(result.warnings).toEqual([])
  })

  it('parses XO set-comparison blocks as standalone constraints', () => {
    const text = `Message has Lead.
  Each Message has at most one Lead.

For each Message, exactly one of the following holds:
  that Message has MatchStatus 'Pending';
  that Message has MatchStatus 'Confirmed';
  that Message has MatchStatus 'Rejected'.`

    const result = parseFORML2(text, [])

    // Reading from the first block
    expect(result.readings).toHaveLength(1)
    expect(result.readings[0].text).toBe('Message has Lead')

    // UC from the reading + XO from the set-comparison block
    const xo = result.constraints.find(c => c.kind === 'XO')
    expect(xo).toBeDefined()
    expect(xo!.reading).toBe('')
    expect(xo!.roles).toEqual([])
    expect(xo!.clauses).toHaveLength(3)
    expect(xo!.entity).toBe('Message')
  })

  it('parses SS subset constraints', () => {
    const text = `If some Message has Lead then that Message has SalesRep.`

    const result = parseFORML2(text, [])

    const ss = result.constraints.find(c => c.kind === 'SS')
    expect(ss).toBeDefined()
    expect(ss!.kind).toBe('SS')
    // SS nouns go into overall noun list, not on the constraint
    expect(result.nouns.find(n => n.name === 'Message')).toBeDefined()
    expect(result.nouns.find(n => n.name === 'Lead')).toBeDefined()
    expect(result.nouns.find(n => n.name === 'SalesRep')).toBeDefined()
  })

  it('handles "Each X has some Y" as MC', () => {
    const text = `Message has Lead.
  Each Message has some Lead.`

    const result = parseFORML2(text, [])

    const mc = result.constraints.find(c => c.kind === 'MC')
    expect(mc).toBeDefined()
    expect(mc!.reading).toBe('Message has Lead')
  })

  it('handles "if and only if" as EQ', () => {
    const text = `Message has Lead.
  Message is matched if and only if Message has Lead.`

    const result = parseFORML2(text, [])

    const eq = result.constraints.find(c => c.kind === 'EQ')
    expect(eq).toBeDefined()
    expect(eq!.reading).toBe('Message has Lead')
  })

  it('parses a full domain with mixed readings and set-comparison blocks', () => {
    const text = `Message has Lead.
  Each Message has at most one Lead.

Lead is assigned to SalesRep.
  Each Lead is assigned to at most one SalesRep.

For each Message, exactly one of the following holds:
  that Message has MatchStatus 'Pending';
  that Message has MatchStatus 'Confirmed';
  that Message has MatchStatus 'Rejected'.

If some Message has Lead then that Message has SalesRep.`

    const result = parseFORML2(text, [])

    // Two readings
    expect(result.readings).toHaveLength(2)

    // 2 UCs from readings + 1 XO + 1 SS
    const uc = result.constraints.filter(c => c.kind === 'UC')
    const xo = result.constraints.filter(c => c.kind === 'XO')
    const ss = result.constraints.filter(c => c.kind === 'SS')
    expect(uc).toHaveLength(2)
    expect(xo).toHaveLength(1)
    expect(ss).toHaveLength(1)

    // All nouns accumulated
    expect(result.nouns.find(n => n.name === 'Message')).toBeDefined()
    expect(result.nouns.find(n => n.name === 'Lead')).toBeDefined()
    expect(result.nouns.find(n => n.name === 'SalesRep')).toBeDefined()
    expect(result.nouns.find(n => n.name === 'MatchStatus')).toBeDefined()

    expect(result.warnings).toEqual([])
  })
})

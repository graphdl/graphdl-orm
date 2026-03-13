import { describe, it, expect } from 'vitest'
import { verifyProse } from './verify'

describe('verifyProse', () => {
  it('matches readings whose nouns appear in prose', () => {
    const domainData = {
      nouns: [
        { id: 'n1', name: 'Customer' },
        { id: 'n2', name: 'Name' },
        { id: 'n3', name: 'SupportRequest' },
        { id: 'n4', name: 'Priority' },
      ],
      readings: [
        { id: 'r1', text: 'Customer has Name' },
        { id: 'r2', text: 'SupportRequest has Priority' },
      ],
      roles: [
        { id: 'ro1', reading: 'r1', noun: 'n1', roleIndex: 0 },
        { id: 'ro2', reading: 'r1', noun: 'n2', roleIndex: 1 },
        { id: 'ro3', reading: 'r2', noun: 'n3', roleIndex: 0 },
        { id: 'ro4', reading: 'r2', noun: 'n4', roleIndex: 1 },
      ],
      constraints: [
        { id: 'c1', kind: 'UC', text: 'Each Customer has at most one Name' },
        { id: 'c2', kind: 'UC', text: 'Each SupportRequest has at most one Priority' },
      ],
      constraintSpans: [
        { constraint: 'c1', role: 'ro1' },
        { constraint: 'c2', role: 'ro3' },
      ],
    }

    const prose = 'The Customer named John submitted a request.'

    const result = verifyProse(prose, domainData)

    // Customer and Name appear → reading r1 matches
    expect(result.matches).toHaveLength(1)
    expect(result.matches[0]).toMatchObject({
      reading: 'Customer has Name',
      nouns: ['Customer'],
    })

    // SupportRequest and Priority don't both appear → c2 unmatched
    expect(result.unmatchedConstraints).toContain(
      'Each SupportRequest has at most one Priority'
    )
  })

  it('returns all constraints as unmatched when prose has no domain nouns', () => {
    const domainData = {
      nouns: [
        { id: 'n1', name: 'Customer' },
        { id: 'n2', name: 'Name' },
      ],
      readings: [
        { id: 'r1', text: 'Customer has Name' },
      ],
      roles: [
        { id: 'ro1', reading: 'r1', noun: 'n1', roleIndex: 0 },
        { id: 'ro2', reading: 'r1', noun: 'n2', roleIndex: 1 },
      ],
      constraints: [
        { id: 'c1', kind: 'UC', text: 'Each Customer has at most one Name' },
      ],
      constraintSpans: [
        { constraint: 'c1', role: 'ro1' },
      ],
    }

    const prose = 'The weather is nice today.'

    const result = verifyProse(prose, domainData)

    expect(result.matches).toHaveLength(0)
    expect(result.unmatchedConstraints).toHaveLength(1)
  })

  it('matches all constraints when all nouns appear in prose', () => {
    const domainData = {
      nouns: [
        { id: 'n1', name: 'Customer' },
        { id: 'n2', name: 'Name' },
      ],
      readings: [
        { id: 'r1', text: 'Customer has Name' },
      ],
      roles: [
        { id: 'ro1', reading: 'r1', noun: 'n1', roleIndex: 0 },
        { id: 'ro2', reading: 'r1', noun: 'n2', roleIndex: 1 },
      ],
      constraints: [
        { id: 'c1', kind: 'UC', text: 'Each Customer has at most one Name' },
      ],
      constraintSpans: [
        { constraint: 'c1', role: 'ro1' },
      ],
    }

    const prose = 'The Customer has a Name of "John Smith".'

    const result = verifyProse(prose, domainData)

    expect(result.matches).toHaveLength(1)
    expect(result.unmatchedConstraints).toHaveLength(0)
  })

  it('handles constraints with no text gracefully', () => {
    const domainData = {
      nouns: [
        { id: 'n1', name: 'Customer' },
        { id: 'n2', name: 'Name' },
      ],
      readings: [
        { id: 'r1', text: 'Customer has Name' },
      ],
      roles: [
        { id: 'ro1', reading: 'r1', noun: 'n1', roleIndex: 0 },
        { id: 'ro2', reading: 'r1', noun: 'n2', roleIndex: 1 },
      ],
      constraints: [
        { id: 'c1', kind: 'UC' }, // no text field
      ],
      constraintSpans: [
        { constraint: 'c1', role: 'ro1' },
      ],
    }

    const prose = 'Hello world.'

    const result = verifyProse(prose, domainData)

    expect(result.unmatchedConstraints).toHaveLength(1)
    expect(result.unmatchedConstraints[0]).toContain('UC constraint c1')
  })

  it('handles empty domain data', () => {
    const domainData = {
      nouns: [],
      readings: [],
      roles: [],
      constraints: [],
      constraintSpans: [],
    }

    const result = verifyProse('Some text', domainData)

    expect(result.matches).toEqual([])
    expect(result.unmatchedConstraints).toEqual([])
  })

  it('correctly splits matched and unmatched constraints in mixed prose', () => {
    const domainData = {
      nouns: [
        { id: 'n1', name: 'Customer' },
        { id: 'n2', name: 'Name' },
        { id: 'n3', name: 'SupportRequest' },
        { id: 'n4', name: 'Priority' },
        { id: 'n5', name: 'Invoice' },
        { id: 'n6', name: 'Amount' },
      ],
      readings: [
        { id: 'r1', text: 'Customer has Name' },
        { id: 'r2', text: 'SupportRequest has Priority' },
        { id: 'r3', text: 'Invoice has Amount' },
      ],
      roles: [
        { id: 'ro1', reading: 'r1', noun: 'n1', roleIndex: 0 },
        { id: 'ro2', reading: 'r1', noun: 'n2', roleIndex: 1 },
        { id: 'ro3', reading: 'r2', noun: 'n3', roleIndex: 0 },
        { id: 'ro4', reading: 'r2', noun: 'n4', roleIndex: 1 },
        { id: 'ro5', reading: 'r3', noun: 'n5', roleIndex: 0 },
        { id: 'ro6', reading: 'r3', noun: 'n6', roleIndex: 1 },
      ],
      constraints: [
        { id: 'c1', kind: 'UC', text: 'Each Customer has at most one Name' },
        { id: 'c2', kind: 'UC', text: 'Each SupportRequest has at most one Priority' },
        { id: 'c3', kind: 'MC', text: 'Each Invoice has at least one Amount' },
      ],
      constraintSpans: [
        { constraint: 'c1', role: 'ro1' },
        { constraint: 'c2', role: 'ro3' },
        { constraint: 'c3', role: 'ro5' },
      ],
    }

    // Prose mentions Customer and SupportRequest, but not Invoice
    const prose = 'The Customer submitted a SupportRequest about billing.'

    const result = verifyProse(prose, domainData)

    // r1 and r2 match (Customer, SupportRequest mentioned), r3 doesn't (Invoice not mentioned)
    expect(result.matches).toHaveLength(2)
    expect(result.matches.map(m => m.reading)).toContain('Customer has Name')
    expect(result.matches.map(m => m.reading)).toContain('SupportRequest has Priority')

    // c3 (Invoice constraint) is unmatched
    expect(result.unmatchedConstraints).toHaveLength(1)
    expect(result.unmatchedConstraints[0]).toContain('Invoice')
  })
})

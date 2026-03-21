import { describe, it, expect } from 'vitest'
import { parseConstraintText, parseSetComparisonBlock, isInformationalPattern } from './parse-constraint'

describe('parseConstraintText', () => {
  describe('uniqueness constraints (UC)', () => {
    it('parses "Each X has at most one Y"', () => {
      const result = parseConstraintText('Each Customer has at most one Name.')
      expect(result).toEqual([
        { kind: 'UC', modality: 'Alethic', nouns: ['Customer', 'Name'], constrainedNoun: 'Customer' },
      ])
    })

    it('parses "Each X belongs to at most one Y"', () => {
      const result = parseConstraintText('Each Domain belongs to at most one Organization.')
      expect(result).toEqual([
        { kind: 'UC', modality: 'Alethic', nouns: ['Domain', 'Organization'], constrainedNoun: 'Domain' },
      ])
    })

    it('parses spanning UC "For each pair of X and Y"', () => {
      const result = parseConstraintText(
        'For each pair of Widget and Widget, that Widget targets that Widget at most once.'
      )
      expect(result).toEqual([
        { kind: 'UC', modality: 'Alethic', nouns: ['Widget', 'Widget'] },
      ])
    })

    it('parses ternary UC "For each combination of X and Y"', () => {
      const result = parseConstraintText(
        'For each combination of Plan and Interval, that Plan has at most one Price per that Interval.'
      )
      expect(result).toEqual([
        { kind: 'UC', modality: 'Alethic', nouns: ['Plan', 'Interval', 'Price'], constrainedNoun: 'Plan' },
      ])
    })
  })

  describe('mandatory constraints (MC)', () => {
    it('parses "Each X has at least one Y"', () => {
      const result = parseConstraintText('Each Organization has at least one Name.')
      expect(result).toEqual([
        { kind: 'MC', modality: 'Alethic', nouns: ['Organization', 'Name'], constrainedNoun: 'Organization' },
      ])
    })
  })

  describe('exactly one (UC + MC)', () => {
    it('parses "Each X has exactly one Y" into two constraints', () => {
      const result = parseConstraintText('Each Section has exactly one Position.')
      expect(result).toEqual([
        { kind: 'UC', modality: 'Alethic', nouns: ['Section', 'Position'], constrainedNoun: 'Section' },
        { kind: 'MC', modality: 'Alethic', nouns: ['Section', 'Position'], constrainedNoun: 'Section' },
      ])
    })
  })

  describe('ring constraints (irreflexive)', () => {
    it('parses "No X [verb] itself" as IR', () => {
      const result = parseConstraintText('No Widget targets itself.')
      expect(result).toEqual([
        { kind: 'IR', modality: 'Alethic', nouns: ['Widget'], constrainedNoun: 'Widget' },
      ])
    })
  })

  describe('deontic wrappers', () => {
    it('parses "It is obligatory that ..."', () => {
      const result = parseConstraintText(
        'It is obligatory that each Customer has at least one Name.'
      )
      expect(result).toEqual([
        { kind: 'MC', modality: 'Deontic', deonticOperator: 'obligatory', nouns: ['Customer', 'Name'], constrainedNoun: 'Customer' },
      ])
    })

    it('parses "It is forbidden that ..." with unrecognized inner', () => {
      const result = parseConstraintText(
        'It is forbidden that SupportResponse contains ProhibitedPunctuation.'
      )
      expect(result).toBeNull()
    })

    it('parses "It is permitted that ..." with unrecognized inner', () => {
      const result = parseConstraintText(
        'It is permitted that each SupportResponse offers Assistance.'
      )
      expect(result).toBeNull()
    })
  })

  describe('mandatory via "has some" (MC)', () => {
    it('parses "Each X has some Y"', () => {
      const result = parseConstraintText('Each Message has some Lead.')
      expect(result).toEqual([
        { kind: 'MC', modality: 'Alethic', nouns: ['Message', 'Lead'], constrainedNoun: 'Message' },
      ])
    })

    it('parses "Each X belongs to some Y"', () => {
      const result = parseConstraintText('Each Lead belongs to some SalesRep.')
      expect(result).toEqual([
        { kind: 'MC', modality: 'Alethic', nouns: ['Lead', 'SalesRep'], constrainedNoun: 'Lead' },
      ])
    })
  })

  describe('equality constraints (EQ)', () => {
    it('parses "if and only if" biconditional', () => {
      const result = parseConstraintText(
        'Message is matched if and only if Message has Lead.'
      )
      expect(result).toEqual([
        { kind: 'EQ', modality: 'Alethic', nouns: ['Message', 'Lead'] },
      ])
    })
  })

  describe('unrecognized patterns', () => {
    it('returns null for arbitrary text', () => {
      expect(parseConstraintText('This is not a constraint.')).toBeNull()
    })

    it('returns null for empty string', () => {
      expect(parseConstraintText('')).toBeNull()
    })
  })
})

describe('parseSetComparisonBlock', () => {
  it('parses XO (exactly one of the following holds)', () => {
    const block = `For each LeadMessageMatch, exactly one of the following holds:
  that LeadMessageMatch has MatchStatus 'Pending';
  that LeadMessageMatch has MatchStatus 'Confirmed';
  that LeadMessageMatch has MatchStatus 'Rejected'.`

    const result = parseSetComparisonBlock(block)
    expect(result).not.toBeNull()
    expect(result!.kind).toBe('XO')
    expect(result!.modality).toBe('Alethic')
    expect(result!.entity).toBe('LeadMessageMatch')
    expect(result!.clauses).toHaveLength(3)
    expect(result!.clauses![0]).toContain('Pending')
    expect(result!.clauses![1]).toContain('Confirmed')
    expect(result!.clauses![2]).toContain('Rejected')
    expect(result!.nouns).toContain('LeadMessageMatch')
    expect(result!.nouns).toContain('MatchStatus')
  })

  it('parses XC (at most one of the following holds)', () => {
    const block = `For each SalesRep, at most one of the following holds:
  that SalesRep is assigned Lead via Email;
  that SalesRep is assigned Lead via Phone.`

    const result = parseSetComparisonBlock(block)
    expect(result).not.toBeNull()
    expect(result!.kind).toBe('XC')
    expect(result!.entity).toBe('SalesRep')
    expect(result!.clauses).toHaveLength(2)
  })

  it('parses OR (at least one of the following holds)', () => {
    const block = `For each Message, at least one of the following holds:
  that Message has EmailAddress;
  that Message has PhoneNumber.`

    const result = parseSetComparisonBlock(block)
    expect(result).not.toBeNull()
    expect(result!.kind).toBe('OR')
    expect(result!.entity).toBe('Message')
    expect(result!.clauses).toHaveLength(2)
  })

  it('parses SS (If some ... then that ...)', () => {
    const block = `If some Message has Lead then that Message has SalesRep.`

    const result = parseSetComparisonBlock(block)
    expect(result).not.toBeNull()
    expect(result!.kind).toBe('SS')
    expect(result!.modality).toBe('Alethic')
    expect(result!.nouns).toContain('Message')
    expect(result!.nouns).toContain('Lead')
    expect(result!.nouns).toContain('SalesRep')
  })

  it('handles multi-word entity names', () => {
    const block = `For each Lead Message Match, exactly one of the following holds:
  that Lead Message Match is confirmed;
  that Lead Message Match is rejected.`

    const result = parseSetComparisonBlock(block)
    expect(result).not.toBeNull()
    expect(result!.kind).toBe('XO')
    expect(result!.entity).toBe('LeadMessageMatch')
  })

  it('returns null for non-set-comparison blocks', () => {
    expect(parseSetComparisonBlock('Customer has Name.')).toBeNull()
    expect(parseSetComparisonBlock('')).toBeNull()
    expect(parseSetComparisonBlock('Each Customer has at most one Name.')).toBeNull()
  })
})

describe('isInformationalPattern', () => {
  it('skips "It is possible that..."', () => {
    expect(isInformationalPattern('It is possible that some Customer has no Name.')).toBe(true)
  })

  it('skips "In each population of..."', () => {
    expect(isInformationalPattern('In each population of Customer, each Customer is identified by EmailAddress.')).toBe(true)
  })

  it('skips "Data Type:" declarations', () => {
    expect(isInformationalPattern('Data Type: String')).toBe(true)
  })

  it('skips "Reference Scheme:" declarations', () => {
    expect(isInformationalPattern('Reference Scheme: Customer(.EmailAddress)')).toBe(true)
  })

  it('skips "Reference Mode:" declarations', () => {
    expect(isInformationalPattern('Reference Mode: .EmailAddress')).toBe(true)
  })

  it('skips entity type declarations', () => {
    expect(isInformationalPattern('Customer is an entity type.')).toBe(true)
  })

  it('skips value type declarations', () => {
    expect(isInformationalPattern('Name is a value type.')).toBe(true)
  })

  it('skips markdown headers', () => {
    expect(isInformationalPattern('## Fact Types')).toBe(true)
  })

  it('skips "Fact Types:" section headers', () => {
    expect(isInformationalPattern('Fact Types:')).toBe(true)
  })

  it('returns true for empty/whitespace', () => {
    expect(isInformationalPattern('')).toBe(true)
    expect(isInformationalPattern('   ')).toBe(true)
  })

  it('does not skip actual readings', () => {
    expect(isInformationalPattern('Customer has Name')).toBe(false)
    expect(isInformationalPattern('Each Customer has at most one Name')).toBe(false)
  })

  it('does not skip constraint patterns', () => {
    expect(isInformationalPattern('For each Message, exactly one of the following holds:')).toBe(false)
  })
})

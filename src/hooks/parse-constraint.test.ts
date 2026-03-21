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

  describe('ring constraints', () => {
    it('parses "No X [verb] itself" as IR (irreflexive)', () => {
      const result = parseConstraintText('No Widget targets itself.')
      expect(result).toEqual([
        { kind: 'IR', modality: 'Alethic', nouns: ['Widget'], constrainedNoun: 'Widget' },
      ])
    })

    it('parses "No Person is a parent of themselves" as IR', () => {
      const result = parseConstraintText('No Person is a parent of itself.')
      expect(result).toEqual([
        { kind: 'IR', modality: 'Alethic', nouns: ['Person'], constrainedNoun: 'Person' },
      ])
    })

    it('parses symmetric: "If X1 verb X2, then X2 verb X1"', () => {
      const result = parseConstraintText('If Person1 is married to Person2, then Person2 is married to Person1.')
      expect(result).toEqual([
        { kind: 'SY', modality: 'Alethic', nouns: ['Person'], constrainedNoun: 'Person' },
      ])
    })

    it('parses asymmetric: "If X1 verb X2, then X2 is not verb X1"', () => {
      const result = parseConstraintText('If Person1 is parent of Person2, then Person2 is not parent of Person1.')
      expect(result).toEqual([
        { kind: 'AS', modality: 'Alethic', nouns: ['Person'], constrainedNoun: 'Person' },
      ])
    })

    it('parses transitive: "If X1 verb X2 and X2 verb X3, then X1 verb X3"', () => {
      const result = parseConstraintText('If Person1 is ancestor of Person2 and Person2 is ancestor of Person3, then Person1 is ancestor of Person3.')
      expect(result).toEqual([
        { kind: 'TR', modality: 'Alethic', nouns: ['Person'], constrainedNoun: 'Person' },
      ])
    })

    it('parses intransitive: "If X1 verb X2 and X2 verb X3, then X1 is not verb X3"', () => {
      const result = parseConstraintText('If Person1 is parent of Person2 and Person2 is parent of Person3, then Person1 is not parent of Person3.')
      expect(result).toEqual([
        { kind: 'IT', modality: 'Alethic', nouns: ['Person'], constrainedNoun: 'Person' },
      ])
    })

    it('parses acyclic as IR + AS (irreflexive + asymmetric)', () => {
      // Acyclic is expressed as two separate constraints in FORML2
      const ir = parseConstraintText('No Noun is subtype of itself.')
      expect(ir).toEqual([
        { kind: 'IR', modality: 'Alethic', nouns: ['Noun'], constrainedNoun: 'Noun' },
      ])
      const as_ = parseConstraintText('If Noun1 is subtype of Noun2, then Noun2 is not subtype of Noun1.')
      expect(as_).toEqual([
        { kind: 'AS', modality: 'Alethic', nouns: ['Noun'], constrainedNoun: 'Noun' },
      ])
    })

    it('parses symmetric + irreflexive as separate constraints', () => {
      const sy = parseConstraintText('If Person1 is sibling of Person2, then Person2 is sibling of Person1.')
      expect(sy).toEqual([
        { kind: 'SY', modality: 'Alethic', nouns: ['Person'], constrainedNoun: 'Person' },
      ])
      const ir = parseConstraintText('No Person is sibling of itself.')
      expect(ir).toEqual([
        { kind: 'IR', modality: 'Alethic', nouns: ['Person'], constrainedNoun: 'Person' },
      ])
    })

    it('parses asymmetric + intransitive as separate constraints', () => {
      const as_ = parseConstraintText('If Person1 is parent of Person2, then Person2 is not parent of Person1.')
      expect(as_).toEqual([
        { kind: 'AS', modality: 'Alethic', nouns: ['Person'], constrainedNoun: 'Person' },
      ])
      const it_ = parseConstraintText('If Person1 is parent of Person2 and Person2 is parent of Person3, then Person1 is not parent of Person3.')
      expect(it_).toEqual([
        { kind: 'IT', modality: 'Alethic', nouns: ['Person'], constrainedNoun: 'Person' },
      ])
    })

    it('parses symmetric + intransitive as separate constraints', () => {
      const sy = parseConstraintText('If Person1 is friend of Person2, then Person2 is friend of Person1.')
      expect(sy).toEqual([
        { kind: 'SY', modality: 'Alethic', nouns: ['Person'], constrainedNoun: 'Person' },
      ])
      const it_ = parseConstraintText('If Person1 is friend of Person2 and Person2 is friend of Person3, then Person1 is not friend of Person3.')
      expect(it_).toEqual([
        { kind: 'IT', modality: 'Alethic', nouns: ['Person'], constrainedNoun: 'Person' },
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

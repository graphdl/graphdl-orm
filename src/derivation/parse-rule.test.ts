import { describe, it, expect } from 'vitest'
import { parseRule, type DerivationRule } from './parse-rule'

// Known nouns used across tests
const graphNouns = [
  'Graph', 'Resource', 'Role', 'Reading', 'Noun', 'Layer',
  'Graph Schema', 'Arity',
  'Layer State', 'Valence', 'Arousal', 'Affect Region',
  'Domain',
]

describe('parseRule', () => {
  describe('join-chain rules', () => {
    it('parses a 4-conjunct join with qualifier', () => {
      const text =
        'Graph stimulates Layer := Graph uses Resource for Role and Role belongs to Reading and Reading references Noun and Layer owns Noun.'
      const rule = parseRule(text, graphNouns)

      expect(rule.kind).toBe('join')
      expect(rule.text).toBe(text)

      // Consequent: Graph stimulates Layer
      expect(rule.consequent.subject).toBe('Graph')
      expect(rule.consequent.predicate).toBe('stimulates')
      expect(rule.consequent.object).toBe('Layer')

      // 4 antecedents
      expect(rule.antecedents).toHaveLength(4)

      // First antecedent: Graph uses Resource for Role (ternary with qualifier)
      expect(rule.antecedents[0].subject).toBe('Graph')
      expect(rule.antecedents[0].predicate).toBe('uses')
      expect(rule.antecedents[0].object).toBe('Resource')
      expect(rule.antecedents[0].qualifier).toEqual({
        predicate: 'for',
        object: 'Role',
      })

      // Second: Role belongs to Reading
      expect(rule.antecedents[1].subject).toBe('Role')
      expect(rule.antecedents[1].predicate).toBe('belongs to')
      expect(rule.antecedents[1].object).toBe('Reading')

      // Third: Reading references Noun
      expect(rule.antecedents[2].subject).toBe('Reading')
      expect(rule.antecedents[2].predicate).toBe('references')
      expect(rule.antecedents[2].object).toBe('Noun')

      // Fourth: Layer owns Noun
      expect(rule.antecedents[3].subject).toBe('Layer')
      expect(rule.antecedents[3].predicate).toBe('owns')
      expect(rule.antecedents[3].object).toBe('Noun')
    })
  })

  describe('value-comparison rules', () => {
    it('parses comparisons and literal value in consequent', () => {
      const text =
        "Layer State has Affect Region 'Excited' := Layer State has Valence > 0.3 and Layer State has Arousal > 0.3."
      const rule = parseRule(text, graphNouns)

      expect(rule.kind).toBe('comparison')

      // Consequent: Layer State has Affect Region with literal 'Excited'
      expect(rule.consequent.subject).toBe('Layer State')
      expect(rule.consequent.predicate).toBe('has')
      expect(rule.consequent.object).toBe('Affect Region')
      expect(rule.consequent.literalValue).toBe('Excited')

      // 2 antecedents with comparisons
      expect(rule.antecedents).toHaveLength(2)

      expect(rule.antecedents[0].subject).toBe('Layer State')
      expect(rule.antecedents[0].predicate).toBe('has')
      expect(rule.antecedents[0].object).toBe('Valence')
      expect(rule.antecedents[0].comparison).toEqual({ op: '>', value: 0.3 })

      expect(rule.antecedents[1].subject).toBe('Layer State')
      expect(rule.antecedents[1].predicate).toBe('has')
      expect(rule.antecedents[1].object).toBe('Arousal')
      expect(rule.antecedents[1].comparison).toEqual({ op: '>', value: 0.3 })
    })
  })

  describe('identity rules', () => {
    it('parses "the same" identity form', () => {
      const text =
        'Domain is visible to Domain := that Domain is the same Domain.'
      const rule = parseRule(text, graphNouns)

      expect(rule.kind).toBe('identity')

      expect(rule.consequent.subject).toBe('Domain')
      expect(rule.consequent.predicate).toBe('is visible to')
      expect(rule.consequent.object).toBe('Domain')

      // Identity rules have one antecedent expressing the identity
      expect(rule.antecedents).toHaveLength(1)
      expect(rule.antecedents[0].subject).toBe('Domain')
      expect(rule.antecedents[0].predicate).toBe('is the same')
      expect(rule.antecedents[0].object).toBe('Domain')
    })
  })

  describe('aggregate rules', () => {
    it('parses count-of aggregate form', () => {
      const text =
        'Graph Schema has Arity := count of Role where Graph Schema has Role.'
      const rule = parseRule(text, graphNouns)

      expect(rule.kind).toBe('aggregate')

      expect(rule.consequent.subject).toBe('Graph Schema')
      expect(rule.consequent.predicate).toBe('has')
      expect(rule.consequent.object).toBe('Arity')

      expect(rule.aggregate).toEqual({ fn: 'count', noun: 'Role' })

      // The "where" clause is the antecedent
      expect(rule.antecedents).toHaveLength(1)
      expect(rule.antecedents[0].subject).toBe('Graph Schema')
      expect(rule.antecedents[0].predicate).toBe('has')
      expect(rule.antecedents[0].object).toBe('Role')
    })
  })

  describe('noun masking for "and" in noun names', () => {
    it('does not split on "and" inside a noun name', () => {
      const nouns = ['Supply and Demand', 'Market', 'Equilibrium']
      const text =
        'Market has Equilibrium := Market has Supply and Demand.'
      const rule = parseRule(text, nouns)

      expect(rule.kind).toBe('join')
      expect(rule.antecedents).toHaveLength(1)
      expect(rule.antecedents[0].subject).toBe('Market')
      expect(rule.antecedents[0].predicate).toBe('has')
      expect(rule.antecedents[0].object).toBe('Supply and Demand')
    })
  })
})

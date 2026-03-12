import { describe, it, expect } from 'vitest'
import { parseConstraintText } from './parse-constraint'

describe('parseConstraintText', () => {
  describe('uniqueness constraints (UC)', () => {
    it('parses "Each X has at most one Y"', () => {
      const result = parseConstraintText('Each Customer has at most one Name.')
      expect(result).toEqual([
        { kind: 'UC', modality: 'Alethic', nouns: ['Customer', 'Name'] },
      ])
    })

    it('parses "Each X belongs to at most one Y"', () => {
      const result = parseConstraintText('Each Domain belongs to at most one Organization.')
      expect(result).toEqual([
        { kind: 'UC', modality: 'Alethic', nouns: ['Domain', 'Organization'] },
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
        { kind: 'UC', modality: 'Alethic', nouns: ['Plan', 'Interval', 'Price'] },
      ])
    })
  })

  describe('mandatory constraints (MC)', () => {
    it('parses "Each X has at least one Y"', () => {
      const result = parseConstraintText('Each Organization has at least one Name.')
      expect(result).toEqual([
        { kind: 'MC', modality: 'Alethic', nouns: ['Organization', 'Name'] },
      ])
    })
  })

  describe('exactly one (UC + MC)', () => {
    it('parses "Each X has exactly one Y" into two constraints', () => {
      const result = parseConstraintText('Each Section has exactly one Position.')
      expect(result).toEqual([
        { kind: 'UC', modality: 'Alethic', nouns: ['Section', 'Position'] },
        { kind: 'MC', modality: 'Alethic', nouns: ['Section', 'Position'] },
      ])
    })
  })

  describe('ring constraints (RC)', () => {
    it('parses "No X [verb] itself"', () => {
      const result = parseConstraintText('No Widget targets itself.')
      expect(result).toEqual([
        { kind: 'RC', modality: 'Alethic', nouns: ['Widget'] },
      ])
    })
  })

  describe('deontic wrappers', () => {
    it('parses "It is obligatory that ..."', () => {
      const result = parseConstraintText(
        'It is obligatory that each Customer has at least one Name.'
      )
      expect(result).toEqual([
        { kind: 'MC', modality: 'Deontic', deonticOperator: 'obligatory', nouns: ['Customer', 'Name'] },
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

  describe('unrecognized patterns', () => {
    it('returns null for arbitrary text', () => {
      expect(parseConstraintText('This is not a constraint.')).toBeNull()
    })

    it('returns null for empty string', () => {
      expect(parseConstraintText('')).toBeNull()
    })
  })
})

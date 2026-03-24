/**
 * Constraint parser tests.
 *
 * NORMA-sourced verbalizations from the University academic model (Halpin white paper)
 * are the primary test fixtures. Additional tests cover edge cases and the
 * isInformationalPattern utility.
 *
 * See parse-constraint.norma.test.ts for the full NORMA verbalization suite.
 */
import { describe, it, expect } from 'vitest'
import { parseConstraintText, parseSetComparisonBlock, isInformationalPattern } from './parse-constraint'

// ── Edge cases and structural tests ──────────────────────────────────

describe('parseConstraintText — edge cases', () => {
  it('returns null for arbitrary text', () => {
    expect(parseConstraintText('This is not a constraint.')).toBeNull()
  })

  it('returns null for empty string', () => {
    expect(parseConstraintText('')).toBeNull()
  })

  it('returns null for whitespace', () => {
    expect(parseConstraintText('   ')).toBeNull()
  })

  it('strips trailing period before matching', () => {
    const withPeriod = parseConstraintText('Each Academic has at most one Room.')
    const without = parseConstraintText('Each Academic has at most one Room')
    expect(withPeriod).toEqual(without)
  })

  it('deontic wrapper with unrecognized inner returns null', () => {
    const result = parseConstraintText(
      'It is forbidden that SupportResponse contains ProhibitedPunctuation.'
    )
    expect(result).toBeNull()
  })

  it('deontic wrapper capitalizes inner text for matching', () => {
    // "It is obligatory that each X..." → inner "each X..." → capitalized "Each X..."
    const result = parseConstraintText(
      'It is obligatory that each Academic has at least one Rank.'
    )
    expect(result).not.toBeNull()
    expect(result![0].kind).toBe('MC')
    expect(result![0].modality).toBe('Deontic')
    expect(result![0].deonticOperator).toBe('obligatory')
  })

  it('equality biconditional: "if and only if"', () => {
    // From Halpin Ch 6.4: equality constraint
    const result = parseConstraintText(
      'Academic has Rank if and only if Academic works for Department.'
    )
    expect(result).not.toBeNull()
    expect(result![0].kind).toBe('EQ')
  })
})

// ── Set comparison blocks ────────────────────────────────────────────

describe('parseSetComparisonBlock — structural tests', () => {
  it('handles multi-word entity names in XO', () => {
    // Multi-word entity names must be recognized and collapsed to PascalCase
    const block = `For each State Machine Definition, exactly one of the following holds:
  that State Machine Definition is active;
  that State Machine Definition is archived.`

    const result = parseSetComparisonBlock(block)
    expect(result).not.toBeNull()
    expect(result!.kind).toBe('XO')
    expect(result!.entity).toBe('StateMachineDefinition')
  })

  it('returns null for non-set-comparison text', () => {
    expect(parseSetComparisonBlock('Academic has Rank.')).toBeNull()
    expect(parseSetComparisonBlock('')).toBeNull()
    expect(parseSetComparisonBlock('Each Academic has at most one Rank.')).toBeNull()
  })

  it('parses XC (exclusion) block', () => {
    // From Halpin: exclusion between role populations
    const block = `For each Academic, at most one of the following holds:
  that Academic is tenured;
  that Academic is contracted.`

    const result = parseSetComparisonBlock(block)
    expect(result).not.toBeNull()
    expect(result!.kind).toBe('XC')
    expect(result!.clauses).toHaveLength(2)
  })

  it('parses OR (inclusive-or / disjunctive mandatory) block', () => {
    // From Halpin: disjunctive mandatory across fact types
    const block = `For each Academic, at least one of the following holds:
  that Academic is tenured;
  that Academic is contracted until some Date.`

    const result = parseSetComparisonBlock(block)
    expect(result).not.toBeNull()
    expect(result!.kind).toBe('OR')
    expect(result!.clauses).toHaveLength(2)
  })
})

// ── Informational pattern detection ──────────────────────────────────

describe('isInformationalPattern', () => {
  it('skips "It is possible that..."', () => {
    // NORMA default verbalization for absence of UC on a role
    expect(isInformationalPattern(
      'It is possible that more than one Academic works for the same Department.'
    )).toBe(true)
  })

  it('skips "In each population of..."', () => {
    expect(isInformationalPattern(
      'In each population of Academic is tenured, each Academic occurs at most once.'
    )).toBe(true)
  })

  it('skips "Data Type:" declarations', () => {
    expect(isInformationalPattern('Data Type: Text: Variable Length (0).')).toBe(true)
  })

  it('skips "Reference Scheme:" declarations', () => {
    expect(isInformationalPattern('Reference Scheme: Academic has Academic_empNr.')).toBe(true)
  })

  it('skips "Reference Mode:" declarations', () => {
    expect(isInformationalPattern('Reference Mode: .empNr.')).toBe(true)
  })

  it('skips entity type declarations', () => {
    expect(isInformationalPattern('Academic is an entity type.')).toBe(true)
  })

  it('skips value type declarations', () => {
    expect(isInformationalPattern('EmpName is a value type.')).toBe(true)
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
    expect(isInformationalPattern('Academic has Rank')).toBe(false)
    expect(isInformationalPattern('Each Academic has at most one Rank')).toBe(false)
  })

  it('does not skip constraint patterns', () => {
    expect(isInformationalPattern('For each Academic, exactly one of the following holds:')).toBe(false)
  })
})

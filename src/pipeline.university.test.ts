/**
 * University academic model — end-to-end integration test.
 *
 * Source: Halpin ORM white paper, built in NORMA.
 * Tests the full pipeline: parse → claims → verify metamodel.
 */
import { describe, it, expect } from 'vitest'
import { parseFORML2 } from './api/parse'

const UNIVERSITY_MODEL = `
# University Academic Model

## Entity Types

Academic(.empNr) is an entity type.
Department(.name) is an entity type.
Subject(.code) is an entity type.
Rank(.code) is an entity type.
Room(.roomNr) is an entity type.
Degree(.code) is an entity type.
University(.code) is an entity type.
Committee(.name) is an entity type.
Chair(.name) is an entity type.
Rating(.nr) is an entity type.
Activity(.name) is an entity type.

## Value Types

EmpName is a value type.
MoneyAmt is a value type.
Date is a value type.

## Fact Types

Academic has EmpName.
Academic works for Department.
Academic has Rank.
Academic holds Chair.
Academic occupies Room.
Academic teaches Subject.
Academic obtained Degree from University.
Academic serves on Committee.
Academic audits Academic.
Academic heads Department.
Academic is contracted until Date.
Academic is tenured.
Teaching gets Rating.
Department has for Activity a budget of MoneyAmt.

## Constraints

Each Academic has at most one EmpName.
For each EmpName, at most one Academic has that EmpName.
Each Academic works for exactly one Department.
Each Academic has exactly one Rank.
Each Academic holds at most one Chair.
For each Chair, at most one Academic holds that Chair.
Each Academic occupies at most one Room.
Each Academic heads at most one Department.
For each Department, at most one Academic heads that Department.
Each Academic is contracted until at most one Date.
Each Teaching gets at most one Rating.
No Academic audits the same Academic.
If some Academic heads some Department then that Academic works for that Department.
The possible values of Rank are 'P', 'SL', 'L'.

For each Academic, exactly one of the following holds:
  that Academic is tenured;
  that Academic is contracted until some Date.

This association with Academic, Subject provides the preferred identification scheme for Teaching.
This association with Academic, Degree provides the preferred identification scheme for AcademicObtainedDegreeFromUniversity.
This association with Academic, Committee provides the preferred identification scheme for AcademicServesOnCommittee.
This association with Department, Activity provides the preferred identification scheme for DepartmentHasForActivityABudgetOfMoneyAmt.
`

describe('University academic model — end-to-end', () => {
  const result = parseFORML2(UNIVERSITY_MODEL, [])

  it('parses all entity types', () => {
    const entityNames = result.nouns.filter(n => n.objectType === 'entity').map(n => n.name)
    expect(entityNames).toContain('Academic')
    expect(entityNames).toContain('Department')
    expect(entityNames).toContain('Subject')
    expect(entityNames).toContain('Rank')
    expect(entityNames).toContain('Room')
    expect(entityNames).toContain('Degree')
    expect(entityNames).toContain('University')
    expect(entityNames).toContain('Committee')
    expect(entityNames).toContain('Chair')
    expect(entityNames).toContain('Rating')
    expect(entityNames).toContain('Activity')
  })

  it('parses all value types', () => {
    const valueNames = result.nouns.filter(n => n.objectType === 'value').map(n => n.name)
    expect(valueNames).toContain('EmpName')
    expect(valueNames).toContain('MoneyAmt')
    expect(valueNames).toContain('Date')
  })

  it('parses reference schemes as implicit 1:1 mandatory binaries', () => {
    // Academic(.empNr) should generate "Academic has empNr" reading
    const empNrReading = result.readings.find(r => r.text === 'Academic has empNr')
    expect(empNrReading).toBeDefined()
    // With MC constraint
    const mcConstraint = result.constraints.find(c =>
      c.kind === 'MC' && c.reading === 'Academic has empNr'
    )
    expect(mcConstraint).toBeDefined()
  })

  it('parses all explicit readings', () => {
    expect(result.readings.find(r => r.text === 'Academic has EmpName')).toBeDefined()
    expect(result.readings.find(r => r.text === 'Academic works for Department')).toBeDefined()
    expect(result.readings.find(r => r.text === 'Academic has Rank')).toBeDefined()
    expect(result.readings.find(r => r.text === 'Academic holds Chair')).toBeDefined()
    expect(result.readings.find(r => r.text === 'Academic occupies Room')).toBeDefined()
    expect(result.readings.find(r => r.text === 'Academic teaches Subject')).toBeDefined()
    expect(result.readings.find(r => r.text === 'Academic obtained Degree from University')).toBeDefined()
    expect(result.readings.find(r => r.text === 'Academic serves on Committee')).toBeDefined()
    expect(result.readings.find(r => r.text === 'Academic audits Academic')).toBeDefined()
    expect(result.readings.find(r => r.text === 'Academic heads Department')).toBeDefined()
    expect(result.readings.find(r => r.text === 'Academic is contracted until Date')).toBeDefined()
    expect(result.readings.find(r => r.text === 'Academic is tenured')).toBeDefined()
    expect(result.readings.find(r => r.text === 'Teaching gets Rating')).toBeDefined()
    expect(result.readings.find(r => r.text === 'Department has for Activity a budget of MoneyAmt')).toBeDefined()
  })

  it('parses UC constraints', () => {
    const ucs = result.constraints.filter(c => c.kind === 'UC')
    // At least: EmpName UC, works for UC+MC, Rank UC+MC, Chair 2x, Room, heads 2x, contracted, Rating
    expect(ucs.length).toBeGreaterThanOrEqual(10)
  })

  it('parses MC constraints from "exactly one"', () => {
    const mcs = result.constraints.filter(c => c.kind === 'MC')
    // "works for exactly one" and "has exactly one Rank" each produce MC
    expect(mcs.length).toBeGreaterThanOrEqual(2)
  })

  it('parses inverse UC: For each EmpName, at most one Academic', () => {
    const inverseUC = result.constraints.find(c =>
      c.kind === 'UC' && c.text?.includes('For each EmpName')
    )
    expect(inverseUC).toBeDefined()
  })

  it('parses irreflexive ring constraint', () => {
    const ir = result.constraints.find(c =>
      c.kind === 'IR' && c.text?.includes('Academic audits')
    )
    expect(ir).toBeDefined()
  })

  it('parses subset constraint', () => {
    const ss = result.constraints.find(c =>
      c.kind === 'SS' && c.text?.includes('heads')
    )
    expect(ss).toBeDefined()
  })

  it('parses value constraint on Rank', () => {
    const rank = result.nouns.find(n => n.name === 'Rank')
    expect(rank).toBeDefined()
    expect(rank!.enumValues).toEqual(['P', 'SL', 'L'])
  })

  it('parses XO (exclusive-or) constraint', () => {
    const xo = result.constraints.find(c => c.kind === 'XO')
    expect(xo).toBeDefined()
    expect(xo!.clauses).toHaveLength(2)
    expect(xo!.clauses![0]).toContain('tenured')
    expect(xo!.clauses![1]).toContain('contracted')
  })

  it('parses objectification preferred identification', () => {
    const teaching = result.nouns.find(n => n.name === 'Teaching')
    expect(teaching).toBeDefined()
    expect(teaching!.refScheme).toEqual(['Academic', 'Subject'])
  })

  it('has good coverage', () => {
    expect(result.coverage).toBeGreaterThan(0.8)
    expect(result.warnings).toHaveLength(0)
  })
})

describe('Parser — inverse readings', () => {
  it('parses forward / inverse reading syntax', () => {
    const result = parseFORML2(`## Entity Types
Academic(.empNr) is an entity type.
Extension(.extNr) is an entity type.

## Fact Types
Academic uses Extension / Extension is used by Academic.`, [])

    // Both forward and inverse readings should be created
    const forward = result.readings.find(r => r.text === 'Academic uses Extension')
    const inverse = result.readings.find(r => r.text === 'Extension is used by Academic')
    expect(forward).toBeDefined()
    expect(inverse).toBeDefined()
  })
})

describe('Parser — formal subtype definitions', () => {
  it.todo('parses "each Teacher is an Academic who..." — lines not reaching SUBTYPE_DEFINITION regex, needs parser flow investigation')
})

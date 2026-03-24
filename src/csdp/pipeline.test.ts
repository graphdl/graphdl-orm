import { describe, it, expect } from 'vitest'
import { runCsdpPipeline, buildSchemaIR } from './pipeline'
import type { ExtractedClaims } from '../claims/ingest'

describe('CSDP pipeline', () => {
  it('rejects invalid schema with violations', () => {
    const claims: ExtractedClaims = {
      nouns: [
        { name: 'A', objectType: 'entity' },
        { name: 'B', objectType: 'entity' },
        { name: 'C', objectType: 'entity' },
      ],
      readings: [{ text: 'A has B for C', nouns: ['A', 'B', 'C'], predicate: 'has for' }],
      constraints: [{
        kind: 'UC',
        modality: 'Alethic',
        reading: 'A has B for C',
        roles: [0], // single-role UC on ternary — violates n-1 rule
      }],
      subtypes: [],
      transitions: [],
      facts: [],
    }
    const result = runCsdpPipeline(claims, 'test-domain')
    expect(result.valid).toBe(false)
    expect(result.violations.length).toBeGreaterThan(0)
    expect(result.violations.some(v => v.type === 'arity_violation')).toBe(true)
  })

  it('accepts valid schema and returns batch + tables', () => {
    const claims: ExtractedClaims = {
      nouns: [
        { name: 'Customer', objectType: 'entity' },
        { name: 'Name', objectType: 'value' },
      ],
      readings: [{ text: 'Customer has Name', nouns: ['Customer', 'Name'], predicate: 'has' }],
      constraints: [{
        kind: 'UC',
        modality: 'Alethic',
        reading: 'Customer has Name',
        roles: [0],
      }],
      subtypes: [],
      transitions: [],
      facts: [],
    }
    const result = runCsdpPipeline(claims, 'test-domain')
    expect(result.valid).toBe(true)
    expect(result.batch).toBeDefined()
    expect(result.batch!.entities.length).toBeGreaterThan(0)
    expect(result.batch!.domain).toBe('test-domain')
    expect(result.tables).toBeDefined()
    expect(result.tables!.length).toBeGreaterThan(0)
  })

  it('includes induced constraints when facts are provided', () => {
    const claims: ExtractedClaims = {
      nouns: [
        { name: 'Person', objectType: 'entity' },
        { name: 'Country', objectType: 'entity' },
      ],
      readings: [{ text: 'Person was born in Country', nouns: ['Person', 'Country'], predicate: 'was born in' }],
      constraints: [{
        kind: 'UC',
        modality: 'Alethic',
        reading: 'Person was born in Country',
        roles: [0],
      }],
      subtypes: [],
      transitions: [],
      facts: [
        { reading: 'Person was born in Country', values: [{ noun: 'Person', value: 'alice' }, { noun: 'Country', value: 'US' }] },
        { reading: 'Person was born in Country', values: [{ noun: 'Person', value: 'bob' }, { noun: 'Country', value: 'UK' }] },
        { reading: 'Person was born in Country', values: [{ noun: 'Person', value: 'carol' }, { noun: 'Country', value: 'CA' }] },
      ],
    }
    const result = runCsdpPipeline(claims, 'test-domain')
    expect(result.valid).toBe(true)
    expect(result.induced).toBeDefined()
    expect(result.induced!.populationStats.totalFacts).toBe(3)
  })

  it('batch contains nouns, readings, and constraints as entities', () => {
    const claims: ExtractedClaims = {
      nouns: [
        { name: 'Employee', objectType: 'entity' },
        { name: 'Salary', objectType: 'value' },
      ],
      readings: [{ text: 'Employee earns Salary', nouns: ['Employee', 'Salary'], predicate: 'earns' }],
      constraints: [
        { kind: 'UC', modality: 'Alethic', reading: 'Employee earns Salary', roles: [0] },
        { kind: 'MC', modality: 'Alethic', reading: 'Employee earns Salary', roles: [0] },
      ],
      subtypes: [],
      transitions: [],
      facts: [],
    }
    const result = runCsdpPipeline(claims, 'hr')
    expect(result.valid).toBe(true)
    const batch = result.batch!
    const nounEntities = batch.entities.filter(e => e.type === 'Noun')
    const readingEntities = batch.entities.filter(e => e.type === 'Reading')
    const constraintEntities = batch.entities.filter(e => e.type === 'Constraint')
    expect(nounEntities).toHaveLength(2)
    expect(readingEntities).toHaveLength(1)
    expect(constraintEntities).toHaveLength(2)
  })

  it('returns no induced field when no facts provided', () => {
    const claims: ExtractedClaims = {
      nouns: [
        { name: 'Book', objectType: 'entity' },
        { name: 'Title', objectType: 'value' },
      ],
      readings: [{ text: 'Book has Title', nouns: ['Book', 'Title'], predicate: 'has' }],
      constraints: [{ kind: 'UC', modality: 'Alethic', reading: 'Book has Title', roles: [0] }],
      subtypes: [],
      transitions: [],
      facts: [],
    }
    const result = runCsdpPipeline(claims, 'library')
    expect(result.valid).toBe(true)
    expect(result.induced).toBeUndefined()
  })

  it('maps subtypes from child/parent to subtype/supertype', () => {
    const claims: ExtractedClaims = {
      nouns: [
        { name: 'Person', objectType: 'entity' },
        { name: 'Male', objectType: 'entity' },
        { name: 'Female', objectType: 'entity' },
        { name: 'Name', objectType: 'value' },
      ],
      readings: [{ text: 'Person has Name', nouns: ['Person', 'Name'], predicate: 'has' }],
      constraints: [
        { kind: 'UC', modality: 'Alethic', reading: 'Person has Name', roles: [0] },
        { kind: 'XO', modality: 'Alethic', reading: '', roles: [], text: 'Person is totally divided into Male, Female' },
      ],
      subtypes: [
        { child: 'Male', parent: 'Person' },
        { child: 'Female', parent: 'Person' },
      ],
      transitions: [],
      facts: [],
    }
    const result = runCsdpPipeline(claims, 'test-domain')
    expect(result.valid).toBe(true)
    // Batch should contain Subtype entities
    const subtypeEntities = result.batch!.entities.filter(e => e.type === 'Subtype')
    expect(subtypeEntities).toHaveLength(2)
  })
})

describe('buildSchemaIR', () => {
  it('converts nouns and readings to SchemaIR format', () => {
    const claims: ExtractedClaims = {
      nouns: [
        { name: 'Customer', objectType: 'entity' },
        { name: 'Name', objectType: 'value' },
      ],
      readings: [{ text: 'Customer has Name', nouns: ['Customer', 'Name'], predicate: 'has' }],
      constraints: [{ kind: 'UC', modality: 'Alethic', reading: 'Customer has Name', roles: [0] }],
    }
    const ir = buildSchemaIR(claims)
    expect(ir.nouns).toHaveLength(2)
    expect(ir.factTypes).toHaveLength(1)
    expect(ir.factTypes[0].roles).toHaveLength(2)
    expect(ir.factTypes[0].roles[0].nounName).toBe('Customer')
    expect(ir.factTypes[0].roles[1].nounName).toBe('Name')
    expect(ir.constraints).toHaveLength(1)
    expect(ir.constraints[0].factTypeId).toBe(ir.factTypes[0].id)
  })

  it('extracts roles from reading text when nouns array is absent', () => {
    const claims: ExtractedClaims = {
      nouns: [
        { name: 'Person', objectType: 'entity' },
        { name: 'Country', objectType: 'entity' },
      ],
      readings: [{ text: 'Person was born in Country', predicate: 'was born in' }],
      constraints: [],
    }
    const ir = buildSchemaIR(claims)
    expect(ir.factTypes[0].roles).toHaveLength(2)
    const nounNames = ir.factTypes[0].roles.map(r => r.nounName).sort()
    expect(nounNames).toEqual(['Country', 'Person'])
  })

  it('maps subtypes from child/parent to subtype/supertype', () => {
    const claims: ExtractedClaims = {
      nouns: [
        { name: 'Animal', objectType: 'entity' },
        { name: 'Dog', objectType: 'entity' },
      ],
      readings: [],
      constraints: [],
      subtypes: [{ child: 'Dog', parent: 'Animal' }],
    }
    const ir = buildSchemaIR(claims)
    expect(ir.subtypes).toBeDefined()
    expect(ir.subtypes![0].subtype).toBe('Dog')
    expect(ir.subtypes![0].supertype).toBe('Animal')
  })

  it('generates stable fact type IDs from reading text', () => {
    const claims: ExtractedClaims = {
      nouns: [
        { name: 'X', objectType: 'entity' },
        { name: 'Y', objectType: 'value' },
      ],
      readings: [{ text: 'X has Y', nouns: ['X', 'Y'], predicate: 'has' }],
      constraints: [],
    }
    const ir1 = buildSchemaIR(claims)
    const ir2 = buildSchemaIR(claims)
    expect(ir1.factTypes[0].id).toBe(ir2.factTypes[0].id)
  })
})

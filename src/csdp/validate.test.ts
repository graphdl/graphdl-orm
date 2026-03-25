import { describe, it, expect } from 'vitest'
import { validateCsdp } from './validate'

describe('CSDP Step 4: arity check', () => {
  it('rejects ternary with UC spanning < n-1 roles', () => {
    const schema = {
      nouns: [
        { name: 'A', objectType: 'entity' },
        { name: 'B', objectType: 'entity' },
        { name: 'C', objectType: 'entity' },
      ],
      factTypes: [{
        id: 'ft1',
        reading: 'A has B for C',
        roles: [
          { nounName: 'A', roleIndex: 0 },
          { nounName: 'B', roleIndex: 1 },
          { nounName: 'C', roleIndex: 2 },
        ],
      }],
      constraints: [{
        kind: 'UC',
        factTypeId: 'ft1',
        roles: [0], // single-role UC on ternary — violates n-1 rule
      }],
    }
    const result = validateCsdp(schema)
    expect(result.valid).toBe(false)
    expect(result.violations).toHaveLength(1)
    expect(result.violations[0].type).toBe('arity_violation')
    expect(result.violations[0].fix).toBeDefined()
  })

  it('accepts ternary with UC spanning n-1 roles', () => {
    const schema = {
      nouns: [
        { name: 'Student', objectType: 'entity' },
        { name: 'Course', objectType: 'entity' },
        { name: 'Rating', objectType: 'value' },
      ],
      factTypes: [{
        id: 'ft1',
        reading: 'Student scored Rating for Course',
        roles: [
          { nounName: 'Student', roleIndex: 0 },
          { nounName: 'Rating', roleIndex: 1 },
          { nounName: 'Course', roleIndex: 2 },
        ],
      }],
      constraints: [{
        kind: 'UC',
        factTypeId: 'ft1',
        roles: [0, 2], // spans 2 of 3 roles = n-1, valid
      }],
    }
    const result = validateCsdp(schema)
    expect(result.valid).toBe(true)
  })

  it('accepts binary with single-role UC', () => {
    const schema = {
      nouns: [
        { name: 'Person', objectType: 'entity' },
        { name: 'Name', objectType: 'value' },
      ],
      factTypes: [{
        id: 'ft1',
        reading: 'Person has Name',
        roles: [
          { nounName: 'Person', roleIndex: 0 },
          { nounName: 'Name', roleIndex: 1 },
        ],
      }],
      constraints: [{
        kind: 'UC',
        factTypeId: 'ft1',
        roles: [0], // spans 1 of 2 = n-1, valid for binary
      }],
    }
    const result = validateCsdp(schema)
    expect(result.valid).toBe(true)
  })
})

describe('CSDP Step 1: undeclared noun check', () => {
  it('rejects constraint referencing undeclared noun', () => {
    const schema = {
      nouns: [{ name: 'Person', objectType: 'entity' }],
      factTypes: [{
        id: 'ft1', reading: 'Person has Name',
        roles: [{ nounName: 'Person', roleIndex: 0 }, { nounName: 'Name', roleIndex: 1 }],
      }],
      constraints: [],
    }
    const result = validateCsdp(schema)
    expect(result.violations.some(v => v.type === 'undeclared_noun')).toBe(true)
  })

  it('accepts fact type where all nouns are declared', () => {
    const schema = {
      nouns: [
        { name: 'Person', objectType: 'entity' },
        { name: 'Name', objectType: 'value' },
      ],
      factTypes: [{
        id: 'ft1', reading: 'Person has Name',
        roles: [{ nounName: 'Person', roleIndex: 0 }, { nounName: 'Name', roleIndex: 1 }],
      }],
      constraints: [],
    }
    const result = validateCsdp(schema)
    expect(result.violations.some(v => v.type === 'undeclared_noun')).toBe(false)
  })
})

describe('CSDP Step 1: non-elementary fact check', () => {
  it('flags reading with potential non-elementary conjunction', () => {
    const schema = {
      nouns: [
        { name: 'Customer', objectType: 'entity' },
        { name: 'Name', objectType: 'value' },
        { name: 'Request', objectType: 'entity' },
      ],
      factTypes: [{
        id: 'ft1', reading: 'Customer has Name and submits Request',
        roles: [
          { nounName: 'Customer', roleIndex: 0 },
          { nounName: 'Name', roleIndex: 1 },
          { nounName: 'Request', roleIndex: 2 },
        ],
      }],
      constraints: [],
    }
    const result = validateCsdp(schema)
    expect(result.violations.some(v => v.type === 'non_elementary_fact')).toBe(true)
  })

  it('accepts reading without conjunction', () => {
    const schema = {
      nouns: [
        { name: 'Person', objectType: 'entity' },
        { name: 'Name', objectType: 'value' },
      ],
      factTypes: [{
        id: 'ft1', reading: 'Person has Name',
        roles: [
          { nounName: 'Person', roleIndex: 0 },
          { nounName: 'Name', roleIndex: 1 },
        ],
      }],
      constraints: [],
    }
    const result = validateCsdp(schema)
    expect(result.violations.some(v => v.type === 'non_elementary_fact')).toBe(false)
  })

  it('does not flag "and" within a noun name', () => {
    const schema = {
      nouns: [
        { name: 'Research and Development', objectType: 'entity' },
        { name: 'Budget', objectType: 'value' },
      ],
      factTypes: [{
        id: 'ft1', reading: 'Research and Development has Budget',
        roles: [
          { nounName: 'Research and Development', roleIndex: 0 },
          { nounName: 'Budget', roleIndex: 1 },
        ],
      }],
      constraints: [],
    }
    const result = validateCsdp(schema)
    expect(result.violations.some(v => v.type === 'non_elementary_fact')).toBe(false)
  })
})

describe('CSDP Step 6: missing subtype constraint', () => {
  it('flags subtypes without totality or exclusion constraint', () => {
    const schema = {
      nouns: [
        { name: 'Person', objectType: 'entity' },
        { name: 'Male', objectType: 'entity' },
        { name: 'Female', objectType: 'entity' },
      ],
      factTypes: [],
      constraints: [],
      subtypes: [
        { subtype: 'Male', supertype: 'Person' },
        { subtype: 'Female', supertype: 'Person' },
      ],
    }
    const result = validateCsdp(schema)
    expect(result.violations.some(v => v.type === 'missing_subtype_constraint')).toBe(true)
  })

  it('accepts subtypes with totality constraint', () => {
    const schema = {
      nouns: [
        { name: 'Person', objectType: 'entity' },
        { name: 'Male', objectType: 'entity' },
        { name: 'Female', objectType: 'entity' },
      ],
      factTypes: [],
      constraints: [{
        kind: 'XO',
        factTypeId: '',
        roles: [],
        text: 'Person is totally divided into Male, Female',
      }],
      subtypes: [
        { subtype: 'Male', supertype: 'Person' },
        { subtype: 'Female', supertype: 'Person' },
      ],
    }
    const result = validateCsdp(schema)
    expect(result.violations.some(v => v.type === 'missing_subtype_constraint')).toBe(false)
  })
})

describe('CSDP Step 7: missing ring constraint', () => {
  it('flags self-referential binary without ring constraint', () => {
    const schema = {
      nouns: [{ name: 'Person', objectType: 'entity' }],
      factTypes: [{
        id: 'ft1', reading: 'Person is parent of Person',
        roles: [{ nounName: 'Person', roleIndex: 0 }, { nounName: 'Person', roleIndex: 1 }],
      }],
      constraints: [],
    }
    const result = validateCsdp(schema)
    expect(result.violations.some(v => v.type === 'missing_ring_constraint')).toBe(true)
  })

  it('accepts self-referential binary with ring constraint', () => {
    const schema = {
      nouns: [{ name: 'Person', objectType: 'entity' }],
      factTypes: [{
        id: 'ft1', reading: 'Person is parent of Person',
        roles: [{ nounName: 'Person', roleIndex: 0 }, { nounName: 'Person', roleIndex: 1 }],
      }],
      constraints: [{
        kind: 'IR',
        factTypeId: 'ft1',
        roles: [0, 1],
      }],
    }
    const result = validateCsdp(schema)
    expect(result.violations.some(v => v.type === 'missing_ring_constraint')).toBe(false)
  })

  it('does not flag binary with different noun types', () => {
    const schema = {
      nouns: [
        { name: 'Person', objectType: 'entity' },
        { name: 'Name', objectType: 'value' },
      ],
      factTypes: [{
        id: 'ft1', reading: 'Person has Name',
        roles: [{ nounName: 'Person', roleIndex: 0 }, { nounName: 'Name', roleIndex: 1 }],
      }],
      constraints: [],
    }
    const result = validateCsdp(schema)
    expect(result.violations.some(v => v.type === 'missing_ring_constraint')).toBe(false)
  })
})

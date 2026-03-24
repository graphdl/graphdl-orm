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

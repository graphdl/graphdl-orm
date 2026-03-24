import { describe, it, expect, vi, beforeEach } from 'vitest'
import { induceConstraints, type InductionResult, type InducedConstraint, type InducedRule } from './induce'

describe('induceConstraints', () => {
  it('returns induced constraints from population', () => {
    const ir = {
      nouns: {
        Person: { worldAssumption: 'closed' },
        Name: { worldAssumption: 'closed' },
      },
      factTypes: {
        ft1: {
          reading: 'Person has Name',
          roles: [{ nounName: 'Person' }, { nounName: 'Name' }],
        },
      },
      constraints: [],
    }
    const population = {
      facts: {
        ft1: [
          { bindings: [['Person', 'Alice'], ['Name', 'Smith']] },
          { bindings: [['Person', 'Bob'], ['Name', 'Jones']] },
          { bindings: [['Person', 'Carol'], ['Name', 'Lee']] },
        ],
      },
    }

    const result = induceConstraints(JSON.stringify(ir), JSON.stringify(population))

    expect(result).toBeDefined()
    expect(result.constraints).toBeInstanceOf(Array)
    expect(result.rules).toBeInstanceOf(Array)
    expect(result.populationStats).toBeDefined()
    expect(result.populationStats.factTypeCount).toBeTypeOf('number')
    expect(result.populationStats.totalFacts).toBeTypeOf('number')
    expect(result.populationStats.entityCount).toBeTypeOf('number')
  })

  it('returns empty result for empty population', () => {
    const ir = {
      nouns: {},
      factTypes: {},
      constraints: [],
    }
    const population = { facts: {} }

    const result = induceConstraints(JSON.stringify(ir), JSON.stringify(population))

    expect(result.constraints).toEqual([])
    expect(result.rules).toEqual([])
    expect(result.populationStats.factTypeCount).toBe(0)
    expect(result.populationStats.totalFacts).toBe(0)
    expect(result.populationStats.entityCount).toBe(0)
  })

  it('includes confidence scores on induced constraints', () => {
    const ir = {
      nouns: {
        Person: { worldAssumption: 'closed' },
        Name: { worldAssumption: 'closed' },
      },
      factTypes: {
        ft1: {
          reading: 'Person has Name',
          roles: [{ nounName: 'Person' }, { nounName: 'Name' }],
        },
      },
      constraints: [],
    }
    const population = {
      facts: {
        ft1: [
          { bindings: [['Person', 'Alice'], ['Name', 'Smith']] },
          { bindings: [['Person', 'Bob'], ['Name', 'Jones']] },
          { bindings: [['Person', 'Carol'], ['Name', 'Lee']] },
        ],
      },
    }

    const result = induceConstraints(JSON.stringify(ir), JSON.stringify(population))

    for (const c of result.constraints) {
      expect(c.confidence).toBeTypeOf('number')
      expect(c.confidence).toBeGreaterThanOrEqual(0)
      expect(c.confidence).toBeLessThanOrEqual(1)
    }
  })

  it('returns well-typed InducedConstraint objects', () => {
    const ir = {
      nouns: {
        Student: { worldAssumption: 'closed' },
        Grade: { worldAssumption: 'closed' },
      },
      factTypes: {
        ft1: {
          reading: 'Student received Grade',
          roles: [{ nounName: 'Student' }, { nounName: 'Grade' }],
        },
      },
      constraints: [],
    }
    const population = {
      facts: {
        ft1: [
          { bindings: [['Student', 'Alice'], ['Grade', 'A']] },
          { bindings: [['Student', 'Bob'], ['Grade', 'B']] },
          { bindings: [['Student', 'Carol'], ['Grade', 'A']] },
        ],
      },
    }

    const result = induceConstraints(JSON.stringify(ir), JSON.stringify(population))

    for (const c of result.constraints) {
      expect(c.kind).toBeTypeOf('string')
      expect(c.factTypeId).toBeTypeOf('string')
      expect(c.reading).toBeTypeOf('string')
      expect(c.roles).toBeInstanceOf(Array)
      expect(c.evidence).toBeTypeOf('string')
    }
  })

  it('returns well-typed InducedRule objects when present', () => {
    // Even if no rules are induced, the shape should be correct
    const result = induceConstraints(
      JSON.stringify({ nouns: {}, factTypes: {}, constraints: [] }),
      JSON.stringify({ facts: {} }),
    )
    expect(result.rules).toBeInstanceOf(Array)
    // Each rule (if any) should have the correct shape
    for (const r of result.rules) {
      expect(r.text).toBeTypeOf('string')
      expect(r.antecedentFactTypeIds).toBeInstanceOf(Array)
      expect(r.consequentFactTypeId).toBeTypeOf('string')
      expect(r.confidence).toBeTypeOf('number')
      expect(r.evidence).toBeTypeOf('string')
    }
  })

  it('reports population stats accurately', () => {
    const ir = {
      nouns: {
        A: { worldAssumption: 'closed' },
        B: { worldAssumption: 'closed' },
        C: { worldAssumption: 'closed' },
      },
      factTypes: {
        ft1: {
          reading: 'A relates to B',
          roles: [{ nounName: 'A' }, { nounName: 'B' }],
        },
        ft2: {
          reading: 'B relates to C',
          roles: [{ nounName: 'B' }, { nounName: 'C' }],
        },
      },
      constraints: [],
    }
    const population = {
      facts: {
        ft1: [
          { bindings: [['A', 'a1'], ['B', 'b1']] },
          { bindings: [['A', 'a2'], ['B', 'b2']] },
        ],
        ft2: [
          { bindings: [['B', 'b1'], ['C', 'c1']] },
        ],
      },
    }

    const result = induceConstraints(JSON.stringify(ir), JSON.stringify(population))

    expect(result.populationStats.factTypeCount).toBe(2)
    expect(result.populationStats.totalFacts).toBe(3)
    // 5 unique entity values: a1, a2, b1, b2, c1
    expect(result.populationStats.entityCount).toBe(5)
  })
})

import { describe, it, expect, vi } from 'vitest'
import {
  checkDeonticConstraints,
  parseDeonticOperator,
  type RegistryStub,
  type EntityStub,
} from './deontic-check'

// ---------------------------------------------------------------------------
// Helper: build mock stubs
// ---------------------------------------------------------------------------

type EntityMap = Record<string, { id: string; type: string; data: Record<string, unknown> }>

function buildStubs(
  entities: EntityMap,
  registryIndex: Record<string, string[]>,
) {
  const registry: RegistryStub = {
    getEntityIds: vi.fn(async (entityType: string, _domain?: string) => {
      return registryIndex[entityType] || []
    }),
  }

  const getStub = (id: string): EntityStub => ({
    get: vi.fn(async () => entities[id] || null),
  })

  return { registry, getStub }
}

// ---------------------------------------------------------------------------
// parseDeonticOperator
// ---------------------------------------------------------------------------

describe('parseDeonticOperator', () => {
  it('detects forbidden', () => {
    expect(parseDeonticOperator('It is forbidden that X does Y')).toBe('forbidden')
  })

  it('detects permitted', () => {
    expect(parseDeonticOperator('It is permitted that X does Y')).toBe('permitted')
  })

  it('defaults to obligatory for other text', () => {
    expect(parseDeonticOperator('It is obligatory that X has Y')).toBe('obligatory')
    expect(parseDeonticOperator('Each X must have Y')).toBe('obligatory')
  })

  it('is case-insensitive', () => {
    expect(parseDeonticOperator('It is FORBIDDEN that X does Y')).toBe('forbidden')
    expect(parseDeonticOperator('It is Permitted that X does Y')).toBe('permitted')
  })
})

// ---------------------------------------------------------------------------
// checkDeonticConstraints
// ---------------------------------------------------------------------------

describe('checkDeonticConstraints', () => {
  it('returns allowed=true when no deontic constraints exist', async () => {
    const { registry, getStub } = buildStubs({}, {
      'Constraint': [],
      'Constraint Span': [],
    })

    const result = await checkDeonticConstraints(
      'Customer', { name: 'Acme' }, 'support', registry, getStub,
    )

    expect(result.allowed).toBe(true)
    expect(result.violations).toHaveLength(0)
  })

  it('returns allowed=true when constraints exist but are all alethic', async () => {
    const entities: EntityMap = {
      'c1': {
        id: 'c1', type: 'Constraint',
        data: { kind: 'UC', modality: 'Alethic', text: 'Each Customer has at most one Name', domain: 'support' },
      },
    }
    const { registry, getStub } = buildStubs(entities, {
      'Constraint': ['c1'],
      'Constraint Span': [],
    })

    const result = await checkDeonticConstraints(
      'Customer', { name: 'Acme' }, 'support', registry, getStub,
    )

    expect(result.allowed).toBe(true)
    expect(result.violations).toHaveLength(0)
  })

  it('returns allowed=true when deontic constraint does not apply to entity type', async () => {
    const entities: EntityMap = {
      'c1': {
        id: 'c1', type: 'Constraint',
        data: { kind: 'MC', modality: 'Deontic', text: 'It is obligatory that each Order has some Product', domain: 'support' },
      },
      'cs1': {
        id: 'cs1', type: 'Constraint Span',
        data: { constraint_id: 'c1', role_id: 'r1' },
      },
      'r1': {
        id: 'r1', type: 'Role',
        data: { noun_id: 'n-order', role_index: 0 },
      },
      'n-order': {
        id: 'n-order', type: 'Noun',
        data: { name: 'Order' },
      },
    }
    const { registry, getStub } = buildStubs(entities, {
      'Constraint': ['c1'],
      'Constraint Span': ['cs1'],
    })

    const result = await checkDeonticConstraints(
      'Customer', { name: 'Acme' }, 'support', registry, getStub,
    )

    expect(result.allowed).toBe(true)
    expect(result.violations).toHaveLength(0)
  })

  it('rejects with error when forbidden constraint applies to entity type', async () => {
    const entities: EntityMap = {
      'c1': {
        id: 'c1', type: 'Constraint',
        data: {
          kind: 'UC', modality: 'Deontic',
          text: 'It is forbidden that Employee has Salary above Limit',
          domain: 'hr',
        },
      },
      'cs1': {
        id: 'cs1', type: 'Constraint Span',
        data: { constraint_id: 'c1', role_id: 'r1' },
      },
      'r1': {
        id: 'r1', type: 'Role',
        data: { noun_id: 'n-emp', role_index: 0 },
      },
      'n-emp': {
        id: 'n-emp', type: 'Noun',
        data: { name: 'Employee' },
      },
    }
    const { registry, getStub } = buildStubs(entities, {
      'Constraint': ['c1'],
      'Constraint Span': ['cs1'],
    })

    const result = await checkDeonticConstraints(
      'Employee', { name: 'Alice', salary: 999999 }, 'hr', registry, getStub,
    )

    expect(result.allowed).toBe(false)
    expect(result.violations).toHaveLength(1)
    expect(result.violations[0].severity).toBe('error')
    expect(result.violations[0].constraintId).toBe('c1')
    expect(result.violations[0].text).toContain('forbidden')
  })

  it('rejects when obligatory MC constraint is unmet (missing required field)', async () => {
    const entities: EntityMap = {
      'c1': {
        id: 'c1', type: 'Constraint',
        data: {
          kind: 'MC', modality: 'Deontic',
          text: 'It is obligatory that each Customer has some Priority',
          domain: 'support',
        },
      },
      'cs1': {
        id: 'cs1', type: 'Constraint Span',
        data: { constraint_id: 'c1', role_id: 'r1' },
      },
      'cs2': {
        id: 'cs2', type: 'Constraint Span',
        data: { constraint_id: 'c1', role_id: 'r2' },
      },
      'r1': {
        id: 'r1', type: 'Role',
        data: { noun_id: 'n-cust', role_index: 0 },
      },
      'r2': {
        id: 'r2', type: 'Role',
        data: { noun_id: 'n-priority', role_index: 1 },
      },
      'n-cust': {
        id: 'n-cust', type: 'Noun',
        data: { name: 'Customer' },
      },
      'n-priority': {
        id: 'n-priority', type: 'Noun',
        data: { name: 'Priority' },
      },
    }
    const { registry, getStub } = buildStubs(entities, {
      'Constraint': ['c1'],
      'Constraint Span': ['cs1', 'cs2'],
    })

    // Missing 'priority' field
    const result = await checkDeonticConstraints(
      'Customer', { name: 'Acme' }, 'support', registry, getStub,
    )

    expect(result.allowed).toBe(false)
    expect(result.violations).toHaveLength(1)
    expect(result.violations[0].severity).toBe('error')
    expect(result.violations[0].constraintId).toBe('c1')
  })

  it('allows when obligatory MC constraint is met (field present)', async () => {
    const entities: EntityMap = {
      'c1': {
        id: 'c1', type: 'Constraint',
        data: {
          kind: 'MC', modality: 'Deontic',
          text: 'It is obligatory that each Customer has some Priority',
          domain: 'support',
        },
      },
      'cs1': {
        id: 'cs1', type: 'Constraint Span',
        data: { constraint_id: 'c1', role_id: 'r1' },
      },
      'cs2': {
        id: 'cs2', type: 'Constraint Span',
        data: { constraint_id: 'c1', role_id: 'r2' },
      },
      'r1': {
        id: 'r1', type: 'Role',
        data: { noun_id: 'n-cust', role_index: 0 },
      },
      'r2': {
        id: 'r2', type: 'Role',
        data: { noun_id: 'n-priority', role_index: 1 },
      },
      'n-cust': {
        id: 'n-cust', type: 'Noun',
        data: { name: 'Customer' },
      },
      'n-priority': {
        id: 'n-priority', type: 'Noun',
        data: { name: 'Priority' },
      },
    }
    const { registry, getStub } = buildStubs(entities, {
      'Constraint': ['c1'],
      'Constraint Span': ['cs1', 'cs2'],
    })

    // 'priority' field IS present
    const result = await checkDeonticConstraints(
      'Customer', { name: 'Acme', priority: 'high' }, 'support', registry, getStub,
    )

    expect(result.allowed).toBe(true)
    expect(result.violations).toHaveLength(0)
  })

  it('allows when obligatory MC constraint is met via Id-suffixed field', async () => {
    const entities: EntityMap = {
      'c1': {
        id: 'c1', type: 'Constraint',
        data: {
          kind: 'MC', modality: 'Deontic',
          text: 'It is obligatory that each Customer has some Priority',
          domain: 'support',
        },
      },
      'cs1': {
        id: 'cs1', type: 'Constraint Span',
        data: { constraint_id: 'c1', role_id: 'r1' },
      },
      'cs2': {
        id: 'cs2', type: 'Constraint Span',
        data: { constraint_id: 'c1', role_id: 'r2' },
      },
      'r1': { id: 'r1', type: 'Role', data: { noun_id: 'n-cust', role_index: 0 } },
      'r2': { id: 'r2', type: 'Role', data: { noun_id: 'n-priority', role_index: 1 } },
      'n-cust': { id: 'n-cust', type: 'Noun', data: { name: 'Customer' } },
      'n-priority': { id: 'n-priority', type: 'Noun', data: { name: 'Priority' } },
    }
    const { registry, getStub } = buildStubs(entities, {
      'Constraint': ['c1'],
      'Constraint Span': ['cs1', 'cs2'],
    })

    // 'priorityId' present (FK reference)
    const result = await checkDeonticConstraints(
      'Customer', { name: 'Acme', priorityId: 'p-high' }, 'support', registry, getStub,
    )

    expect(result.allowed).toBe(true)
    expect(result.violations).toHaveLength(0)
  })

  it('allows when obligatory MC constraint is met via snake_case field', async () => {
    const entities: EntityMap = {
      'c1': {
        id: 'c1', type: 'Constraint',
        data: {
          kind: 'MC', modality: 'Deontic',
          text: 'It is obligatory that each Customer has some RiskLevel',
          domain: 'support',
        },
      },
      'cs1': { id: 'cs1', type: 'Constraint Span', data: { constraint_id: 'c1', role_id: 'r1' } },
      'cs2': { id: 'cs2', type: 'Constraint Span', data: { constraint_id: 'c1', role_id: 'r2' } },
      'r1': { id: 'r1', type: 'Role', data: { noun_id: 'n-cust', role_index: 0 } },
      'r2': { id: 'r2', type: 'Role', data: { noun_id: 'n-risk', role_index: 1 } },
      'n-cust': { id: 'n-cust', type: 'Noun', data: { name: 'Customer' } },
      'n-risk': { id: 'n-risk', type: 'Noun', data: { name: 'RiskLevel' } },
    }
    const { registry, getStub } = buildStubs(entities, {
      'Constraint': ['c1'],
      'Constraint Span': ['cs1', 'cs2'],
    })

    // snake_case variant 'risk_level' is present
    const result = await checkDeonticConstraints(
      'Customer', { name: 'Acme', risk_level: 'high' }, 'support', registry, getStub,
    )

    expect(result.allowed).toBe(true)
    expect(result.violations).toHaveLength(0)
  })

  it('does not produce violations for permitted constraints', async () => {
    const entities: EntityMap = {
      'c1': {
        id: 'c1', type: 'Constraint',
        data: {
          kind: 'MC', modality: 'Deontic',
          text: 'It is permitted that Customer has Discount',
          domain: 'sales',
        },
      },
      'cs1': { id: 'cs1', type: 'Constraint Span', data: { constraint_id: 'c1', role_id: 'r1' } },
      'r1': { id: 'r1', type: 'Role', data: { noun_id: 'n-cust', role_index: 0 } },
      'n-cust': { id: 'n-cust', type: 'Noun', data: { name: 'Customer' } },
    }
    const { registry, getStub } = buildStubs(entities, {
      'Constraint': ['c1'],
      'Constraint Span': ['cs1'],
    })

    const result = await checkDeonticConstraints(
      'Customer', { name: 'Acme' }, 'sales', registry, getStub,
    )

    expect(result.allowed).toBe(true)
    expect(result.violations).toHaveLength(0)
  })

  it('produces warning for non-MC obligatory constraint', async () => {
    const entities: EntityMap = {
      'c1': {
        id: 'c1', type: 'Constraint',
        data: {
          kind: 'UC', modality: 'Deontic',
          text: 'It is obligatory that each Customer has at most one Email',
          domain: 'support',
        },
      },
      'cs1': { id: 'cs1', type: 'Constraint Span', data: { constraint_id: 'c1', role_id: 'r1' } },
      'r1': { id: 'r1', type: 'Role', data: { noun_id: 'n-cust', role_index: 0 } },
      'n-cust': { id: 'n-cust', type: 'Noun', data: { name: 'Customer' } },
    }
    const { registry, getStub } = buildStubs(entities, {
      'Constraint': ['c1'],
      'Constraint Span': ['cs1'],
    })

    const result = await checkDeonticConstraints(
      'Customer', { name: 'Acme', email: 'a@b.com' }, 'support', registry, getStub,
    )

    // Non-MC obligatory produces warning, not error → allowed
    expect(result.allowed).toBe(true)
    expect(result.violations).toHaveLength(1)
    expect(result.violations[0].severity).toBe('warning')
  })

  it('handles unreachable Constraint DOs gracefully', async () => {
    const registry: RegistryStub = {
      getEntityIds: vi.fn(async (entityType: string) => {
        if (entityType === 'Constraint') return ['c-gone']
        return []
      }),
    }
    const getStub = (_id: string): EntityStub => ({
      get: vi.fn().mockRejectedValue(new Error('DO unreachable')),
    })

    const result = await checkDeonticConstraints(
      'Customer', { name: 'Acme' }, 'support', registry, getStub,
    )

    // Unreachable DOs should not block writes
    expect(result.allowed).toBe(true)
    expect(result.violations).toHaveLength(0)
  })

  it('uses explicit deontic_operator field when present on constraint data', async () => {
    const entities: EntityMap = {
      'c1': {
        id: 'c1', type: 'Constraint',
        data: {
          kind: 'UC', modality: 'Deontic',
          deontic_operator: 'forbidden',
          text: 'Some constraint text without keyword',
          domain: 'hr',
        },
      },
      'cs1': { id: 'cs1', type: 'Constraint Span', data: { constraint_id: 'c1', role_id: 'r1' } },
      'r1': { id: 'r1', type: 'Role', data: { noun_id: 'n-emp', role_index: 0 } },
      'n-emp': { id: 'n-emp', type: 'Noun', data: { name: 'Employee' } },
    }
    const { registry, getStub } = buildStubs(entities, {
      'Constraint': ['c1'],
      'Constraint Span': ['cs1'],
    })

    const result = await checkDeonticConstraints(
      'Employee', { name: 'Alice' }, 'hr', registry, getStub,
    )

    // deontic_operator='forbidden' takes precedence over text parsing
    expect(result.allowed).toBe(false)
    expect(result.violations).toHaveLength(1)
    expect(result.violations[0].severity).toBe('error')
  })

  it('handles multiple constraints, mixing errors and warnings', async () => {
    const entities: EntityMap = {
      // Forbidden constraint (error)
      'c1': {
        id: 'c1', type: 'Constraint',
        data: {
          kind: 'UC', modality: 'Deontic',
          text: 'It is forbidden that Intern accesses SecretData',
          domain: 'hr',
        },
      },
      // Obligatory UC constraint (warning)
      'c2': {
        id: 'c2', type: 'Constraint',
        data: {
          kind: 'UC', modality: 'Deontic',
          text: 'It is obligatory that each Intern has at most one Manager',
          domain: 'hr',
        },
      },
      'cs1': { id: 'cs1', type: 'Constraint Span', data: { constraint_id: 'c1', role_id: 'r1' } },
      'cs2': { id: 'cs2', type: 'Constraint Span', data: { constraint_id: 'c2', role_id: 'r1' } },
      'r1': { id: 'r1', type: 'Role', data: { noun_id: 'n-intern', role_index: 0 } },
      'n-intern': { id: 'n-intern', type: 'Noun', data: { name: 'Intern' } },
    }
    const { registry, getStub } = buildStubs(entities, {
      'Constraint': ['c1', 'c2'],
      'Constraint Span': ['cs1', 'cs2'],
    })

    const result = await checkDeonticConstraints(
      'Intern', { name: 'Bob' }, 'hr', registry, getStub,
    )

    // One error + one warning = not allowed (error takes precedence)
    expect(result.allowed).toBe(false)
    expect(result.violations).toHaveLength(2)
    expect(result.violations.some(v => v.severity === 'error')).toBe(true)
    expect(result.violations.some(v => v.severity === 'warning')).toBe(true)
  })
})

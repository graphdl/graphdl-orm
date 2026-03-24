import { describe, it, expect, vi } from 'vitest'
import { buildCascadeDeleteBatch, CASCADE_GRAPH, type CascadeRegistryStub, type CascadeEntityStub } from './cascade'

/**
 * Helper: build a mock registry that returns entity IDs by type.
 */
function mockRegistry(data: Record<string, string[]>): CascadeRegistryStub {
  return {
    getEntityIds: vi.fn(async (nounType: string, _domainSlug?: string) => {
      return data[nounType] || []
    }),
  }
}

/**
 * Helper: build a getStub function that returns entity data by ID.
 */
function mockGetStub(
  entities: Record<string, { id: string; type: string; data: Record<string, unknown> }>,
): (id: string) => CascadeEntityStub {
  return (id: string) => ({
    get: vi.fn(async () => entities[id] || null),
  })
}

describe('cascade', () => {
  describe('buildCascadeDeleteBatch', () => {
    it('deleting a Noun cascades to Roles referencing that noun', async () => {
      const nounId = 'noun-1'
      const roleId = 'role-1'
      const otherRoleId = 'role-2'

      const registry = mockRegistry({
        'Reading': [],
        'Role': [roleId, otherRoleId],
        'Constraint': [],
        'State Machine Definition': [],
      })

      const entities: Record<string, { id: string; type: string; data: Record<string, unknown> }> = {
        [roleId]: { id: roleId, type: 'Role', data: { nounId: nounId, name: 'has' } },
        [otherRoleId]: { id: otherRoleId, type: 'Role', data: { nounId: 'noun-other', name: 'is' } },
      }

      const result = await buildCascadeDeleteBatch(
        nounId,
        'Noun',
        registry,
        mockGetStub(entities),
        'test-domain',
      )

      // Root + one matching Role
      expect(result).toHaveLength(2)
      expect(result[0]).toEqual({ id: nounId, type: 'Noun', domain: 'test-domain', data: {} })
      expect(result[1]).toMatchObject({ id: roleId, type: 'Role' })
    })

    it('deleting a Graph Schema cascades to Readings and Roles', async () => {
      const schemaId = 'schema-1'
      const readingId = 'reading-1'
      const roleId = 'role-1'

      const registry = mockRegistry({
        'Reading': [readingId],
        'Role': [roleId],
      })

      const entities: Record<string, { id: string; type: string; data: Record<string, unknown> }> = {
        [readingId]: { id: readingId, type: 'Reading', data: { graphSchemaId: schemaId, text: '{0} has {1}' } },
        [roleId]: { id: roleId, type: 'Role', data: { graphSchemaId: schemaId, name: 'has' } },
      }

      const result = await buildCascadeDeleteBatch(
        schemaId,
        'Graph Schema',
        registry,
        mockGetStub(entities),
        'test-domain',
      )

      expect(result).toHaveLength(3)
      expect(result[0]).toMatchObject({ id: schemaId, type: 'Graph Schema' })
      const types = result.slice(1).map(e => e.type)
      expect(types).toContain('Reading')
      expect(types).toContain('Role')
    })

    it('deleting a Constraint cascades to Constraint Spans', async () => {
      const constraintId = 'constraint-1'
      const spanId = 'span-1'
      const span2Id = 'span-2'

      const registry = mockRegistry({
        'Constraint Span': [spanId, span2Id],
      })

      const entities: Record<string, { id: string; type: string; data: Record<string, unknown> }> = {
        [spanId]: { id: spanId, type: 'Constraint Span', data: { constraintId: constraintId, position: 0 } },
        [span2Id]: { id: span2Id, type: 'Constraint Span', data: { constraintId: 'other-constraint', position: 1 } },
      }

      const result = await buildCascadeDeleteBatch(
        constraintId,
        'Constraint',
        registry,
        mockGetStub(entities),
        'test-domain',
      )

      expect(result).toHaveLength(2)
      expect(result[0]).toMatchObject({ id: constraintId, type: 'Constraint' })
      expect(result[1]).toMatchObject({ id: spanId, type: 'Constraint Span' })
    })

    it('supports recursive cascade (Noun → Constraint → Constraint Span)', async () => {
      const nounId = 'noun-1'
      const constraintId = 'constraint-1'
      const spanId = 'span-1'

      const registry = mockRegistry({
        // For Noun dependents
        'Reading': [],
        'Role': [],
        'Constraint': [constraintId],
        'State Machine Definition': [],
        // For Constraint dependents
        'Constraint Span': [spanId],
      })

      const entities: Record<string, { id: string; type: string; data: Record<string, unknown> }> = {
        [constraintId]: { id: constraintId, type: 'Constraint', data: { nounId: nounId, kind: 'UC' } },
        [spanId]: { id: spanId, type: 'Constraint Span', data: { constraintId: constraintId, position: 0 } },
      }

      const result = await buildCascadeDeleteBatch(
        nounId,
        'Noun',
        registry,
        mockGetStub(entities),
        'test-domain',
      )

      // Root Noun + Constraint + Constraint Span
      expect(result).toHaveLength(3)
      expect(result[0]).toMatchObject({ id: nounId, type: 'Noun' })

      const ids = result.map(e => e.id)
      expect(ids).toContain(constraintId)
      expect(ids).toContain(spanId)
    })

    it('no cascade for types not in CASCADE_GRAPH', async () => {
      const entityId = 'random-entity-1'

      const registry = mockRegistry({})

      const result = await buildCascadeDeleteBatch(
        entityId,
        'SomeUnknownType',
        registry,
        mockGetStub({}),
        'test-domain',
      )

      // Only the root entity
      expect(result).toHaveLength(1)
      expect(result[0]).toMatchObject({ id: entityId, type: 'SomeUnknownType', domain: 'test-domain' })
    })

    it('empty result when no dependents match the root', async () => {
      const nounId = 'noun-1'

      const registry = mockRegistry({
        'Reading': ['r1'],
        'Role': ['role-1'],
        'Constraint': [],
        'State Machine Definition': [],
      })

      // All entities reference a *different* noun
      const entities: Record<string, { id: string; type: string; data: Record<string, unknown> }> = {
        'r1': { id: 'r1', type: 'Reading', data: { graphSchemaId: 'other-schema' } },
        'role-1': { id: 'role-1', type: 'Role', data: { nounId: 'noun-other' } },
      }

      const result = await buildCascadeDeleteBatch(
        nounId,
        'Noun',
        registry,
        mockGetStub(entities),
        'test-domain',
      )

      // Only the root entity — no dependents matched
      expect(result).toHaveLength(1)
      expect(result[0]).toMatchObject({ id: nounId, type: 'Noun' })
    })

    it('uses empty string for domain when none provided', async () => {
      const entityId = 'entity-1'

      const registry = mockRegistry({})

      const result = await buildCascadeDeleteBatch(
        entityId,
        'SomeType',
        registry,
        mockGetStub({}),
      )

      expect(result[0].domain).toBe('')
    })

    it('handles null entities returned from getStub gracefully', async () => {
      const schemaId = 'schema-1'

      const registry = mockRegistry({
        'Reading': ['r1', 'r2'],
        'Role': [],
      })

      // r1 returns null (deleted or missing), r2 returns valid data
      const entities: Record<string, { id: string; type: string; data: Record<string, unknown> }> = {
        'r2': { id: 'r2', type: 'Reading', data: { graphSchemaId: schemaId, text: 'something' } },
      }

      const result = await buildCascadeDeleteBatch(
        schemaId,
        'Graph Schema',
        registry,
        mockGetStub(entities),
        'test-domain',
      )

      // Root + r2 only (r1 returned null, so skipped)
      expect(result).toHaveLength(2)
      expect(result[1]).toMatchObject({ id: 'r2', type: 'Reading' })
    })
  })

  describe('CASCADE_GRAPH', () => {
    it('has entries for known metamodel entity types', () => {
      expect(CASCADE_GRAPH).toHaveProperty('Noun')
      expect(CASCADE_GRAPH).toHaveProperty('Graph Schema')
      expect(CASCADE_GRAPH).toHaveProperty('Constraint')
      expect(CASCADE_GRAPH).toHaveProperty('State Machine Definition')
    })

    it('Noun cascade includes Reading, Role, Constraint, State Machine Definition', () => {
      const nounDeps = CASCADE_GRAPH['Noun']
      const types = nounDeps.map(d => d.type)
      expect(types).toContain('Reading')
      expect(types).toContain('Role')
      expect(types).toContain('Constraint')
      expect(types).toContain('State Machine Definition')
    })
  })
})

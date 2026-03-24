import { describe, it, expect, vi } from 'vitest'
import { EntityDataLoader } from './entity-data-loader'
import type { RegistryStub, EntityStub } from './entity-data-loader'

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function makeStubs(entities: { id: string; type: string; data: Record<string, any> }[]) {
  const stubs = new Map<string, EntityStub>(
    entities.map(e => [e.id, { get: vi.fn().mockResolvedValue(e) }]),
  )
  return stubs
}

function makeRegistry(mapping: Record<string, string[]>): RegistryStub {
  return {
    getEntityIds: vi.fn().mockImplementation((type: string, _domain?: string) => {
      return Promise.resolve(mapping[type] ?? [])
    }),
  }
}

function makeLoader(
  entities: { id: string; type: string; data: Record<string, any> }[],
  registryMapping: Record<string, string[]>,
) {
  const stubs = makeStubs(entities)
  const registry = makeRegistry(registryMapping)
  const loader = new EntityDataLoader(registry, (id) => stubs.get(id)!)
  return { loader, registry, stubs }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

describe('EntityDataLoader', () => {
  // -----------------------------------------------------------------------
  // queryNouns (existing tests preserved)
  // -----------------------------------------------------------------------

  it('loads nouns by fan-out to entity stubs', async () => {
    const entities = [
      { id: 'n1', type: 'Noun', data: { name: 'Customer', object_type: 'entity', domain_id: 'd1' } },
      { id: 'n2', type: 'Noun', data: { name: 'Order', object_type: 'entity', domain_id: 'd1' } },
    ]
    const stubs = new Map<string, EntityStub>(
      entities.map(e => [e.id, { get: vi.fn().mockResolvedValue(e) }]),
    )
    const registry: RegistryStub = { getEntityIds: vi.fn().mockResolvedValue(['n1', 'n2']) }
    const loader = new EntityDataLoader(registry, (id) => stubs.get(id)!)
    const nouns = await loader.queryNouns('d1')
    expect(nouns).toHaveLength(2)
    expect(nouns[0].name).toBe('Customer')
    expect(nouns[1].name).toBe('Order')
  })

  it('returns empty array when no entities found', async () => {
    const registry: RegistryStub = { getEntityIds: vi.fn().mockResolvedValue([]) }
    const loader = new EntityDataLoader(registry, () => ({ get: vi.fn().mockResolvedValue(null) }))
    const nouns = await loader.queryNouns('d1')
    expect(nouns).toHaveLength(0)
  })

  it('filters null responses from unreachable DOs', async () => {
    const registry: RegistryStub = { getEntityIds: vi.fn().mockResolvedValue(['n1', 'n2']) }
    const stubs = new Map<string, EntityStub>([
      ['n1', { get: vi.fn().mockResolvedValue({ id: 'n1', type: 'Noun', data: { name: 'A', object_type: 'entity', domain_id: 'd1' } }) }],
      ['n2', { get: vi.fn().mockResolvedValue(null) }],
    ])
    const loader = new EntityDataLoader(registry, (id) => stubs.get(id)!)
    const nouns = await loader.queryNouns('d1')
    expect(nouns).toHaveLength(1)
    expect(nouns[0].name).toBe('A')
  })

  it('passes entity type and domain to registry', async () => {
    const registry: RegistryStub = { getEntityIds: vi.fn().mockResolvedValue([]) }
    const loader = new EntityDataLoader(registry, () => ({ get: vi.fn().mockResolvedValue(null) }))
    await loader.queryNouns('my-domain')
    expect(registry.getEntityIds).toHaveBeenCalledWith('Noun', 'my-domain')
  })

  it('batches fan-out at 50 concurrent requests', async () => {
    // Create 75 entity IDs to verify batching (50 + 25)
    const ids = Array.from({ length: 75 }, (_, i) => `n${i}`)
    const registry: RegistryStub = { getEntityIds: vi.fn().mockResolvedValue(ids) }

    let maxConcurrent = 0
    let currentConcurrent = 0

    const loader = new EntityDataLoader(registry, (id) => ({
      get: vi.fn().mockImplementation(async () => {
        currentConcurrent++
        if (currentConcurrent > maxConcurrent) maxConcurrent = currentConcurrent
        // Simulate async delay
        await new Promise(r => setTimeout(r, 1))
        currentConcurrent--
        return { id, type: 'Noun', data: { name: `Noun_${id}`, object_type: 'entity', domain_id: 'd1' } }
      }),
    }))

    const nouns = await loader.queryNouns('d1')
    expect(nouns).toHaveLength(75)
    expect(maxConcurrent).toBeLessThanOrEqual(50)
  })

  it('maps entity data fields to Row-compatible format', async () => {
    const entity = {
      id: 'n1',
      type: 'Noun',
      data: {
        name: 'Priority',
        object_type: 'value',
        domain_id: 'd1',
        value_type: 'string',
        enum_values: '["low","high"]',
        super_type_id: null,
        super_type_name: 'BaseValue',
        plural: 'Priorities',
        prompt_text: 'The priority level',
      },
    }
    const registry: RegistryStub = { getEntityIds: vi.fn().mockResolvedValue(['n1']) }
    const stubs = new Map<string, EntityStub>([['n1', { get: vi.fn().mockResolvedValue(entity) }]])
    const loader = new EntityDataLoader(registry, (id) => stubs.get(id)!)
    const nouns = await loader.queryNouns('d1')
    expect(nouns).toHaveLength(1)
    const row = nouns[0]
    expect(row.id).toBe('n1')
    expect(row.name).toBe('Priority')
    expect(row.object_type).toBe('value')
    expect(row.domain_id).toBe('d1')
    expect(row.value_type).toBe('string')
    expect(row.enum_values).toBe('["low","high"]')
    expect(row.super_type_name).toBe('BaseValue')
    expect(row.plural).toBe('Priorities')
    expect(row.prompt_text).toBe('The priority level')
  })

  // -----------------------------------------------------------------------
  // queryGraphSchemas
  // -----------------------------------------------------------------------

  describe('queryGraphSchemas', () => {
    it('fans out to Graph Schema entities', async () => {
      const { loader, registry } = makeLoader(
        [
          { id: 'gs1', type: 'Graph Schema', data: { name: 'PersonHasName', domain_id: 'd1' } },
          { id: 'gs2', type: 'Graph Schema', data: { name: 'PersonHasAge', domain_id: 'd1' } },
        ],
        { 'Graph Schema': ['gs1', 'gs2'] },
      )
      const rows = await loader.queryGraphSchemas('d1')
      expect(rows).toHaveLength(2)
      expect(rows[0]).toEqual({ id: 'gs1', name: 'PersonHasName', domain_id: 'd1' })
      expect(rows[1]).toEqual({ id: 'gs2', name: 'PersonHasAge', domain_id: 'd1' })
      expect(registry.getEntityIds).toHaveBeenCalledWith('Graph Schema', 'd1')
    })

    it('returns empty array when no graph schemas exist', async () => {
      const { loader } = makeLoader([], { 'Graph Schema': [] })
      const rows = await loader.queryGraphSchemas('d1')
      expect(rows).toHaveLength(0)
    })
  })

  // -----------------------------------------------------------------------
  // queryReadings
  // -----------------------------------------------------------------------

  describe('queryReadings', () => {
    it('fans out to Reading entities', async () => {
      const { loader, registry } = makeLoader(
        [
          { id: 'r1', type: 'Reading', data: { text: '{0} has {1}', graph_schema_id: 'gs1', domain_id: 'd1' } },
        ],
        { 'Reading': ['r1'] },
      )
      const rows = await loader.queryReadings('d1')
      expect(rows).toHaveLength(1)
      expect(rows[0]).toEqual({ id: 'r1', text: '{0} has {1}', graph_schema_id: 'gs1', domain_id: 'd1' })
      expect(registry.getEntityIds).toHaveBeenCalledWith('Reading', 'd1')
    })
  })

  // -----------------------------------------------------------------------
  // queryRoles
  // -----------------------------------------------------------------------

  describe('queryRoles', () => {
    it('fans out to Role entities and includes domain_id from data', async () => {
      const { loader, registry } = makeLoader(
        [
          { id: 'role1', type: 'Role', data: { graph_schema_id: 'gs1', noun_id: 'n1', role_index: 0, domain_id: 'd1' } },
          { id: 'role2', type: 'Role', data: { graph_schema_id: 'gs1', noun_id: 'n2', role_index: 1, domain_id: 'd1' } },
        ],
        { 'Role': ['role1', 'role2'] },
      )
      const rows = await loader.queryRoles()
      expect(rows).toHaveLength(2)
      expect(rows[0]).toEqual({ id: 'role1', graph_schema_id: 'gs1', noun_id: 'n1', role_index: 0, domain_id: 'd1' })
      expect(rows[1]).toEqual({ id: 'role2', graph_schema_id: 'gs1', noun_id: 'n2', role_index: 1, domain_id: 'd1' })
      // queryRoles has no domain filter — fetches all
      expect(registry.getEntityIds).toHaveBeenCalledWith('Role')
    })
  })

  // -----------------------------------------------------------------------
  // queryConstraints
  // -----------------------------------------------------------------------

  describe('queryConstraints', () => {
    it('fans out to Constraint entities', async () => {
      const { loader, registry } = makeLoader(
        [
          { id: 'c1', type: 'Constraint', data: { kind: 'UC', modality: 'Alethic', text: 'Each Person has at most one Name', domain_id: 'd1' } },
        ],
        { 'Constraint': ['c1'] },
      )
      const rows = await loader.queryConstraints('d1')
      expect(rows).toHaveLength(1)
      expect(rows[0]).toEqual({ id: 'c1', kind: 'UC', modality: 'Alethic', text: 'Each Person has at most one Name', domain_id: 'd1' })
      expect(registry.getEntityIds).toHaveBeenCalledWith('Constraint', 'd1')
    })
  })

  // -----------------------------------------------------------------------
  // queryConstraintSpans
  // -----------------------------------------------------------------------

  describe('queryConstraintSpans', () => {
    it('fans out to Constraint Span entities with constraint and role references', async () => {
      const { loader, registry } = makeLoader(
        [
          {
            id: 'cs1',
            type: 'Constraint Span',
            data: {
              constraint_id: 'c1',
              role_id: 'role1',
              domain_id: 'd1',
              graph_schema_id: 'gs1',
              role_index: 0,
              subset_autofill: 0,
            },
          },
          {
            id: 'cs2',
            type: 'Constraint Span',
            data: {
              constraint_id: 'c1',
              role_id: 'role2',
              domain_id: 'd1',
              graph_schema_id: 'gs1',
              role_index: 1,
              subset_autofill: 0,
            },
          },
        ],
        { 'Constraint Span': ['cs1', 'cs2'] },
      )
      const rows = await loader.queryConstraintSpans()
      expect(rows).toHaveLength(2)
      // DomainModel expects: constraint_id, domain_id, graph_schema_id, role_index
      expect(rows[0]).toEqual({
        id: 'cs1',
        constraint_id: 'c1',
        role_id: 'role1',
        domain_id: 'd1',
        graph_schema_id: 'gs1',
        role_index: 0,
        subset_autofill: 0,
      })
      expect(rows[1]).toEqual({
        id: 'cs2',
        constraint_id: 'c1',
        role_id: 'role2',
        domain_id: 'd1',
        graph_schema_id: 'gs1',
        role_index: 1,
        subset_autofill: 0,
      })
      // queryConstraintSpans has no domain filter — fetches all
      expect(registry.getEntityIds).toHaveBeenCalledWith('Constraint Span')
    })

    it('returns raw row data without resolving constraint/role references', async () => {
      // EntityDataLoader returns the raw data blob — DomainModel handles resolution
      const { loader } = makeLoader(
        [
          {
            id: 'cs1',
            type: 'Constraint Span',
            data: { constraint_id: 'c99', role_id: 'role99', domain_id: 'd1', graph_schema_id: 'gs5', role_index: 2 },
          },
        ],
        { 'Constraint Span': ['cs1'] },
      )
      const rows = await loader.queryConstraintSpans()
      expect(rows).toHaveLength(1)
      expect(rows[0].constraint_id).toBe('c99')
      expect(rows[0].role_id).toBe('role99')
      expect(rows[0].graph_schema_id).toBe('gs5')
      expect(rows[0].role_index).toBe(2)
    })
  })

  // -----------------------------------------------------------------------
  // queryStateMachineDefs
  // -----------------------------------------------------------------------

  describe('queryStateMachineDefs', () => {
    it('fans out to State Machine Definition entities', async () => {
      const { loader, registry } = makeLoader(
        [
          { id: 'smd1', type: 'State Machine Definition', data: { noun_id: 'n1', domain_id: 'd1' } },
        ],
        { 'State Machine Definition': ['smd1'] },
      )
      const rows = await loader.queryStateMachineDefs('d1')
      expect(rows).toHaveLength(1)
      expect(rows[0]).toEqual({ id: 'smd1', noun_id: 'n1', domain_id: 'd1' })
      expect(registry.getEntityIds).toHaveBeenCalledWith('State Machine Definition', 'd1')
    })
  })

  // -----------------------------------------------------------------------
  // queryStatuses
  // -----------------------------------------------------------------------

  describe('queryStatuses', () => {
    it('fans out to Status entities', async () => {
      const { loader, registry } = makeLoader(
        [
          { id: 's1', type: 'Status', data: { name: 'Open', state_machine_definition_id: 'smd1', domain_id: 'd1' } },
          { id: 's2', type: 'Status', data: { name: 'Closed', state_machine_definition_id: 'smd1', domain_id: 'd1' } },
        ],
        { 'Status': ['s1', 's2'] },
      )
      const rows = await loader.queryStatuses('d1')
      expect(rows).toHaveLength(2)
      expect(rows[0]).toEqual({ id: 's1', name: 'Open', state_machine_definition_id: 'smd1', domain_id: 'd1' })
      expect(rows[1]).toEqual({ id: 's2', name: 'Closed', state_machine_definition_id: 'smd1', domain_id: 'd1' })
      expect(registry.getEntityIds).toHaveBeenCalledWith('Status', 'd1')
    })
  })

  // -----------------------------------------------------------------------
  // queryTransitions
  // -----------------------------------------------------------------------

  describe('queryTransitions', () => {
    it('fans out to Transition entities', async () => {
      const { loader, registry } = makeLoader(
        [
          {
            id: 't1',
            type: 'Transition',
            data: {
              from_status_id: 's1',
              to_status_id: 's2',
              event_type_id: 'et1',
              verb_id: 'v1',
              state_machine_definition_id: 'smd1',
              domain_id: 'd1',
            },
          },
        ],
        { 'Transition': ['t1'] },
      )
      const rows = await loader.queryTransitions('d1')
      expect(rows).toHaveLength(1)
      expect(rows[0]).toEqual({
        id: 't1',
        from_status_id: 's1',
        to_status_id: 's2',
        event_type_id: 'et1',
        verb_id: 'v1',
        state_machine_definition_id: 'smd1',
        domain_id: 'd1',
      })
      expect(registry.getEntityIds).toHaveBeenCalledWith('Transition', 'd1')
    })
  })

  // -----------------------------------------------------------------------
  // queryEventTypes
  // -----------------------------------------------------------------------

  describe('queryEventTypes', () => {
    it('fans out to Event Type entities', async () => {
      const { loader, registry } = makeLoader(
        [
          { id: 'et1', type: 'Event Type', data: { name: 'Close', domain_id: 'd1' } },
        ],
        { 'Event Type': ['et1'] },
      )
      const rows = await loader.queryEventTypes('d1')
      expect(rows).toHaveLength(1)
      expect(rows[0]).toEqual({ id: 'et1', name: 'Close', domain_id: 'd1' })
      expect(registry.getEntityIds).toHaveBeenCalledWith('Event Type', 'd1')
    })
  })

  // -----------------------------------------------------------------------
  // queryGuards
  // -----------------------------------------------------------------------

  describe('queryGuards', () => {
    it('fans out to Guard entities', async () => {
      const { loader, registry } = makeLoader(
        [
          { id: 'g1', type: 'Guard', data: { transition_id: 't1', graph_schema_id: 'gs1', domain_id: 'd1' } },
        ],
        { 'Guard': ['g1'] },
      )
      const rows = await loader.queryGuards('d1')
      expect(rows).toHaveLength(1)
      expect(rows[0]).toEqual({ id: 'g1', transition_id: 't1', graph_schema_id: 'gs1', domain_id: 'd1' })
      expect(registry.getEntityIds).toHaveBeenCalledWith('Guard', 'd1')
    })
  })

  // -----------------------------------------------------------------------
  // queryVerbs
  // -----------------------------------------------------------------------

  describe('queryVerbs', () => {
    it('fans out to Verb entities', async () => {
      const { loader, registry } = makeLoader(
        [
          { id: 'v1', type: 'Verb', data: { name: 'close', status_id: 's2', transition_id: 't1', domain_id: 'd1' } },
        ],
        { 'Verb': ['v1'] },
      )
      const rows = await loader.queryVerbs('d1')
      expect(rows).toHaveLength(1)
      expect(rows[0]).toEqual({ id: 'v1', name: 'close', status_id: 's2', transition_id: 't1', domain_id: 'd1' })
      expect(registry.getEntityIds).toHaveBeenCalledWith('Verb', 'd1')
    })
  })

  // -----------------------------------------------------------------------
  // queryFunctions
  // -----------------------------------------------------------------------

  describe('queryFunctions', () => {
    it('fans out to Function entities', async () => {
      const { loader, registry } = makeLoader(
        [
          { id: 'f1', type: 'Function', data: { callback_url: 'https://example.com/close', http_method: 'POST', verb_id: 'v1', domain_id: 'd1' } },
        ],
        { 'Function': ['f1'] },
      )
      const rows = await loader.queryFunctions('d1')
      expect(rows).toHaveLength(1)
      expect(rows[0]).toEqual({ id: 'f1', callback_url: 'https://example.com/close', http_method: 'POST', verb_id: 'v1', domain_id: 'd1' })
      expect(registry.getEntityIds).toHaveBeenCalledWith('Function', 'd1')
    })
  })

  // -----------------------------------------------------------------------
  // Cross-cutting: null filtering for all methods
  // -----------------------------------------------------------------------

  describe('null filtering', () => {
    it('filters null responses across all entity types', async () => {
      const entities = [
        { id: 'gs1', type: 'Graph Schema', data: { name: 'PersonHasName', domain_id: 'd1' } },
      ]
      const stubs = new Map<string, EntityStub>([
        ['gs1', { get: vi.fn().mockResolvedValue(entities[0]) }],
        ['gs2', { get: vi.fn().mockResolvedValue(null) }],
      ])
      const registry = makeRegistry({ 'Graph Schema': ['gs1', 'gs2'] })
      const loader = new EntityDataLoader(registry, (id) => stubs.get(id)!)
      const rows = await loader.queryGraphSchemas('d1')
      expect(rows).toHaveLength(1)
      expect(rows[0].name).toBe('PersonHasName')
    })
  })
})

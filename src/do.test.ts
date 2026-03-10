import { describe, it, expect } from 'vitest'
import { COLLECTION_TABLE_MAP, COLLECTION_SLUGS } from './collections'

describe('collections', () => {
  it('maps all Payload collection slugs to table names', () => {
    expect(COLLECTION_TABLE_MAP['nouns']).toBe('nouns')
    expect(COLLECTION_TABLE_MAP['graph-schemas']).toBe('graph_schemas')
    expect(COLLECTION_TABLE_MAP['readings']).toBe('readings')
    expect(COLLECTION_TABLE_MAP['constraint-spans']).toBe('constraint_spans')
    expect(COLLECTION_TABLE_MAP['state-machine-definitions']).toBe('state_machine_definitions')
    expect(COLLECTION_TABLE_MAP['state-machines']).toBe('state_machines')
    expect(COLLECTION_TABLE_MAP['resource-roles']).toBe('resource_roles')
    expect(COLLECTION_TABLE_MAP['event-types']).toBe('event_types')
    expect(COLLECTION_TABLE_MAP['guard-runs']).toBe('guard_runs')
    expect(COLLECTION_TABLE_MAP['org-memberships']).toBe('org_memberships')
  })

  it('lists all collection slugs', () => {
    expect(COLLECTION_SLUGS.length).toBeGreaterThanOrEqual(23)
    expect(COLLECTION_SLUGS).toContain('nouns')
    expect(COLLECTION_SLUGS).toContain('graph-schemas')
    expect(COLLECTION_SLUGS).toContain('domains')
  })
})

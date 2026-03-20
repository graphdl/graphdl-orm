import { describe, it, expect } from 'vitest'
import { COLLECTION_TABLE_MAP, COLLECTION_SLUGS } from './collections'
import { BOOTSTRAP_DDL } from './schema/bootstrap'
import { WIPE_TABLES } from './wipe-tables'

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

describe('wipeAllData coverage', () => {
  it('WIPE_TABLES includes every table defined in BOOTSTRAP_DDL', () => {
    // Extract all table names from CREATE TABLE statements in the bootstrap DDL
    const bootstrapTables: string[] = []
    for (const ddl of BOOTSTRAP_DDL) {
      const match = ddl.match(/CREATE\s+TABLE\s+IF\s+NOT\s+EXISTS\s+(\w+)/i)
      if (match) {
        bootstrapTables.push(match[1])
      }
    }

    // Sanity check: we should have found a reasonable number of tables
    expect(bootstrapTables.length).toBeGreaterThanOrEqual(25)

    // Every bootstrap table must appear in the WIPE_TABLES list
    const missingTables = bootstrapTables.filter(t => !WIPE_TABLES.includes(t))
    expect(missingTables).toEqual([])
  })

  it('WIPE_TABLES has no duplicate entries', () => {
    const unique = new Set(WIPE_TABLES)
    expect(unique.size).toBe(WIPE_TABLES.length)
  })

  it('WIPE_TABLES includes the cdc_events infrastructure table', () => {
    expect(WIPE_TABLES).toContain('cdc_events')
  })
})

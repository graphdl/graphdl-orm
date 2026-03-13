import { describe, it, expect } from 'vitest'
import { INSTANCE_DDL } from './instances'

describe('instance DDL', () => {
  it('exports DDL statements', () => {
    expect(Array.isArray(INSTANCE_DDL)).toBe(true)
    expect(INSTANCE_DDL.length).toBeGreaterThan(0)
  })

  it('includes all runtime instance tables', () => {
    const joined = INSTANCE_DDL.join('\n')
    const expectedTables = [
      'citations', 'graph_citations',
      'graphs', 'resources', 'resource_roles',
      'state_machines', 'events', 'guard_runs',
    ]
    for (const table of expectedTables) {
      expect(joined).toContain(`CREATE TABLE IF NOT EXISTS ${table}`)
    }
  })
})

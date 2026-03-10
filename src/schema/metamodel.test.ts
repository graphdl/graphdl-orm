import { describe, it, expect } from 'vitest'
import { METAMODEL_DDL } from './metamodel'

describe('metamodel DDL', () => {
  it('exports DDL statements as an array of strings', () => {
    expect(Array.isArray(METAMODEL_DDL)).toBe(true)
    expect(METAMODEL_DDL.length).toBeGreaterThan(0)
  })

  it('includes CREATE TABLE for all core metamodel tables', () => {
    const joined = METAMODEL_DDL.join('\n')
    const expectedTables = [
      'organizations', 'org_memberships', 'domains', 'nouns',
      'graph_schemas', 'readings', 'roles', 'constraints', 'constraint_spans',
    ]
    for (const table of expectedTables) {
      expect(joined).toContain(`CREATE TABLE IF NOT EXISTS ${table}`)
    }
  })

  it('includes CREATE INDEX statements', () => {
    const joined = METAMODEL_DDL.join('\n')
    expect(joined).toContain('CREATE INDEX')
  })
})

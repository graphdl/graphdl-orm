import { describe, it, expect } from 'vitest'
import { STATE_DDL } from './state'

describe('state machine DDL', () => {
  it('exports DDL statements', () => {
    expect(Array.isArray(STATE_DDL)).toBe(true)
    expect(STATE_DDL.length).toBeGreaterThan(0)
  })

  it('includes all state machine tables', () => {
    const joined = STATE_DDL.join('\n')
    const expectedTables = [
      'state_machine_definitions', 'statuses', 'transitions',
      'guards', 'event_types', 'verbs', 'functions', 'streams',
    ]
    for (const table of expectedTables) {
      expect(joined).toContain(`CREATE TABLE IF NOT EXISTS ${table}`)
    }
  })
})

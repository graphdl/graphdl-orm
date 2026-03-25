import { describe, it, expect } from 'vitest'
import {
  METAMODEL_TABLES,
} from './domain-do'

describe('domain-do', () => {
  describe('METAMODEL_TABLES', () => {
    it('contains all expected metamodel table names', () => {
      expect(METAMODEL_TABLES).toContain('nouns')
      expect(METAMODEL_TABLES).toContain('graph_schemas')
      expect(METAMODEL_TABLES).toContain('readings')
      expect(METAMODEL_TABLES).toContain('roles')
      expect(METAMODEL_TABLES).toContain('constraints')
      expect(METAMODEL_TABLES).toContain('constraint_spans')
      expect(METAMODEL_TABLES).toContain('state_machine_definitions')
      expect(METAMODEL_TABLES).toContain('statuses')
      expect(METAMODEL_TABLES).toContain('transitions')
      expect(METAMODEL_TABLES).toContain('guards')
      expect(METAMODEL_TABLES).toContain('event_types')
      expect(METAMODEL_TABLES).toContain('verbs')
      expect(METAMODEL_TABLES).toContain('functions')
      expect(METAMODEL_TABLES).toContain('streams')
      expect(METAMODEL_TABLES).toContain('generators')
    })

    it('does NOT contain instance tables', () => {
      expect(METAMODEL_TABLES).not.toContain('resources')
      expect(METAMODEL_TABLES).not.toContain('graphs')
      expect(METAMODEL_TABLES).not.toContain('resource_roles')
      expect(METAMODEL_TABLES).not.toContain('state_machines')
      expect(METAMODEL_TABLES).not.toContain('events')
      expect(METAMODEL_TABLES).not.toContain('guard_runs')
      expect(METAMODEL_TABLES).not.toContain('agents')
      expect(METAMODEL_TABLES).not.toContain('completions')
    })
  })
})

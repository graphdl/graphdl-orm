import { describe, it, expect } from 'vitest'
import { generateSQLite } from './sqlite'

describe('self-hosting bootstrap', () => {
  it('generateSQLite produces DDL for core metamodel entities', () => {
    const openapi = {
      components: {
        schemas: {
          UpdateNoun: {
            title: 'Noun',
            type: 'object',
            properties: {
              name: { type: 'string' },
              objectType: { type: 'string', enum: ['entity', 'value'] },
              plural: { type: 'string' },
              valueType: { type: 'string' },
              format: { type: 'string' },
              enumValues: { type: 'string' },
              promptText: { type: 'string' },
            },
          },
          NewNoun: { title: 'Noun', type: 'object' },
          Noun: { title: 'Noun', type: 'object' },
          UpdateReading: {
            title: 'Reading',
            type: 'object',
            properties: {
              text: { type: 'string' },
              graphSchema: { oneOf: [{ type: 'string' }, { $ref: '#/components/schemas/GraphSchema' }] },
            },
          },
          NewReading: { title: 'Reading', type: 'object' },
          Reading: { title: 'Reading', type: 'object' },
          UpdateGraphSchema: { title: 'GraphSchema', type: 'object', properties: { name: { type: 'string' } } },
          NewGraphSchema: { title: 'GraphSchema', type: 'object' },
          GraphSchema: { title: 'GraphSchema', type: 'object' },
        },
      },
    }

    const result = generateSQLite(openapi)

    // Should generate tables for Noun, Reading, GraphSchema
    expect(result.ddl.some(d => d.includes('nouns'))).toBe(true)
    expect(result.ddl.some(d => d.includes('readings'))).toBe(true)
    expect(result.ddl.some(d => d.includes('graph_schemas'))).toBe(true)

    // Noun table should have the expected columns
    const nounDDL = result.ddl.find(d => d.includes('CREATE TABLE') && d.includes('nouns'))!
    expect(nounDDL).toContain('name TEXT')
    expect(nounDDL).toContain('object_type TEXT')
    expect(nounDDL).toContain('value_type TEXT')

    // Reading should FK to graph_schemas
    const readingDDL = result.ddl.find(d => d.includes('CREATE TABLE') && d.includes('readings'))!
    expect(readingDDL).toContain('graph_schema_id TEXT REFERENCES graph_schemas(id)')
  })
})

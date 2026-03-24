import { describe, it, expect } from 'vitest'
import { rmap } from './procedure'

describe('RMAP Step 1: compound UC to separate table', () => {
  it('maps M:N binary to separate table with compound PK', () => {
    const schema = {
      nouns: [
        { name: 'Person', objectType: 'entity', refScheme: 'name' },
        { name: 'Language', objectType: 'entity', refScheme: 'code' },
      ],
      factTypes: [{
        id: 'ft1',
        reading: 'Person speaks Language',
        roles: [{ nounName: 'Person', roleIndex: 0 }, { nounName: 'Language', roleIndex: 1 }],
      }],
      constraints: [{
        kind: 'UC',
        factTypeId: 'ft1',
        roles: [0, 1], // spanning UC — compound key
      }],
    }
    const tables = rmap(schema)
    const speaksTable = tables.find(t => t.columns.some(c => c.name === 'person_id'))
    expect(speaksTable).toBeDefined()
    expect(speaksTable!.primaryKey).toEqual(['person_id', 'language_id'])
  })
})

describe('RMAP Step 2: functional roles grouped by entity', () => {
  it('groups functional fact types into entity table', () => {
    const schema = {
      nouns: [
        { name: 'Person', objectType: 'entity', refScheme: 'name' },
        { name: 'Name', objectType: 'value' },
        { name: 'Country', objectType: 'entity', refScheme: 'code' },
      ],
      factTypes: [
        { id: 'ft1', reading: 'Person has Name', roles: [
          { nounName: 'Person', roleIndex: 0 }, { nounName: 'Name', roleIndex: 1 }
        ]},
        { id: 'ft2', reading: 'Person was born in Country', roles: [
          { nounName: 'Person', roleIndex: 0 }, { nounName: 'Country', roleIndex: 1 }
        ]},
      ],
      constraints: [
        { kind: 'UC', factTypeId: 'ft1', roles: [0] },
        { kind: 'UC', factTypeId: 'ft2', roles: [0] },
      ],
    }
    const tables = rmap(schema)
    const personTable = tables.find(t => t.name === 'person')
    expect(personTable).toBeDefined()
    expect(personTable!.columns.map(c => c.name)).toContain('name')
    expect(personTable!.columns.map(c => c.name)).toContain('country_id')
  })
})

describe('RMAP Step 6: constraint mapping', () => {
  it('maps MC to NOT NULL', () => {
    const schema = {
      nouns: [
        { name: 'Person', objectType: 'entity', refScheme: 'name' },
        { name: 'Name', objectType: 'value' },
      ],
      factTypes: [{ id: 'ft1', reading: 'Person has Name', roles: [
        { nounName: 'Person', roleIndex: 0 }, { nounName: 'Name', roleIndex: 1 }
      ]}],
      constraints: [
        { kind: 'UC', factTypeId: 'ft1', roles: [0] },
        { kind: 'MC', factTypeId: 'ft1', roles: [0] }, // mandatory on Person role
      ],
    }
    const tables = rmap(schema)
    const personTable = tables.find(t => t.name === 'person')
    const nameCol = personTable?.columns.find(c => c.name === 'name')
    expect(nameCol?.nullable).toBe(false)
  })
})

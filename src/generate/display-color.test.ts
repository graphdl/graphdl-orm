import { describe, it, expect, beforeEach } from 'vitest'
import { generateOpenAPI } from './openapi'
import { generateSQLite } from './sqlite'
import {
  createMockModel,
  mkNounDef,
  mkValueNounDef,
  mkFactType,
  mkConstraint,
  resetIds,
} from '../model/test-utils'

// ---------------------------------------------------------------------------
// End-to-end: Status has Display Color → OpenAPI → SQLite DDL
// ---------------------------------------------------------------------------

describe('display_color column generation', () => {
  beforeEach(() => resetIds())

  it('generates displayColor property in OpenAPI when Status has Display Color reading exists', async () => {
    // Domain model: Status (entity) has Display Color (value type with enum)
    const statusNoun = mkNounDef({ name: 'Status' })
    const displayColorNoun = mkValueNounDef({
      name: 'Display Color',
      valueType: 'string',
      enumValues: ['green', 'amber', 'red', 'blue', 'violet', 'gray'],
    })

    const ft = mkFactType({
      id: 'gs1',
      reading: 'Status has Display Color',
      roles: [
        { nounDef: statusNoun, roleIndex: 0 },
        { nounDef: displayColorNoun, roleIndex: 1 },
      ],
    })

    const model = createMockModel({
      nouns: [statusNoun, displayColorNoun],
      factTypes: [ft],
      constraints: [
        mkConstraint({
          kind: 'UC',
          spans: [{ factTypeId: ft.id, roleIndex: 0 }],
        }),
      ],
    })

    const result = await generateOpenAPI(model)
    const s = result.components.schemas

    expect(s['Status']).toBeDefined()
    expect(s['UpdateStatus']).toBeDefined()

    // The property should be named "displayColor" (camelCase from "Display Color")
    const prop = s['Status'].properties?.displayColor
    expect(prop).toBeDefined()
    expect(prop.type).toBe('string')
    expect(prop.enum).toEqual(['green', 'amber', 'red', 'blue', 'violet', 'gray'])
  })

  it('generates display_color column in SQLite DDL from OpenAPI schema', async () => {
    // Same domain model as above, but go all the way to DDL
    const statusNoun = mkNounDef({ name: 'Status' })
    const displayColorNoun = mkValueNounDef({
      name: 'Display Color',
      valueType: 'string',
      enumValues: ['green', 'amber', 'red', 'blue', 'violet', 'gray'],
    })

    const ft = mkFactType({
      id: 'gs1',
      reading: 'Status has Display Color',
      roles: [
        { nounDef: statusNoun, roleIndex: 0 },
        { nounDef: displayColorNoun, roleIndex: 1 },
      ],
    })

    const model = createMockModel({
      nouns: [statusNoun, displayColorNoun],
      factTypes: [ft],
      constraints: [
        mkConstraint({
          kind: 'UC',
          spans: [{ factTypeId: ft.id, roleIndex: 0 }],
        }),
      ],
    })

    const openapi = await generateOpenAPI(model)
    const { ddl, tableMap, fieldMap } = generateSQLite(openapi)

    // Status maps to "statuses" table via NOUN_TABLE_MAP
    expect(tableMap['Status']).toBe('statuses')

    // DDL should include a CREATE TABLE with display_color column
    const createTable = ddl.find(
      (s) => s.startsWith('CREATE TABLE') && s.includes('statuses'),
    )
    expect(createTable).toBeDefined()
    expect(createTable).toContain('display_color TEXT')

    // fieldMap should map displayColor → display_color
    expect(fieldMap['statuses']?.displayColor).toBe('display_color')
  })

  it('generates display_color alongside other Status properties', async () => {
    // Status has both Name (existing) and Display Color (new)
    const statusNoun = mkNounDef({ name: 'Status' })
    const nameNoun = mkValueNounDef({ name: 'Name', valueType: 'string' })
    const displayColorNoun = mkValueNounDef({
      name: 'Display Color',
      valueType: 'string',
      enumValues: ['green', 'amber', 'red', 'blue', 'violet', 'gray'],
    })

    const ft1 = mkFactType({
      id: 'gs1',
      reading: 'Status has Name',
      roles: [
        { nounDef: statusNoun, roleIndex: 0 },
        { nounDef: nameNoun, roleIndex: 1 },
      ],
    })

    const ft2 = mkFactType({
      id: 'gs2',
      reading: 'Status has Display Color',
      roles: [
        { nounDef: statusNoun, roleIndex: 0 },
        { nounDef: displayColorNoun, roleIndex: 1 },
      ],
    })

    const model = createMockModel({
      nouns: [statusNoun, nameNoun, displayColorNoun],
      factTypes: [ft1, ft2],
      constraints: [
        mkConstraint({
          kind: 'UC',
          spans: [{ factTypeId: ft1.id, roleIndex: 0 }],
        }),
        mkConstraint({
          kind: 'UC',
          spans: [{ factTypeId: ft2.id, roleIndex: 0 }],
        }),
      ],
    })

    const openapi = await generateOpenAPI(model)
    const { ddl } = generateSQLite(openapi)

    const createTable = ddl.find(
      (s) => s.startsWith('CREATE TABLE') && s.includes('statuses'),
    )
    expect(createTable).toBeDefined()
    expect(createTable).toContain('name TEXT')
    expect(createTable).toContain('display_color TEXT')
  })
})

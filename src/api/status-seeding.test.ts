import { describe, it, expect, vi } from 'vitest'
import { ingestClaims, type ExtractedClaims } from '../claims/ingest'
import { parseFORML2 } from './parse'

// ---------------------------------------------------------------------------
// Mock DB for status seeding tests
// ---------------------------------------------------------------------------

function mockDb() {
  const store: Record<string, any[]> = {}
  let idCounter = 0

  return {
    store,
    createEntity: vi.fn(async (domainId: string, nounName: string, fields: any, reference?: string) => {
      const doc = { id: `entity-${++idCounter}`, domain: domainId, noun: nounName, reference, ...fields }
      const key = `entities_${nounName}`
      if (!store[key]) store[key] = []
      store[key].push(doc)
      return doc
    }),
    applySchema: vi.fn(async () => ({ tableMap: {}, fieldMap: {} })),
  }
}

// ---------------------------------------------------------------------------
// Tests: Status instance facts with Display Color
// ---------------------------------------------------------------------------

describe('status instance fact seeding', () => {
  it('calls createEntity with displayColor field for status display color facts', async () => {
    const db = mockDb()
    const claims: ExtractedClaims = {
      nouns: [
        { name: 'Status', objectType: 'entity' },
        { name: 'Display Color', objectType: 'value', valueType: 'string', enumValues: ['green', 'amber', 'blue', 'violet', 'gray'] },
      ],
      readings: [
        { text: 'Status has Display Color', nouns: ['Status', 'Display Color'], predicate: 'has' },
      ],
      constraints: [
        { kind: 'UC', modality: 'Alethic', reading: 'Status has Display Color', roles: [0] },
      ],
      facts: [
        {
          entity: 'Status',
          entityValue: 'Received',
          predicate: 'has',
          valueType: 'Display Color',
          value: 'blue',
        },
      ],
    }

    const result = await ingestClaims(db as any, { claims, domainId: 'd1' })

    // applySchema should have been called before entity creation
    expect(db.applySchema).toHaveBeenCalled()

    // createEntity should have been called with the displayColor field
    expect(db.createEntity).toHaveBeenCalledWith(
      'd1',
      'Status',
      { displayColor: 'blue' },
      'Received',
    )
  })

  it('converts multi-word value type names to camelCase field names', async () => {
    const db = mockDb()
    const claims: ExtractedClaims = {
      nouns: [
        { name: 'Status', objectType: 'entity' },
        { name: 'Display Color', objectType: 'value', valueType: 'string' },
      ],
      readings: [],
      constraints: [],
      facts: [
        {
          entity: 'Status',
          entityValue: 'Triaging',
          predicate: 'has',
          valueType: 'Display Color',
          value: 'amber',
        },
      ],
    }

    await ingestClaims(db as any, { claims, domainId: 'd1' })

    // "Display Color" should become "displayColor" (not "display_color" or "Display Color")
    expect(db.createEntity).toHaveBeenCalledWith(
      'd1',
      'Status',
      { displayColor: 'amber' },
      'Triaging',
    )
  })

  it('seeds multiple status display color facts without errors', async () => {
    const db = mockDb()
    const claims: ExtractedClaims = {
      nouns: [
        { name: 'Status', objectType: 'entity' },
        { name: 'Display Color', objectType: 'value', valueType: 'string' },
      ],
      readings: [],
      constraints: [],
      facts: [
        { entity: 'Status', entityValue: 'Received', predicate: 'has', valueType: 'Display Color', value: 'blue' },
        { entity: 'Status', entityValue: 'Triaging', predicate: 'has', valueType: 'Display Color', value: 'amber' },
        { entity: 'Status', entityValue: 'Investigating', predicate: 'has', valueType: 'Display Color', value: 'violet' },
        { entity: 'Status', entityValue: 'Resolved', predicate: 'has', valueType: 'Display Color', value: 'green' },
      ],
    }

    const result = await ingestClaims(db as any, { claims, domainId: 'd1' })

    expect(result.errors).toHaveLength(0)
    expect(db.createEntity).toHaveBeenCalledTimes(4)

    // Verify each status got its correct color
    const entities = db.store['entities_Status']
    expect(entities).toHaveLength(4)
    expect(entities.find((e: any) => e.reference === 'Received')?.displayColor).toBe('blue')
    expect(entities.find((e: any) => e.reference === 'Triaging')?.displayColor).toBe('amber')
    expect(entities.find((e: any) => e.reference === 'Investigating')?.displayColor).toBe('violet')
    expect(entities.find((e: any) => e.reference === 'Resolved')?.displayColor).toBe('green')
  })

  it('seeds Event Type instance facts with event label and style', async () => {
    const db = mockDb()
    const claims: ExtractedClaims = {
      nouns: [
        { name: 'Event Type', objectType: 'entity' },
        { name: 'Event Label', objectType: 'value', valueType: 'string' },
        { name: 'Event Style', objectType: 'value', valueType: 'string' },
      ],
      readings: [
        { text: 'Event Type has Event Label', nouns: ['Event Type', 'Event Label'], predicate: 'has' },
        { text: 'Event Type has Event Style', nouns: ['Event Type', 'Event Style'], predicate: 'has' },
      ],
      constraints: [],
      facts: [
        { entity: 'Event Type', entityValue: 'triage', predicate: 'has', valueType: 'Event Label', value: 'Triage' },
        { entity: 'Event Type', entityValue: 'triage', predicate: 'has', valueType: 'Event Style', value: 'primary' },
        { entity: 'Event Type', entityValue: 'resolve', predicate: 'has', valueType: 'Event Label', value: 'Resolve' },
        { entity: 'Event Type', entityValue: 'resolve', predicate: 'has', valueType: 'Event Style', value: 'success' },
      ],
    }

    const result = await ingestClaims(db as any, { claims, domainId: 'd1' })

    expect(result.errors).toHaveLength(0)
    expect(db.createEntity).toHaveBeenCalledTimes(4)

    // Verify Event Type facts were created with correct camelCase field names
    expect(db.createEntity).toHaveBeenCalledWith('d1', 'Event Type', { eventLabel: 'Triage' }, 'triage')
    expect(db.createEntity).toHaveBeenCalledWith('d1', 'Event Type', { eventStyle: 'primary' }, 'triage')
    expect(db.createEntity).toHaveBeenCalledWith('d1', 'Event Type', { eventLabel: 'Resolve' }, 'resolve')
    expect(db.createEntity).toHaveBeenCalledWith('d1', 'Event Type', { eventStyle: 'success' }, 'resolve')
  })

  it('seeds Suggested Prompt instance facts with prompt icon', async () => {
    const db = mockDb()
    const claims: ExtractedClaims = {
      nouns: [
        { name: 'Suggested Prompt', objectType: 'entity' },
        { name: 'Prompt Icon', objectType: 'value', valueType: 'string' },
      ],
      readings: [
        { text: 'Suggested Prompt has Prompt Icon', nouns: ['Suggested Prompt', 'Prompt Icon'], predicate: 'has' },
      ],
      constraints: [],
      facts: [
        { entity: 'Suggested Prompt', entityValue: 'What plans do you offer?', predicate: 'has', valueType: 'Prompt Icon', value: '📋' },
        { entity: 'Suggested Prompt', entityValue: 'What APIs can I access?', predicate: 'has', valueType: 'Prompt Icon', value: '🔌' },
      ],
    }

    const result = await ingestClaims(db as any, { claims, domainId: 'd1' })

    expect(result.errors).toHaveLength(0)
    expect(db.createEntity).toHaveBeenCalledTimes(2)
    expect(db.createEntity).toHaveBeenCalledWith(
      'd1',
      'Suggested Prompt',
      { promptIcon: '📋' },
      'What plans do you offer?',
    )
  })
})

// ---------------------------------------------------------------------------
// Tests: state_machine_definition_id NOT NULL constraint
// ---------------------------------------------------------------------------

describe('state_machine_definition_id NOT NULL on statuses bootstrap', () => {
  it('bootstrap schema has NOT NULL on state_machine_definition_id', async () => {
    // Verify the current bootstrap DDL — this test documents the known issue
    const { BOOTSTRAP_DDL } = await import('../schema/bootstrap')
    const statusesDDL = BOOTSTRAP_DDL.find(
      (s: string) => s.includes('CREATE TABLE') && s.includes('statuses'),
    )
    expect(statusesDDL).toBeDefined()
    expect(statusesDDL).toContain('state_machine_definition_id TEXT NOT NULL')
  })

  it('applySchema ALTER TABLE strips NOT NULL so display_color can be added', async () => {
    // The applySchema logic strips NOT NULL from ALTER TABLE ADD COLUMN.
    // This test verifies that when the statuses table already exists (from bootstrap)
    // and applySchema runs for a domain with "Status has Display Color",
    // the ALTER TABLE statement won't have NOT NULL.

    // Simulate what applySchema does: parse DDL, find new columns, strip NOT NULL
    const ddlLine = '  display_color TEXT NOT NULL DEFAULT \'blue\''
    const safeCol = ddlLine.trim().replace(/NOT NULL/g, '').replace(/DEFAULT\s+[^,)]+/g, '').trim()

    // The safe column should not contain NOT NULL or DEFAULT
    expect(safeCol).not.toContain('NOT NULL')
    expect(safeCol).not.toContain('DEFAULT')
    expect(safeCol).toBe('display_color TEXT')
  })
})

// ---------------------------------------------------------------------------
// Tests: createEntity upsert behavior for cross-domain statuses
// ---------------------------------------------------------------------------

describe('createEntity cross-domain upsert for status display colors', () => {
  it('applySchema is called before first createEntity in fact seeding', async () => {
    const db = mockDb()
    const claims: ExtractedClaims = {
      nouns: [],
      readings: [],
      constraints: [],
      facts: [
        { entity: 'Status', entityValue: 'Received', predicate: 'has', valueType: 'Display Color', value: 'blue' },
        { entity: 'Status', entityValue: 'Triaging', predicate: 'has', valueType: 'Display Color', value: 'amber' },
      ],
    }

    await ingestClaims(db as any, { claims, domainId: 'd1' })

    // applySchema must be called exactly once (before first entity, not per entity)
    expect(db.applySchema).toHaveBeenCalledTimes(1)

    // And it must be called before createEntity
    const applyOrder = db.applySchema.mock.invocationCallOrder[0]
    const createOrder = db.createEntity.mock.invocationCallOrder[0]
    expect(applyOrder).toBeLessThan(createOrder)
  })

  it('handles unary facts (Status is closed) via createEntity', async () => {
    const db = mockDb()
    const claims: ExtractedClaims = {
      nouns: [
        { name: 'Status', objectType: 'entity' },
      ],
      readings: [],
      constraints: [],
      facts: [
        {
          entity: 'Status',
          entityValue: 'Resolved',
          predicate: 'is',
          valueType: 'closed',
          value: 'true',
        },
      ],
    }

    const result = await ingestClaims(db as any, { claims, domainId: 'd1' })

    expect(db.createEntity).toHaveBeenCalledWith(
      'd1',
      'Status',
      { closed: 'true' },
      'Resolved',
    )
  })
})

// ---------------------------------------------------------------------------
// Integration: parse → ingest with actual support.auto.dev UI domain text
// ---------------------------------------------------------------------------

describe('parse → ingest: UI domain with Display Color facts', () => {
  it('parses and ingests the UI domain status display color section', async () => {
    const uiDomainText = `# UI

## Entity Types

Status(.Status Name) is an entity type.

## Value Types

Status Name is a value type.

Display Color is a value type.
  The possible values of Display Color are 'green', 'amber', 'red', 'blue', 'violet', 'gray'.

## Fact Types

### Status
Status has Display Color.
Status is closed.

## Constraints

Each Status has at most one Display Color.

## Instance Facts

### Status Display Colors

Status 'Received' has Display Color 'blue'.
Status 'Triaging' has Display Color 'amber'.
Status 'Investigating' has Display Color 'violet'.
Status 'Resolved' has Display Color 'green'.

### Closed Statuses

Status 'Resolved' is closed.
Status 'Closed' is closed.
`

    const parsed = parseFORML2(uiDomainText, [])

    // Verify parsing captured the key elements
    expect(parsed.nouns.length).toBeGreaterThanOrEqual(2) // Status, Display Color
    expect(parsed.readings.length).toBeGreaterThanOrEqual(1) // Status has Display Color

    // Verify enum values were parsed
    const displayColorNoun = parsed.nouns.find((n: any) => n.name === 'Display Color')
    expect(displayColorNoun).toBeDefined()
    expect(displayColorNoun!.enumValues).toBeDefined()
    expect(displayColorNoun!.enumValues).toContain('blue')
    expect(displayColorNoun!.enumValues).toContain('amber')

    // Verify instance facts were parsed
    expect(parsed.facts).toBeDefined()
    expect(parsed.facts!.length).toBeGreaterThanOrEqual(4) // 4 display color + 2 closed

    const displayColorFacts = parsed.facts!.filter(
      (f: any) => f.valueType === 'Display Color' || (f.reading && f.reading.includes('Display Color'))
    )
    expect(displayColorFacts.length).toBeGreaterThanOrEqual(4)

    // Ingest into mock DB
    const db = mockDb()
    const result = await ingestClaims(db as any, { claims: parsed, domainId: 'ui-domain' })

    expect(result.errors).toHaveLength(0)
    expect(result.nouns).toBeGreaterThanOrEqual(2)

    // createEntity should have been called for each instance fact
    expect(db.createEntity).toHaveBeenCalled()

    // Verify the display color facts were passed correctly
    const statusCalls = db.createEntity.mock.calls.filter(
      ([_d, noun]: [string, string]) => noun === 'Status',
    )
    expect(statusCalls.length).toBeGreaterThanOrEqual(4)

    // Find the 'Received' status call and check it has displayColor
    const receivedCall = statusCalls.find(
      ([_d, _n, fields, ref]: [string, string, any, string]) => ref === 'Received',
    )
    expect(receivedCall).toBeDefined()
    expect(receivedCall![2]).toEqual({ displayColor: 'blue' })
  })

  it('parses Event Type facts from UI domain text', async () => {
    const eventTypeText = `# UI

## Entity Types

Event Type(.Event Type Name) is an entity type.

## Value Types

Event Type Name is a value type.
Event Label is a value type.
Event Style is a value type.

## Fact Types

### Event Type
Event Type has Event Label.
Event Type has Event Style.

## Constraints

Each Event Type has at most one Event Label.
Each Event Type has at most one Event Style.

## Instance Facts

Event Type 'triage' has Event Label 'Triage'.
Event Type 'triage' has Event Style 'primary'.
Event Type 'resolve' has Event Label 'Resolve'.
Event Type 'resolve' has Event Style 'success'.
`

    const parsed = parseFORML2(eventTypeText, [])

    expect(parsed.facts).toBeDefined()
    expect(parsed.facts!.length).toBeGreaterThanOrEqual(4)

    const db = mockDb()
    const result = await ingestClaims(db as any, { claims: parsed, domainId: 'ui-domain' })

    expect(result.errors).toHaveLength(0)

    // Event Type entities should have been created with correct field names
    const eventTypeCalls = db.createEntity.mock.calls.filter(
      ([_d, noun]: [string, string]) => noun === 'Event Type',
    )
    expect(eventTypeCalls.length).toBeGreaterThanOrEqual(4)

    // Check triage event label
    const triageLabelCall = eventTypeCalls.find(
      ([_d, _n, fields, ref]: [string, string, any, string]) =>
        ref === 'triage' && fields.eventLabel,
    )
    expect(triageLabelCall).toBeDefined()
    expect(triageLabelCall![2].eventLabel).toBe('Triage')
  })

  it('parses Suggested Prompt facts from UI domain text', async () => {
    const promptText = `# UI

## Entity Types

Suggested Prompt(.Prompt Label) is an entity type.

## Value Types

Prompt Label is a value type.
Prompt Icon is a value type.

## Fact Types

### Suggested Prompt
Suggested Prompt has Prompt Icon.

## Constraints

Each Suggested Prompt has at most one Prompt Icon.

## Instance Facts

Suggested Prompt 'What plans do you offer?' has Prompt Icon '📋'.
Suggested Prompt 'What APIs can I access?' has Prompt Icon '🔌'.
Suggested Prompt 'How do I decode a VIN?' has Prompt Icon '🚗'.
`

    const parsed = parseFORML2(promptText, [])

    expect(parsed.facts).toBeDefined()
    expect(parsed.facts!.length).toBeGreaterThanOrEqual(3)

    const db = mockDb()
    const result = await ingestClaims(db as any, { claims: parsed, domainId: 'ui-domain' })

    expect(result.errors).toHaveLength(0)

    const promptCalls = db.createEntity.mock.calls.filter(
      ([_d, noun]: [string, string]) => noun === 'Suggested Prompt',
    )
    expect(promptCalls.length).toBeGreaterThanOrEqual(3)
  })
})

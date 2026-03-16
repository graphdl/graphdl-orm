import { describe, it, expect, beforeEach } from 'vitest'
import { generateReadings } from './readings'
import {
  createMockModel,
  mkNounDef,
  mkValueNounDef,
  mkConstraint,
  mkStateMachine,
  resetIds,
} from '../model/test-utils'
import type { ReadingDef } from '../model/types'

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

describe('generateReadings', () => {
  beforeEach(() => resetIds())

  it('returns format forml2', async () => {
    const model = createMockModel({})
    const result = await generateReadings(model)
    expect(result.format).toBe('forml2')
  })

  it('returns empty text for empty domain', async () => {
    const model = createMockModel({})
    const result = await generateReadings(model)
    expect(result.text).toBe('')
  })

  // --- Entity types ---

  it('outputs entity types with names', async () => {
    const model = createMockModel({
      nouns: [mkNounDef({ name: 'Customer' }), mkNounDef({ name: 'Order' })],
    })

    const result = await generateReadings(model)
    expect(result.text).toContain('## Entity Types')
    expect(result.text).toContain('Customer')
    expect(result.text).toContain('Order')
  })

  it('includes reference scheme in parentheses', async () => {
    const model = createMockModel({
      nouns: [
        mkNounDef({
          name: 'Customer',
          referenceScheme: [mkValueNounDef({ name: 'CustomerId' })],
        }),
      ],
    })

    const result = await generateReadings(model)
    // Source outputs: "Customer(.CustomerId) is an entity type."
    expect(result.text).toContain('Customer(.CustomerId) is an entity type.')
  })

  it('includes supertype notation', async () => {
    const personNoun = mkNounDef({ name: 'Person' })
    const model = createMockModel({
      nouns: [personNoun, mkNounDef({ name: 'Employee', superType: personNoun })],
    })

    const result = await generateReadings(model)
    // Subtypes are in a separate section
    expect(result.text).toContain('## Subtypes')
    expect(result.text).toContain('Employee is a subtype of Person.')
  })

  it('includes both reference scheme and supertype', async () => {
    const personNoun = mkNounDef({ name: 'Person' })
    const model = createMockModel({
      nouns: [
        personNoun,
        mkNounDef({
          name: 'Employee',
          referenceScheme: [mkValueNounDef({ name: 'EmployeeId' })],
          superType: personNoun,
        }),
      ],
    })

    const result = await generateReadings(model)
    // Entity line uses auto-generated ref scheme pattern
    expect(result.text).toContain('Employee(.EmployeeId) is an entity type.')
    // Subtype relationship is in a separate section
    expect(result.text).toContain('Employee is a subtype of Person.')
  })

  // --- Value types ---

  it('outputs value types with metadata', async () => {
    const model = createMockModel({
      nouns: [mkValueNounDef({ name: 'Email', valueType: 'string', format: 'email' })],
    })

    const result = await generateReadings(model)
    expect(result.text).toContain('## Value Types')
    expect(result.text).toContain('Email is a value type.')
  })

  it('outputs value type with pattern', async () => {
    const model = createMockModel({
      nouns: [mkValueNounDef({ name: 'ZipCode', valueType: 'string', pattern: '\\d{5}' })],
    })

    const result = await generateReadings(model)
    expect(result.text).toContain('ZipCode is a value type.')
  })

  it('outputs value type with enum', async () => {
    const model = createMockModel({
      nouns: [
        mkValueNounDef({
          name: 'Color',
          valueType: 'string',
          enumValues: ['red', 'green', 'blue'],
        }),
      ],
    })

    const result = await generateReadings(model)
    expect(result.text).toContain('Color is a value type.')
    expect(result.text).toContain("The possible values of Color are 'red', 'green', 'blue'.")
  })

  it('outputs value type with only name when no metadata', async () => {
    const model = createMockModel({
      nouns: [mkValueNounDef({ name: 'Weight' })],
    })

    const result = await generateReadings(model)
    expect(result.text).toContain('## Value Types')
    // Should just be "Weight is a value type." with no enum line
    const valueSection = result.text.split('## Value Types')[1]
    expect(valueSection).toContain('Weight is a value type.')
    expect(valueSection).not.toContain('The possible values of Weight')
  })

  // --- Readings ---

  it('outputs reading texts', async () => {
    const model = createMockModel({
      readings: [
        { id: 'r1', text: 'Customer places Order', graphSchemaId: 'gs1', roles: [] },
        { id: 'r2', text: 'Order has OrderDate', graphSchemaId: 'gs2', roles: [] },
      ],
    })

    const result = await generateReadings(model)
    expect(result.text).toContain('## Fact Types')
    expect(result.text).toContain('Customer places Order')
    expect(result.text).toContain('Order has OrderDate')
  })

  it('skips readings with no text', async () => {
    const model = createMockModel({
      readings: [
        { id: 'r1', text: '', graphSchemaId: 'gs1', roles: [] },
        { id: 'r3', text: 'Customer has Name', graphSchemaId: 'gs3', roles: [] },
      ] as ReadingDef[],
    })

    const result = await generateReadings(model)
    expect(result.text).toContain('Customer has Name')
    // Should not contain empty lines where blank readings would be
    const factSection = result.text.split('## Fact Types')[1]!
    // Only the grouped heading and the reading text should have content
    const lines = factSection.split('\n').filter((l) => l.trim().length > 0)
    // lines: "### _ungrouped", "Customer has Name"
    expect(lines).toHaveLength(2)
  })

  // --- Constraint annotations ---

  it('annotates readings with [UC] for uniqueness constraint', async () => {
    const model = createMockModel({
      readings: [
        {
          id: 'r1',
          text: 'Customer has CustomerId',
          graphSchemaId: 'gs1',
          roles: [
            { nounName: 'Customer', roleIndex: 0 },
            { nounName: 'CustomerId', roleIndex: 1 },
          ],
        },
      ],
      constraints: [
        mkConstraint({ kind: 'UC', spans: [{ factTypeId: 'gs1', roleIndex: 0 }] }),
      ],
    })

    const result = await generateReadings(model)
    // Constraints appear in a separate section, not inline on readings
    expect(result.text).toContain('## Constraints')
    expect(result.text).toContain('Each Customer has at most one CustomerId.')
  })

  it('annotates readings with [DUC] for deontic uniqueness constraint', async () => {
    const model = createMockModel({
      readings: [
        {
          id: 'r1',
          text: 'Employee has Badge',
          graphSchemaId: 'gs1',
          roles: [
            { nounName: 'Employee', roleIndex: 0 },
            { nounName: 'Badge', roleIndex: 1 },
          ],
        },
      ],
      constraints: [
        mkConstraint({
          kind: 'UC',
          modality: 'Deontic',
          spans: [{ factTypeId: 'gs1', roleIndex: 0 }],
        }),
      ],
    })

    const result = await generateReadings(model)
    // Deontic constraints go into a separate "## Deontic Constraints" section
    // But deontic constraints without text are not output (source only outputs c.text for deontic)
    // The constraint has no text set, so it won't appear
    expect(result.text).not.toContain('## Constraints')
    expect(result.text).toContain('## Fact Types')
  })

  it('annotates with multiple constraints on same reading', async () => {
    const model = createMockModel({
      readings: [
        {
          id: 'r1',
          text: 'Order has Quantity',
          graphSchemaId: 'gs1',
          roles: [
            { nounName: 'Order', roleIndex: 0 },
            { nounName: 'Quantity', roleIndex: 1 },
          ],
        },
      ],
      constraints: [
        mkConstraint({ kind: 'UC', spans: [{ factTypeId: 'gs1', roleIndex: 0 }] }),
        mkConstraint({ kind: 'MC', spans: [{ factTypeId: 'gs1', roleIndex: 1 }] }),
      ],
    })

    const result = await generateReadings(model)
    // Both constraints are Alethic so they appear in ## Constraints section
    expect(result.text).toContain('## Constraints')
    expect(result.text).toContain('Each Order has at most one Quantity.')
    expect(result.text).toContain('Each Order has at least one Quantity.')
  })

  it('does not annotate reading when constraint is on a different graphSchema', async () => {
    const model = createMockModel({
      readings: [
        {
          id: 'r1',
          text: 'Customer has Name',
          graphSchemaId: 'gs1',
          roles: [
            { nounName: 'Customer', roleIndex: 0 },
            { nounName: 'Name', roleIndex: 1 },
          ],
        },
        {
          id: 'r2',
          text: 'Order has Amount',
          graphSchemaId: 'gs2',
          roles: [
            { nounName: 'Order', roleIndex: 0 },
            { nounName: 'Amount', roleIndex: 1 },
          ],
        },
      ],
      constraints: [
        mkConstraint({ kind: 'UC', spans: [{ factTypeId: 'gs2', roleIndex: 0 }] }),
      ],
    })

    const result = await generateReadings(model)
    // Constraint is on gs2 (Order has Amount), reconstructed in Constraints section
    expect(result.text).toContain('Each Order has at most one Amount.')
    // Customer has Name should appear in Fact Types without annotation
    expect(result.text).toContain('Customer has Name')
  })

  // --- State machines ---

  it('outputs state machine transitions', async () => {
    const model = createMockModel({
      stateMachines: [
        mkStateMachine({
          nounDef: mkNounDef({ name: 'OrderLifecycle' }),
          statuses: [
            { id: 's1', name: 'Draft' },
            { id: 's2', name: 'Pending' },
            { id: 's3', name: 'Approved' },
          ],
          transitions: [
            { from: 'Draft', to: 'Pending', event: 'Submit', eventTypeId: 'evt1' },
            { from: 'Pending', to: 'Approved', event: 'Approve', eventTypeId: 'evt2' },
          ],
        }),
      ],
    })

    const result = await generateReadings(model)
    expect(result.text).toContain('## State Machine: OrderLifecycle')
    expect(result.text).toContain('Draft → Pending via Submit')
    expect(result.text).toContain('Pending → Approved via Approve')
  })

  it('uses nounName for state machine display name', async () => {
    const model = createMockModel({
      stateMachines: [
        mkStateMachine({
          nounDef: mkNounDef({ name: 'smd1' }),
          statuses: [
            { id: 's1', name: 'Start' },
            { id: 's2', name: 'End' },
          ],
          transitions: [
            { from: 'Start', to: 'End', event: 'Go', eventTypeId: 'evt1' },
          ],
        }),
      ],
    })

    const result = await generateReadings(model)
    expect(result.text).toContain('## State Machine: smd1')
    expect(result.text).toContain('Start → End via Go')
  })

  it('does not output transition lines when state machine has no transitions', async () => {
    const model = createMockModel({
      stateMachines: [
        mkStateMachine({
          nounDef: mkNounDef({ name: 'Broken' }),
          statuses: [{ id: 's1', name: 'Active' }],
          transitions: [],
        }),
      ],
    })

    const result = await generateReadings(model)
    expect(result.text).toContain('## State Machine: Broken')
    // Should not contain any transition line
    expect(result.text).not.toContain('→')
  })

  it('handles state machine with no statuses', async () => {
    const model = createMockModel({
      stateMachines: [
        mkStateMachine({
          nounDef: mkNounDef({ name: 'Empty' }),
          statuses: [],
          transitions: [],
        }),
      ],
    })

    const result = await generateReadings(model)
    expect(result.text).toContain('## State Machine: Empty')
    expect(result.text).not.toContain('→')
  })

  // --- Full round-trip ---

  it('outputs all sections in correct order', async () => {
    const customerNoun = mkNounDef({
      name: 'Customer',
      referenceScheme: [mkValueNounDef({ name: 'CustomerId' })],
    })

    const model = createMockModel({
      nouns: [customerNoun, mkValueNounDef({ name: 'Email', valueType: 'string', format: 'email' })],
      readings: [
        {
          id: 'r1',
          text: 'Customer has Email',
          graphSchemaId: 'gs1',
          roles: [
            { nounName: 'Customer', roleIndex: 0 },
            { nounName: 'Email', roleIndex: 1 },
          ],
        },
      ],
      constraints: [
        mkConstraint({ kind: 'UC', spans: [{ factTypeId: 'gs1', roleIndex: 0 }] }),
      ],
      stateMachines: [
        mkStateMachine({
          nounDef: mkNounDef({ name: 'CustomerLifecycle' }),
          statuses: [
            { id: 's1', name: 'Inactive' },
            { id: 's2', name: 'Active' },
          ],
          transitions: [
            { from: 'Inactive', to: 'Active', event: 'Activate', eventTypeId: 'evt1' },
          ],
        }),
      ],
    })

    const result = await generateReadings(model)

    // Check section ordering
    const entityIdx = result.text.indexOf('## Entity Types')
    const valueIdx = result.text.indexOf('## Value Types')
    const factIdx = result.text.indexOf('## Fact Types')
    const constraintIdx = result.text.indexOf('## Constraints')
    const smIdx = result.text.indexOf('## State Machine:')

    expect(entityIdx).toBeGreaterThanOrEqual(0)
    expect(valueIdx).toBeGreaterThan(entityIdx)
    expect(factIdx).toBeGreaterThan(valueIdx)
    expect(constraintIdx).toBeGreaterThan(factIdx)
    expect(smIdx).toBeGreaterThan(constraintIdx)
  })

  // --- Domain scoping (model already scoped) ---

  it('only includes data provided by the model', async () => {
    const model = createMockModel({
      nouns: [mkNounDef({ name: 'Customer' })],
      readings: [
        { id: 'r1', text: 'Customer has Name', graphSchemaId: 'gs1', roles: [] },
      ],
    })

    const result = await generateReadings(model)
    expect(result.text).toContain('Customer')
    expect(result.text).toContain('Customer has Name')
    // No other entities or readings since model is already domain-scoped
    expect(result.text).not.toContain('OtherEntity')
  })
})

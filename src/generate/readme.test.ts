import { describe, it, expect, beforeEach } from 'vitest'
import { generateReadme } from './readme'
import {
  createMockModel,
  mkNounDef,
  mkValueNounDef,
  mkFactType,
  mkConstraint,
  mkStateMachine,
  resetIds,
} from '../model/test-utils'

describe('generateReadme', () => {
  beforeEach(() => resetIds())

  it('returns markdown with title and summary for empty domain', async () => {
    const model = createMockModel({ domainId: 'test-domain', nouns: [], factTypes: [], constraints: [] })
    const result = await generateReadme(model)

    expect(result.format).toBe('markdown')
    expect(result.text).toContain('# test-domain')
    expect(result.text).toContain('0 entities')
  })

  it('lists entities with properties table from binary readings', async () => {
    const customer = mkNounDef({ name: 'Customer' })
    const name = mkValueNounDef({ name: 'Name', valueType: 'string' })

    const ft = mkFactType({
      reading: 'Customer has Name',
      roles: [
        { nounDef: customer, roleIndex: 0 },
        { nounDef: name, roleIndex: 1 },
      ],
    })

    const model = createMockModel({
      nouns: [customer, name],
      factTypes: [ft],
      constraints: [mkConstraint({ kind: 'UC', spans: [{ factTypeId: ft.id, roleIndex: 0 }] })],
      readings: [{ id: 'r1', text: 'Customer has Name', graphSchemaId: ft.id, roles: ft.roles }],
    })

    const result = await generateReadme(model)
    expect(result.text).toContain('### Customer')
    expect(result.text).toContain('| Name | string | Customer has Name |')
  })

  it('shows supertype relationships', async () => {
    const resource = mkNounDef({ name: 'Resource' })
    const request = mkNounDef({ name: 'Request', superType: 'Resource' })
    const sr = mkNounDef({ name: 'SupportRequest', superType: 'Request' })

    const model = createMockModel({
      nouns: [resource, request, sr],
      factTypes: [],
      constraints: [],
    })

    const result = await generateReadme(model)
    expect(result.text).toContain('### Request ← Resource')
    expect(result.text).toContain('### SupportRequest ← Request')
    expect(result.text).toContain('**Subtypes:** Request')
  })

  it('lists value types with enum values', async () => {
    const priority = mkValueNounDef({
      name: 'Priority',
      valueType: 'string',
      enumValues: ['Low', 'Medium', 'High'],
    })

    const model = createMockModel({
      nouns: [priority],
      factTypes: [],
      constraints: [],
    })

    const result = await generateReadme(model)
    expect(result.text).toContain('## Value Types')
    expect(result.text).toContain('`Low`')
    expect(result.text).toContain('`Medium`')
    expect(result.text).toContain('`High`')
  })

  it('renders state machines with transition table', async () => {
    const srNoun = mkNounDef({ name: 'SupportRequest' })
    const sm = mkStateMachine({
      nounDef: srNoun,
      statuses: [
        { id: 's1', name: 'Received' },
        { id: 's2', name: 'Triaging' },
        { id: 's3', name: 'Resolved' },
      ],
      transitions: [
        { from: 'Received', to: 'Triaging', event: 'acknowledge', eventTypeId: 'et1' },
        { from: 'Triaging', to: 'Resolved', event: 'resolve', eventTypeId: 'et2' },
      ],
    })

    const model = createMockModel({
      nouns: [srNoun],
      factTypes: [],
      constraints: [],
      stateMachines: [sm],
    })

    const result = await generateReadme(model)
    expect(result.text).toContain('## State Machines')
    expect(result.text).toContain('### SupportRequest')
    expect(result.text).toContain('Received → Triaging → Resolved')
    expect(result.text).toContain('| Received | Triaging | `acknowledge` |')
    expect(result.text).toContain('| Triaging | Resolved | `resolve` |')
  })

  it('renders deontic constraints with icons', async () => {
    const model = createMockModel({
      nouns: [],
      factTypes: [],
      constraints: [
        mkConstraint({
          kind: 'MC',
          modality: 'Deontic',
          deonticOperator: 'obligatory',
          text: 'It is obligatory that each SupportResponse is localized to Localization \'en-US\'',
        }),
        mkConstraint({
          kind: 'MC',
          modality: 'Deontic',
          deonticOperator: 'forbidden',
          text: 'It is forbidden that SupportResponse reveals GraphSchema',
        }),
      ],
    })

    const result = await generateReadme(model)
    expect(result.text).toContain('### Policy Constraints')
    expect(result.text).toContain('📋')
    expect(result.text).toContain('🚫')
  })

  it('renders entity-to-entity relationships with arrow notation', async () => {
    const order = mkNounDef({ name: 'Order' })
    const customer = mkNounDef({ name: 'Customer' })

    const ft = mkFactType({
      reading: 'Order belongs to Customer',
      roles: [
        { nounDef: order, roleIndex: 0 },
        { nounDef: customer, roleIndex: 1 },
      ],
    })

    const model = createMockModel({
      nouns: [order, customer],
      factTypes: [ft],
      constraints: [mkConstraint({ kind: 'UC', spans: [{ factTypeId: ft.id, roleIndex: 0 }] })],
      readings: [{ id: 'r1', text: 'Order belongs to Customer', graphSchemaId: ft.id, roles: ft.roles }],
    })

    const result = await generateReadme(model)
    expect(result.text).toContain('→ Customer')
  })
})

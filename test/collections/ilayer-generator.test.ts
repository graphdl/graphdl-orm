import { describe, it, expect, beforeAll } from 'vitest'
import { initPayload } from '../helpers/initPayload'
import { seedSupportDomain, seedStateMachine } from '../helpers/seed'

let payload: any
let output: any

describe('iLayer Generator', () => {
  beforeAll(async () => {
    payload = await initPayload()
    await payload.db.connection.dropDatabase()

    // Seed domain readings AND state machine
    await seedSupportDomain(payload)
    await seedStateMachine(payload)

    const generator = await payload.create({
      collection: 'generators',
      data: {
        title: 'Support UI Layers',
        version: '1.0.0',
        databaseEngine: 'Payload',
        outputFormat: 'ilayer',
      },
    })

    output = generator.output
  }, 120_000)

  it('should generate output.files', () => {
    expect(output).toBeDefined()
    expect(output.files).toBeDefined()
  })

  it('should generate layers/index.json with items for each entity', () => {
    expect(output.files['layers/index.json']).toBeDefined()
    const index = JSON.parse(output.files['layers/index.json'])
    expect(index.name).toBe('index')
    expect(index.type).toBe('layer')
    expect(Array.isArray(index.items)).toBe(true)
    expect(index.items.length).toBeGreaterThan(0)
    // Should have items for entity nouns
    const itemTexts = index.items.map((i: any) => i.text)
    expect(itemTexts).toContain('Customer')
    expect(itemTexts).toContain('SupportRequest')
  })

  it('should generate entity FormLayer files', () => {
    const layerFiles = Object.keys(output.files).filter((f) => f.startsWith('layers/') && f !== 'layers/index.json')
    expect(layerFiles.length).toBeGreaterThan(0)
    // Should have a layer for support-requests
    expect(output.files['layers/support-requests.json']).toBeDefined()
    expect(output.files['layers/customers.json']).toBeDefined()
  })

  it('should generate valid FormLayer structure', () => {
    const layer = JSON.parse(output.files['layers/support-requests.json'])
    expect(layer.type).toBe('formLayer')
    expect(layer.layout).toBe('Rounded')
    expect(layer.title).toBe('SupportRequest')
    expect(layer.name).toBe('support-requests')
    expect(Array.isArray(layer.fieldsets)).toBe(true)
    expect(layer.fieldsets.length).toBeGreaterThan(0)
    expect(layer.fieldsets[0].header).toBe('SupportRequest')
  })

  it('should map string value nouns to text fields', () => {
    const layer = JSON.parse(output.files['layers/support-requests.json'])
    const fields = layer.fieldsets[0].fields
    const subjectField = fields.find((f: any) => f.id === 'subject')
    expect(subjectField).toBeDefined()
    expect(subjectField.type).toBe('text')
    expect(subjectField.label).toBe('Subject')
  })

  it('should map enum value nouns to select fields with options', () => {
    const layer = JSON.parse(output.files['layers/support-requests.json'])
    const fields = layer.fieldsets[0].fields
    const priorityField = fields.find((f: any) => f.id === 'priority')
    expect(priorityField).toBeDefined()
    expect(priorityField.type).toBe('select')
    expect(priorityField.options).toBeDefined()
    expect(priorityField.options).toContain('low')
    expect(priorityField.options).toContain('urgent')
  })

  it('should map email format value nouns to email fields', () => {
    const layer = JSON.parse(output.files['layers/customers.json'])
    const fields = layer.fieldsets[0].fields
    const emailField = fields.find((f: any) => f.id === 'emailAddress')
    expect(emailField).toBeDefined()
    expect(emailField.type).toBe('email')
  })

  it('should map integer value nouns to numeric fields', () => {
    const layer = JSON.parse(output.files['layers/feature-requests.json'])
    const fields = layer.fieldsets[0].fields
    const voteCountField = fields.find((f: any) => f.id === 'voteCount')
    expect(voteCountField).toBeDefined()
    expect(voteCountField.type).toBe('numeric')
  })

  it('should include state machine events as action buttons on SupportRequest', () => {
    const layer = JSON.parse(output.files['layers/support-requests.json'])
    expect(layer.actionButtons).toBeDefined()
    expect(layer.actionButtons.length).toBeGreaterThan(0)
    const buttonTexts = layer.actionButtons.map((b: any) => b.text.replace(/\s/g, '').toLowerCase())
    // Events from state machine: triage, investigate, requestInfo, customerRespond, resolve
    expect(buttonTexts).toContain('triage')
    expect(buttonTexts).toContain('resolve')
  })

  it('should include action button addresses with /state/{Entity}/{event} format', () => {
    const layer = JSON.parse(output.files['layers/support-requests.json'])
    const triageBtn = layer.actionButtons.find((b: any) => b.address.includes('triage'))
    expect(triageBtn).toBeDefined()
    expect(triageBtn.address).toBe('/state/SupportRequest/triage')
  })

  it('should include navigation links to related entities', () => {
    const layer = JSON.parse(output.files['layers/support-requests.json'])
    expect(layer.navigation).toBeDefined()
    expect(layer.navigation.length).toBeGreaterThan(0)
    // SupportRequest concerns APIProduct — should have a nav link
    const navTexts = layer.navigation.map((n: any) => n.text)
    expect(navTexts).toContain('APIProduct')
  })

  it('should not include action buttons on entities without state machines', () => {
    const layer = JSON.parse(output.files['layers/customers.json'])
    // Customer has no state machine, so no actionButtons key or empty
    expect(layer.actionButtons).toBeUndefined()
  })

  it('should include items in index layer with links to entity layers', () => {
    const index = JSON.parse(output.files['layers/index.json'])
    const srItem = index.items.find((i: any) => i.text === 'SupportRequest')
    expect(srItem).toBeDefined()
    expect(srItem.link).toBe('/layers/support-requests')
  })

  it('should map string value nouns with enum as select type', () => {
    const layer = JSON.parse(output.files['layers/support-requests.json'])
    const fields = layer.fieldsets[0].fields
    const channelField = fields.find((f: any) => f.id === 'channelName')
    expect(channelField).toBeDefined()
    expect(channelField.type).toBe('select')
    expect(channelField.options).toContain('Slack')
    expect(channelField.options).toContain('Email')
  })
}, 120_000)

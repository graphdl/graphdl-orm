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
    expect(index.title).toBe('Home')
    expect(Array.isArray(index.items)).toBe(true)
    expect(index.items.length).toBeGreaterThan(0)
    // Items are in a list group
    const listItems = index.items[0].items
    const itemTexts = listItems.map((i: any) => i.text)
    expect(itemTexts).toContain('Customer')
    expect(itemTexts).toContain('SupportRequest')
  })

  it('should generate list layers (NavigationLayer) for entities with list permission', () => {
    expect(output.files['layers/support-requests.json']).toBeDefined()
    const layer = JSON.parse(output.files['layers/support-requests.json'])
    expect(layer.type).toBe('layer')
    expect(layer.title).toBe('SupportRequest')
    expect(layer.name).toBe('support-requests')
    expect(Array.isArray(layer.items)).toBe(true)
  })

  it('should generate detail layers (FormLayer) for entities with read permission', () => {
    expect(output.files['layers/support-requests-detail.json']).toBeDefined()
    const layer = JSON.parse(output.files['layers/support-requests-detail.json'])
    expect(layer.type).toBe('formLayer')
    expect(layer.layout).toBe('Rounded')
    expect(layer.title).toBe('SupportRequest')
    expect(Array.isArray(layer.fieldsets)).toBe(true)
    expect(layer.fieldsets.length).toBeGreaterThan(0)
    // Detail fields are read-only labels
    const fields = layer.fieldsets[0].fields
    expect(fields.every((f: any) => f.type === 'label')).toBe(true)
  })

  it('should generate create layers for entities with create permission', () => {
    expect(output.files['layers/support-requests-new.json']).toBeDefined()
    const layer = JSON.parse(output.files['layers/support-requests-new.json'])
    expect(layer.type).toBe('formLayer')
    expect(layer.title).toContain('New')
    expect(layer.actionButtons).toBeDefined()
    const saveBtn = layer.actionButtons.find((b: any) => b.action === 'create')
    expect(saveBtn).toBeDefined()
  })

  it('should generate edit layers for entities with update permission', () => {
    expect(output.files['layers/support-requests-edit.json']).toBeDefined()
    const layer = JSON.parse(output.files['layers/support-requests-edit.json'])
    expect(layer.type).toBe('formLayer')
    expect(layer.title).toContain('Edit')
    const saveBtn = layer.actionButtons.find((b: any) => b.action === 'update')
    expect(saveBtn).toBeDefined()
  })

  it('should map string value nouns to text fields in create layer', () => {
    const layer = JSON.parse(output.files['layers/support-requests-new.json'])
    const fields = layer.fieldsets[0].fields
    const subjectField = fields.find((f: any) => f.id === 'subject')
    expect(subjectField).toBeDefined()
    expect(subjectField.type).toBe('text')
    expect(subjectField.label).toBe('Subject')
  })

  it('should map enum value nouns to select fields with options', () => {
    const layer = JSON.parse(output.files['layers/support-requests-new.json'])
    const fields = layer.fieldsets[0].fields
    const priorityField = fields.find((f: any) => f.id === 'priority')
    expect(priorityField).toBeDefined()
    expect(priorityField.type).toBe('select')
    expect(priorityField.options).toContain('low')
    expect(priorityField.options).toContain('urgent')
  })

  it('should map email format value nouns to email fields', () => {
    expect(output.files['layers/customers-new.json']).toBeDefined()
    const layer = JSON.parse(output.files['layers/customers-new.json'])
    const fields = layer.fieldsets[0].fields
    const emailField = fields.find((f: any) => f.id === 'emailAddress')
    expect(emailField).toBeDefined()
    expect(emailField.type).toBe('email')
  })

  it('should map integer value nouns to numeric fields', () => {
    expect(output.files['layers/feature-requests-new.json']).toBeDefined()
    const layer = JSON.parse(output.files['layers/feature-requests-new.json'])
    const fields = layer.fieldsets[0].fields
    const voteCountField = fields.find((f: any) => f.id === 'voteCount')
    expect(voteCountField).toBeDefined()
    expect(voteCountField.type).toBe('numeric')
  })

  it('should include state machine events as action buttons on detail layer', () => {
    const layer = JSON.parse(output.files['layers/support-requests-detail.json'])
    expect(layer.actionButtons).toBeDefined()
    expect(layer.actionButtons.length).toBeGreaterThan(0)
    const buttonTexts = layer.actionButtons.map((b: any) => b.text.replace(/\s/g, '').toLowerCase())
    expect(buttonTexts).toContain('triage')
    expect(buttonTexts).toContain('resolve')
  })

  it('should include action button addresses with /state/{Entity}/{event} format', () => {
    const layer = JSON.parse(output.files['layers/support-requests-detail.json'])
    const triageBtn = layer.actionButtons.find((b: any) => b.address?.includes('triage'))
    expect(triageBtn).toBeDefined()
    expect(triageBtn.address).toBe('/state/SupportRequest/triage')
  })

  it('should include navigation links to related entities on detail layer', () => {
    const layer = JSON.parse(output.files['layers/support-requests-detail.json'])
    expect(layer.navigation).toBeDefined()
    expect(layer.navigation.length).toBeGreaterThan(0)
    const navTexts = layer.navigation.map((n: any) => n.text)
    expect(navTexts).toContain('APIProduct')
  })

  it('should include index items with address links to entity list layers', () => {
    const index = JSON.parse(output.files['layers/index.json'])
    const listItems = index.items[0].items
    const srItem = listItems.find((i: any) => i.text === 'SupportRequest')
    expect(srItem).toBeDefined()
    expect(srItem.address).toBe('/support-requests')
  })

  it('should include "New" button on list layer for entities with create permission', () => {
    const layer = JSON.parse(output.files['layers/support-requests.json'])
    expect(layer.actionButtons).toBeDefined()
    const newBtn = layer.actionButtons.find((b: any) => b.address?.includes('/new'))
    expect(newBtn).toBeDefined()
  })
}, 120_000)

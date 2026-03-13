import { describe, it, expect, beforeEach } from 'vitest'
import { generateILayer } from './ilayer'
import { createMockModel, mkNounDef, mkValueNounDef, mkFactType, mkStateMachine, resetIds } from '../model/test-utils'

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

beforeEach(() => resetIds())

describe('generateILayer', () => {
  describe('entity with list+read+create permissions', () => {
    it('generates list, detail, create layers and index', async () => {
      const customerNoun = mkNounDef({ name: 'Customer', permissions: ['list', 'read', 'create'], plural: 'customers' })
      const nameNoun = mkValueNounDef({ name: 'Name', valueType: 'string' })
      const emailNoun = mkValueNounDef({ name: 'Email', valueType: 'string', format: 'email' })

      const model = createMockModel({
        nouns: [customerNoun, nameNoun, emailNoun],
        factTypes: [
          mkFactType({ reading: 'Customer has Name', roles: [{ nounDef: customerNoun, roleIndex: 0 }, { nounDef: nameNoun, roleIndex: 1 }] }),
          mkFactType({ reading: 'Customer has Email', roles: [{ nounDef: customerNoun, roleIndex: 0 }, { nounDef: emailNoun, roleIndex: 1 }] }),
        ],
        stateMachines: [],
      })

      const result = await generateILayer(model)

      // Should have list, detail, create, and index layers
      expect(result.files['layers/customers.json']).toBeDefined()
      expect(result.files['layers/customers-detail.json']).toBeDefined()
      expect(result.files['layers/customers-new.json']).toBeDefined()
      expect(result.files['layers/index.json']).toBeDefined()

      // List layer
      const list = JSON.parse(result.files['layers/customers.json'])
      expect(list.name).toBe('customers')
      expect(list.title).toBe('Customer')
      expect(list.type).toBe('layer')
      expect(list.items[0].type).toBe('list')
      expect(list.items[0].items.length).toBe(2) // only 2 fields exist (capped at 3)
      expect(list.actionButtons).toEqual([{ id: 'create', text: 'New Customer', address: '/customers/new' }])

      // Detail layer
      const detail = JSON.parse(result.files['layers/customers-detail.json'])
      expect(detail.name).toBe('customers-detail')
      expect(detail.type).toBe('formLayer')
      expect(detail.layout).toBe('Rounded')
      expect(detail.fieldsets[0].header).toBe('Customer')
      // Detail fields should be labels
      expect(detail.fieldsets[0].fields[0].type).toBe('label')
      expect(detail.fieldsets[0].fields[1].type).toBe('label')

      // Create layer
      const create = JSON.parse(result.files['layers/customers-new.json'])
      expect(create.name).toBe('customers-new')
      expect(create.title).toBe('New Customer')
      expect(create.type).toBe('formLayer')
      expect(create.fieldsets[0].fields[0].type).toBe('text') // Name -> text
      expect(create.fieldsets[0].fields[1].type).toBe('email') // Email -> email
      expect(create.actionButtons).toContainEqual({ id: 'save', text: 'Save', action: 'create' })
      expect(create.actionButtons).toContainEqual({ id: 'cancel', text: 'Cancel', address: '/customers' })

      // Index
      const index = JSON.parse(result.files['layers/index.json'])
      expect(index.name).toBe('index')
      expect(index.title).toBe('Home')
      expect(index.items[0].items).toContainEqual(
        expect.objectContaining({ text: 'Customer', address: '/customers' }),
      )
    })
  })

  describe('field type mapping', () => {
    it('maps string->text, boolean->bool, enum->select, email->email, date->date, number->numeric', async () => {
      const entity = mkNounDef({ name: 'Task', permissions: ['create'], plural: 'tasks' })
      const titleNoun = mkValueNounDef({ name: 'Title', valueType: 'string' })
      const activeNoun = mkValueNounDef({ name: 'Active', valueType: 'boolean' })
      const priorityNoun = mkValueNounDef({ name: 'Priority', valueType: 'string', enumValues: ['Low', 'Medium', 'High'] })
      const emailNoun = mkValueNounDef({ name: 'ContactEmail', valueType: 'string', format: 'email' })
      const dueDateNoun = mkValueNounDef({ name: 'DueDate', valueType: 'string', format: 'date' })
      const countNoun = mkValueNounDef({ name: 'ItemCount', valueType: 'number' })

      const model = createMockModel({
        nouns: [entity, titleNoun, activeNoun, priorityNoun, emailNoun, dueDateNoun, countNoun],
        factTypes: [
          mkFactType({ reading: 'Task has Title', roles: [{ nounDef: entity, roleIndex: 0 }, { nounDef: titleNoun, roleIndex: 1 }] }),
          mkFactType({ reading: 'Task has Active', roles: [{ nounDef: entity, roleIndex: 0 }, { nounDef: activeNoun, roleIndex: 1 }] }),
          mkFactType({ reading: 'Task has Priority', roles: [{ nounDef: entity, roleIndex: 0 }, { nounDef: priorityNoun, roleIndex: 1 }] }),
          mkFactType({ reading: 'Task has ContactEmail', roles: [{ nounDef: entity, roleIndex: 0 }, { nounDef: emailNoun, roleIndex: 1 }] }),
          mkFactType({ reading: 'Task has DueDate', roles: [{ nounDef: entity, roleIndex: 0 }, { nounDef: dueDateNoun, roleIndex: 1 }] }),
          mkFactType({ reading: 'Task has ItemCount', roles: [{ nounDef: entity, roleIndex: 0 }, { nounDef: countNoun, roleIndex: 1 }] }),
        ],
        stateMachines: [],
      })

      const result = await generateILayer(model)
      const create = JSON.parse(result.files['layers/tasks-new.json'])
      const fields = create.fieldsets[0].fields

      const byId = (id: string) => fields.find((f: any) => f.id === id)

      expect(byId('title').type).toBe('text')
      expect(byId('active').type).toBe('bool')
      expect(byId('priority').type).toBe('select')
      expect(byId('priority').options).toEqual(['Low', 'Medium', 'High'])
      expect(byId('contactEmail').type).toBe('email')
      expect(byId('dueDate').type).toBe('date')
      expect(byId('itemCount').type).toBe('numeric')
    })
  })

  describe('navigation from entity-to-entity readings', () => {
    it('includes navigation links on detail and edit layers', async () => {
      const orderNoun = mkNounDef({ name: 'Order', permissions: ['read', 'update'], plural: 'orders' })
      const customerNoun = mkNounDef({ name: 'Customer', permissions: ['list'], plural: 'customers' })
      const amountNoun = mkValueNounDef({ name: 'Amount', valueType: 'number' })

      const model = createMockModel({
        nouns: [orderNoun, customerNoun, amountNoun],
        factTypes: [
          mkFactType({ reading: 'Order has Amount', roles: [{ nounDef: orderNoun, roleIndex: 0 }, { nounDef: amountNoun, roleIndex: 1 }] }),
          mkFactType({ reading: 'Order belongs to Customer', roles: [{ nounDef: orderNoun, roleIndex: 0 }, { nounDef: customerNoun, roleIndex: 1 }] }),
        ],
        stateMachines: [],
      })

      const result = await generateILayer(model)

      const detail = JSON.parse(result.files['layers/orders-detail.json'])
      expect(detail.navigation).toEqual([{ text: 'Customer', address: '/customers' }])

      const edit = JSON.parse(result.files['layers/orders-edit.json'])
      expect(edit.navigation).toEqual([{ text: 'Customer', address: '/customers' }])
    })
  })

  describe('event buttons from state machine transitions', () => {
    it('adds event buttons to detail and edit layers', async () => {
      const ticketNoun = mkNounDef({ name: 'Ticket', permissions: ['read', 'update'], plural: 'tickets' })
      const titleNoun = mkValueNounDef({ name: 'Title', valueType: 'string' })

      const model = createMockModel({
        nouns: [ticketNoun, titleNoun],
        factTypes: [
          mkFactType({ reading: 'Ticket has Title', roles: [{ nounDef: ticketNoun, roleIndex: 0 }, { nounDef: titleNoun, roleIndex: 1 }] }),
        ],
        stateMachines: [
          mkStateMachine({
            nounDef: ticketNoun,
            statuses: [{ id: 'status-1', name: 'Open' }, { id: 'status-2', name: 'Closed' }],
            transitions: [
              { from: 'Open', to: 'Closed', event: 'Escalate', eventTypeId: 'et-1' },
              { from: 'Closed', to: 'Open', event: 'Close', eventTypeId: 'et-2' },
            ],
          }),
        ],
      })

      const result = await generateILayer(model)

      const detail = JSON.parse(result.files['layers/tickets-detail.json'])
      // Detail should have edit + event buttons
      const eventBtns = detail.actionButtons.filter((b: any) => b.address)
      expect(eventBtns).toContainEqual({ id: 'Escalate', text: 'Escalate', address: '/state/Ticket/Escalate' })
      expect(eventBtns).toContainEqual({ id: 'Close', text: 'Close', address: '/state/Ticket/Close' })

      const edit = JSON.parse(result.files['layers/tickets-edit.json'])
      const editEventBtns = edit.actionButtons.filter((b: any) => b.address && b.id !== 'cancel')
      expect(editEventBtns).toContainEqual({ id: 'Escalate', text: 'Escalate', address: '/state/Ticket/Escalate' })
    })
  })

  describe('index layer lists all entities with list/read permission', () => {
    it('includes multiple entities in index', async () => {
      const customerNoun = mkNounDef({ name: 'Customer', permissions: ['list'], plural: 'customers' })
      const orderNoun = mkNounDef({ name: 'Order', permissions: ['read'], plural: 'orders' })
      const secretNoun = mkNounDef({ name: 'Secret', permissions: ['create'] }) // no list/read

      const model = createMockModel({
        nouns: [customerNoun, orderNoun, secretNoun],
        factTypes: [],
        stateMachines: [],
      })

      const result = await generateILayer(model)
      const index = JSON.parse(result.files['layers/index.json'])
      const texts = index.items[0].items.map((i: any) => i.text)

      expect(texts).toContain('Customer')
      expect(texts).toContain('Order')
      expect(texts).not.toContain('Secret')
    })
  })

  describe('entity without permissions', () => {
    it('generates no layers for the entity', async () => {
      const noPermEntity = mkNounDef({ name: 'Hidden', permissions: [] })

      const model = createMockModel({
        nouns: [noPermEntity],
        factTypes: [],
        stateMachines: [],
      })

      const result = await generateILayer(model)

      // Only the index layer should exist (with no items)
      expect(Object.keys(result.files)).toEqual(['layers/index.json'])
      const index = JSON.parse(result.files['layers/index.json'])
      expect(index.items[0].items).toEqual([])
    })
  })

  describe('update+delete permissions', () => {
    it('generates edit layer with save/cancel/event buttons and delete on detail', async () => {
      const entity = mkNounDef({ name: 'Item', permissions: ['read', 'update', 'delete'], plural: 'items' })
      const nameNoun = mkValueNounDef({ name: 'Name', valueType: 'string' })

      const model = createMockModel({
        nouns: [entity, nameNoun],
        factTypes: [
          mkFactType({ reading: 'Item has Name', roles: [{ nounDef: entity, roleIndex: 0 }, { nounDef: nameNoun, roleIndex: 1 }] }),
        ],
        stateMachines: [],
      })

      const result = await generateILayer(model)

      // Detail should have edit + delete buttons
      const detail = JSON.parse(result.files['layers/items-detail.json'])
      expect(detail.actionButtons).toContainEqual({ id: 'edit', text: 'Edit', action: 'edit' })
      expect(detail.actionButtons).toContainEqual({ id: 'delete', text: 'Delete', action: 'delete' })

      // Edit layer
      const edit = JSON.parse(result.files['layers/items-edit.json'])
      expect(edit.name).toBe('items-edit')
      expect(edit.title).toBe('Edit Item')
      expect(edit.actionButtons).toContainEqual({ id: 'save', text: 'Save', action: 'update' })
      expect(edit.actionButtons).toContainEqual({ id: 'cancel', text: 'Cancel', address: '/items/{id}' })
    })
  })

  describe('slug generation', () => {
    it('uses plural if available', async () => {
      const entity = mkNounDef({ name: 'Person', permissions: ['list'], plural: 'people' })

      const model = createMockModel({
        nouns: [entity],
        factTypes: [],
        stateMachines: [],
      })

      const result = await generateILayer(model)
      expect(result.files['layers/people.json']).toBeDefined()
    })

    it('generates slug from PascalCase name when no plural', async () => {
      const entity = mkNounDef({ name: 'SupportRequest', permissions: ['list'] })

      const model = createMockModel({
        nouns: [entity],
        factTypes: [],
        stateMachines: [],
      })

      const result = await generateILayer(model)
      expect(result.files['layers/support-requests.json']).toBeDefined()
    })
  })

  describe('deduplication of fields', () => {
    it('deduplicates fields by field ID', async () => {
      const entity = mkNounDef({ name: 'Item', permissions: ['create'], plural: 'items' })
      const nameNoun = mkValueNounDef({ name: 'Name', valueType: 'string' })

      const model = createMockModel({
        nouns: [entity, nameNoun],
        factTypes: [
          // Two fact types referencing the same value noun
          mkFactType({ reading: 'Item has Name', roles: [{ nounDef: entity, roleIndex: 0 }, { nounDef: nameNoun, roleIndex: 1 }] }),
          mkFactType({ reading: 'Item identifies Name', roles: [{ nounDef: entity, roleIndex: 0 }, { nounDef: nameNoun, roleIndex: 1 }] }),
        ],
        stateMachines: [],
      })

      const result = await generateILayer(model)
      const create = JSON.parse(result.files['layers/items-new.json'])
      const nameFields = create.fieldsets[0].fields.filter((f: any) => f.id === 'name')
      expect(nameFields.length).toBe(1)
    })
  })

  describe('list layer field cap', () => {
    it('shows at most 3 fields in list items', async () => {
      const entity = mkNounDef({ name: 'Item', permissions: ['list'], plural: 'items' })
      const f1 = mkValueNounDef({ name: 'FieldOne', valueType: 'string' })
      const f2 = mkValueNounDef({ name: 'FieldTwo', valueType: 'string' })
      const f3 = mkValueNounDef({ name: 'FieldThree', valueType: 'string' })
      const f4 = mkValueNounDef({ name: 'FieldFour', valueType: 'string' })

      const model = createMockModel({
        nouns: [entity, f1, f2, f3, f4],
        factTypes: [
          mkFactType({ reading: 'Item has FieldOne', roles: [{ nounDef: entity, roleIndex: 0 }, { nounDef: f1, roleIndex: 1 }] }),
          mkFactType({ reading: 'Item has FieldTwo', roles: [{ nounDef: entity, roleIndex: 0 }, { nounDef: f2, roleIndex: 1 }] }),
          mkFactType({ reading: 'Item has FieldThree', roles: [{ nounDef: entity, roleIndex: 0 }, { nounDef: f3, roleIndex: 1 }] }),
          mkFactType({ reading: 'Item has FieldFour', roles: [{ nounDef: entity, roleIndex: 0 }, { nounDef: f4, roleIndex: 1 }] }),
        ],
        stateMachines: [],
      })

      const result = await generateILayer(model)
      const list = JSON.parse(result.files['layers/items.json'])
      expect(list.items[0].items.length).toBe(3)
    })
  })

  describe('email detection by name', () => {
    it('infers email type from noun name containing "email"', async () => {
      const entity = mkNounDef({ name: 'Item', permissions: ['create'], plural: 'items' })
      const emailNoun = mkValueNounDef({ name: 'ContactEmail', valueType: 'string' })

      const model = createMockModel({
        nouns: [entity, emailNoun],
        factTypes: [
          mkFactType({ reading: 'Item has ContactEmail', roles: [{ nounDef: entity, roleIndex: 0 }, { nounDef: emailNoun, roleIndex: 1 }] }),
        ],
        stateMachines: [],
      })

      const result = await generateILayer(model)
      const create = JSON.parse(result.files['layers/items-new.json'])
      expect(create.fieldsets[0].fields[0].type).toBe('email')
    })
  })
})

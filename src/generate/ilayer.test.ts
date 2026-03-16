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

  describe('displayFields selection (primary, secondary, date)', () => {
    it('selects a date-format field for the date slot', async () => {
      const entity = mkNounDef({ name: 'Event', permissions: ['list'], plural: 'events' })
      const titleNoun = mkValueNounDef({ name: 'Title', valueType: 'string' })
      const startDateNoun = mkValueNounDef({ name: 'StartDate', valueType: 'string', format: 'date' })
      const descNoun = mkValueNounDef({ name: 'Description', valueType: 'string' })

      const model = createMockModel({
        nouns: [entity, titleNoun, startDateNoun, descNoun],
        factTypes: [
          mkFactType({ reading: 'Event has Title', roles: [{ nounDef: entity, roleIndex: 0 }, { nounDef: titleNoun, roleIndex: 1 }] }),
          mkFactType({ reading: 'Event has StartDate', roles: [{ nounDef: entity, roleIndex: 0 }, { nounDef: startDateNoun, roleIndex: 1 }] }),
          mkFactType({ reading: 'Event has Description', roles: [{ nounDef: entity, roleIndex: 0 }, { nounDef: descNoun, roleIndex: 1 }] }),
        ],
        stateMachines: [],
      })

      const result = await generateILayer(model)
      const list = JSON.parse(result.files['layers/events.json'])

      // Title is an identifier → primary
      expect(list.displayFields.primary).toBe('title')
      // Description is secondary
      expect(list.displayFields.secondary).toBe('description')
      // StartDate should be in the date slot
      expect(list.displayFields.date).toBe('startDate')
    })

    it('falls back to createdAt when no temporal fields exist', async () => {
      const entity = mkNounDef({ name: 'Item', permissions: ['list'], plural: 'items' })
      const nameNoun = mkValueNounDef({ name: 'Name', valueType: 'string' })

      const model = createMockModel({
        nouns: [entity, nameNoun],
        factTypes: [
          mkFactType({ reading: 'Item has Name', roles: [{ nounDef: entity, roleIndex: 0 }, { nounDef: nameNoun, roleIndex: 1 }] }),
        ],
        stateMachines: [],
      })

      const result = await generateILayer(model)
      const list = JSON.parse(result.files['layers/items.json'])

      expect(list.displayFields.primary).toBe('name')
      // With no date-format field, date should default to createdAt
      expect(list.displayFields.date).toBe('createdAt')
    })
  })

  describe('action buttons on list and detail layers', () => {
    it('list layer shows create button when entity has create permission', async () => {
      const entity = mkNounDef({ name: 'Product', permissions: ['list', 'create'], plural: 'products' })
      const nameNoun = mkValueNounDef({ name: 'Name', valueType: 'string' })

      const model = createMockModel({
        nouns: [entity, nameNoun],
        factTypes: [
          mkFactType({ reading: 'Product has Name', roles: [{ nounDef: entity, roleIndex: 0 }, { nounDef: nameNoun, roleIndex: 1 }] }),
        ],
        stateMachines: [],
      })

      const result = await generateILayer(model)
      const list = JSON.parse(result.files['layers/products.json'])

      expect(list.actionButtons).toHaveLength(1)
      expect(list.actionButtons[0]).toEqual({ id: 'create', text: 'New Product', address: '/products/new' })
    })

    it('list layer has no action buttons when entity lacks create permission', async () => {
      const entity = mkNounDef({ name: 'Log', permissions: ['list', 'read'], plural: 'logs' })
      const textNoun = mkValueNounDef({ name: 'Message', valueType: 'string' })

      const model = createMockModel({
        nouns: [entity, textNoun],
        factTypes: [
          mkFactType({ reading: 'Log has Message', roles: [{ nounDef: entity, roleIndex: 0 }, { nounDef: textNoun, roleIndex: 1 }] }),
        ],
        stateMachines: [],
      })

      const result = await generateILayer(model)
      const list = JSON.parse(result.files['layers/logs.json'])

      // No create permission → no actionButtons
      expect(list.actionButtons).toBeUndefined()
    })

    it('detail layer shows edit and delete buttons for read+update+delete', async () => {
      const entity = mkNounDef({ name: 'Task', permissions: ['read', 'update', 'delete'], plural: 'tasks' })
      const titleNoun = mkValueNounDef({ name: 'Title', valueType: 'string' })

      const model = createMockModel({
        nouns: [entity, titleNoun],
        factTypes: [
          mkFactType({ reading: 'Task has Title', roles: [{ nounDef: entity, roleIndex: 0 }, { nounDef: titleNoun, roleIndex: 1 }] }),
        ],
        stateMachines: [],
      })

      const result = await generateILayer(model)
      const detail = JSON.parse(result.files['layers/tasks-detail.json'])

      expect(detail.actionButtons).toContainEqual({ id: 'edit', text: 'Edit', action: 'edit' })
      expect(detail.actionButtons).toContainEqual({ id: 'delete', text: 'Delete', action: 'delete' })
    })

    it('detail layer omits delete button when entity has no delete permission', async () => {
      const entity = mkNounDef({ name: 'Task', permissions: ['read', 'update'], plural: 'tasks' })
      const titleNoun = mkValueNounDef({ name: 'Title', valueType: 'string' })

      const model = createMockModel({
        nouns: [entity, titleNoun],
        factTypes: [
          mkFactType({ reading: 'Task has Title', roles: [{ nounDef: entity, roleIndex: 0 }, { nounDef: titleNoun, roleIndex: 1 }] }),
        ],
        stateMachines: [],
      })

      const result = await generateILayer(model)
      const detail = JSON.parse(result.files['layers/tasks-detail.json'])

      expect(detail.actionButtons).toContainEqual({ id: 'edit', text: 'Edit', action: 'edit' })
      const deleteBtn = detail.actionButtons.find((b: any) => b.id === 'delete')
      expect(deleteBtn).toBeUndefined()
    })
  })

  describe('navigation links between related entities', () => {
    it('adds navigation links for multiple entity-to-entity relationships', async () => {
      const orderNoun = mkNounDef({ name: 'Order', permissions: ['read', 'update'], plural: 'orders' })
      const customerNoun = mkNounDef({ name: 'Customer', permissions: ['list'], plural: 'customers' })
      const warehouseNoun = mkNounDef({ name: 'Warehouse', permissions: ['list'], plural: 'warehouses' })
      const amountNoun = mkValueNounDef({ name: 'Amount', valueType: 'number' })

      const model = createMockModel({
        nouns: [orderNoun, customerNoun, warehouseNoun, amountNoun],
        factTypes: [
          mkFactType({ reading: 'Order has Amount', roles: [{ nounDef: orderNoun, roleIndex: 0 }, { nounDef: amountNoun, roleIndex: 1 }] }),
          mkFactType({ reading: 'Order belongs to Customer', roles: [{ nounDef: orderNoun, roleIndex: 0 }, { nounDef: customerNoun, roleIndex: 1 }] }),
          mkFactType({ reading: 'Order ships from Warehouse', roles: [{ nounDef: orderNoun, roleIndex: 0 }, { nounDef: warehouseNoun, roleIndex: 1 }] }),
        ],
        stateMachines: [],
      })

      const result = await generateILayer(model)
      const detail = JSON.parse(result.files['layers/orders-detail.json'])

      expect(detail.navigation).toHaveLength(2)
      expect(detail.navigation).toContainEqual({ text: 'Customer', address: '/customers' })
      expect(detail.navigation).toContainEqual({ text: 'Warehouse', address: '/warehouses' })

      const edit = JSON.parse(result.files['layers/orders-edit.json'])
      expect(edit.navigation).toHaveLength(2)
      expect(edit.navigation).toContainEqual({ text: 'Customer', address: '/customers' })
      expect(edit.navigation).toContainEqual({ text: 'Warehouse', address: '/warehouses' })
    })

    it('does not add navigation for value-type readings', async () => {
      const orderNoun = mkNounDef({ name: 'Order', permissions: ['read'], plural: 'orders' })
      const totalNoun = mkValueNounDef({ name: 'Total', valueType: 'number' })

      const model = createMockModel({
        nouns: [orderNoun, totalNoun],
        factTypes: [
          mkFactType({ reading: 'Order has Total', roles: [{ nounDef: orderNoun, roleIndex: 0 }, { nounDef: totalNoun, roleIndex: 1 }] }),
        ],
        stateMachines: [],
      })

      const result = await generateILayer(model)
      const detail = JSON.parse(result.files['layers/orders-detail.json'])

      // Value-type reading should not produce navigation
      expect(detail.navigation).toBeUndefined()
    })
  })

  describe('semantic display field selection', () => {
    it('picks Subject as primary and a multi-value enum as secondary (not positional)', async () => {
      const requestNoun = mkNounDef({ name: 'SupportRequest', permissions: ['list'], plural: 'support-requests' })
      const channelNoun = mkValueNounDef({ name: 'ChannelName', valueType: 'string', enumValues: ['Email'] })
      const priorityNoun = mkValueNounDef({ name: 'Priority', valueType: 'string', enumValues: ['low', 'medium', 'high'] })
      const issueTypeNoun = mkValueNounDef({ name: 'IssueType', valueType: 'string', enumValues: ['general', 'billing', 'technical'] })
      const subjectNoun = mkValueNounDef({ name: 'Subject', valueType: 'string' })

      const model = createMockModel({
        nouns: [requestNoun, channelNoun, priorityNoun, issueTypeNoun, subjectNoun],
        factTypes: [
          mkFactType({ reading: 'SupportRequest arrives via ChannelName', roles: [{ nounDef: requestNoun, roleIndex: 0 }, { nounDef: channelNoun, roleIndex: 1 }] }),
          mkFactType({ reading: 'SupportRequest has Priority', roles: [{ nounDef: requestNoun, roleIndex: 0 }, { nounDef: priorityNoun, roleIndex: 1 }] }),
          mkFactType({ reading: 'SupportRequest has IssueType', roles: [{ nounDef: requestNoun, roleIndex: 0 }, { nounDef: issueTypeNoun, roleIndex: 1 }] }),
          mkFactType({ reading: 'SupportRequest has Subject', roles: [{ nounDef: requestNoun, roleIndex: 0 }, { nounDef: subjectNoun, roleIndex: 1 }] }),
        ],
        stateMachines: [],
      })

      const result = await generateILayer(model)
      const list = JSON.parse(result.files['layers/support-requests.json'])

      // Subject is an identifying label → primary
      expect(list.displayFields.primary).toBe('subject')
      // Secondary should be a multi-value enum (Priority or IssueType), NOT channelName (single-value enum)
      expect(['priority', 'issueType']).toContain(list.displayFields.secondary)
      expect(list.displayFields.secondary).not.toBe('channelName')
    })

    it('picks Name as primary over description for a Customer entity', async () => {
      const customerNoun = mkNounDef({ name: 'Customer', permissions: ['list'], plural: 'customers' })
      const descriptionNoun = mkValueNounDef({ name: 'Description', valueType: 'string' })
      const nameNoun = mkValueNounDef({ name: 'Name', valueType: 'string' })

      const model = createMockModel({
        nouns: [customerNoun, descriptionNoun, nameNoun],
        factTypes: [
          mkFactType({ reading: 'Customer has Description', roles: [{ nounDef: customerNoun, roleIndex: 0 }, { nounDef: descriptionNoun, roleIndex: 1 }] }),
          mkFactType({ reading: 'Customer has Name', roles: [{ nounDef: customerNoun, roleIndex: 0 }, { nounDef: nameNoun, roleIndex: 1 }] }),
        ],
        stateMachines: [],
      })

      const result = await generateILayer(model)
      const list = JSON.parse(result.files['layers/customers.json'])

      // Name is an identifier → primary (not Description, which was first in readings)
      expect(list.displayFields.primary).toBe('name')
      expect(list.displayFields.secondary).toBe('description')
    })

    it('follows supertype chain to find inherited fields', async () => {
      const requestNoun = mkNounDef({ name: 'Request', permissions: [] })
      const supportRequestNoun = mkNounDef({ name: 'SupportRequest', permissions: ['list'], superType: requestNoun })
      const subjectNoun = mkValueNounDef({ name: 'Subject', valueType: 'string' })
      const issueTypeNoun = mkValueNounDef({ name: 'IssueType', valueType: 'string', enumValues: ['general', 'billing'] })

      const model = createMockModel({
        nouns: [requestNoun, supportRequestNoun, subjectNoun, issueTypeNoun],
        factTypes: [
          // Subject is on the parent type Request
          mkFactType({ reading: 'Request has Subject', roles: [{ nounDef: requestNoun, roleIndex: 0 }, { nounDef: subjectNoun, roleIndex: 1 }] }),
          mkFactType({ reading: 'SupportRequest has IssueType', roles: [{ nounDef: supportRequestNoun, roleIndex: 0 }, { nounDef: issueTypeNoun, roleIndex: 1 }] }),
        ],
        stateMachines: [],
      })

      const result = await generateILayer(model)
      const list = JSON.parse(result.files['layers/support-requests.json'])

      // Subject is inherited from Request via supertype chain
      // IssueType is the best secondary (enum categorization)
      expect(list.displayFields.primary).toBe('issueType')
      expect(list.displayFields.secondary).toBeUndefined()
    })
  })
})

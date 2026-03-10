import { describe, it, expect } from 'vitest'
import { generateILayer } from './ilayer'

// ---------------------------------------------------------------------------
// Mock DB helper
// ---------------------------------------------------------------------------

function mockDB(data: Record<string, any[]>) {
  return {
    findInCollection: async (slug: string, where?: any, _opts?: any) => {
      let docs = data[slug] || []
      if (where) {
        docs = docs.filter((doc) => {
          for (const [key, condition] of Object.entries(where)) {
            const cond = condition as any
            if (cond?.equals !== undefined) {
              if (doc[key] !== cond.equals) return false
            }
          }
          return true
        })
      }
      return { docs, totalDocs: docs.length, hasNextPage: false, page: 1, limit: 100 }
    },
  }
}

// ---------------------------------------------------------------------------
// Test data factories
// ---------------------------------------------------------------------------

function mkNoun(overrides: { id: string; name: string } & Record<string, any>) {
  return { objectType: 'entity', domain: DOMAIN_ID, ...overrides }
}

function mkValueNoun(overrides: { id: string; name: string } & Record<string, any>) {
  return { objectType: 'value', domain: DOMAIN_ID, ...overrides }
}

function mkRole(id: string, nounId: string) {
  return { id, noun: { value: nounId } }
}

function mkReading(id: string, text: string, roleIds: string[]) {
  return { id, text, graphSchema: 'gs-1', roles: roleIds, domain: DOMAIN_ID }
}

function mkStateMachineDef(id: string, nounId: string) {
  return { id, noun: nounId, domain: DOMAIN_ID }
}

function mkStatus(id: string, smDefId: string) {
  return { id, stateMachineDefinition: smDefId }
}

function mkTransition(id: string, fromStatusId: string, toStatusId: string, eventTypeId: string) {
  return { id, from: fromStatusId, to: toStatusId, eventType: eventTypeId }
}

function mkEventType(id: string, name: string) {
  return { id, name, domain: DOMAIN_ID }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

const DOMAIN_ID = 'domain-1'

describe('generateILayer', () => {
  describe('entity with list+read+create permissions', () => {
    it('generates list, detail, create layers and index', async () => {
      const customerNoun = mkNoun({ id: 'n-customer', name: 'Customer', permissions: ['list', 'read', 'create'], plural: 'customers' })
      const nameNoun = mkValueNoun({ id: 'n-name', name: 'Name', valueType: 'string' })
      const emailNoun = mkValueNoun({ id: 'n-email', name: 'Email', valueType: 'string', format: 'email' })

      const db = mockDB({
        nouns: [customerNoun, nameNoun, emailNoun],
        readings: [
          mkReading('r1', 'Customer has Name', ['role-1', 'role-2']),
          mkReading('r2', 'Customer has Email', ['role-3', 'role-4']),
        ],
        'state-machine-definitions': [],
        roles: [
          mkRole('role-1', 'n-customer'),
          mkRole('role-2', 'n-name'),
          mkRole('role-3', 'n-customer'),
          mkRole('role-4', 'n-email'),
        ],
        statuses: [],
        'event-types': [],
      })

      const result = await generateILayer(db, DOMAIN_ID)

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
      expect(create.fieldsets[0].fields[0].type).toBe('text') // Name → text
      expect(create.fieldsets[0].fields[1].type).toBe('email') // Email → email
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
    it('maps string→text, boolean→bool, enum→select, email→email, date→date, number→numeric', async () => {
      const entity = mkNoun({ id: 'n-task', name: 'Task', permissions: ['create'], plural: 'tasks' })
      const titleNoun = mkValueNoun({ id: 'n-title', name: 'Title', valueType: 'string' })
      const activeNoun = mkValueNoun({ id: 'n-active', name: 'Active', valueType: 'boolean' })
      const priorityNoun = mkValueNoun({ id: 'n-priority', name: 'Priority', valueType: 'string', enum: 'Low,Medium,High' })
      const emailNoun = mkValueNoun({ id: 'n-email', name: 'ContactEmail', valueType: 'string', format: 'email' })
      const dueDateNoun = mkValueNoun({ id: 'n-due', name: 'DueDate', valueType: 'string', format: 'date' })
      const countNoun = mkValueNoun({ id: 'n-count', name: 'ItemCount', valueType: 'number' })

      const db = mockDB({
        nouns: [entity, titleNoun, activeNoun, priorityNoun, emailNoun, dueDateNoun, countNoun],
        readings: [
          mkReading('r1', 'Task has Title', ['role-1', 'role-2']),
          mkReading('r2', 'Task has Active', ['role-3', 'role-4']),
          mkReading('r3', 'Task has Priority', ['role-5', 'role-6']),
          mkReading('r4', 'Task has ContactEmail', ['role-7', 'role-8']),
          mkReading('r5', 'Task has DueDate', ['role-9', 'role-10']),
          mkReading('r6', 'Task has ItemCount', ['role-11', 'role-12']),
        ],
        'state-machine-definitions': [],
        roles: [
          mkRole('role-1', 'n-task'), mkRole('role-2', 'n-title'),
          mkRole('role-3', 'n-task'), mkRole('role-4', 'n-active'),
          mkRole('role-5', 'n-task'), mkRole('role-6', 'n-priority'),
          mkRole('role-7', 'n-task'), mkRole('role-8', 'n-email'),
          mkRole('role-9', 'n-task'), mkRole('role-10', 'n-due'),
          mkRole('role-11', 'n-task'), mkRole('role-12', 'n-count'),
        ],
        statuses: [],
        'event-types': [],
      })

      const result = await generateILayer(db, DOMAIN_ID)
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
      const orderNoun = mkNoun({ id: 'n-order', name: 'Order', permissions: ['read', 'update'], plural: 'orders' })
      const customerNoun = mkNoun({ id: 'n-customer', name: 'Customer', permissions: ['list'], plural: 'customers' })
      const amountNoun = mkValueNoun({ id: 'n-amount', name: 'Amount', valueType: 'number' })

      const db = mockDB({
        nouns: [orderNoun, customerNoun, amountNoun],
        readings: [
          mkReading('r1', 'Order has Amount', ['role-1', 'role-2']),
          mkReading('r2', 'Order belongs to Customer', ['role-3', 'role-4']),
        ],
        'state-machine-definitions': [],
        roles: [
          mkRole('role-1', 'n-order'), mkRole('role-2', 'n-amount'),
          mkRole('role-3', 'n-order'), mkRole('role-4', 'n-customer'),
        ],
        statuses: [],
        'event-types': [],
      })

      const result = await generateILayer(db, DOMAIN_ID)

      const detail = JSON.parse(result.files['layers/orders-detail.json'])
      expect(detail.navigation).toEqual([{ text: 'Customer', address: '/customers' }])

      const edit = JSON.parse(result.files['layers/orders-edit.json'])
      expect(edit.navigation).toEqual([{ text: 'Customer', address: '/customers' }])
    })
  })

  describe('event buttons from state machine transitions', () => {
    it('adds event buttons to detail and edit layers', async () => {
      const ticketNoun = mkNoun({ id: 'n-ticket', name: 'Ticket', permissions: ['read', 'update'], plural: 'tickets' })
      const titleNoun = mkValueNoun({ id: 'n-title', name: 'Title', valueType: 'string' })

      const db = mockDB({
        nouns: [ticketNoun, titleNoun],
        readings: [mkReading('r1', 'Ticket has Title', ['role-1', 'role-2'])],
        'state-machine-definitions': [mkStateMachineDef('sm-1', 'n-ticket')],
        statuses: [
          mkStatus('status-1', 'sm-1'),
          mkStatus('status-2', 'sm-1'),
        ],
        transitions: [
          mkTransition('t-1', 'status-1', 'status-2', 'et-1'),
          mkTransition('t-2', 'status-2', 'status-1', 'et-2'),
        ],
        'event-types': [mkEventType('et-1', 'Escalate'), mkEventType('et-2', 'Close')],
      })

      const result = await generateILayer(db, DOMAIN_ID)

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
      const customerNoun = mkNoun({ id: 'n-customer', name: 'Customer', permissions: ['list'], plural: 'customers' })
      const orderNoun = mkNoun({ id: 'n-order', name: 'Order', permissions: ['read'], plural: 'orders' })
      const secretNoun = mkNoun({ id: 'n-secret', name: 'Secret', permissions: ['create'] }) // no list/read

      const db = mockDB({
        nouns: [customerNoun, orderNoun, secretNoun],
        readings: [],
        'state-machine-definitions': [],
        roles: [],
        statuses: [],
        'event-types': [],
      })

      const result = await generateILayer(db, DOMAIN_ID)
      const index = JSON.parse(result.files['layers/index.json'])
      const texts = index.items[0].items.map((i: any) => i.text)

      expect(texts).toContain('Customer')
      expect(texts).toContain('Order')
      expect(texts).not.toContain('Secret')
    })
  })

  describe('entity without permissions', () => {
    it('generates no layers for the entity', async () => {
      const noPermEntity = mkNoun({ id: 'n-hidden', name: 'Hidden', permissions: [] })

      const db = mockDB({
        nouns: [noPermEntity],
        readings: [],
        'state-machine-definitions': [],
        roles: [],
        statuses: [],
        'event-types': [],
      })

      const result = await generateILayer(db, DOMAIN_ID)

      // Only the index layer should exist (with no items)
      expect(Object.keys(result.files)).toEqual(['layers/index.json'])
      const index = JSON.parse(result.files['layers/index.json'])
      expect(index.items[0].items).toEqual([])
    })
  })

  describe('update+delete permissions', () => {
    it('generates edit layer with save/cancel/event buttons and delete on detail', async () => {
      const entity = mkNoun({ id: 'n-item', name: 'Item', permissions: ['read', 'update', 'delete'], plural: 'items' })
      const nameNoun = mkValueNoun({ id: 'n-name', name: 'Name', valueType: 'string' })

      const db = mockDB({
        nouns: [entity, nameNoun],
        readings: [mkReading('r1', 'Item has Name', ['role-1', 'role-2'])],
        'state-machine-definitions': [],
        roles: [mkRole('role-1', 'n-item'), mkRole('role-2', 'n-name')],
        statuses: [],
        'event-types': [],
      })

      const result = await generateILayer(db, DOMAIN_ID)

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
      const entity = mkNoun({ id: 'n-person', name: 'Person', permissions: ['list'], plural: 'people' })

      const db = mockDB({
        nouns: [entity],
        readings: [],
        'state-machine-definitions': [],
        roles: [],
        statuses: [],
        'event-types': [],
      })

      const result = await generateILayer(db, DOMAIN_ID)
      expect(result.files['layers/people.json']).toBeDefined()
    })

    it('generates slug from PascalCase name when no plural', async () => {
      const entity = mkNoun({ id: 'n-sr', name: 'SupportRequest', permissions: ['list'] })

      const db = mockDB({
        nouns: [entity],
        readings: [],
        'state-machine-definitions': [],
        roles: [],
        statuses: [],
        'event-types': [],
      })

      const result = await generateILayer(db, DOMAIN_ID)
      expect(result.files['layers/support-requests.json']).toBeDefined()
    })
  })

  describe('deduplication of fields', () => {
    it('deduplicates fields by field ID', async () => {
      const entity = mkNoun({ id: 'n-item', name: 'Item', permissions: ['create'], plural: 'items' })
      const nameNoun = mkValueNoun({ id: 'n-name', name: 'Name', valueType: 'string' })

      const db = mockDB({
        nouns: [entity, nameNoun],
        readings: [
          // Two readings referencing the same value noun
          mkReading('r1', 'Item has Name', ['role-1', 'role-2']),
          mkReading('r2', 'Item identifies Name', ['role-3', 'role-4']),
        ],
        'state-machine-definitions': [],
        roles: [
          mkRole('role-1', 'n-item'), mkRole('role-2', 'n-name'),
          mkRole('role-3', 'n-item'), mkRole('role-4', 'n-name'),
        ],
        statuses: [],
        'event-types': [],
      })

      const result = await generateILayer(db, DOMAIN_ID)
      const create = JSON.parse(result.files['layers/items-new.json'])
      const nameFields = create.fieldsets[0].fields.filter((f: any) => f.id === 'name')
      expect(nameFields.length).toBe(1)
    })
  })

  describe('list layer field cap', () => {
    it('shows at most 3 fields in list items', async () => {
      const entity = mkNoun({ id: 'n-item', name: 'Item', permissions: ['list'], plural: 'items' })
      const f1 = mkValueNoun({ id: 'v1', name: 'FieldOne', valueType: 'string' })
      const f2 = mkValueNoun({ id: 'v2', name: 'FieldTwo', valueType: 'string' })
      const f3 = mkValueNoun({ id: 'v3', name: 'FieldThree', valueType: 'string' })
      const f4 = mkValueNoun({ id: 'v4', name: 'FieldFour', valueType: 'string' })

      const db = mockDB({
        nouns: [entity, f1, f2, f3, f4],
        readings: [
          mkReading('r1', 'Item has FieldOne', ['r-1', 'r-2']),
          mkReading('r2', 'Item has FieldTwo', ['r-3', 'r-4']),
          mkReading('r3', 'Item has FieldThree', ['r-5', 'r-6']),
          mkReading('r4', 'Item has FieldFour', ['r-7', 'r-8']),
        ],
        'state-machine-definitions': [],
        roles: [
          mkRole('r-1', 'n-item'), mkRole('r-2', 'v1'),
          mkRole('r-3', 'n-item'), mkRole('r-4', 'v2'),
          mkRole('r-5', 'n-item'), mkRole('r-6', 'v3'),
          mkRole('r-7', 'n-item'), mkRole('r-8', 'v4'),
        ],
        statuses: [],
        'event-types': [],
      })

      const result = await generateILayer(db, DOMAIN_ID)
      const list = JSON.parse(result.files['layers/items.json'])
      expect(list.items[0].items.length).toBe(3)
    })
  })

  describe('email detection by name', () => {
    it('infers email type from noun name containing "email"', async () => {
      const entity = mkNoun({ id: 'n-item', name: 'Item', permissions: ['create'], plural: 'items' })
      const emailNoun = mkValueNoun({ id: 'n-contact-email', name: 'ContactEmail', valueType: 'string' })

      const db = mockDB({
        nouns: [entity, emailNoun],
        readings: [mkReading('r1', 'Item has ContactEmail', ['r-1', 'r-2'])],
        'state-machine-definitions': [],
        roles: [mkRole('r-1', 'n-item'), mkRole('r-2', 'n-contact-email')],
        statuses: [],
        'event-types': [],
      })

      const result = await generateILayer(db, DOMAIN_ID)
      const create = JSON.parse(result.files['layers/items-new.json'])
      expect(create.fieldsets[0].fields[0].type).toBe('email')
    })
  })
})

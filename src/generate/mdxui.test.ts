import { describe, it, expect, beforeEach } from 'vitest'
import { generateMdxui } from './mdxui'
import { createMockModel, mkNounDef, mkValueNounDef, mkFactType, mkConstraint, resetIds } from '../model/test-utils'

describe('generateMdxui', () => {
  beforeEach(() => resetIds())

  it('generates entity detail page with form fields from binary fact types', async () => {
    const customer = mkNounDef({ name: 'Customer', plural: 'customers', permissions: ['list', 'read', 'create'] })
    const name = mkValueNounDef({ name: 'Name', valueType: 'string' })
    const email = mkValueNounDef({ name: 'Email', valueType: 'string', format: 'email' })
    const ft1 = mkFactType({ reading: 'Customer has Name', roles: [{ nounDef: customer, roleIndex: 0 }, { nounDef: name, roleIndex: 1 }] })
    const ft2 = mkFactType({ reading: 'Customer has Email', roles: [{ nounDef: customer, roleIndex: 0 }, { nounDef: email, roleIndex: 1 }] })

    const model = createMockModel({ nouns: [customer, name, email], factTypes: [ft1, ft2] })
    const result = await generateMdxui(model)

    expect(result.files['pages/customers.mdx']).toBeDefined()
    const page = result.files['pages/customers.mdx']
    expect(page).toContain('Customer')
    expect(page).toContain('Name')
    expect(page).toContain('Email')
  })

  it('generates index page with Cards for all entities', async () => {
    const customer = mkNounDef({ name: 'Customer', plural: 'customers', permissions: ['list'] })
    const order = mkNounDef({ name: 'Order', plural: 'orders', permissions: ['list'] })

    const model = createMockModel({ nouns: [customer, order] })
    const result = await generateMdxui(model)

    expect(result.files['pages/index.mdx']).toBeDefined()
    const index = result.files['pages/index.mdx']
    expect(index).toContain('Customer')
    expect(index).toContain('Order')
  })

  it('maps value types to appropriate mdxui components', async () => {
    const entity = mkNounDef({ name: 'Task', plural: 'tasks', permissions: ['create'] })
    const priority = mkValueNounDef({ name: 'Priority', valueType: 'string', enumValues: ['low', 'medium', 'high'] })
    const active = mkValueNounDef({ name: 'Active', valueType: 'boolean' })
    const dueDate = mkValueNounDef({ name: 'DueDate', valueType: 'string', format: 'date' })

    const ft1 = mkFactType({ reading: 'Task has Priority', roles: [{ nounDef: entity, roleIndex: 0 }, { nounDef: priority, roleIndex: 1 }] })
    const ft2 = mkFactType({ reading: 'Task is Active', roles: [{ nounDef: entity, roleIndex: 0 }, { nounDef: active, roleIndex: 1 }] })
    const ft3 = mkFactType({ reading: 'Task has DueDate', roles: [{ nounDef: entity, roleIndex: 0 }, { nounDef: dueDate, roleIndex: 1 }] })

    const model = createMockModel({ nouns: [entity, priority, active, dueDate], factTypes: [ft1, ft2, ft3] })
    const result = await generateMdxui(model)

    const page = result.files['pages/tasks.mdx']
    expect(page).toContain('Select') // enum -> Select
    expect(page).toContain('Checkbox') // boolean -> Checkbox
    expect(page).toContain('DatePicker') // date format -> DatePicker
  })

  it('includes constraint documentation as Callouts', async () => {
    const customer = mkNounDef({ name: 'Customer', plural: 'customers', permissions: ['read'] })
    const name = mkValueNounDef({ name: 'Name', valueType: 'string' })
    const ft = mkFactType({ reading: 'Customer has Name', roles: [{ nounDef: customer, roleIndex: 0 }, { nounDef: name, roleIndex: 1 }] })
    const uc = mkConstraint({ kind: 'UC', text: 'Each Customer has at most one Name', spans: [{ factTypeId: ft.id, roleIndex: 1 }] })

    const model = createMockModel({ nouns: [customer, name], factTypes: [ft], constraints: [uc] })
    const result = await generateMdxui(model)

    const page = result.files['pages/customers.mdx']
    expect(page).toContain('Callout')
    expect(page).toContain('Each Customer has at most one Name')
  })

  it('generates entity-to-entity navigation as Card links', async () => {
    const customer = mkNounDef({ name: 'Customer', plural: 'customers', permissions: ['read'] })
    const order = mkNounDef({ name: 'Order', plural: 'orders', permissions: ['read'] })
    const ft = mkFactType({ reading: 'Customer has Order', roles: [{ nounDef: customer, roleIndex: 0 }, { nounDef: order, roleIndex: 1 }] })

    const model = createMockModel({ nouns: [customer, order], factTypes: [ft] })
    const result = await generateMdxui(model)

    const page = result.files['pages/customers.mdx']
    expect(page).toContain('Card')
    expect(page).toContain('Order')
  })

  it('skips entities without permissions', async () => {
    const hidden = mkNounDef({ name: 'Hidden', permissions: [] })
    const model = createMockModel({ nouns: [hidden] })
    const result = await generateMdxui(model)

    expect(result.files['pages/hiddens.mdx']).toBeUndefined()
  })
})

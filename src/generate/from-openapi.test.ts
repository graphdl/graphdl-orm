import { describe, it, expect } from 'vitest'
import { fromOpenAPI, type OpenAPISpec } from './from-openapi'

describe('fromOpenAPI', () => {
  it('converts schema objects to entity types', () => {
    const spec: OpenAPISpec = {
      info: { title: 'Test API' },
      components: {
        schemas: {
          Customer: {
            type: 'object',
            properties: {
              id: { type: 'string' },
              email: { type: 'string' },
              name: { type: 'string' },
            },
            required: ['id', 'email'],
          },
        },
      },
    }

    const result = fromOpenAPI(spec, 'test')
    expect(result).toContain('Customer(.id) is an entity type.')
    expect(result).toContain('Customer has Email.')
    expect(result).toContain('Customer has Name.')
    expect(result).toContain('Each Customer has exactly one Email.')
  })

  it('converts enum properties to value types', () => {
    const spec: OpenAPISpec = {
      components: {
        schemas: {
          Subscription: {
            type: 'object',
            properties: {
              status: { type: 'string', enum: ['active', 'past_due', 'canceled', 'trialing'] },
            },
          },
        },
      },
    }

    const result = fromOpenAPI(spec, 'test')
    expect(result).toContain("The possible values of Status are 'active', 'past_due', 'canceled', 'trialing'.")
    expect(result).toContain('Subscription has Status.')
  })

  it('converts $ref properties to relationship fact types', () => {
    const spec: OpenAPISpec = {
      components: {
        schemas: {
          Invoice: {
            type: 'object',
            properties: {
              customer: { $ref: '#/components/schemas/Customer' },
              amount: { type: 'integer' },
            },
          },
          Customer: {
            type: 'object',
            properties: {
              id: { type: 'string' },
            },
          },
        },
      },
    }

    const result = fromOpenAPI(spec, 'billing')
    expect(result).toContain('Invoice(.id) is an entity type.')
    expect(result).toContain('Customer(.id) is an entity type.')
    expect(result).toContain('Invoice has Customer.')
    expect(result).toContain('Each Invoice has at most one Customer.')
  })

  it('converts array properties to multi-valued fact types', () => {
    const spec: OpenAPISpec = {
      components: {
        schemas: {
          Customer: {
            type: 'object',
            properties: {
              subscriptions: { type: 'array', items: { $ref: '#/components/schemas/Subscription' } },
            },
          },
          Subscription: {
            type: 'object',
            properties: { id: { type: 'string' } },
          },
        },
      },
    }

    const result = fromOpenAPI(spec, 'test')
    expect(result).toContain('Customer has Subscription.')
    // Array → no "at most one" constraint
    expect(result).not.toContain('Each Customer has at most one Subscription.')
  })

  it('generates verb/function wiring from paths', () => {
    const spec: OpenAPISpec = {
      paths: {
        '/v1/customers': {
          post: {
            operationId: 'createCustomer',
            responses: {
              '200': {
                description: 'OK',
                content: { 'application/json': { schema: { $ref: '#/components/schemas/Customer' } } },
              },
            },
          },
        },
      },
      components: {
        schemas: {
          Customer: { type: 'object', properties: { id: { type: 'string' } } },
        },
      },
    }

    const result = fromOpenAPI(spec, 'test')
    expect(result).toContain("Verb 'createCustomer' executes Function 'createCustomer'.")
    expect(result).toContain("Function 'createCustomer' has HTTP Method 'POST'.")
    expect(result).toContain("Function 'createCustomer' has callback URI '/v1/customers'.")
  })

  it('generates domain visibility instance fact', () => {
    const spec: OpenAPISpec = { components: { schemas: {} } }
    const result = fromOpenAPI(spec, 'billing')
    expect(result).toContain("Domain 'billing' has Visibility 'public'.")
  })

  it('handles top-level enum schemas as value types', () => {
    const spec: OpenAPISpec = {
      components: {
        schemas: {
          Currency: { type: 'string', enum: ['usd', 'eur', 'gbp'] },
        },
      },
    }

    const result = fromOpenAPI(spec, 'test')
    expect(result).toContain('Currency is a value type.')
    expect(result).toContain("The possible values of Currency are 'usd', 'eur', 'gbp'.")
  })

  it('converts a billing-like spec to complete readings', () => {
    const spec: OpenAPISpec = {
      info: { title: 'Billing API', version: '2024-01-01' },
      components: {
        schemas: {
          Customer: {
            type: 'object',
            required: ['id', 'email'],
            properties: {
              id: { type: 'string' },
              email: { type: 'string' },
              name: { type: 'string' },
              balance: { type: 'integer' },
            },
          },
          Subscription: {
            type: 'object',
            required: ['id', 'status', 'customer'],
            properties: {
              id: { type: 'string' },
              status: { type: 'string', enum: ['active', 'past_due', 'canceled', 'trialing', 'incomplete'] },
              customer: { $ref: '#/components/schemas/Customer' },
              currentPeriodEnd: { type: 'integer' },
            },
          },
          Invoice: {
            type: 'object',
            required: ['id', 'customer', 'total'],
            properties: {
              id: { type: 'string' },
              customer: { $ref: '#/components/schemas/Customer' },
              subscription: { $ref: '#/components/schemas/Subscription' },
              total: { type: 'integer' },
              status: { type: 'string', enum: ['draft', 'open', 'paid', 'void', 'uncollectible'] },
            },
          },
        },
      },
      paths: {
        '/v1/customers/{id}': {
          get: {
            operationId: 'getCustomer',
            responses: { '200': { description: 'OK', content: { 'application/json': { schema: { $ref: '#/components/schemas/Customer' } } } } },
          },
        },
        '/v1/subscriptions': {
          post: {
            operationId: 'createSubscription',
            responses: { '200': { description: 'OK', content: { 'application/json': { schema: { $ref: '#/components/schemas/Subscription' } } } } },
          },
        },
      },
    }

    const result = fromOpenAPI(spec, 'billing')

    // Entity types
    expect(result).toContain('Customer(.id) is an entity type.')
    expect(result).toContain('Subscription(.id) is an entity type.')
    expect(result).toContain('Invoice(.id) is an entity type.')

    // Relationships
    expect(result).toContain('Subscription has Customer.')
    expect(result).toContain('Invoice has Customer.')
    expect(result).toContain('Invoice has Subscription.')

    // Value types with enums
    expect(result).toContain("'active', 'past_due', 'canceled', 'trialing', 'incomplete'")

    // Mandatory constraints
    expect(result).toContain('Each Subscription has exactly one Customer.')
    expect(result).toContain('Each Invoice has exactly one Total.')

    // Verbs
    expect(result).toContain("Verb 'getCustomer'")
    expect(result).toContain("Verb 'createSubscription'")
  })
})

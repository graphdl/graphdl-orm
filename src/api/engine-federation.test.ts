import { describe, it, expect, vi, beforeEach } from 'vitest'
import type { FederatedSource, ServiceEndpoint } from './engine'

const mockFetch = vi.fn()
vi.stubGlobal('fetch', mockFetch)

describe('federation', () => {
  beforeEach(() => {
    mockFetch.mockReset()
  })

  it('federation config structure supports service endpoints', () => {
    const config: FederatedSource = {
      endpoints: {
        'Vehicle': {
          service: 'apis',
          url: 'https://api.example.com/data/{id}',
          responsePath: 'vehicle',
        },
        'Customer': {
          service: 'auth-service',
          url: 'https://auth.example.com/api/users',
          authHeader: 'Authorization',
          responsePath: 'docs',
          fieldMap: { 'email': 'Email', 'plan': 'Plan' },
        },
      },
      resolveSecret: async (service) => {
        if (service === 'apis') return 'test-key'
        if (service === 'auth-service') return 'auth-key'
        return null
      },
    }

    expect(config.endpoints['Vehicle'].service).toBe('apis')
    expect(config.endpoints['Customer'].service).toBe('auth-service')
    expect(config.endpoints['Customer'].authHeader).toBe('Authorization')
    expect(config.endpoints['Customer'].fieldMap?.['email']).toBe('Email')
  })

  it('secret resolution is per-service', async () => {
    const config: FederatedSource = {
      endpoints: {
        'NounA': { service: 'svc1', url: 'https://svc1.example.com/a' },
        'NounB': { service: 'svc2', url: 'https://svc2.example.com/b' },
      },
      resolveSecret: async (service) => {
        const secrets: Record<string, string> = { 'svc1': 'key1', 'svc2': 'key2' }
        return secrets[service] ?? null
      },
    }

    expect(await config.resolveSecret!('svc1')).toBe('key1')
    expect(await config.resolveSecret!('svc2')).toBe('key2')
    expect(await config.resolveSecret!('unknown')).toBeNull()
  })

  it('field maps translate response fields to noun fields', () => {
    const endpoint: ServiceEndpoint = {
      service: 'test',
      url: 'https://example.com/api',
      fieldMap: {
        'vehicle.year': 'year',
        'vehicle.make': 'make',
        'retailListing.price': 'price',
      },
    }

    expect(endpoint.fieldMap?.['vehicle.year']).toBe('year')
    expect(endpoint.fieldMap?.['retailListing.price']).toBe('price')
  })

  it('response path navigates nested JSON', () => {
    const endpoint: ServiceEndpoint = {
      service: 'test',
      url: 'https://example.com/api',
      responsePath: 'data.items',
    }

    expect(endpoint.responsePath).toBe('data.items')
    expect(endpoint.responsePath!.split('.')).toEqual(['data', 'items'])
  })

  it('supports direct backing store access alongside services', () => {
    const config: FederatedSource = {
      endpoints: {
        'Vehicle': { service: 'apis', url: 'https://api.example.com/data/{id}' },
        'Raw Resource': { service: 'olap-db', url: 'https://db.internal:8443/?query=SELECT * FROM resources FORMAT JSON' },
      },
    }

    expect(config.endpoints['Vehicle'].service).toBe('apis')
    expect(config.endpoints['Raw Resource'].service).toBe('olap-db')
  })
})

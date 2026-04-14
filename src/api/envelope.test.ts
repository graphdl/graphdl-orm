import { describe, it, expect } from 'vitest'
import { envelope } from './envelope'

describe('envelope (Theorem 5 repr shape)', () => {
  it('wraps data with empty defaults when no options given', () => {
    const out = envelope({ id: 'ord-1', total: 10 })
    expect(out.data).toEqual({ id: 'ord-1', total: 10 })
    expect(out.derived).toEqual({})
    expect(out.violations).toEqual([])
    expect(out._links.transitions).toEqual([])
    expect(out._links.navigation).toEqual({})
  })

  it('carries violations verbatim', () => {
    const v = {
      reading: 'Each Order was placed by exactly one Customer.',
      constraintId: 'c4',
      modality: 'alethic' as const,
      detail: 'ord-1 has 2 customers',
    }
    const out = envelope({ id: 'ord-1' }, { violations: [v] })
    expect(out.violations).toEqual([v])
  })

  it('surfaces transition links in _links.transitions', () => {
    const out = envelope({ id: 'ord-1', status: 'Placed' }, {
      transitions: [
        { event: 'ship', href: '/orders/ord-1/transition', method: 'POST' },
      ],
    })
    expect(out._links.transitions).toHaveLength(1)
    expect(out._links.transitions?.[0].event).toBe('ship')
  })

  it('surfaces navigation uris in _links.navigation', () => {
    const out = envelope({ id: 'ord-1' }, {
      navigation: {
        'customer': '/customers/acme',
        'line-items': '/orders/ord-1/line-items',
      },
    })
    expect(out._links.navigation?.['customer']).toBe('/customers/acme')
  })

  it('carries derived facts separately from data', () => {
    const out = envelope({ id: 'u1', email: 'a@b' }, {
      derived: { 'user_accesses_domain': true, 'is_premium': false },
    })
    expect(out.derived).toEqual({ 'user_accesses_domain': true, 'is_premium': false })
    expect(out.data).toEqual({ id: 'u1', email: 'a@b' })
  })

  it('wraps arrays as the data payload unchanged', () => {
    const list = [{ id: 'a' }, { id: 'b' }]
    const out = envelope(list)
    expect(out.data).toBe(list)
  })
})

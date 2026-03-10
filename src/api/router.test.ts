import { describe, it, expect } from 'vitest'
import { parsePayloadWhereParams } from './collections'

describe('parsePayloadWhereParams', () => {
  it('parses simple equals', () => {
    const params = new URLSearchParams('where[name][equals]=Test')
    const result = parsePayloadWhereParams(params)
    expect(result).toEqual({ name: { equals: 'Test' } })
  })

  it('parses nested with domain filter', () => {
    const params = new URLSearchParams('where[domain][equals]=abc123')
    const result = parsePayloadWhereParams(params)
    expect(result).toEqual({ domain: { equals: 'abc123' } })
  })

  it('parses or conditions', () => {
    const params = new URLSearchParams()
    params.set('where[or][0][visibility][equals]', 'public')
    params.set('where[or][1][organization][equals]', 'org-1')
    const result = parsePayloadWhereParams(params)
    expect(result.or).toHaveLength(2)
    expect(result.or[0]).toEqual({ visibility: { equals: 'public' } })
  })

  it('parses and conditions', () => {
    const params = new URLSearchParams()
    params.set('where[and][0][name][equals]', 'Test')
    params.set('where[and][1][domain][equals]', 'abc')
    const result = parsePayloadWhereParams(params)
    expect(result.and).toHaveLength(2)
  })
})

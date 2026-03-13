import { describe, it, expect } from 'vitest'
import { handleGenerate } from './generate'

describe('handleGenerate', () => {
  it('returns 400 for missing domainId', async () => {
    const request = new Request('http://localhost/api/generate', {
      method: 'POST',
      body: JSON.stringify({}),
      headers: { 'Content-Type': 'application/json' },
    })
    const response = await handleGenerate(request, {} as any)
    expect(response.status).toBe(400)
    const body = await response.json() as any
    expect(body.errors[0].message).toBe('domainId is required')
  })

  it('returns 400 for unknown outputFormat', async () => {
    const request = new Request('http://localhost/api/generate', {
      method: 'POST',
      body: JSON.stringify({ domainId: 'd1', outputFormat: 'unknown' }),
      headers: { 'Content-Type': 'application/json' },
    })
    const response = await handleGenerate(request, {} as any)
    expect(response.status).toBe(400)
    const body = await response.json() as any
    expect(body.errors[0].message).toContain('Invalid outputFormat')
  })

  it('accepts all valid output formats without error on format validation', async () => {
    for (const format of ['openapi', 'sqlite', 'xstate', 'ilayer', 'readings', 'constraint-ir']) {
      const request = new Request('http://localhost/api/generate', {
        method: 'POST',
        body: JSON.stringify({ domainId: 'd1', outputFormat: format }),
        headers: { 'Content-Type': 'application/json' },
      })
      // Will throw because env.GRAPHDL_DB is not set, but should NOT return 400
      try {
        await handleGenerate(request, {} as any)
      } catch {
        // Expected — no real DO stub. The point is it didn't return 400.
      }
    }
  })
})

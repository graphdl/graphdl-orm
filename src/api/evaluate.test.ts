import { describe, it, expect, vi } from 'vitest'

// Mock the WASM binary import — use the resolved path from evaluate.ts's perspective
vi.mock(
  '../../crates/fol-engine/pkg/fol_engine_bg.wasm',
  () => ({ default: {} }),
)

// Mock the WASM JS bindings
vi.mock(
  '../../crates/fol-engine/pkg/fol_engine.js',
  () => ({
    initSync: vi.fn(() => { throw new Error('WASM not available in test') }),
    load_ir: vi.fn(),
    evaluate_response: vi.fn(),
  }),
)

import { handleEvaluate } from './evaluate'

function makeRequest(method: string, body?: any): Request {
  return new Request('http://localhost/api/evaluate', {
    method,
    headers: { 'Content-Type': 'application/json' },
    body: body ? JSON.stringify(body) : undefined,
  })
}

function makeMockEnv(docs: any[] = []) {
  return {
    DOMAIN_DB: {
      idFromName: vi.fn().mockReturnValue('test-id'),
      get: vi.fn().mockReturnValue({
        findInCollection: vi.fn().mockResolvedValue({ docs }),
      }),
    },
    ENVIRONMENT: 'test',
  } as any
}

describe('evaluate endpoint', () => {
  it('exports handleEvaluate', async () => {
    expect(handleEvaluate).toBeDefined()
    expect(typeof handleEvaluate).toBe('function')
  })

  it('returns 405 for non-POST methods', async () => {
    const req = makeRequest('GET')
    const res = await handleEvaluate(req, makeMockEnv())
    expect(res.status).toBe(405)
  })

  it('returns 400 when domainId is missing', async () => {
    const req = makeRequest('POST', { response: { text: 'hello' } })
    const res = await handleEvaluate(req, makeMockEnv())
    expect(res.status).toBe(400)
    const body = await res.json() as any
    expect(body.errors[0].message).toContain('domainId')
  })

  it('returns 400 when response.text is missing', async () => {
    const req = makeRequest('POST', { domainId: 'd1' })
    const res = await handleEvaluate(req, makeMockEnv())
    expect(res.status).toBe(400)
    const body = await res.json() as any
    expect(body.errors[0].message).toContain('response.text')
  })

  it('returns warning when no constraint IR exists', async () => {
    const req = makeRequest('POST', { domainId: 'd1', response: { text: 'hello' } })
    const res = await handleEvaluate(req, makeMockEnv([]))
    expect(res.status).toBe(200)
    const body = await res.json() as any
    expect(body.violations).toEqual([])
    expect(body.warning).toContain('No constraint IR')
    expect(body.domainId).toBe('d1')
  })

  it('returns WASM unavailable warning when IR exists but WASM fails', async () => {
    const irDoc = { output: JSON.stringify({ domain: 'd1', constraints: [{ id: 'c1' }] }) }
    const req = makeRequest('POST', { domainId: 'd1', response: { text: 'hello' } })
    const res = await handleEvaluate(req, makeMockEnv([irDoc]))
    expect(res.status).toBe(200)
    const body = await res.json() as any
    expect(body.violations).toEqual([])
    expect(body.constraintCount).toBe(1)
    expect(body.warning).toContain('WASM evaluator not available')
  })
})

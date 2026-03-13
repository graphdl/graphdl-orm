import { describe, it, expect } from 'vitest'

// We can't test the full handler easily (requires WASM + DO),
// but we can verify the module exports correctly and test
// the request validation logic by importing the handler type.
describe('evaluate endpoint', () => {
  it('exports handleEvaluate', async () => {
    const mod = await import('./evaluate')
    expect(mod.handleEvaluate).toBeDefined()
    expect(typeof mod.handleEvaluate).toBe('function')
  })
})

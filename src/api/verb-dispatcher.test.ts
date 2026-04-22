/**
 * Unit tests for the unified-verb dispatcher (#200).
 *
 * Each verb gets a narrow shape test here. Behavioral correctness
 * lives in the engine's Rust test suite — this file only guards
 * against regressions in the JS→WASM plumbing:
 *   - verb added to UNIFIED_VERBS so the HTTP router wires a route
 *   - input-shape validation (throws on missing required fields)
 */
import { describe, it, expect } from 'vitest'
import { UNIFIED_VERBS, dispatchVerb } from './verb-dispatcher'

describe('UNIFIED_VERBS', () => {
  it('includes external_browse so the HTTP router auto-wires /api/external_browse (#343)', () => {
    expect(UNIFIED_VERBS).toContain('external_browse')
  })
})

describe("dispatchVerb('external_browse')", () => {
  it('rejects body without `system`', async () => {
    await expect(dispatchVerb('external_browse', { path: ['Person'] }))
      .rejects.toThrow(/system/)
  })
})

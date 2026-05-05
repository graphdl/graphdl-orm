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
import { UNIFIED_VERBS, dispatchVerb, openApiCellSuffix } from './verb-dispatcher'

describe('UNIFIED_VERBS', () => {
  it('includes external_browse so the HTTP router auto-wires /api/external_browse (#343)', () => {
    expect(UNIFIED_VERBS).toContain('external_browse')
  })

  // T2 (#700): hand-coded routes consolidated into uniform verb cells.
  // Each new verb below collapses one (or more) router.ts hand-coded
  // routes whose body was just `engine.system(h, key, input)` wrapped
  // in JSON. Routing under `/api/{verb}` is auto-wired by the for-loop
  // in router.ts; the legacy URL shapes stay alive as thin delegators.
  it('includes debug so /api/debug auto-wires the compiled-state projection', () => {
    expect(UNIFIED_VERBS).toContain('debug')
  })

  it('includes rmap so /api/rmap auto-wires the RMAP table projection', () => {
    expect(UNIFIED_VERBS).toContain('rmap')
  })

  it('includes forward_chain so /api/forward_chain auto-wires the derivation runner', () => {
    expect(UNIFIED_VERBS).toContain('forward_chain')
  })

  it('includes openapi so /api/openapi auto-wires the App-scoped OpenAPI cell read', () => {
    expect(UNIFIED_VERBS).toContain('openapi')
  })
})

describe("dispatchVerb('external_browse')", () => {
  it('rejects body without `system`', async () => {
    await expect(dispatchVerb('external_browse', { path: ['Person'] }))
      .rejects.toThrow(/system/)
  })
})

describe("dispatchVerb('forward_chain')", () => {
  it('rejects body without `population`', async () => {
    await expect(dispatchVerb('forward_chain', {}))
      .rejects.toThrow(/population/)
  })
})

describe("dispatchVerb('openapi')", () => {
  it('rejects body without `app`', async () => {
    await expect(dispatchVerb('openapi', {}))
      .rejects.toThrow(/app/)
  })
})

describe('openApiCellSuffix', () => {
  // Mirror of `crate::rmap::to_snake` in Rust. The suffix MUST match
  // the cell key the compiler emits for the App's OpenAPI generator
  // (see `format!("openapi:{}", crate::rmap::to_snake(app))` in
  // crates/arest/src/compile.rs). Drift here = silent 404 at the route.
  it('lowercases an all-lowercase slug unchanged', () => {
    expect(openApiCellSuffix('myapp')).toBe('myapp')
  })

  it('snake-cases CamelCase by inserting underscores at lower→upper boundaries', () => {
    expect(openApiCellSuffix('MyApp')).toBe('my_app')
    expect(openApiCellSuffix('myCoolApp')).toBe('my_cool_app')
  })

  it('replaces space and hyphen with underscore', () => {
    expect(openApiCellSuffix('my app')).toBe('my_app')
    expect(openApiCellSuffix('my-app')).toBe('my_app')
  })
})

/**
 * Tests for the DynRdg-T3 (#562) worker-side load_reading adapter.
 *
 * Verification surface (per the task brief):
 *   1. Tenant A's cell graph contains the reading's facts; tenant B's
 *      doesn't (per-tenant scoping via the manifest cell key).
 *   2. Restart the worker (simulate via `_resetCaches`); the reading
 *      replays cleanly.
 *   3. POST a reading with a deontic violation; LoadValidationReport
 *      surfaces it; cell state isn't mutated.
 *
 * The tests inject a mocked `HandleProvider` so they don't depend on
 * the WASM engine at all — that keeps the unit tests fast and
 * deterministic. Integration with the real engine is exercised
 * through the existing /api/parse and /api/evaluate test surface
 * (and the verb-dispatcher round-trips). The contract this file
 * tests is the adapter's wiring: validation gate → DO write →
 * registry index → cold-start replay.
 */

import { describe, it, expect, beforeEach, vi } from 'vitest'

// Mock the WASM engine — every test in this file injects its own
// HandleProvider, so the real engine import is only here to satisfy
// the load-reading.ts module-load `import * as engine`. Without the
// mock the import-time WebAssembly.Instance call inside arest.js
// fails under Vitest's wasm-stub plugin.
vi.mock('./engine', () => ({
  compileDomainReadings: () => 0,
  system: () => '',
  currentDomainHandle: () => -1,
}))

import {
  manifestCellKey,
  parseManifestCellKey,
  decodeValidationReport,
  computeContentHash,
  resolveTenant,
  loadReading,
  coldStartReplay,
  getTenantHandle,
  _resetCaches,
  type CellStub,
  type IndexStub,
  type HandleProvider,
  type LoadReadingDeps,
  type LoadReadingResponse,
  type LoadedReadingManifest,
} from './load-reading'

// ── Test fixtures: in-memory DO stubs ──────────────────────────────

interface CellRecord {
  id: string
  type: string
  data: Record<string, unknown>
}

class FakeCellStore {
  private cells = new Map<string, CellRecord>()

  stub(key: string): CellStub {
    return {
      get: async () => this.cells.get(key) ?? null,
      put: async (input) => {
        const merged: CellRecord = {
          id: input.id,
          type: input.type,
          data: { ...input.data },
        }
        this.cells.set(key, merged)
        return merged
      },
    }
  }

  has(key: string): boolean {
    return this.cells.has(key)
  }

  get(key: string): CellRecord | undefined {
    return this.cells.get(key)
  }

  keys(): string[] {
    return Array.from(this.cells.keys())
  }
}

class FakeRegistry {
  // Keyed by tenant — each tenant has its own (nounType, entityId) set.
  private byTenant = new Map<string, Map<string, Set<string>>>()

  stub(tenant: string): IndexStub {
    return {
      indexEntity: async (nounType, entityId, domainSlug) => {
        const t = domainSlug || tenant
        if (!this.byTenant.has(t)) this.byTenant.set(t, new Map())
        const m = this.byTenant.get(t)!
        if (!m.has(nounType)) m.set(nounType, new Set())
        m.get(nounType)!.add(entityId)
      },
      getEntityIds: async (nounType, domainSlug) => {
        const t = domainSlug || tenant
        const m = this.byTenant.get(t)
        if (!m) return []
        const set = m.get(nounType)
        return set ? Array.from(set) : []
      },
    }
  }

  forTenant(tenant: string, nounType: string): string[] {
    const m = this.byTenant.get(tenant)
    if (!m) return []
    const set = m.get(nounType)
    return set ? Array.from(set) : []
  }
}

// In-memory engine: tracks per-handle "applied bodies" so we can
// assert per-tenant isolation without dragging the real WASM in.
class FakeEngine implements HandleProvider {
  private nextHandle = 1
  // handle → list of applied bodies (in load order)
  readonly applied = new Map<number, string[]>()
  // body → diagnostic line emission (for tests that want a violation)
  readonly diagnostics = new Map<string, string>()
  // body → applyBody result (default true = success)
  readonly applyOutcomes = new Map<string, boolean>()

  createHandle(): number {
    const h = this.nextHandle++
    this.applied.set(h, [])
    return h
  }

  applyBody(handle: number, body: string): boolean {
    const list = this.applied.get(handle) ?? []
    list.push(body)
    this.applied.set(handle, list)
    return this.applyOutcomes.get(body) ?? true
  }

  checkBody(_handle: number, body: string): string {
    return this.diagnostics.get(body) ?? ''
  }
}

// Build a complete deps bundle for a tenant. The same cell store is
// shared across tenants because cell keys include the tenant — that's
// the property under test.
function makeDeps(
  tenant: string,
  cells: FakeCellStore,
  registry: FakeRegistry,
  engine: HandleProvider,
): LoadReadingDeps {
  return {
    tenant,
    handleProvider: engine,
    getCellStub: (key) => cells.stub(key),
    getIndexStub: (t) => registry.stub(t),
    now: () => '2026-04-30T00:00:00Z',
  }
}

beforeEach(() => {
  _resetCaches()
})

// ── Pure helpers ───────────────────────────────────────────────────

describe('manifestCellKey — per-tenant scoping (#205, #217)', () => {
  it('embeds the tenant in the cell key so two tenants under the same name land on disjoint DOs', () => {
    const a = manifestCellKey('alpha', 'catalog')
    const b = manifestCellKey('beta', 'catalog')
    expect(a).toBe('_loaded_reading:alpha:catalog')
    expect(b).toBe('_loaded_reading:beta:catalog')
    expect(a).not.toBe(b)
  })

  it('falls back to "global" when tenant is empty so legacy callers still get a stable key', () => {
    expect(manifestCellKey('', 'catalog')).toBe('_loaded_reading:global:catalog')
    expect(manifestCellKey('   ', 'catalog')).toBe('_loaded_reading:global:catalog')
  })

  it('round-trips via parseManifestCellKey', () => {
    const key = manifestCellKey('acme', 'product-catalog')
    expect(parseManifestCellKey(key)).toEqual({
      tenant: 'acme',
      name: 'product-catalog',
    })
  })

  it('returns null from parseManifestCellKey when prefix is wrong', () => {
    expect(parseManifestCellKey('Order:abc:def')).toBeNull()
  })
})

describe('computeContentHash — mirror of arest::load_reading_core::compute_content_hash', () => {
  it('produces a 16-char lowercase hex digest', () => {
    const h = computeContentHash('Product(.SKU) is an entity type.\n')
    expect(h).toMatch(/^[0-9a-f]{16}$/)
  })

  it('is byte-deterministic across calls', () => {
    const a = computeContentHash('Alpha(.Name) is an entity type.\n')
    const b = computeContentHash('Alpha(.Name) is an entity type.\n')
    expect(a).toBe(b)
  })

  it('byte-different bodies yield different digests', () => {
    const a = computeContentHash('Alpha(.Name) is an entity type.\n')
    const b = computeContentHash('Beta(.Name) is an entity type.\n')
    expect(a).not.toBe(b)
  })

  it('empty body still produces a 16-char digest (FNV offset basis)', () => {
    expect(computeContentHash('')).toMatch(/^[0-9a-f]{16}$/)
  })
})

describe('decodeValidationReport — partitioning diagnostics by source', () => {
  it('empty raw input means the gate passed', () => {
    const r = decodeValidationReport('')
    expect(r.passes).toBe(true)
    expect(r.alethicViolations).toEqual([])
    expect(r.deonticViolations).toEqual([])
  })

  it('routes parse errors to alethicViolations', () => {
    const raw = '[ERROR parse] foo: unexpected token'
    const r = decodeValidationReport(raw)
    expect(r.passes).toBe(false)
    expect(r.alethicViolations).toHaveLength(1)
    expect(r.deonticViolations).toHaveLength(0)
    expect(r.alethicViolations[0]!.message).toBe('unexpected token')
  })

  it('routes resolve errors to alethicViolations (per #559 partition)', () => {
    const raw = '[ERROR resolve] some reading: unknown noun'
    const r = decodeValidationReport(raw)
    expect(r.alethicViolations).toHaveLength(1)
    expect(r.deonticViolations).toHaveLength(0)
  })

  it('routes deontic errors to deonticViolations', () => {
    const raw = '[ERROR deontic] X: cardinality violation'
    const r = decodeValidationReport(raw)
    expect(r.passes).toBe(false)
    expect(r.alethicViolations).toHaveLength(0)
    expect(r.deonticViolations).toHaveLength(1)
  })

  it('drops warnings and hints (mirror of #559 silent passthrough)', () => {
    const raw = [
      '[WARN parse] foo: minor issue',
      '[HINT resolve] bar: maybe try this',
    ].join('\n')
    const r = decodeValidationReport(raw)
    expect(r.passes).toBe(true)
    expect(r.alethicViolations).toEqual([])
    expect(r.deonticViolations).toEqual([])
  })

  it('handles multi-line outputs with mixed sources', () => {
    const raw = [
      '[ERROR parse] r1: bad',
      '[ERROR deontic] r2: violation',
      '[WARN resolve] r3: warn-only',
    ].join('\n')
    const r = decodeValidationReport(raw)
    expect(r.alethicViolations).toHaveLength(1)
    expect(r.deonticViolations).toHaveLength(1)
    expect(r.passes).toBe(false)
  })
})

describe('resolveTenant', () => {
  it('reads x-tenant header', () => {
    const r = new Request('https://x/y', { headers: { 'x-tenant': 'acme' } })
    expect(resolveTenant(r)).toBe('acme')
  })

  it('falls back to ?tenant= query', () => {
    const r = new Request('https://x/y?tenant=beta')
    expect(resolveTenant(r)).toBe('beta')
  })

  it('defaults to "global"', () => {
    const r = new Request('https://x/y')
    expect(resolveTenant(r)).toBe('global')
  })

  it('strips colons defensively (cell-key separator)', () => {
    const r = new Request('https://x/y', { headers: { 'x-tenant': 'a:b' } })
    // Cell-key separator is ":". A stray colon would fragment the key.
    expect(resolveTenant(r)).toBe('a_b')
  })
})

// ── End-to-end adapter tests ───────────────────────────────────────

describe('loadReading — DynRdg-T3 adapter', () => {
  it('happy path: persists the manifest cell + indexes it for replay', async () => {
    const cells = new FakeCellStore()
    const registry = new FakeRegistry()
    const engine = new FakeEngine()
    const deps = makeDeps('alpha', cells, registry, engine)

    const result = await loadReading(
      'catalog',
      'Product(.SKU) is an entity type.\n',
      deps,
    )

    expect(result.ok).toBe(true)
    expect(result.status).toBe(200)
    const r = result.response as LoadReadingResponse
    expect(r.name).toBe('catalog')
    expect(r.tenant).toBe('alpha')
    expect(r.contentHash).toMatch(/^[0-9a-f]{16}$/)
    expect(r.versionStamp).toBe(1)
    expect(r.validation.passes).toBe(true)

    // Manifest cell landed on its own DO.
    const cellKey = manifestCellKey('alpha', 'catalog')
    expect(cells.has(cellKey)).toBe(true)
    const cell = cells.get(cellKey)!
    expect(cell.type).toBe('_LoadedReading')
    const manifest = cell.data as unknown as LoadedReadingManifest
    expect(manifest.body).toBe('Product(.SKU) is an entity type.\n')
    expect(manifest.contentHash).toBe(r.contentHash)
    expect(manifest.tenant).toBe('alpha')

    // Registry index registered the cell.
    expect(registry.forTenant('alpha', '_LoadedReading')).toContain(cellKey)
  })

  it('per-tenant scoping: tenant A and tenant B keep disjoint cell sets (verification #1)', async () => {
    const cells = new FakeCellStore()
    const registry = new FakeRegistry()
    const engine = new FakeEngine()

    const depsA = makeDeps('alpha', cells, registry, engine)
    const depsB = makeDeps('beta', cells, registry, engine)

    await loadReading('catalog', 'Product(.SKU) is an entity type.\n', depsA)

    // Tenant A's cell is in the cell store; tenant B's is not.
    expect(cells.has(manifestCellKey('alpha', 'catalog'))).toBe(true)
    expect(cells.has(manifestCellKey('beta', 'catalog'))).toBe(false)

    // Tenant B's registry index doesn't see tenant A's reading.
    expect(registry.forTenant('beta', '_LoadedReading')).toEqual([])
    expect(registry.forTenant('alpha', '_LoadedReading')).toContain(
      manifestCellKey('alpha', 'catalog'),
    )

    // Tenant B can load the same name without clobbering A's cell.
    await loadReading('catalog', 'Service(.Name) is an entity type.\n', depsB)
    expect(cells.has(manifestCellKey('beta', 'catalog'))).toBe(true)

    const cellA = cells.get(manifestCellKey('alpha', 'catalog'))!
    const cellB = cells.get(manifestCellKey('beta', 'catalog'))!
    expect((cellA.data as any).body).toContain('Product')
    expect((cellB.data as any).body).toContain('Service')
  })

  it('per-tenant engine isolation: tenant A\'s body lands on tenant A\'s handle only', async () => {
    const cells = new FakeCellStore()
    const registry = new FakeRegistry()
    const engine = new FakeEngine()

    const depsA = makeDeps('alpha', cells, registry, engine)
    const depsB = makeDeps('beta', cells, registry, engine)

    await loadReading('a-cat', 'Alpha(.Name) is an entity type.\n', depsA)
    await loadReading('b-cat', 'Beta(.Name) is an entity type.\n', depsB)

    // Each tenant got its own handle; bodies didn't cross-pollinate.
    const handleA = await getTenantHandle(depsA)
    const handleB = await getTenantHandle(depsB)
    expect(handleA).not.toBe(handleB)

    const appliedA = engine.applied.get(handleA) ?? []
    const appliedB = engine.applied.get(handleB) ?? []
    expect(appliedA.some((b) => b.includes('Alpha'))).toBe(true)
    expect(appliedA.some((b) => b.includes('Beta'))).toBe(false)
    expect(appliedB.some((b) => b.includes('Beta'))).toBe(true)
    expect(appliedB.some((b) => b.includes('Alpha'))).toBe(false)
  })

  it('deontic violation: surfaces in the report and does NOT mutate cell state (verification #3)', async () => {
    const cells = new FakeCellStore()
    const registry = new FakeRegistry()
    const engine = new FakeEngine()
    const deps = makeDeps('alpha', cells, registry, engine)

    // Stage the engine to emit a deontic violation for this body.
    const body = 'A constraint-violating reading.\n'
    engine.diagnostics.set(body, '[ERROR deontic] R: cardinality violation')

    const result = await loadReading('bad', body, deps)
    expect(result.ok).toBe(false)
    expect(result.status).toBe(422)
    const r = result.response as { error: string; validation: any }
    expect(r.error).toBe('deontic_violation')
    expect(r.validation.deonticViolations).toHaveLength(1)
    expect(r.validation.passes).toBe(false)

    // Cell state was NOT mutated — the manifest cell was never written.
    expect(cells.has(manifestCellKey('alpha', 'bad'))).toBe(false)
    expect(registry.forTenant('alpha', '_LoadedReading')).toEqual([])

    // Engine compile path was NOT invoked — only the check verb ran.
    const handle = await getTenantHandle(deps)
    expect(engine.applied.get(handle) ?? []).toEqual([])
  })

  it('alethic violation: routed to alethicViolations and rejects the load', async () => {
    const cells = new FakeCellStore()
    const registry = new FakeRegistry()
    const engine = new FakeEngine()
    const deps = makeDeps('alpha', cells, registry, engine)

    const body = 'malformed input\n'
    engine.diagnostics.set(body, '[ERROR parse] R: unexpected token')

    const result = await loadReading('bad', body, deps)
    expect(result.ok).toBe(false)
    const r = result.response as { error: string; validation: any }
    expect(r.error).toBe('alethic_violation')
    expect(r.validation.alethicViolations).toHaveLength(1)
    expect(r.validation.deonticViolations).toHaveLength(0)
    expect(cells.has(manifestCellKey('alpha', 'bad'))).toBe(false)
  })

  it('rejects empty / whitespace-only name and body', async () => {
    const cells = new FakeCellStore()
    const registry = new FakeRegistry()
    const engine = new FakeEngine()
    const deps = makeDeps('alpha', cells, registry, engine)

    const a = await loadReading('', 'body', deps)
    expect(a.ok).toBe(false)
    expect(a.status).toBe(400)

    const b = await loadReading('name', '', deps)
    expect(b.ok).toBe(false)
    expect(b.status).toBe(400)

    const c = await loadReading('name', '   \n  ', deps)
    expect(c.ok).toBe(false)
    expect(c.status).toBe(400)
  })

  it('rejects names with control characters (manifests-as-cell-id safety)', async () => {
    const cells = new FakeCellStore()
    const registry = new FakeRegistry()
    const engine = new FakeEngine()
    const deps = makeDeps('alpha', cells, registry, engine)

    // U+0001 is a control character (Cc category). Names land in DO
    // cell-ids; control chars in those would break legibility and
    // tooling. Mirror of `arest::load_reading_core::load_reading` step 2.
    const ctrlName = 'bad' + String.fromCharCode(1) + 'name'
    const r = await loadReading(ctrlName, 'Alpha(.Name) is an entity type.\n', deps)
    expect(r.ok).toBe(false)
    expect(r.status).toBe(400)
    expect(cells.has(manifestCellKey('alpha', ctrlName))).toBe(false)
  })

  it('versionStamp is monotonic per-tenant across successive loads', async () => {
    const cells = new FakeCellStore()
    const registry = new FakeRegistry()
    const engine = new FakeEngine()
    const deps = makeDeps('alpha', cells, registry, engine)

    const r1 = (await loadReading('a', 'Alpha(.Name) is an entity type.\n', deps)).response as LoadReadingResponse
    const r2 = (await loadReading('b', 'Beta(.Name) is an entity type.\n', deps)).response as LoadReadingResponse
    const r3 = (await loadReading('c', 'Gamma(.Name) is an entity type.\n', deps)).response as LoadReadingResponse
    expect(r1.versionStamp).toBe(1)
    expect(r2.versionStamp).toBe(2)
    expect(r3.versionStamp).toBe(3)
  })
})

// ── Cold-start replay ─────────────────────────────────────────────

describe('coldStartReplay — verification #2 (worker restart re-applies persisted readings)', () => {
  it('on a fresh isolate, replays every indexed manifest cell against the new handle', async () => {
    const cells = new FakeCellStore()
    const registry = new FakeRegistry()
    const engine = new FakeEngine()
    const depsBoot1 = makeDeps('alpha', cells, registry, engine)

    // Boot 1: load two readings.
    await loadReading('a', 'Alpha(.Name) is an entity type.\n', depsBoot1)
    await loadReading('b', 'Beta(.Name) is an entity type.\n', depsBoot1)
    expect(registry.forTenant('alpha', '_LoadedReading')).toHaveLength(2)

    const handleBoot1 = await getTenantHandle(depsBoot1)
    const appliedBoot1 = engine.applied.get(handleBoot1) ?? []
    // First boot applied each body once (compile call from loadReading).
    expect(appliedBoot1.filter((b) => b.includes('Alpha'))).toHaveLength(1)
    expect(appliedBoot1.filter((b) => b.includes('Beta'))).toHaveLength(1)

    // Boot 2: simulate cold-start. Same cell store / registry, but
    // the in-process handle cache is wiped (mirrors a fresh isolate).
    _resetCaches()

    const depsBoot2 = makeDeps('alpha', cells, registry, engine)
    const handleBoot2 = await getTenantHandle(depsBoot2)
    expect(handleBoot2).not.toBe(handleBoot1)

    // The replay re-applied both bodies on the new handle.
    const appliedBoot2 = engine.applied.get(handleBoot2) ?? []
    expect(appliedBoot2.filter((b) => b.includes('Alpha'))).toHaveLength(1)
    expect(appliedBoot2.filter((b) => b.includes('Beta'))).toHaveLength(1)
  })

  it('replay is idempotent: calling getTenantHandle twice on the same isolate replays once', async () => {
    const cells = new FakeCellStore()
    const registry = new FakeRegistry()
    const engine = new FakeEngine()
    const deps = makeDeps('alpha', cells, registry, engine)

    // Seed an existing manifest as if from a prior boot.
    await loadReading('a', 'Alpha(.Name) is an entity type.\n', deps)
    _resetCaches()

    const fresh = makeDeps('alpha', cells, registry, engine)
    const h1 = await getTenantHandle(fresh)
    const h2 = await getTenantHandle(fresh)
    expect(h1).toBe(h2)
    const applied = engine.applied.get(h1) ?? []
    // The body is applied once during cold-start replay; the second
    // getTenantHandle call returns the cached handle without replay.
    expect(applied.filter((b) => b.includes('Alpha'))).toHaveLength(1)
  })

  it('a corrupt / unparseable manifest body is skipped without wedging the boot', async () => {
    const cells = new FakeCellStore()
    const registry = new FakeRegistry()
    const engine = new FakeEngine()
    const deps = makeDeps('alpha', cells, registry, engine)

    // Plant a good record + a bad record.
    await loadReading('good', 'Alpha(.Name) is an entity type.\n', deps)
    await loadReading('alsoGood', 'Beta(.Name) is an entity type.\n', deps)
    // Now stage the engine to refuse the second body on replay.
    engine.applyOutcomes.set('Beta(.Name) is an entity type.\n', false)

    _resetCaches()
    const fresh = makeDeps('alpha', cells, registry, engine)
    const h = await getTenantHandle(fresh)
    const applied = engine.applied.get(h) ?? []
    // The good body landed; the bad body's apply was attempted but
    // the engine returned ⊥ — replay continued either way.
    expect(applied.filter((b) => b.includes('Alpha'))).toHaveLength(1)
  })

  it('cold-start replay with no persisted readings returns 0 applied (no-op on fresh tenant)', async () => {
    const cells = new FakeCellStore()
    const registry = new FakeRegistry()
    const engine = new FakeEngine()
    const deps = makeDeps('untouched', cells, registry, engine)

    const h = engine.createHandle()
    const applied = await coldStartReplay(h, deps)
    expect(applied).toBe(0)
  })
})

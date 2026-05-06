/**
 * AREST engine — SYSTEM:x = ⟨o, D'⟩
 *
 * Two WASM exports: create, system. SYSTEM is the only function.
 * Self-modification: system(h, 'compile', readings_text) ingests readings.
 * All other operations: system(h, key, input) dispatches via ρ.
 */

import { create, create_bare, release, system } from '../../crates/arest/pkg/arest.js'

// ── Per-DO engine lifecycle (#764) ──────────────────────────────────
//
// `_h` below is a per-PROCESS engine handle — fine for the legacy
// shared-domain code paths (apply/query/etc. that all read the same
// in-process metamodel), but wrong for EntityDB DOs. Each DO instance
// IS a cell per the whitepaper (§3.3, §202): its engine state is the
// per-cell fold `D_n' = foldl μ_n D_n E_n` and must persist across
// the DO's lifetime independently of any other DO. The two helpers
// below — `freezeHandle` and `thawHandle` — are the wasm-bindgen
// surface the DO uses to round-trip a handle's whole D through DO
// storage between invocations.
//
// Both are thin shims over the engine's `system(handle, "freeze"|
// "thaw", …)` keys, which encode the freeze image as lowercase hex
// (string-only transport — wasm-bindgen + MCP both hand strings).
// See `crates/arest/src/lib.rs` line 1814 for the engine side.

/**
 * Snapshot the engine state for `handle` as a hex-encoded freeze image.
 * Returns the bytes ready to be written to durable storage.
 *
 * The hex form is stable + byte-deterministic — two freezes of the
 * same state produce identical strings, which keeps the persisted
 * `engine_state_bytes` blob diffable.
 */
export function freezeHandle(handle: number): string {
  ensureWasm()
  return system(handle, 'freeze', '')
}

/**
 * Replace the engine state for `handle` with the contents of
 * `hexBytes` (a hex-encoded freeze image previously produced by
 * `freezeHandle`). Returns `true` on success, `false` if the engine
 * rejected the bytes (malformed hex, bad magic, truncated payload).
 *
 * The DO uses this on cold start to hydrate its per-cell engine
 * state from `state.storage.get('engine_state_bytes')` before any
 * call observes the handle.
 */
export function thawHandle(handle: number, hexBytes: string): boolean {
  ensureWasm()
  return system(handle, 'thaw', hexBytes) === 'ok'
}

/**
 * Look up the chain head's `version_id` for `cellName` in the engine
 * pinned at `handle` (#721 / S1e). Returns the decimal version_id as
 * a JS number for chain cells, or `null` for cells that the engine
 * does not yet know about (the engine returns `"⊥"` in that case —
 * see `crates/arest/src/lib.rs::system_impl` for the `cell_pin`
 * dispatch and §S1c eq:cellfold for why "the chain IS the version
 * stamp").
 *
 * AEAD callers (#767 / S1i) source the `CellAddress.version` field
 * from this helper instead of the worker-minted SQL counter:
 * binding the AAD to the engine's true storage version is what eq:
 * cellfold guarantees, so a captured-then-replayed older sealed
 * envelope at the same `(scope, domain, cell_name)` fails to decrypt
 * because the current chain head's version no longer matches the
 * captured ciphertext's AAD.
 *
 * Returns `null` (NOT throw) when:
 *   - `handle` is out of range / not allocated
 *   - the named cell has no chain entry (pre-S1b raw cell, or never
 *     written through the engine)
 *
 * The caller is expected to fall back to its legacy version source
 * (the worker `cell.version` SQL column) when this returns `null`,
 * so existing-cell encryption/decryption still works during the
 * migration window before #768 drops the column.
 */
export function callCellPin(handle: number, cellName: string): number | null {
  ensureWasm()
  const raw = system(handle, 'cell_pin', cellName)
  if (raw === '⊥' || raw === '') return null
  // Decimal-encoded u64. Parse via Number — version_ids in practice
  // stay well under 2^53 (one bump per write, per cell, per DO; even
  // a 1-Hz writer for a century is ~3e9 ≪ 9e15). If we ever need to
  // handle larger values we'd return BigInt here, but the AAD
  // canonical encoder downstream takes `bigint | number` already
  // (see `cell-encryption.ts::canonicalAddressBytes`).
  const n = Number(raw)
  return Number.isFinite(n) ? n : null
}

// wasm-pack --target bundler auto-initializes the WASM when arest.js is
// imported (via wasm.__wbindgen_start() inside the wrapper). No explicit
// initSync call is needed here — ensureWasm is kept as a no-op so the
// existing call sites don't need to change.
function ensureWasm() { /* auto-init via bundler target */ }

let _h = -1
function h(handle?: number): number { return handle !== undefined && handle >= 0 ? handle : _h }

export { system }

export function currentDomainHandle(): number { return _h }

export function release_domain(handle: number): void { ensureWasm(); release(handle) }

/**
 * create + compile: allocate D with the bundled metamodel loaded and ingest
 * user readings on top. Use this for apps — you get a fully self-describing
 * engine without having to pass metamodel readings yourself.
 */
export function compileDomainReadings(...readings: string[]): number {
  ensureWasm()
  const handle = create()
  for (const text of readings) {
    system(handle, 'compile', text)
  }
  return handle
}

/**
 * Bare variant: allocate D with ONLY the platform primitives (compile,
 * apply, verify_signature) and nothing else. Use this when testing a new
 * core, or for paper-verification tests that supply the metamodel fragments
 * explicitly via STATE_READINGS / ORDER_READINGS fixtures.
 */
export function compileDomainReadingsBare(...readings: string[]): number {
  ensureWasm()
  const handle = create_bare()
  for (const text of readings) {
    system(handle, 'compile', text)
  }
  return handle
}

export async function loadDomainSchema(
  registry: any,
  getStub: (id: string) => any,
  domainSlug: string,
): Promise<number> {
  ensureWasm()
  const defsCell = await getStub(`defs:${domainSlug}`).get().catch(() => null)
  const readings = defsCell?.data?.readings
  if (!readings) return -1
  _h = compileDomainReadings(readings)
  return _h
}

// ── Applications of SYSTEM ──────────────────────────────────────────

export function evaluateConstraints(text: string, population: string, handle?: number) {
  ensureWasm()
  return JSON.parse(system(h(handle), 'evaluate', JSON.stringify({ text, population })))
}

export function forwardChain(population: string, handle?: number) {
  ensureWasm()
  return JSON.parse(system(h(handle), 'forward_chain', population))
}

export function getTransitions(noun: string, status: string, handle?: number) {
  ensureWasm()
  return JSON.parse(system(h(handle), `transitions:${noun}`, status))
}

export function applyCommand(command: any, population: string, handle?: number) {
  ensureWasm()
  return JSON.parse(system(h(handle), 'apply', JSON.stringify({ command, population })))
}

export function querySchema(schemaId: string, targetRole: number, filterBindings: any, population: string, handle?: number) {
  ensureWasm()
  return JSON.parse(system(h(handle), 'query', JSON.stringify({ schemaId, targetRole, filterBindings, population })))
}

export function getNounSchemas(noun: string, handle?: number) {
  ensureWasm()
  return JSON.parse(system(h(handle), 'noun_schemas', noun))
}

export function computeRMAP(handle?: number) {
  ensureWasm()
  return JSON.parse(system(h(handle), 'rmap', ''))
}

export function parseReadings(markdown: string, domain: string) {
  ensureWasm()
  return JSON.parse(system(0, 'parse', JSON.stringify({ markdown, domain })))
}

export function parseReadingsWithNouns(markdown: string, domain: string, existingNounsJson: string) {
  ensureWasm()
  return JSON.parse(system(0, 'parse_with_nouns', JSON.stringify({ markdown, domain, nouns: JSON.parse(existingNounsJson) })))
}

// ── Population from EntityDB (↑FILE:D) ──────────────────────────────

export async function buildPopulation(
  registry: any,
  getStub: (id: string) => any,
  domainSlug: string,
): Promise<string> {
  const counts = await registry.getEntityCounts(domainSlug) as Array<{ nounType: string; count: number }>
  const facts: Record<string, Array<{ factTypeId: string; bindings: Array<[string, string]> }>> = {}
  const schemaTypes = new Set(['Noun', 'Reading', 'Fact Type', 'Role', 'Constraint', 'CompiledSchema', 'Derivation Rule', 'State Machine Definition', 'Status', 'Transition', 'External System', 'Instance Fact'])
  const entitySettled = await Promise.allSettled(
    counts.filter(({ nounType }) => !schemaTypes.has(nounType)).flatMap(({ nounType }) =>
      registry.getEntityIds(nounType, domainSlug).then((ids: string[]) =>
        Promise.allSettled(ids.map(async (id: string) => {
          const entity = await getStub(id).get()
          return entity ? { ...entity, nounType } : null
        }))
      )
    )
  )
  entitySettled
    .filter((r): r is PromiseFulfilledResult<any> => r.status === 'fulfilled')
    .flatMap(r => r.value)
    .filter((r: any): r is PromiseFulfilledResult<any> => r.status === 'fulfilled' && r.value)
    .map((r: any) => r.value)
    .forEach((entity: any) => {
      Object.entries(entity.data || {}).forEach(([field, value]) => {
        if (field.startsWith('_')) return
        if (typeof value !== 'string' && typeof value !== 'number' && typeof value !== 'boolean') return
        const ftId = `${entity.nounType || entity.type}_has_${field}`
        const list = facts[ftId] || []
        list.push({ factTypeId: ftId, bindings: [[entity.nounType || entity.type, entity.id], [field, String(value)]] })
        facts[ftId] = list
      })
    })
  return JSON.stringify({ facts })
}

export async function loadDomainAndPopulation(
  registry: any,
  getStub: (id: string) => any,
  domainSlug: string,
): Promise<string> {
  await loadDomainSchema(registry, getStub, domainSlug)
  return buildPopulation(registry, getStub, domainSlug)
}


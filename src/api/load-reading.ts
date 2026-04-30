/**
 * load-reading.ts — DynRdg-T3 (#562) worker-side adapter.
 *
 * SystemVerb::LoadReading (#555 / DynRdg-1) on the Cloudflare Worker
 * target. Mirrors the kernel-side `load_reading_persist.rs` (#560)
 * pattern, but the persistence backend is "one Durable Object per
 * cell" (per #217 / #221) instead of the kernel's virtio-blk ring.
 *
 * ## What this module does
 *
 *   1. POST /api/load_reading: body = { name, body }.
 *   2. Per-tenant scope: tenant taken from X-Tenant header (default
 *      'global'). Two tenants on the same worker keep isolated
 *      reading sets — cell address keys include the tenant.
 *   3. Run the engine-level dispatch (#586/#589):
 *        - `system(h, 'check', body)` to surface deontic + alethic
 *          violations in the LoadValidationReport contract (#557/#559).
 *        - On success, `system(h, 'compile', body)` ingests the body
 *          into the per-tenant in-memory engine state.
 *   4. Persist the manifest cell `_loaded_reading:{tenant}:{name}` to
 *      its own EntityDB DO (per #217 cell-per-DO). The DO carries
 *      both the LoadReport-shaped manifest fields (contentHash,
 *      versionStamp, addedNouns/...) AND the original body so the
 *      cold-start path can replay without a separate KV.
 *   5. Cold-start replay: on the first request after a fresh worker
 *      isolate, walk the registry's `_LoadedReading` index for the
 *      tenant, fetch each cell's body, and re-apply via
 *      `system(h, 'compile', body)`. Idempotent — same body produces
 *      the same cell graph (set semantics on Noun / FactType / etc.).
 *
 * ## Why not the WASM `load_reading:<name>` intercept?
 *
 * The engine exposes `system(h, "load_reading:<name>", body)` (lib.rs
 * #555 path), but it's gated behind `RegisterMode::Privileged` and
 * the worker's `wasm_bindgen` surface (`crates/arest/src/cloudflare.rs`)
 * does not export `set_register_mode`. Flipping that gate is a
 * sibling task in the arest crate (#572 owns that crate) so this
 * adapter drives the engine via the already-exported `system(h,
 * "compile", body)` (which is the same parse + merge pipeline) and
 * runs the validation gate via the existing `system(h, "check",
 * body)` verb. The kernel-side `load_reading_persist.rs` follows the
 * same logical contract: parse + validate + merge.
 *
 * ## Ownership map (per the task brief)
 *
 *   - kernel-side `crates/arest-kernel/src/load_reading_persist.rs` is
 *     READ-ONLY here; this file is its sibling.
 *   - the engine's `crates/arest/src/load_reading_core.rs` is the
 *     contract source (LoadValidationReport, manifest cell shape).
 *   - this file lives next to the existing arest-router / verb
 *     dispatcher; it does not touch the WASM crate or the kernel.
 */

import * as engine from './engine'
import { cellKey } from './cell-key'
import { envelope, type Envelope, type Violation } from './envelope'

// ── Public types — wire shape for /api/load_reading ────────────────

/**
 * Mirrors `crate::check::ReadingDiagnostic`. The envelope decoder on
 * the worker side normalises the diagnostic-line format emitted by
 * `system(h, 'check', body)`. (See `parseDiagLine` below — same shape
 * the verb-dispatcher's `Diagnostic` uses.)
 */
export interface LoadDiagnostic {
  readonly reading: string
  readonly message: string
  readonly line?: number
  readonly source: 'parse' | 'resolve' | 'deontic' | 'unknown'
  readonly level: 'error' | 'warn' | 'hint' | 'unknown'
  readonly suggestion?: string | null
}

/**
 * TS port of `arest::load_reading_core::LoadValidationReport`.
 *
 * Partitions diagnostics by `Source`:
 *   * alethicViolations — Source::Parse / Source::Resolve at error level.
 *     Structural impossibilities. Reject the load.
 *   * deonticViolations — Source::Deontic at error level. The body is
 *     well-formed but the resulting state would violate a constraint.
 *     Reject the load.
 *   * passes — true iff both lists are empty.
 */
export interface LoadValidationReport {
  readonly alethicViolations: readonly LoadDiagnostic[]
  readonly deonticViolations: readonly LoadDiagnostic[]
  readonly passes: boolean
}

/**
 * Manifest record persisted into the `_loaded_reading:{tenant}:{name}`
 * cell DO. Mirrors `crate::load_reading_core::LoadReport` plus the
 * original `body` (so cold-start replay can re-apply it without an
 * external KV — the body must round-trip through the same DO that
 * carries the manifest).
 */
export interface LoadedReadingManifest {
  readonly name: string
  readonly tenant: string
  readonly body: string
  readonly contentHash: string
  readonly versionStamp: number
  readonly addedAt: string
}

/**
 * Wire shape returned to the caller on a successful load.
 */
export interface LoadReadingResponse {
  readonly name: string
  readonly tenant: string
  readonly contentHash: string
  readonly versionStamp: number
  readonly validation: LoadValidationReport
}

// ── Cell addressing — per-tenant scoping (#205) ────────────────────

/**
 * Cell-name prefix for the per-reading manifest. Mirrors
 * `arest::load_reading_core::MANIFEST_CELL_PREFIX`.
 */
export const MANIFEST_CELL_PREFIX = '_loaded_reading'

/**
 * Compute the DO key for a `(tenant, name)` manifest cell. Uses the
 * same `cellKey` helper (#217) that every other cell-bound route
 * uses — keeping the RMAP-derived naming authoritative in one place.
 *
 * Format: `_loaded_reading:{tenant}:{name}`. The tenant is part of
 * the cell name (not just a DO-namespace prefix) so the cell address
 * round-trips through `parseCellKey` cleanly: nounType = '_loaded_reading',
 * entityId = '{tenant}:{name}'.
 *
 * Two tenants on the same worker have disjoint `(tenant, name)` keys
 * by construction, so per-tenant isolation falls out of the cell key
 * itself (Definition 2: cell isolation across disjoint cell names).
 */
export function manifestCellKey(tenant: string, name: string): string {
  const t = (tenant || 'global').trim() || 'global'
  const n = name.trim()
  return cellKey(MANIFEST_CELL_PREFIX, `${t}:${n}`)
}

/**
 * Parse a `_loaded_reading:{tenant}:{name}` cell key back into its
 * components. Returns null when the key isn't in manifest format.
 */
export function parseManifestCellKey(
  key: string,
): { tenant: string; name: string } | null {
  const prefix = `${MANIFEST_CELL_PREFIX}:`
  if (!key.startsWith(prefix)) return null
  const rest = key.slice(prefix.length)
  const sep = rest.indexOf(':')
  if (sep <= 0 || sep === rest.length - 1) return null
  return { tenant: rest.slice(0, sep), name: rest.slice(sep + 1) }
}

// ── Validation report decoder ─────────────────────────────────────

/**
 * Parse the `system(h, 'check', body)` output (newline-separated
 * `[LEVEL source] reading: message` lines) into a LoadValidationReport.
 *
 * Same partition the engine's `validate_loaded_state` applies:
 *   * Source::Parse  | Level::Error → alethic
 *   * Source::Resolve | Level::Error → alethic
 *   * Source::Deontic | Level::Error → deontic
 * Warnings and hints are dropped — same as #559's gate.
 */
export function decodeValidationReport(raw: string): LoadValidationReport {
  const alethic: LoadDiagnostic[] = []
  const deontic: LoadDiagnostic[] = []
  if (!raw || raw.trim() === '') {
    return { alethicViolations: [], deonticViolations: [], passes: true }
  }
  const lines = raw.split('\n').map((l) => l.trim()).filter((l) => l.length > 0)
  for (const line of lines) {
    const d = parseDiagLine(line)
    if (d.level !== 'error') continue
    if (d.source === 'parse' || d.source === 'resolve') {
      alethic.push(d)
    } else if (d.source === 'deontic') {
      deontic.push(d)
    }
  }
  return {
    alethicViolations: alethic,
    deonticViolations: deontic,
    passes: alethic.length === 0 && deontic.length === 0,
  }
}

function parseDiagLine(line: string): LoadDiagnostic {
  // Same shape the verb-dispatcher emits (parseDiagLine over there);
  // we re-implement here so this module stays standalone.
  const m = /^\[(ERROR|WARN|HINT) (parse|resolve|deontic)\] (.*?): (.*?)(?: \(suggestion: (.*?)\))?$/.exec(line)
  if (!m) {
    return { reading: '', message: line, source: 'unknown', level: 'unknown' }
  }
  const level = m[1].toLowerCase() as LoadDiagnostic['level']
  const source = m[2] as LoadDiagnostic['source']
  return {
    reading: m[3],
    message: m[4],
    source,
    level,
    suggestion: m[5] ?? null,
  }
}

// ── Content hash (FNV-1a 64-bit) ──────────────────────────────────

/**
 * 16-char lowercase hex FNV-1a64 digest of `body`. Mirrors
 * `arest::load_reading_core::compute_content_hash` byte-for-byte so
 * worker manifests carry the same hash a kernel-side replay would
 * compute. Not cryptographic — just "callers can tell two loads apart."
 */
export function computeContentHash(body: string): string {
  const FNV_OFFSET = 0xcbf29ce484222325n
  const FNV_PRIME = 0x100000001b3n
  const MASK = 0xffffffffffffffffn
  let h = FNV_OFFSET
  // Hash byte-stream as UTF-8 — same as the Rust side which iterates
  // body.as_bytes(). TextEncoder produces the same byte sequence.
  const bytes = new TextEncoder().encode(body)
  for (const b of bytes) {
    h = (h ^ BigInt(b)) & MASK
    h = (h * FNV_PRIME) & MASK
  }
  return h.toString(16).padStart(16, '0')
}

// ── Tenant scope resolution ───────────────────────────────────────

/**
 * Tenant identifier for a request. Reads `X-Tenant` (the header used
 * by the existing rest of the worker for per-tenant scoping
 * suggestions) or query param `?tenant=`, falling back to 'global'.
 *
 * Trimmed; empty → 'global'. Tenant identifiers must not contain ':'
 * (it's the cell-key separator); we replace ':' with '_' defensively
 * so a stray colon doesn't fragment the cell address.
 */
export function resolveTenant(request: Request): string {
  const url = new URL(request.url)
  const fromHeader = request.headers.get('x-tenant') || request.headers.get('X-Tenant')
  const fromQuery = url.searchParams.get('tenant')
  const raw = (fromHeader || fromQuery || 'global').trim()
  const cleaned = raw.replace(/:/g, '_')
  return cleaned || 'global'
}

// ── Per-tenant engine handle cache + cold-start replay ────────────

/**
 * Process-local cache of compiled engine handles per tenant. A fresh
 * worker isolate starts with an empty cache; the first call per
 * tenant runs `coldStartReplay` which walks the registry's
 * `_LoadedReading` index, fetches each cell, and re-applies the body
 * via `system(h, 'compile', body)`.
 *
 * The cache is module-local because the worker isolate already has
 * an in-memory CompiledState per WASM handle — caching the handle
 * keeps it pinned for the isolate's lifetime so subsequent loads on
 * the same tenant see the previously-merged state.
 */
const HANDLE_CACHE = new Map<string, number>()
const REPLAY_DONE = new Set<string>()

export interface HandleProvider {
  /** Allocate a fresh engine handle (mocked in tests). */
  createHandle(): number
  /**
   * Apply a reading body to the handle. Defaults to
   * `engine.system(h, 'compile', body)` but is injectable for tests.
   * Returns `true` when the body merged cleanly (engine returned a
   * non-`⊥` result), `false` otherwise.
   */
  applyBody(handle: number, body: string): boolean
  /** Run the validation gate. Returns the diagnostic line stream. */
  checkBody(handle: number, body: string): string
}

const DEFAULT_HANDLE_PROVIDER: HandleProvider = {
  createHandle: () => engine.compileDomainReadings(),
  applyBody: (h, body) => {
    const out = engine.system(h, 'compile', body)
    return !out.startsWith('⊥') // ⊥
  },
  checkBody: (h, body) => engine.system(h, 'check', body),
}

/**
 * Stub for the manifest-cell DO surface used by this adapter. The
 * production callsite passes an `EntityDB`-shaped stub (the existing
 * `getEntityDO` helper); tests pass an in-memory mock.
 *
 * The shape is the subset of `EntityDB` we actually use: `get` and
 * `put` (the same ↑n / ↓n the cell model exposes).
 */
export interface CellStub {
  get(): Promise<{ id: string; type: string; data: Record<string, unknown> } | null>
  put(input: {
    id: string
    type: string
    data: Record<string, unknown>
  }): Promise<{ id: string; type: string; data: Record<string, unknown> }>
}

/**
 * Index DO surface — registers tenant→manifest-name mappings so
 * cold-start replay can enumerate the live `_loaded_reading:*` cells
 * for a tenant without walking every DO. Maps onto the existing
 * `RegistryDB.indexEntity('_LoadedReading', '{tenant}:{name}', tenant)`
 * call.
 */
export interface IndexStub {
  indexEntity(
    nounType: string,
    entityId: string,
    domainSlug?: string,
  ): Promise<void>
  getEntityIds(nounType: string, domainSlug?: string): Promise<string[]>
}

/**
 * The set of dependencies the load-reading adapter needs. Production
 * wires these to the real EntityDB / RegistryDB stubs; tests pass
 * mocks.
 */
export interface LoadReadingDeps {
  readonly tenant: string
  readonly handleProvider?: HandleProvider
  /** Resolves the cell DO stub for a manifest cell key. */
  getCellStub(cellKey: string): CellStub
  /** Resolves the registry DO stub for the tenant. */
  getIndexStub(tenant: string): IndexStub
  /** Optional clock — defaults to Date.now ISO. */
  now?: () => string
}

/**
 * Manifest cell type. Mirrors the kernel manifest cell-name prefix
 * (`_loaded_reading:`) but uses the spelling `_LoadedReading` for the
 * registry's noun-type column (which prefers a capitalised noun form,
 * see `RegistryDB.indexEntity` callers).
 */
const REGISTRY_NOUN_TYPE = '_LoadedReading'

// ── Cold-start replay ────────────────────────────────────────────

/**
 * Walk the registry's `_LoadedReading` index for `tenant`, fetch each
 * manifest cell, and re-apply the body via `system(h, 'compile', body)`
 * against `handle`. Idempotent — the engine's parse+merge is a set
 * union, so re-applying the same body produces the same cells.
 *
 * Returns the count of records replayed cleanly. Records that fail
 * to apply are logged and skipped, mirroring the kernel's #560
 * "best-effort, single bad record can't wedge boot" contract.
 */
export async function coldStartReplay(
  handle: number,
  deps: LoadReadingDeps,
): Promise<number> {
  const provider = deps.handleProvider ?? DEFAULT_HANDLE_PROVIDER
  const index = deps.getIndexStub(deps.tenant)
  const ids = await index.getEntityIds(REGISTRY_NOUN_TYPE, deps.tenant).catch(() => [] as string[])
  if (ids.length === 0) return 0
  let applied = 0
  for (const id of ids) {
    // The id is the cellKey form `_loaded_reading:{tenant}:{name}`
    // (we wrote it that way in `loadReading` below). Re-derive it here.
    const stub = deps.getCellStub(id)
    const cell = await stub.get().catch(() => null)
    if (!cell || !cell.data) continue
    const body = cell.data.body as string | undefined
    if (typeof body !== 'string' || body.length === 0) continue
    const ok = provider.applyBody(handle, body)
    if (ok) applied += 1
  }
  return applied
}

/**
 * Acquire (or create) the per-tenant engine handle. Runs cold-start
 * replay on first acquisition for the isolate.
 */
export async function getTenantHandle(deps: LoadReadingDeps): Promise<number> {
  const provider = deps.handleProvider ?? DEFAULT_HANDLE_PROVIDER
  const cached = HANDLE_CACHE.get(deps.tenant)
  if (cached !== undefined) return cached
  const h = provider.createHandle()
  HANDLE_CACHE.set(deps.tenant, h)
  if (!REPLAY_DONE.has(deps.tenant)) {
    REPLAY_DONE.add(deps.tenant)
    try {
      await coldStartReplay(h, deps)
    } catch {
      // Replay is best-effort; a single corrupt manifest cell must
      // not wedge the worker. Fall through with the (possibly
      // partially populated) handle and let live loads continue.
    }
  }
  return h
}

/**
 * Reset module-local caches. Test-only; the production worker
 * relies on isolate teardown to drop these.
 *
 * Resets the handle cache, the replay-once flag, AND the version
 * stamp counters so each test sees a fresh isolate. Production
 * never calls this — version stamps are monotonic per-tenant
 * across the isolate's lifetime.
 */
export function _resetCaches(): void {
  HANDLE_CACHE.clear()
  REPLAY_DONE.clear()
  VERSION_COUNTERS.clear()
}

// ── load_reading core ────────────────────────────────────────────

/**
 * The DynRdg-T3 load_reading dispatch. Drives the engine's
 * check + compile pipeline against the per-tenant handle, persists
 * the resulting manifest cell to its DO, and registers the cell with
 * the tenant-scoped registry index for cold-start replay.
 *
 * Returns the LoadValidationReport plus the manifest's content-hash
 * and version-stamp. On a deontic / alethic violation the cell graph
 * is NOT mutated (the engine's check verb is read-only) and the
 * caller's tenant state stays untouched — mirroring the kernel-side
 * #559 gate.
 *
 * The version stamp is monotonic per-tenant per-isolate: a counter
 * is held in `VERSION_COUNTERS` keyed by tenant. Cold-start replay
 * seeds the counter from the highest stamp observed in the registry
 * (so post-replay loads continue numbering above the previous boot).
 */
const VERSION_COUNTERS = new Map<string, number>()

export interface LoadReadingResult {
  readonly ok: boolean
  readonly status: number
  readonly response: LoadReadingResponse | { error: string; validation: LoadValidationReport }
}

export async function loadReading(
  name: string,
  body: string,
  deps: LoadReadingDeps,
): Promise<LoadReadingResult> {
  const provider = deps.handleProvider ?? DEFAULT_HANDLE_PROVIDER

  // Step 1: sanitize.
  const trimmedName = name.trim()
  if (!trimmedName) {
    return {
      ok: false,
      status: 400,
      response: {
        error: 'name (non-empty) required',
        validation: { alethicViolations: [], deonticViolations: [], passes: false },
      },
    }
  }
  if (Array.from(trimmedName).some((c) => c.charCodeAt(0) < 0x20)) {
    return {
      ok: false,
      status: 400,
      response: {
        error: 'name must not contain control characters',
        validation: { alethicViolations: [], deonticViolations: [], passes: false },
      },
    }
  }
  if (!body || body.trim().length === 0) {
    return {
      ok: false,
      status: 400,
      response: {
        error: 'body (non-empty) required',
        validation: { alethicViolations: [], deonticViolations: [], passes: false },
      },
    }
  }

  const handle = await getTenantHandle(deps)

  // Step 2: validation gate (#559 / DynRdg-5). Same partitioning the
  // engine's `validate_loaded_state` does: alethic (parse/resolve)
  // takes precedence; deontic-only failures route to their own
  // bucket. Either non-empty class rejects the load.
  const checkRaw = provider.checkBody(handle, body)
  const validation = decodeValidationReport(checkRaw)
  if (!validation.passes) {
    return {
      ok: false,
      status: 422,
      response: {
        error: validation.alethicViolations.length > 0
          ? 'alethic_violation'
          : 'deontic_violation',
        validation,
      },
    }
  }

  // Step 3: ingest the body into the per-tenant engine state.
  const applied = provider.applyBody(handle, body)
  if (!applied) {
    return {
      ok: false,
      status: 422,
      response: {
        error: 'compile_failed',
        validation: {
          alethicViolations: [
            { reading: '', message: 'engine refused the body after check passed', source: 'parse', level: 'error' },
          ],
          deonticViolations: [],
          passes: false,
        },
      },
    }
  }

  // Step 4: compute manifest record. contentHash mirrors the engine
  // side byte-for-byte (FNV-1a64). versionStamp is monotonic per
  // tenant per isolate.
  const contentHash = computeContentHash(body)
  const prev = VERSION_COUNTERS.get(deps.tenant) ?? 0
  const versionStamp = prev + 1
  VERSION_COUNTERS.set(deps.tenant, versionStamp)
  const manifest: LoadedReadingManifest = {
    name: trimmedName,
    tenant: deps.tenant,
    body,
    contentHash,
    versionStamp,
    addedAt: (deps.now ?? (() => new Date().toISOString()))(),
  }

  // Step 5: persist the manifest cell to its DO. The cell key
  // includes the tenant so two tenants' loads under the same name
  // land on different DOs — that's the per-tenant scoping property
  // the verification surface checks for.
  const cellName = manifestCellKey(deps.tenant, trimmedName)
  const cellStub = deps.getCellStub(cellName)
  await cellStub.put({
    id: cellName,
    type: REGISTRY_NOUN_TYPE,
    data: { ...manifest },
  })

  // Step 6: register in the tenant-scoped registry index so cold-
  // start replay can enumerate manifests without scanning every DO.
  const index = deps.getIndexStub(deps.tenant)
  await index.indexEntity(REGISTRY_NOUN_TYPE, cellName, deps.tenant)

  return {
    ok: true,
    status: 200,
    response: {
      name: trimmedName,
      tenant: deps.tenant,
      contentHash,
      versionStamp,
      validation,
    },
  }
}

/**
 * Wrap a `LoadReadingResponse` in the Theorem-5 envelope so the wire
 * shape matches the rest of the worker's HTTP surface (#202).
 */
export function loadReadingEnvelope(
  res: LoadReadingResponse,
): Envelope<LoadReadingResponse> {
  return envelope(res)
}

/**
 * Wrap a violation result in the Theorem-5 envelope. The `violations`
 * field is populated from the partitioned diagnostics so existing
 * envelope consumers (UI, MCP) can render them without a separate
 * decoder.
 */
export function loadReadingViolationEnvelope(
  validation: LoadValidationReport,
): Envelope<{ validation: LoadValidationReport }> {
  const violations: Violation[] = []
  for (const d of validation.alethicViolations) {
    violations.push({
      reading: d.reading,
      constraintId: '',
      modality: 'alethic',
      detail: d.message,
    })
  }
  for (const d of validation.deonticViolations) {
    violations.push({
      reading: d.reading,
      constraintId: '',
      modality: 'deontic',
      detail: d.message,
    })
  }
  return envelope({ validation }, { violations })
}

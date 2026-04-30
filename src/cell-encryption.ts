/**
 * Cell-level encryption for the Worker DO storage path (#660).
 *
 * Routes through the same `crate::cell_aead` ChaCha20-Poly1305
 * primitive the kernel + freeze paths already use. The previous
 * AES-256-GCM Web Crypto path (#659 part 4) is gone — single AEAD
 * format, byte-for-byte identical sealed envelopes between kernel,
 * worker, and any future tier (FPGA, mobile, host CLI).
 *
 * ## Backend
 *
 * `cell_seal_wasm` / `cell_open_wasm` exported from
 * `crates/arest/pkg/arest.js` (built by `yarn build:wasm`). The
 * shims live in `crates/arest/src/cloudflare.rs` (gated behind the
 * `cloudflare` feature, same as the rest of the worker WASM
 * surface). Entropy for the per-seal nonce is supplied by
 * `cloudflare_entropy::install_worker_entropy` — installed once at
 * module load via `__wasm_init`.
 *
 * ## Wire format
 *
 *   sealed = [12-byte nonce | ciphertext | 16-byte Poly1305 tag]
 *   AAD    = canonicalAddressBytes(cell_address)  (length-prefixed
 *            encoding of scope, domain, cell_name, version — same
 *            bytes as Rust's `CellAddress::canonical_bytes()`)
 *   key    = HKDF-SHA256(master, salt = AAD,
 *                        info = "arest-cell-key/v1")[..32]
 *
 * See `crates/arest/src/cell_aead.rs` module-level comment for the
 * cross-tier contract. The AAD shape and HKDF info string MUST stay
 * in sync with that comment — this module does not own the format.
 *
 * ## Key strategy
 *
 *   master    = HKDF-SHA256 over (TENANT_MASTER_SEED, tenant_id_salt)
 *               (matches `TenantMasterKey::derive` byte-for-byte —
 *                same `arest-tenant-master/v1` info label)
 *
 * The TENANT_MASTER_SEED is a Worker secret bound at deploy time
 * (`wrangler secret put TENANT_MASTER_SEED`). The `tenant_id_salt`
 * is the tenant-scoped string the EntityDB uses today as its DO
 * routing key.
 */

import { cell_seal_wasm, cell_open_wasm } from '../crates/arest/pkg/arest.js'

// ── Types ──────────────────────────────────────────────────────────

/** Canonical four-tuple — exact mirror of the Rust `CellAddress`. */
export interface CellAddress {
  scope: string
  domain: string
  cellName: string
  version: bigint | number
}

/** A per-tenant key wrapper. The raw 32 bytes never leave this
 *  module's call site — `derive` returns one and the seal/open helpers
 *  consume it; the consumer has no business hashing it themselves. */
export interface TenantMasterKey {
  readonly _bytes: Uint8Array
}

export class CellAeadError extends Error {
  readonly kind: 'truncated' | 'auth'
  constructor(kind: 'truncated' | 'auth', message: string) {
    super(message)
    this.name = 'CellAeadError'
    this.kind = kind
  }
}

// ── Constants ──────────────────────────────────────────────────────

/** ChaCha20-Poly1305 nonce length in bytes. RFC 8439 IETF variant. */
export const NONCE_LEN = 12

/** Poly1305 authentication tag length in bytes. */
export const TAG_LEN = 16

/** Per-tenant root key width in bytes (= ChaCha20-Poly1305 key). */
export const CELL_KEY_LEN = 32

// HKDF info labels — kept in sync with cell_aead.rs constants. The
// per-cell HKDF/AEAD parameters are computed inside the WASM module;
// only the master-derivation label needs a TS-side handle.
const HKDF_INFO_MASTER = new TextEncoder().encode('arest-tenant-master/v1')

// ── Master key derivation ──────────────────────────────────────────

/**
 * Derive a tenant master from a seed (the `TENANT_MASTER_SEED`
 * Worker secret) plus a tenant-scoped salt (typically the
 * `tenantId` string already used as DO routing key).
 *
 * HKDF-SHA256 with the seed as input keying material and the salt
 * as the salt parameter — matches the Rust-side
 * `TenantMasterKey::derive` exactly so the byte sequences agree
 * if the same (seed, salt) pair is fed in. Implemented via Web
 * Crypto's HKDF (which Workers expose) rather than the WASM module
 * because the master derivation is a pure function the worker
 * already paid for in #659; only the per-cell AEAD primitive needed
 * to migrate to ChaCha20-Poly1305 for cross-tier compat.
 */
export async function deriveTenantMasterKey(
  seed: Uint8Array | string,
  salt: Uint8Array | string,
): Promise<TenantMasterKey> {
  const seedBytes = typeof seed === 'string' ? new TextEncoder().encode(seed) : seed
  const saltBytes = typeof salt === 'string' ? new TextEncoder().encode(salt) : salt
  const ikmKey = await crypto.subtle.importKey('raw', seedBytes, 'HKDF', false, ['deriveBits'])
  const masterBits = await crypto.subtle.deriveBits(
    { name: 'HKDF', hash: 'SHA-256', salt: saltBytes, info: HKDF_INFO_MASTER },
    ikmKey,
    CELL_KEY_LEN * 8,
  )
  return { _bytes: new Uint8Array(masterBits) }
}

// ── Cell address canonical encoding ────────────────────────────────

/**
 * Length-prefixed canonical encoding of a cell address. Mirrors the
 * Rust `CellAddress::canonical_bytes` byte-for-byte:
 *
 *     [u32 LE scope_len  | scope bytes]
 *     [u32 LE domain_len | domain bytes]
 *     [u32 LE name_len   | name bytes]
 *     [u64 LE version]
 *
 * Length prefixes prevent boundary-collision: (scope = "ab",
 * domain = "c") cannot collide with (scope = "a", domain = "bc").
 *
 * The bytes are passed through to `cell_seal_wasm` /
 * `cell_open_wasm` as-is; the Rust shim deserialises them back into
 * a `CellAddress` to feed `cell_aead::cell_seal` / `cell_open`. A
 * cross-tier round-trip test (Rust seals → TS opens, TS seals →
 * Rust opens) pins this format in CI.
 */
export function canonicalAddressBytes(address: CellAddress): Uint8Array {
  const enc = new TextEncoder()
  const scope = enc.encode(address.scope)
  const domain = enc.encode(address.domain)
  const name = enc.encode(address.cellName)
  const out = new Uint8Array(4 + scope.length + 4 + domain.length + 4 + name.length + 8)
  const view = new DataView(out.buffer)
  let off = 0
  view.setUint32(off, scope.length, true); off += 4
  out.set(scope, off); off += scope.length
  view.setUint32(off, domain.length, true); off += 4
  out.set(domain, off); off += domain.length
  view.setUint32(off, name.length, true); off += 4
  out.set(name, off); off += name.length
  // u64 little-endian — split into low/high u32 because DataView lacks
  // a portable BigInt setter on some Worker engines.
  const v = BigInt(address.version)
  view.setUint32(off, Number(v & 0xffffffffn), true); off += 4
  view.setUint32(off, Number((v >> 32n) & 0xffffffffn), true); off += 4
  return out
}

// ── Public AEAD API ────────────────────────────────────────────────

/**
 * Seal `plaintext` for the named cell. Returns
 * `[NONCE_LEN-byte nonce | ciphertext | TAG_LEN-byte tag]`. Routes
 * through `cell_seal_wasm` — the engine's own ChaCha20-Poly1305
 * primitive, with the nonce drawn from the WASM-side csprng (seeded
 * by the worker entropy adapter installed at `__wasm_init`).
 *
 * Public signature is async to match the previous Web Crypto path;
 * the underlying WASM call is synchronous, so the Promise resolves
 * on the next microtask. Keeping the API async prevents call-site
 * churn in `entity-do.ts` and any other consumer that already
 * `await`-ed the AES-GCM helper.
 */
export async function cellSeal(
  master: TenantMasterKey,
  address: CellAddress,
  plaintext: Uint8Array | string,
): Promise<Uint8Array> {
  const aad = canonicalAddressBytes(address)
  const pt = typeof plaintext === 'string' ? new TextEncoder().encode(plaintext) : plaintext
  return cell_seal_wasm(master._bytes, aad, pt)
}

/**
 * Open a sealed envelope at the named cell address. Returns the
 * recovered plaintext on success; throws `CellAeadError` with
 * `kind = 'truncated'` if the envelope is shorter than 28 bytes,
 * or `kind = 'auth'` if AEAD tag verification fails (wrong master /
 * wrong AAD / tampered ciphertext).
 *
 * The WASM shim raises a `JsError` for both failure modes; this
 * wrapper inspects the message to map back to the typed `kind` so
 * callers can switch on it (the structural-bad-envelope log shape
 * differs from the auth-failure shape in the storage path).
 */
export async function cellOpen(
  master: TenantMasterKey,
  address: CellAddress,
  sealed: Uint8Array,
): Promise<Uint8Array> {
  if (sealed.length < NONCE_LEN + TAG_LEN) {
    throw new CellAeadError('truncated', 'sealed envelope shorter than NONCE_LEN + TAG_LEN')
  }
  const aad = canonicalAddressBytes(address)
  try {
    return cell_open_wasm(master._bytes, aad, sealed)
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err)
    if (msg.includes('truncated')) {
      throw new CellAeadError('truncated', msg)
    }
    throw new CellAeadError('auth', msg || 'AEAD authentication failed')
  }
}

// ── Tenant master rotation (#662) ──────────────────────────────────
//
// Operator workflow: a deployment switches from master A → master B
// without losing access to existing cells. Mirror of the Rust-side
// `cell_aead::rotate_cell` / `rotate_tenant` contract.
//
// ## Read-only window assumption
//
// First-version rotation is non-zero-downtime. The CALLER MUST hold
// a per-tenant write lock for the duration of the walk:
//
//   - Each individual EntityDB DO is single-writer by Cloudflare's
//     design (one DO instance handles requests serially).
//   - The rotation walk against a tenant's set of DOs is NOT
//     single-DO — concurrent writes against different EntityDBs in
//     the same tenant would otherwise interleave with rotation.
//   - The orchestrator (`worker.ts` rotation handler / RegistryDB
//     rotation method) holds a per-tenant semaphore at the
//     dispatcher seam, so the rotation walk and any other tenant
//     write are mutually exclusive.
//
// ## Operator workflow (worker)
//
//   1. `wrangler secret put TENANT_MASTER_SEED_v2 <new>` — the
//      existing v1 slot stays bound during the rotation window.
//   2. Acquire the per-tenant write semaphore at the orchestrator.
//   3. Enumerate the tenant's EntityDB cells (via the per-tenant
//      scoping from #205); for each cell call `rotateCell` to
//      produce the new sealed bytes; write them back atomically.
//   4. When the report has zero failures (or operator accepts the
//      reported losses): `wrangler secret put TENANT_MASTER_SEED <new>`,
//      `wrangler secret delete TENANT_MASTER_SEED_v2`, deploy.
//   5. Release the per-tenant write semaphore.

/** A single cell that failed to rotate. The corresponding storage
 *  row is left untouched — operator decides whether to retry,
 *  zeroize, or accept the loss. */
export interface RotationFailure {
  address: CellAddress
  /** Mirrors the Rust `AeadError`: `'truncated'` for a structurally
   *  malformed envelope (storage row shorter than 28 bytes), `'auth'`
   *  for any other open failure (wrong master / tampered ciphertext /
   *  re-target). */
  kind: 'truncated' | 'auth'
}

/** Outcome of `rotateTenant`. Caller atomic-swaps each `rotated`
 *  entry into storage; `failures` is reported back without aborting
 *  the rest of the walk. */
export interface RotationReport {
  rotated: Array<{ address: CellAddress; sealed: Uint8Array }>
  failures: RotationFailure[]
}

/** Single-cell rotation primitive. Opens `sealed` under `oldMaster`,
 *  re-seals the recovered plaintext under `newMaster` at the same
 *  `address` (fresh nonce).
 *
 *  Throws `CellAeadError` with `kind = 'truncated' | 'auth'` if the
 *  open step fails — the storage row is then left untouched and the
 *  caller decides what to do. The seal step never throws on finite
 *  inputs.
 */
export async function rotateCell(
  oldMaster: TenantMasterKey,
  newMaster: TenantMasterKey,
  address: CellAddress,
  sealed: Uint8Array,
): Promise<Uint8Array> {
  const plaintext = await cellOpen(oldMaster, address, sealed)
  return cellSeal(newMaster, address, plaintext)
}

/** Multi-cell rotation walk. Iterates the supplied `(address, sealed)`
 *  pairs; for each one calls `rotateCell`. Successes go into
 *  `report.rotated`; per-cell failures go into `report.failures`
 *  without aborting the rest of the walk.
 *
 *  The caller MUST hold the per-tenant write lock for the duration
 *  of this call (see read-only window assumption above). The walk
 *  itself does no I/O — it consumes an iterable of sealed bytes and
 *  produces re-sealed bytes; the caller is responsible for reading
 *  the input from storage and atomic-swapping the output back.
 */
export async function rotateTenant(
  oldMaster: TenantMasterKey,
  newMaster: TenantMasterKey,
  cells: Iterable<{ address: CellAddress; sealed: Uint8Array }>,
): Promise<RotationReport> {
  const rotated: Array<{ address: CellAddress; sealed: Uint8Array }> = []
  const failures: RotationFailure[] = []
  for (const { address, sealed } of cells) {
    try {
      const newSealed = await rotateCell(oldMaster, newMaster, address, sealed)
      rotated.push({ address, sealed: newSealed })
    } catch (e) {
      if (e instanceof CellAeadError) {
        failures.push({ address, kind: e.kind })
      } else {
        // Unknown error — surface as auth (most likely an underlying
        // OperationError that escaped the typed wrapper). Re-throwing
        // would abort the whole walk; per spec the walk continues.
        failures.push({ address, kind: 'auth' })
      }
    }
  }
  return { rotated, failures }
}

// ── Convenience: JSON cell helpers ─────────────────────────────────
//
// EntityDB stores cell.data as a JSON string today; the storage path
// can stay JSON-shaped if the encryption layer round-trips
// `(string ↔ ciphertext base64)` on write/read. The base64 encode
// keeps the SQLite TEXT column happy without a schema migration.

/**
 * Seal a JSON-serialisable value into base64 ciphertext. The
 * returned string is what the DO writes to the SQLite TEXT column.
 */
export async function sealJson(
  master: TenantMasterKey,
  address: CellAddress,
  value: unknown,
): Promise<string> {
  const json = JSON.stringify(value)
  const sealed = await cellSeal(master, address, json)
  return bytesToBase64(sealed)
}

/**
 * Open a base64 ciphertext column back into the JSON value.
 * Returns `null` if the column is empty (a freshly-created cell
 * with no data committed yet); throws `CellAeadError` for any
 * AEAD failure so callers can distinguish "no data" from "data
 * present but unreadable".
 */
export async function openJson(
  master: TenantMasterKey,
  address: CellAddress,
  blob: string | null | undefined,
): Promise<unknown> {
  if (blob == null || blob === '' || blob === '{}') return blob === '{}' ? {} : null
  const sealed = base64ToBytes(blob)
  const plain = await cellOpen(master, address, sealed)
  const text = new TextDecoder().decode(plain)
  return JSON.parse(text)
}

// ── Base64 helpers (Workers don't expose Buffer) ───────────────────

function bytesToBase64(bytes: Uint8Array): string {
  // btoa accepts a binary string — chunked because btoa chokes on
  // very long argument strings in some engines.
  let binary = ''
  const CHUNK = 0x8000
  for (let i = 0; i < bytes.length; i += CHUNK) {
    binary += String.fromCharCode(...bytes.subarray(i, Math.min(i + CHUNK, bytes.length)))
  }
  return btoa(binary)
}

function base64ToBytes(b64: string): Uint8Array {
  const binary = atob(b64)
  const out = new Uint8Array(binary.length)
  for (let i = 0; i < binary.length; i++) out[i] = binary.charCodeAt(i)
  return out
}

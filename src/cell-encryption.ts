/**
 * Cell-level encryption for the Worker DO storage path (#659).
 *
 * Mirrors the Rust-side `arest::cell_aead` contract: every cell value
 * crossing the DO boundary (encrypt-on-put, decrypt-on-get) is sealed
 * against a per-tenant master key, with HKDF-SHA256 derivation keyed
 * on the cell address (scope, domain, cell_name, version).
 *
 * ## Algorithm choice
 *
 * Rust uses ChaCha20-Poly1305. Cloudflare Workers' Web Crypto does
 * NOT expose ChaCha20 — only AES-GCM, AES-CTR, AES-CBC, AES-KW. This
 * Worker-side adapter therefore uses AES-256-GCM with HKDF-SHA256
 * derivation, both of which are natively supported by `crypto.subtle`.
 *
 * Cross-tier format compat (Rust kernel ⇄ TS Worker) is a follow-up
 * once the engine's `cell_seal` / `cell_open` get wasm-bindgen
 * exports — this scope ships per-tier sealing only, which still
 * satisfies the #659 "ciphertext at every serialization boundary"
 * goal: a leaked DO-stored row decrypts only with the tenant master
 * the DO was sealed under, regardless of which side did the seal.
 *
 * ## Envelope
 *
 *   sealed = [12-byte IV | ciphertext | 16-byte tag]
 *
 * IV is drawn from `crypto.getRandomValues` (the same Web Crypto seam
 * the engine's csprng adapter sits on, #572). AAD = canonical bytes
 * of the cell address — same length-prefixed format as the Rust side
 * so a future port-over swap of AES-GCM → ChaCha20-Poly1305 doesn't
 * change the AAD shape.
 *
 * ## Key strategy
 *
 *   master    = HKDF-SHA256 over (TENANT_MASTER_SEED, tenant_id_salt)
 *   cell key  = HKDF-SHA256 over (master, address_canonical_bytes,
 *                                  info = "arest-cell-key/v1")[..32]
 *
 * The TENANT_MASTER_SEED is a Worker secret bound at deploy time
 * (`wrangler secret put TENANT_MASTER_SEED`). The `tenant_id_salt`
 * is the tenant-scoped string the EntityDB uses today as its DO
 * routing key.
 */

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

/** AES-GCM IV / nonce length in bytes. Web Crypto rejects shorter
 *  IVs in 256-bit mode; 12 is the documented length for AEAD use. */
export const NONCE_LEN = 12

/** AES-GCM authentication tag length in bytes. Web Crypto's default
 *  is 128 bits; we pin it explicitly so a future encrypt-side change
 *  doesn't silently shrink the tag. */
export const TAG_LEN = 16

const HKDF_INFO_MASTER = new TextEncoder().encode('arest-tenant-master/v1')
const HKDF_INFO_CELL = new TextEncoder().encode('arest-cell-key/v1')
const CELL_KEY_BITS = 256

// ── Master key derivation ──────────────────────────────────────────

/**
 * Derive a tenant master from a seed (the `TENANT_MASTER_SEED`
 * Worker secret) plus a tenant-scoped salt (typically the
 * `tenantId` string already used as DO routing key).
 *
 * HKDF-SHA256 with the seed as input keying material and the salt
 * as the salt parameter — matches the Rust-side
 * `TenantMasterKey::derive` exactly so the byte sequences agree
 * if the same (seed, salt) pair is fed in.
 */
export async function deriveTenantMasterKey(
  seed: Uint8Array | string,
  salt: Uint8Array | string,
): Promise<TenantMasterKey> {
  const seedBytes = typeof seed === 'string' ? new TextEncoder().encode(seed) : seed
  const saltBytes = typeof salt === 'string' ? new TextEncoder().encode(salt) : salt
  // Web Crypto: HKDF derive 32 bytes off the seed.
  const ikmKey = await crypto.subtle.importKey('raw', seedBytes, 'HKDF', false, ['deriveBits'])
  const masterBits = await crypto.subtle.deriveBits(
    { name: 'HKDF', hash: 'SHA-256', salt: saltBytes, info: HKDF_INFO_MASTER },
    ikmKey,
    256, // bits — 32 bytes
  )
  return { _bytes: new Uint8Array(masterBits) }
}

// ── Cell-key derivation ────────────────────────────────────────────

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

/**
 * Derive the per-cell AES-256-GCM key from the master + the address.
 * Returns a CryptoKey already imported for AES-GCM use; callers
 * should treat the underlying bytes as opaque.
 */
async function deriveCellKey(master: TenantMasterKey, address: CellAddress): Promise<CryptoKey> {
  const salt = canonicalAddressBytes(address)
  const masterKeyHandle = await crypto.subtle.importKey(
    'raw',
    master._bytes,
    'HKDF',
    false,
    ['deriveBits'],
  )
  const cellKeyBits = await crypto.subtle.deriveBits(
    { name: 'HKDF', hash: 'SHA-256', salt, info: HKDF_INFO_CELL },
    masterKeyHandle,
    CELL_KEY_BITS,
  )
  return crypto.subtle.importKey(
    'raw',
    cellKeyBits,
    { name: 'AES-GCM', length: CELL_KEY_BITS },
    false,
    ['encrypt', 'decrypt'],
  )
}

// ── Public AEAD API ────────────────────────────────────────────────

/**
 * Seal `plaintext` for the named cell. Returns a Uint8Array of
 * `[NONCE_LEN-byte IV | ciphertext | TAG_LEN-byte tag]`. The
 * ciphertext+tag region is what `crypto.subtle.encrypt` returns
 * — Web Crypto already concatenates the tag onto the ciphertext.
 *
 * The IV is drawn from `crypto.getRandomValues`, which on Workers
 * is the same Web Crypto entropy seam #572 wires the engine's
 * csprng adapter to. Per-seal nonce uniqueness is statistical;
 * with 96 bits of nonce + a per-cell key, collision probability
 * remains under 2^{-48} after 2^{48} seals for the same cell.
 */
export async function cellSeal(
  master: TenantMasterKey,
  address: CellAddress,
  plaintext: Uint8Array | string,
): Promise<Uint8Array> {
  const cellKey = await deriveCellKey(master, address)
  const iv = new Uint8Array(NONCE_LEN)
  crypto.getRandomValues(iv)
  const aad = canonicalAddressBytes(address)
  const pt = typeof plaintext === 'string' ? new TextEncoder().encode(plaintext) : plaintext
  const ctBuf = await crypto.subtle.encrypt(
    { name: 'AES-GCM', iv, additionalData: aad, tagLength: TAG_LEN * 8 },
    cellKey,
    pt,
  )
  const ct = new Uint8Array(ctBuf)
  const out = new Uint8Array(NONCE_LEN + ct.length)
  out.set(iv, 0)
  out.set(ct, NONCE_LEN)
  return out
}

/**
 * Open a sealed envelope at the named cell address. Returns the
 * recovered plaintext on success; throws `CellAeadError` with
 * `kind = 'truncated'` if the envelope is shorter than 28 bytes,
 * or `kind = 'auth'` if Web Crypto's GCM tag verification fails
 * (wrong master / wrong AAD / tampered ciphertext).
 */
export async function cellOpen(
  master: TenantMasterKey,
  address: CellAddress,
  sealed: Uint8Array,
): Promise<Uint8Array> {
  if (sealed.length < NONCE_LEN + TAG_LEN) {
    throw new CellAeadError('truncated', 'sealed envelope shorter than NONCE_LEN + TAG_LEN')
  }
  const iv = sealed.slice(0, NONCE_LEN)
  const ct = sealed.slice(NONCE_LEN)
  const cellKey = await deriveCellKey(master, address)
  const aad = canonicalAddressBytes(address)
  try {
    const pt = await crypto.subtle.decrypt(
      { name: 'AES-GCM', iv, additionalData: aad, tagLength: TAG_LEN * 8 },
      cellKey,
      ct,
    )
    return new Uint8Array(pt)
  } catch {
    // Web Crypto raises a generic OperationError on tag mismatch;
    // surface it as our typed Auth error so callers can switch on
    // kind and log differently from a structurally-bad envelope.
    throw new CellAeadError('auth', 'AEAD authentication failed')
  }
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

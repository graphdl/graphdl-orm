// Platform adapter for Cloudflare Workers.
// CF Workers do not support WASM Component Model.
// SYSTEM is the only function. create() bootstraps D with compile ∘ parse.

use wasm_bindgen::prelude::*;

/// Install console_error_panic_hook so Rust panics across the WASM
/// boundary surface as console.error messages instead of opaque
/// `unreachable` traps. Also installs the Cloudflare Worker
/// `EntropySource` adapter (#572 / Rand-T4) into the engine's global
/// slot — every Worker isolate exposes Web Crypto via
/// `crypto.getRandomValues`, and the adapter wraps `getrandom`'s `js`
/// backend (Cargo.toml gates `getrandom = { features = ["js"] }` for
/// wasm32) behind the engine's `EntropySource` trait.
///
/// Order matters: the entropy install MUST run before any route
/// handler can call `csprng::random_bytes` (its lazy-seed path panics
/// with "no entropy source installed" otherwise). The
/// `#[wasm_bindgen(start)]` attribute guarantees this body runs once
/// per WASM module instantiation, before any exported function (`fetch`
/// / `system` / `parse_and_compile` / etc.) is reachable from JS.
///
/// `entropy::install` REPLACES the previously installed source
/// (`entropy.rs:116`); production code must avoid double-installation
/// — running here is the exception that proves the rule (we are in
/// fact "boot"), and this body fires exactly once per instance so the
/// concern is moot.
#[wasm_bindgen(start)]
pub fn __wasm_init() {
    console_error_panic_hook::set_once();
    crate::cloudflare_entropy::install_worker_entropy();
}

/// Allocate D with the bundled metamodel and platform primitives loaded.
/// Produces a fully self-describing engine ready for user domain readings.
#[wasm_bindgen]
pub fn create() -> u32 { crate::create_impl() }

/// Allocate an empty D with ONLY platform primitives registered in DEFS.
/// Use this when testing a new core or rebuilding the metamodel from scratch.
/// Most apps should use `create` instead.
#[wasm_bindgen]
pub fn create_bare() -> u32 { crate::create_bare_impl() }

/// SYSTEM:x = ⟨o, D'⟩. The only function.
/// Ingesting readings: system(handle, "compile", readings_text)
/// All other operations: system(handle, key, input)
#[wasm_bindgen]
pub fn system(handle: u32, key: &str, input: &str) -> String {
    crate::system_impl(handle, key, input)
}

/// Release a compiled domain handle.
#[wasm_bindgen]
pub fn release(handle: u32) { crate::release_impl(handle); }

/// Legacy: parse_and_compile as create + system(h, "compile", readings).
/// Kept for backward compatibility during migration.
#[wasm_bindgen]
pub fn parse_and_compile(readings_json: &str) -> Result<u32, JsError> {
    let readings: Vec<(String, String)> = serde_json::from_str(readings_json)
        .map_err(|e| JsError::new(&e.to_string()))?;
    crate::parse_and_compile_impl(readings).map_err(|e| JsError::new(&e))
}

// ── Cell AEAD shims (#660) ─────────────────────────────────────────
//
// Wasm-bindgen surface over `crate::cell_aead`. Drops the Worker's
// TypeScript AES-256-GCM path and routes the `src/cell-encryption.ts`
// helpers through the *same* ChaCha20-Poly1305 implementation the
// kernel + freeze paths already use. One AEAD format, byte-for-byte
// identical sealed envelopes across every tier.
//
// The shims take raw `&[u8]` for both the master key and the
// pre-serialised cell address (the worker side computes
// `canonical_bytes` in TS — same length-prefixed format as Rust's
// `CellAddress::canonical_bytes` per `cell_aead.rs` docs — and hands
// the bytes through opaque). This avoids round-tripping a struct
// across the WASM boundary just to immediately re-flatten it.
//
// `master_bytes` MUST be exactly 32 bytes; shorter / longer slices
// are rejected with a descriptive `JsError` so the worker side gets
// a typed exception rather than a silent truncation. The address
// bytes are passed through verbatim — same shape as the kernel /
// freeze paths use. The worker's TS helpers carry the only
// canonicalisation logic and are unit-tested for byte-equality
// against Rust's `CellAddress::canonical_bytes` via the cross-tier
// round-trip suite in #660.

/// Seal `plaintext` for the cell at `cell_address_canonical` under the
/// 32-byte tenant master. Returns the sealed envelope as
/// `[12-byte nonce | ciphertext | 16-byte Poly1305 tag]` — the wire
/// format documented in `cell_aead.rs`. Errors only if `master_bytes`
/// is not exactly 32 bytes long.
#[wasm_bindgen]
pub fn cell_seal_wasm(
    master_bytes: &[u8],
    cell_address_canonical: &[u8],
    plaintext: &[u8],
) -> Result<Vec<u8>, JsError> {
    let master = master_from_slice(master_bytes)?;
    let address = decode_canonical_address(cell_address_canonical)?;
    Ok(crate::cell_aead::cell_seal(&master, &address, plaintext))
}

/// Open a sealed envelope at the cell address. Returns the recovered
/// plaintext on success; raises a `JsError` on auth failure (wrong
/// master / wrong address / tampered bytes), truncation (envelope
/// shorter than 28 bytes of overhead), or a malformed master /
/// address slice.
#[wasm_bindgen]
pub fn cell_open_wasm(
    master_bytes: &[u8],
    cell_address_canonical: &[u8],
    sealed: &[u8],
) -> Result<Vec<u8>, JsError> {
    let master = master_from_slice(master_bytes)?;
    let address = decode_canonical_address(cell_address_canonical)?;
    crate::cell_aead::cell_open(&master, &address, sealed).map_err(|e| match e {
        crate::cell_aead::AeadError::Truncated => {
            JsError::new("cell_open_wasm: sealed envelope truncated")
        }
        crate::cell_aead::AeadError::Auth => {
            JsError::new("cell_open_wasm: AEAD authentication failed")
        }
    })
}

/// Wrap a 32-byte slice as a `TenantMasterKey`. Surfaces a typed
/// `JsError` to JS callers if the slice is the wrong width — the
/// engine's underlying constructor takes a `[u8; 32]` and would
/// otherwise be a panic.
fn master_from_slice(bytes: &[u8]) -> Result<crate::cell_aead::TenantMasterKey, JsError> {
    if bytes.len() != crate::cell_aead::CELL_KEY_LEN {
        return Err(JsError::new(
            "cell AEAD master key must be exactly 32 bytes",
        ));
    }
    let mut buf = [0u8; crate::cell_aead::CELL_KEY_LEN];
    buf.copy_from_slice(bytes);
    Ok(crate::cell_aead::TenantMasterKey::from_bytes(buf))
}

/// Decode the length-prefixed canonical address bytes back into a
/// `CellAddress`. The worker side serialises this with the same
/// format as `CellAddress::canonical_bytes` (length prefixes are
/// what defeat boundary-collision attacks), so the round-trip is
/// pure structural unpacking.
///
/// Format (little-endian):
///   [u32 scope_len  | scope_bytes]
///   [u32 domain_len | domain_bytes]
///   [u32 name_len   | name_bytes]
///   [u64 version]
fn decode_canonical_address(bytes: &[u8]) -> Result<crate::cell_aead::CellAddress, JsError> {
    let err = || JsError::new("cell address canonical bytes are malformed");
    let mut cur = 0usize;
    let read_u32 = |buf: &[u8], cur: &mut usize| -> Result<u32, JsError> {
        if *cur + 4 > buf.len() {
            return Err(err());
        }
        let v = u32::from_le_bytes([buf[*cur], buf[*cur + 1], buf[*cur + 2], buf[*cur + 3]]);
        *cur += 4;
        Ok(v)
    };
    let read_str = |buf: &[u8], cur: &mut usize, len: usize| -> Result<String, JsError> {
        if *cur + len > buf.len() {
            return Err(err());
        }
        let s = core::str::from_utf8(&buf[*cur..*cur + len]).map_err(|_| err())?;
        *cur += len;
        Ok(s.to_string())
    };
    let scope_len = read_u32(bytes, &mut cur)? as usize;
    let scope = read_str(bytes, &mut cur, scope_len)?;
    let domain_len = read_u32(bytes, &mut cur)? as usize;
    let domain = read_str(bytes, &mut cur, domain_len)?;
    let name_len = read_u32(bytes, &mut cur)? as usize;
    let cell_name = read_str(bytes, &mut cur, name_len)?;
    if cur + 8 > bytes.len() {
        return Err(err());
    }
    let version = u64::from_le_bytes([
        bytes[cur], bytes[cur + 1], bytes[cur + 2], bytes[cur + 3],
        bytes[cur + 4], bytes[cur + 5], bytes[cur + 6], bytes[cur + 7],
    ]);
    cur += 8;
    if cur != bytes.len() {
        return Err(err());
    }
    Ok(crate::cell_aead::CellAddress::new(
        scope, domain, cell_name, version,
    ))
}

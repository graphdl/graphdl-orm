// crates/arest/src/cell_aead.rs
//
// Per-cell authenticated encryption (#659).
//
// Every AREST cell becomes ciphertext at every serialization boundary
// outside the engine's in-memory operating set. Plaintext exists only
// inside the per-tenant compiled state. The moment a cell crosses a
// DO write, kernel block_storage flush, freeze/thaw bytes, or network
// frame, it's encrypted with a per-cell key derived from a per-tenant
// master via HKDF-SHA256 over the cell address.
//
// ## Why per-cell, not per-tenant
//
// Aligns with the canonical DO-per-cell physical mapping (#217). One
// cell = one ciphertext unit = one HKDF context. A leaked DO doesn't
// decrypt sibling cells without the tenant master. Multi-tenancy
// isolation is a side effect of per-tenant master scoping.
//
// ## Crypto choice — ChaCha20-Poly1305
//
// Composes with the existing `arest::csprng` ChaCha20 RNG (#568) and
// the `arest::crypto` nonce path (#578) without an AES dependency.
// no_std-friendly, no hardware AES required, kernel + worker + WASM
// all share one path.
//
// ## AEAD envelope
//
//   sealed = [12-byte nonce | ciphertext | 16-byte tag]
//   plain  = ciphertext minus the trailing 16-byte Poly1305 tag
//   key    = HKDF-SHA256(master, salt = address_bytes, info = "arest-cell-key")[..32]
//   AAD    = address_bytes  (re-targeted ciphertext at a different
//                            address fails decrypt — replay-safe)
//
// Version (per #558's monotonic counter) is part of the address, so
// an older ciphertext at the same name+scope+domain fails decrypt —
// replay protection for free.
//
// ## no_std contract
//
// Pure no_std. Reaches `chacha20poly1305`, `hkdf`, `sha2` (all with
// default-features off) plus `crate::csprng::random_bytes` for the
// nonce — same csprng singleton every other crypto site funnels
// through (#578). Targets MUST install an entropy source before any
// `cell_seal` call (the panic-on-uninstalled-source contract is
// inherited from csprng).
//
// ## Wire format (cross-tier contract — #660)
//
// Exposed across the wasm-bindgen boundary as `cell_seal_wasm` /
// `cell_open_wasm` (`crates/arest/src/cloudflare.rs`). The Worker's
// TS path (`src/cell-encryption.ts`) and the kernel's sealed
// checkpoint path (`crates/arest-kernel/src/block_storage.rs`) speak
// this format byte-for-byte. Future tiers (FPGA, mobile, host CLI)
// inherit the contract from this comment, not by reverse-engineering
// either implementation.
//
//     sealed envelope = [12-byte nonce][ciphertext = plaintext.len() bytes][16-byte Poly1305 tag]
//     AAD             = CellAddress::canonical_bytes()
//                       (= [u32 LE scope_len | scope]
//                          [u32 LE domain_len | domain]
//                          [u32 LE cell_name_len | cell_name]
//                          [u64 LE version])
//     cell key        = HKDF-SHA256(ikm = master, salt = canonical_bytes,
//                                    info = "arest-cell-key/v1")[..32]
//     AEAD            = ChaCha20-Poly1305 (chacha20poly1305 crate, RFC 8439)
//
// Sealed-envelope overhead is `NONCE_LEN + TAG_LEN` = 28 bytes.
// Tampering with any envelope byte (nonce, ciphertext, or tag), or
// opening under a different address / master, surfaces as
// `AeadError::Auth`; envelopes shorter than 28 bytes surface as
// `AeadError::Truncated`.

#[allow(unused_imports)]
use alloc::{vec, vec::Vec, string::{String, ToString}, format};
use core::fmt;

use chacha20poly1305::{
    aead::{Aead, KeyInit, Payload},
    ChaCha20Poly1305, Key, Nonce,
};
use hkdf::Hkdf;
use sha2::Sha256;

use crate::csprng;

/// Width of the ChaCha20-Poly1305 nonce, in bytes. Pinned by RFC 8439
/// (12-byte / 96-bit IETF variant). Embedded as the first 12 bytes of
/// every sealed envelope.
pub const NONCE_LEN: usize = 12;

/// Width of the Poly1305 authentication tag, in bytes. Appended after
/// the ciphertext. Sealed-envelope overhead is `NONCE_LEN + TAG_LEN`
/// = 28 bytes per cell.
pub const TAG_LEN: usize = 16;

/// Width of the per-cell symmetric key derived from the tenant master.
/// Matches ChaCha20-Poly1305's required key width.
pub const CELL_KEY_LEN: usize = 32;

/// HKDF "info" label that domain-separates cell-key derivation from
/// any other future HKDF use of the same master (event signing,
/// snapshot ids, etc.). Bumping this string rotates every per-cell
/// key without changing the master.
const HKDF_INFO: &[u8] = b"arest-cell-key/v1";

// ── Tenant master key ──────────────────────────────────────────────

/// 32-byte per-tenant root key. Boot paths construct one of these
/// from target-specific entropy + (where applicable) a salt persisted
/// in a freeze blob; every cell key in that tenant's compiled state
/// is then HKDF-derived from this master plus the cell address.
///
/// `[u8; 32]` rather than `Vec<u8>` so the type guarantees the input
/// width to HKDF. The struct is `Clone` so the boot path can hand it
/// to multiple subsystems (freeze, block_storage, DO adapter) without
/// reaching for `Arc`. `Drop` is intentionally NOT custom — call
/// sites are expected to keep this in long-lived globals; an
/// over-eager zeroise would need a separate primitive (out of scope
/// for #659).
#[derive(Clone)]
pub struct TenantMasterKey([u8; CELL_KEY_LEN]);

impl TenantMasterKey {
    /// Wrap exactly 32 bytes as a tenant master. Callers are expected
    /// to source the bytes from a target adapter — `EFI_RNG_PROTOCOL`
    /// + persisted salt for the kernel, a Worker secret + `tenant_id`
    /// for Cloudflare, OS getrandom for the host CLI.
    pub fn from_bytes(bytes: [u8; CELL_KEY_LEN]) -> Self {
        Self(bytes)
    }

    /// Derive a tenant master from arbitrary input keying material plus
    /// a salt, via HKDF-SHA256-extract. Used by target adapters that
    /// don't already have 32 bytes in hand — they pass an opaque
    /// `seed` (Cloudflare Secret bytes, EFI_RNG bootstrap entropy,
    /// `tenant_master.bin` contents) and a salt scoped to the tenant.
    ///
    /// `salt` is required (not `Option`-ish) — passing an empty slice
    /// is allowed by HKDF, but every target in this code path has a
    /// natural salt: the on-disk freeze-blob `tenant_salt` for
    /// kernels, the `tenant_id` string for workers, the
    /// `~/.arest/tenant_salt.bin` file for the host CLI.
    pub fn derive(seed: &[u8], salt: &[u8]) -> Self {
        let hk = Hkdf::<Sha256>::new(Some(salt), seed);
        let mut out = [0u8; CELL_KEY_LEN];
        hk.expand(b"arest-tenant-master/v1", &mut out)
            .expect("HKDF-SHA256 expand of 32 bytes is always within 255 * HashLen");
        Self(out)
    }

    /// Borrow the underlying 32 bytes. Pub(crate) — only the cell-key
    /// derivation path needs this; external callers should not be
    /// hashing the master themselves.
    fn as_bytes(&self) -> &[u8; CELL_KEY_LEN] {
        &self.0
    }
}

impl fmt::Debug for TenantMasterKey {
    /// Hide the bytes from accidental log captures. A printed master
    /// in serial logs / tracing spans would leak every cell in the
    /// tenant; redaction here is the cheapest hard-stop.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("TenantMasterKey(<redacted>)")
    }
}

// ── Process-global tenant master slot (#663) ───────────────────────
//
// Every target installs exactly one tenant master at boot:
//
//   * host CLI:  `~/.arest/tenant_master.bin` (32 random bytes,
//                generated on first run, persisted with mode 0600).
//                See `crate::cli::tenant_master_host`.
//   * kernel:    bootstrap entropy + freeze-blob salt (HKDF-derive).
//   * worker:    Cloudflare Secret + tenant_id (HKDF-derive).
//
// The slot is `spin::Once` (not `RwLock`) because the master is
// install-once-and-read-thereafter — we never need to swap it after
// boot, and `Once::call_once` gives us a one-shot install with no
// runtime locking on the hot read path.
//
// Mirrors the shape of `crate::entropy::GLOBAL_SOURCE` but with
// stricter "install exactly once" semantics: the entropy source can
// be replaced (so tests can swap a `DeterministicSource`); the tenant
// master cannot. Tests that need a different master construct one
// inline via `TenantMasterKey::from_bytes` rather than touching the
// global.

/// Process-wide tenant master, installed once at boot. `None` until
/// `install_tenant_master` runs; returning `None` from
/// `current_tenant_master` after that lets call sites distinguish
/// "the boot path forgot to install" from "decryption failed".
///
/// `Mutex<Option<_>>` rather than `Once<_>` so `reset_tenant_master_for_test`
/// can clear the slot without `unsafe` reference-casting. The read
/// path takes a brief lock and clones the 32-byte key; that's cheaper
/// than the AEAD work the caller will then do, so the lock isn't a
/// hot-path concern. (#665 — replaces the prior `Once`+ptr-write impl
/// that tripped `invalid_reference_casting`.)
static GLOBAL_TENANT_MASTER: spin::Mutex<Option<TenantMasterKey>> =
    spin::Mutex::new(None);

/// Install the process-wide tenant master. The first install wins;
/// subsequent calls are silently ignored to preserve the boot-time-
/// install-once contract that the prior `spin::Once`-backed impl
/// established. Targets that need a different master across runs MUST
/// be in different processes; tests instantiating `TenantMasterKey`
/// ad-hoc don't go through this slot at all.
///
/// The boot order is: `entropy::install` first (so `csprng` can lazy-
/// seed), THEN this — the host CLI's `install_or_generate_master` may
/// call `csprng::random_bytes` to produce the 32 bytes when the master
/// file is absent, and that path requires an installed entropy source.
pub fn install_tenant_master(master: TenantMasterKey) {
    let mut slot = GLOBAL_TENANT_MASTER.lock();
    if slot.is_none() {
        *slot = Some(master);
    }
}

/// Read the installed tenant master. Returns `None` until
/// `install_tenant_master` has run (boot bug — surface as a clear
/// error at the call site rather than panicking). Returns an owned
/// clone so callers don't hold the slot lock; the 32-byte clone is
/// negligible against the AEAD work each call site goes on to perform.
pub fn current_tenant_master() -> Option<TenantMasterKey> {
    GLOBAL_TENANT_MASTER.lock().clone()
}

/// Test-only: clear the installed master so the next test's
/// `install_tenant_master` call wins the slot.
///
/// Held under the same cross-module test lock as the entropy slot
/// (`entropy::TEST_LOCK`) so concurrent cases can't observe a torn
/// state. No `unsafe`, no lint suppression — straightforward Mutex
/// mutation now that the slot type is `Mutex<Option<_>>`.
///
/// Production code MUST NOT call this.
///
/// Exposed across the arest-foundation→arest crate boundary under
/// `test-helpers` (#686) so arest's tests (cell_aead doc-tests live
/// in foundation now, but tenant-master-rotation flows in arest's
/// cli/tenant_master_host + freeze + cloudflare tests still reach in
/// to clear the slot between cases). Production builds never see the
/// symbol because arest only enables `test-helpers` in dev-deps.
#[cfg(any(test, feature = "test-helpers"))]
#[allow(dead_code)]
pub fn reset_tenant_master_for_test() {
    *GLOBAL_TENANT_MASTER.lock() = None;
}

// ── Cell address ───────────────────────────────────────────────────

/// Canonical four-tuple identifying a cell across the engine. Acts as
///   * HKDF salt (per-cell key separation)
///   * AEAD AAD (re-target detection — same ciphertext at a different
///     address fails decrypt)
///   * replay defence vector (the version field is per-cell monotonic
///     per #558, so an older ciphertext at the same scope/domain/name
///     fails decrypt against a newer key by construction)
///
/// S1i (#725) contract: the `version` field IS the post-S1b chain's
/// `version_id` (see ast.rs `cell_pin` / `version_entry_id`). Callers
/// in the worker EntityDB, kernel block_storage, and host CLI MUST
/// thread the chain's monotonic id here — not their own counter — so
/// the AAD reflects the true storage version at seal time. The
/// `aad_mismatch_fails_open` test (Test 1.3 below) covers the
/// version-replay invariant explicitly: sealing under v=N and
/// opening under v=N+1 returns AeadError::Auth.
///
/// `Clone + Eq + Hash` so callers can stash addresses in maps; the
/// `canonical_bytes` representation is a deterministic
/// length-prefixed concatenation suitable for both HKDF and AAD.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CellAddress {
    pub scope: String,
    pub domain: String,
    pub cell_name: String,
    pub version: u64,
}

impl CellAddress {
    /// Build a fresh address. `version` is the monotonic counter
    /// (#558) the engine bumps each time a cell's contents change;
    /// passing zero is fine for a freshly-created cell.
    pub fn new(scope: impl Into<String>, domain: impl Into<String>, cell_name: impl Into<String>, version: u64) -> Self {
        Self {
            scope: scope.into(),
            domain: domain.into(),
            cell_name: cell_name.into(),
            version,
        }
    }

    /// Length-prefixed deterministic byte encoding. Used as both the
    /// HKDF salt and the AEAD AAD so a single canonical form ties the
    /// two together. Format:
    ///
    /// ```text
    /// [u32 LE scope_len  | scope_bytes]
    /// [u32 LE domain_len | domain_bytes]
    /// [u32 LE name_len   | name_bytes]
    /// [u64 LE version]
    /// ```
    ///
    /// Length prefixes prevent boundary-collision attacks where
    /// e.g. (scope = "ab", domain = "cde") would collide with
    /// (scope = "abcd", domain = "e") under naive concatenation.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(
            4 + self.scope.len()
                + 4 + self.domain.len()
                + 4 + self.cell_name.len()
                + 8,
        );
        out.extend_from_slice(&(self.scope.len() as u32).to_le_bytes());
        out.extend_from_slice(self.scope.as_bytes());
        out.extend_from_slice(&(self.domain.len() as u32).to_le_bytes());
        out.extend_from_slice(self.domain.as_bytes());
        out.extend_from_slice(&(self.cell_name.len() as u32).to_le_bytes());
        out.extend_from_slice(self.cell_name.as_bytes());
        out.extend_from_slice(&self.version.to_le_bytes());
        out
    }
}

// ── Errors ─────────────────────────────────────────────────────────

/// AEAD failure modes. Coarse on purpose — callers don't need to
/// know whether the nonce was truncated, the tag was wrong, or the
/// AAD didn't match: in every case the answer is "do not trust this
/// ciphertext". The `Truncated` and `Auth` distinction is kept for
/// targeted logging at the call site (a torn DO write surfaces as
/// `Truncated`; a swapped-cell ciphertext as `Auth`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AeadError {
    /// Sealed envelope was shorter than `NONCE_LEN + TAG_LEN`. The
    /// upstream layer stored fewer bytes than were ever produced —
    /// likely a torn write or a wrong-format read.
    Truncated,
    /// Poly1305 tag verification failed. The ciphertext, the
    /// associated data (cell address), or the key did not match
    /// the original. Most common cause is an attempted re-target
    /// (replay against a different cell address).
    Auth,
}

impl fmt::Display for AeadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated => f.write_str("sealed envelope truncated"),
            Self::Auth => f.write_str("AEAD authentication failed"),
        }
    }
}

// ── Per-cell key derivation ────────────────────────────────────────

/// HKDF-SHA256 the master into a 32-byte cell key bound to the given
/// address. Pure function; same inputs → same key. Never panics on
/// input — HKDF only fails when the requested output is wider than
/// 255 × HashLen (8160 bytes for SHA-256), and 32 bytes is far inside
/// that bound.
fn derive_cell_key(master: &TenantMasterKey, address: &CellAddress) -> [u8; CELL_KEY_LEN] {
    let salt = address.canonical_bytes();
    let hk = Hkdf::<Sha256>::new(Some(&salt), master.as_bytes());
    let mut key = [0u8; CELL_KEY_LEN];
    hk.expand(HKDF_INFO, &mut key)
        .expect("HKDF expand of 32 bytes is always within 255 * HashLen");
    key
}

// ── Public AEAD API ────────────────────────────────────────────────

/// Seal `plaintext` for the named cell. Returns
/// `[NONCE_LEN-byte nonce | ciphertext | TAG_LEN-byte tag]`.
///
/// The nonce is drawn from `crate::csprng::random_bytes` (the
/// process-wide ChaCha20 CSPRNG seeded from the installed entropy
/// source — see #568 / #578). With a 96-bit nonce + a 32-byte cell
/// key derived from the master + cell address, nonce reuse across
/// distinct seal calls for the same cell is statistically negligible
/// (collision probability ≈ 2^{-48} per cell after 2^{48} seals).
///
/// Panics only if no entropy source is installed (inherited from
/// csprng). Targets MUST install one at boot — failure here is a
/// bring-up bug, not a runtime outage.
pub fn cell_seal(
    master: &TenantMasterKey,
    address: &CellAddress,
    plaintext: &[u8],
) -> Vec<u8> {
    let key_bytes = derive_cell_key(master, address);
    let cipher = ChaCha20Poly1305::new(Key::from_slice(&key_bytes));

    // Fresh per-seal nonce — drawn from the process-wide CSPRNG so we
    // share entropy provenance with every other crypto site (#578).
    let mut nonce_bytes = [0u8; NONCE_LEN];
    csprng::random_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let aad = address.canonical_bytes();
    let payload = Payload {
        msg: plaintext,
        aad: &aad,
    };
    let ciphertext = cipher
        .encrypt(nonce, payload)
        .expect("ChaCha20-Poly1305 encrypt is infallible for finite inputs");

    let mut out = Vec::with_capacity(NONCE_LEN + ciphertext.len());
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ciphertext);
    out
}

/// Open a sealed envelope at the named cell address. Returns the
/// recovered plaintext on success, `Err(AeadError::Auth)` on a tag
/// mismatch (wrong key / wrong AAD / tampered ciphertext), or
/// `Err(AeadError::Truncated)` if the envelope is shorter than the
/// minimum 28 bytes of overhead.
pub fn cell_open(
    master: &TenantMasterKey,
    address: &CellAddress,
    sealed: &[u8],
) -> Result<Vec<u8>, AeadError> {
    if sealed.len() < NONCE_LEN + TAG_LEN {
        return Err(AeadError::Truncated);
    }
    let (nonce_bytes, ciphertext) = sealed.split_at(NONCE_LEN);
    let key_bytes = derive_cell_key(master, address);
    let cipher = ChaCha20Poly1305::new(Key::from_slice(&key_bytes));
    let nonce = Nonce::from_slice(nonce_bytes);
    let aad = address.canonical_bytes();
    let payload = Payload {
        msg: ciphertext,
        aad: &aad,
    };
    cipher.decrypt(nonce, payload).map_err(|_| AeadError::Auth)
}

// ── Tenant master rotation (#662) ──────────────────────────────────
//
// Operator workflow: a deployment switches from master A → master B
// without losing access to existing cells. The rotation re-seals each
// cell under the new master, leaving the cell address untouched —
// per-cell HKDF still derives a distinct key, but from the new root.
//
// Rotation is single-cell atomic: open the sealed envelope under the
// old master, re-seal the recovered plaintext under the new master at
// the SAME `CellAddress`. A fresh nonce is drawn (per-seal nonce
// uniqueness is unchanged from `cell_seal`); the AAD bytes are
// identical pre/post-rotation, so a future opener that knows only the
// new master must still present the correct address to authenticate.
//
// ## Tenant-locked walk
//
// `rotate_tenant` is the multi-cell driver. It iterates the caller-
// supplied `(CellAddress, sealed_bytes)` stream and produces a
// `RotationReport` collecting:
//   * `rotated`: every cell that opened cleanly under the old master
//     and re-sealed under the new master. Caller atomic-swaps these
//     into storage.
//   * `failures`: every cell the old master could NOT open. These are
//     retained under the old master untouched — operator decides
//     whether to retry, zeroize, or accept the loss. The walk does
//     NOT abort on the first failure: a single corrupt cell would
//     otherwise hold the entire tenant hostage to the old master.
//
// ## Read-only window assumption
//
// First-version rotation is non-zero-downtime. The caller MUST hold a
// per-tenant write lock for the duration of the walk (kernel: the
// per-slot RwLock from #155; worker: the per-tenant write semaphore
// at the EntityDB orchestrator level — each individual EntityDB DO is
// already single-writer by Cloudflare's design, so the per-tenant
// guard is what serialises the cross-DO walk). Concurrent writes
// during rotation would observe a half-rotated cell set: some cells
// readable only by `old`, some only by `new`. Either drains the
// rotation or holds writes until it completes. This module documents
// the assumption; the kernel and worker call sites enforce it via
// their respective lock primitives.
//
// ## Operator workflow (kernel side)
//
//   1. Persist the new 32-byte master in the freeze-blob "pending"
//      slot alongside the existing "active" slot.
//   2. Acquire the per-tenant write lock (kernel #155 path).
//   3. Enumerate sealed cells via `block_storage` reserved-region
//      iteration; pass them through `rotate_tenant`.
//   4. Atomic-swap each `RotationReport.rotated` entry into storage.
//   5. Promote "pending" → "active" in the freeze-blob; drop the old.
//   6. Release the per-tenant write lock.
//
// ## Operator workflow (worker side)
//
//   1. `wrangler secret put TENANT_MASTER_SEED_v2 <new>` (the v1
//      slot stays bound during the rotation window).
//   2. Acquire the per-tenant write lock at the orchestrator (the
//      RegistryDB / dispatcher seam, so concurrent writes against
//      the tenant's EntityDBs cannot interleave).
//   3. Enumerate the tenant's EntityDB cells via the per-tenant
//      scoping from #205; for each cell read the sealed row, run
//      `rotate_cell`, write the new sealed row back atomically.
//   4. Once the report is empty of failures (or operator accepts the
//      reported losses), `wrangler secret put TENANT_MASTER_SEED <new>`
//      and `wrangler secret delete TENANT_MASTER_SEED_v2`.
//   5. Release the per-tenant write lock.
//
// ## Out of scope (deferred to follow-ups)
//
//   * Zero-downtime rotation. First version takes the tenant
//     read-only for the rotation window (caller-enforced lock above).
//   * Per-cell metadata indicating which master a cell is sealed
//     under. Without it the operator MUST keep both masters loaded
//     simultaneously through the rotation; once promotion lands the
//     old master can be zeroized.
//   * Automated rotation triggers. Operator-initiated only via the
//     `SystemVerb::RotateTenantMaster` privileged dispatch in
//     `arest::lib::system_impl` — gated behind `RegisterMode::Privileged`
//     so an HTTP/MCP frontend cannot trigger a rotation remotely.

/// Outcome of a multi-cell rotation walk. Plaintext-only fields — the
/// `rotated` payloads are sealed bytes (still ciphertext) so logging
/// or returning a `RotationReport` does not leak cell contents. Cell
/// addresses are also plaintext at the storage layer (they double as
/// routing keys), so surfacing them here is safe.
#[derive(Debug, Clone)]
pub struct RotationReport {
    /// Cells that re-sealed cleanly under the new master. Each entry
    /// is the new sealed envelope for the matching `CellAddress`; the
    /// caller atomic-swaps these into storage to complete the
    /// rotation. Order matches the input iterator's order.
    pub rotated: Vec<(CellAddress, Vec<u8>)>,
    /// Cells the old master could not open. These were left untouched
    /// — the storage row is still valid under the old master, the new
    /// master cannot read it. Operator decides whether to retry,
    /// zeroize, or accept the loss. Order matches the input
    /// iterator's order.
    pub failures: Vec<(CellAddress, AeadError)>,
}

impl RotationReport {
    /// Returns `true` when every supplied cell rotated successfully.
    /// Convenience for callers that gate the master-promotion step on
    /// "no losses".
    pub fn is_complete(&self) -> bool {
        self.failures.is_empty()
    }
}

/// Single-cell rotation primitive. Opens `sealed` with `old`, re-seals
/// the recovered plaintext under `new` at the SAME `addr` (fresh
/// nonce). Atomic at the cell level: either both halves succeed and
/// the caller has a new sealed envelope to swap in, or the call
/// returns the open-side `AeadError` and the on-disk row is
/// untouched.
///
/// Returns `Err(AeadError::Truncated)` if `sealed` is structurally
/// malformed under the AEAD envelope (shorter than `NONCE_LEN +
/// TAG_LEN`); returns `Err(AeadError::Auth)` if the old master cannot
/// open the envelope at this address (wrong master / wrong AAD /
/// tampered ciphertext / cell sealed under a third key entirely).
///
/// Panics propagate from `cell_seal` — same contract as elsewhere in
/// this module: an uninstalled entropy source on the target is a
/// bring-up bug, not a runtime outage.
pub fn rotate_cell(
    old: &TenantMasterKey,
    new: &TenantMasterKey,
    addr: &CellAddress,
    sealed: &[u8],
) -> Result<Vec<u8>, AeadError> {
    let plaintext = cell_open(old, addr, sealed)?;
    Ok(cell_seal(new, addr, &plaintext))
}

/// Walk a tenant's sealed cells, rotating each from `old` → `new`.
///
/// The caller is responsible for:
///   * Holding the per-tenant write lock for the duration of the walk
///     (read-only window — see module-level documentation above).
///   * Atomic-swapping each `RotationReport.rotated` entry into
///     storage. The walk itself is pure: it reads the input iterator
///     and produces sealed bytes; nothing here touches a backend.
///   * Deciding what to do with `RotationReport.failures` (retry the
///     individual cells, zeroize them, or accept the loss).
///
/// One failed cell does NOT abort the walk. A single corrupt envelope
/// would otherwise force the operator to keep the old master active
/// for the entire tenant. Per-cell failures are collected; the rest
/// of the cells continue to rotate.
pub fn rotate_tenant(
    old: &TenantMasterKey,
    new: &TenantMasterKey,
    cells: impl Iterator<Item = (CellAddress, Vec<u8>)>,
) -> RotationReport {
    let mut rotated: Vec<(CellAddress, Vec<u8>)> = Vec::new();
    let mut failures: Vec<(CellAddress, AeadError)> = Vec::new();
    for (addr, sealed) in cells {
        match rotate_cell(old, new, &addr, &sealed) {
            Ok(new_sealed) => rotated.push((addr, new_sealed)),
            Err(e) => failures.push((addr, e)),
        }
    }
    RotationReport { rotated, failures }
}

// ── Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entropy::{self, DeterministicSource};
    use alloc::boxed::Box;

    /// Fixture: install a deterministic entropy source so nonce draws
    /// are reproducible across runs, then reset both the entropy
    /// source and the CSPRNG state at the end. Same shape as the
    /// crypto.rs / csprng.rs fixtures — the cross-module
    /// `entropy::TEST_LOCK` keeps a concurrent install from sabotaging
    /// our nonce draw mid-flight.
    fn with_deterministic_entropy<F: FnOnce()>(seed: [u8; 32], body: F) {
        let _guard = entropy::TEST_LOCK.lock();
        entropy::install(Box::new(DeterministicSource::new(seed)));
        crate::csprng::reseed();
        body();
        entropy::uninstall();
        crate::csprng::reseed();
    }

    fn fixture_master(byte: u8) -> TenantMasterKey {
        TenantMasterKey::from_bytes([byte; 32])
    }

    fn fixture_address() -> CellAddress {
        CellAddress::new("acme", "orders", "Order#42", 1)
    }

    /// Test 1.1 — round-trip. seal(plaintext) → open() = plaintext.
    /// The smoke test that proves the AEAD primitive itself is wired
    /// correctly end to end.
    #[test]
    fn round_trip_recovers_plaintext() {
        let master = fixture_master(0xAB);
        let addr = fixture_address();
        let plaintext = b"hello world, plaintext payload bytes";
        with_deterministic_entropy([7u8; 32], || {
            let sealed = cell_seal(&master, &addr, plaintext);
            assert!(
                sealed.len() == plaintext.len() + NONCE_LEN + TAG_LEN,
                "sealed envelope = nonce + ciphertext + tag",
            );
            let recovered = cell_open(&master, &addr, &sealed)
                .expect("round-trip open must succeed");
            assert_eq!(recovered.as_slice(), plaintext);
        });
    }

    /// Test 1.2 — tamper detection. Mutating any byte of the sealed
    /// envelope (nonce, ciphertext, or tag) must surface as
    /// `AeadError::Auth` on open. Probes three positions to cover
    /// every part of the envelope.
    #[test]
    fn tamper_detection_fails_open() {
        let master = fixture_master(0xCD);
        let addr = fixture_address();
        let plaintext = b"tamper-test payload";
        with_deterministic_entropy([11u8; 32], || {
            let sealed = cell_seal(&master, &addr, plaintext);

            // Mutate inside the nonce.
            let mut t1 = sealed.clone();
            t1[0] ^= 0x01;
            assert_eq!(cell_open(&master, &addr, &t1), Err(AeadError::Auth));

            // Mutate inside the ciphertext body.
            let mut t2 = sealed.clone();
            let mid = NONCE_LEN + plaintext.len() / 2;
            t2[mid] ^= 0x80;
            assert_eq!(cell_open(&master, &addr, &t2), Err(AeadError::Auth));

            // Mutate inside the trailing tag.
            let mut t3 = sealed.clone();
            let last = t3.len() - 1;
            t3[last] ^= 0xFF;
            assert_eq!(cell_open(&master, &addr, &t3), Err(AeadError::Auth));
        });
    }

    /// Test 1.3 — AAD mismatch. Same sealed bytes, opened with a
    /// different cell address (different scope, different name, or
    /// different version), must fail `AeadError::Auth`. This is what
    /// gives free replay protection when version (#558) bumps.
    #[test]
    fn aad_mismatch_fails_open() {
        let master = fixture_master(0x11);
        let addr_a = CellAddress::new("acme", "orders", "Order#1", 7);
        let plaintext = b"AAD-mismatch payload";
        with_deterministic_entropy([13u8; 32], || {
            let sealed = cell_seal(&master, &addr_a, plaintext);

            // Different cell name within the same domain.
            let addr_b = CellAddress::new("acme", "orders", "Order#2", 7);
            assert_eq!(cell_open(&master, &addr_b, &sealed), Err(AeadError::Auth));

            // Different domain.
            let addr_c = CellAddress::new("acme", "billing", "Order#1", 7);
            assert_eq!(cell_open(&master, &addr_c, &sealed), Err(AeadError::Auth));

            // Different scope.
            let addr_d = CellAddress::new("globex", "orders", "Order#1", 7);
            assert_eq!(cell_open(&master, &addr_d, &sealed), Err(AeadError::Auth));

            // Same address but bumped version — replay-against-newer-key fails.
            let addr_e = CellAddress::new("acme", "orders", "Order#1", 8);
            assert_eq!(cell_open(&master, &addr_e, &sealed), Err(AeadError::Auth));
        });
    }

    /// Test 1.4 — cross-tenant. master_a's sealed bytes opened under
    /// master_b must fail. Multi-tenancy isolation: a leaked tenant A
    /// master must not let an attacker decrypt tenant B's cells.
    #[test]
    fn cross_tenant_fails_open() {
        let master_a = fixture_master(0xA1);
        let master_b = fixture_master(0xB2);
        assert_ne!(master_a.as_bytes(), master_b.as_bytes(),
            "test fixtures must be distinct");
        let addr = fixture_address();
        let plaintext = b"cross-tenant isolation probe";
        with_deterministic_entropy([17u8; 32], || {
            let sealed = cell_seal(&master_a, &addr, plaintext);
            // Same address, different master — must fail.
            assert_eq!(cell_open(&master_b, &addr, &sealed), Err(AeadError::Auth));
            // Sanity: master_a still opens correctly.
            assert_eq!(cell_open(&master_a, &addr, &sealed).unwrap().as_slice(), plaintext);
        });
    }

    // ── Supporting tests ────────────────────────────────────────────

    #[test]
    fn truncated_envelope_returns_truncated_error() {
        // Anything shorter than NONCE_LEN + TAG_LEN (= 28 bytes) is
        // structurally malformed — surface that distinctly from a
        // tag-mismatch so a torn DO write logs differently from a
        // re-target attempt.
        let master = fixture_master(0x22);
        let addr = fixture_address();
        let too_short = [0u8; NONCE_LEN + TAG_LEN - 1];
        assert_eq!(cell_open(&master, &addr, &too_short), Err(AeadError::Truncated));
        let empty: [u8; 0] = [];
        assert_eq!(cell_open(&master, &addr, &empty), Err(AeadError::Truncated));
    }

    #[test]
    fn empty_plaintext_round_trips() {
        // Zero-length payload is a degenerate but legal AEAD input;
        // sealed length = NONCE_LEN + TAG_LEN exactly. Some cells
        // serialise to the empty Object on first creation, so the
        // freeze layer must not crash on it.
        let master = fixture_master(0x33);
        let addr = fixture_address();
        with_deterministic_entropy([19u8; 32], || {
            let sealed = cell_seal(&master, &addr, &[]);
            assert_eq!(sealed.len(), NONCE_LEN + TAG_LEN);
            let recovered = cell_open(&master, &addr, &sealed).unwrap();
            assert!(recovered.is_empty());
        });
    }

    #[test]
    fn derive_is_deterministic() {
        // HKDF must be a pure function of (seed, salt) — two calls
        // with identical inputs must yield identical masters. Without
        // this, every reboot would lose access to its own checkpoints.
        let m1 = TenantMasterKey::derive(b"seed bytes 0123456789", b"tenant-salt-A");
        let m2 = TenantMasterKey::derive(b"seed bytes 0123456789", b"tenant-salt-A");
        assert_eq!(m1.as_bytes(), m2.as_bytes());
    }

    #[test]
    fn derive_changes_with_salt() {
        // Salt change → different master. Multi-tenant boot path on a
        // shared entropy seed (one EFI_RNG draw, many tenants) relies
        // on this to scope per-tenant masters.
        let m1 = TenantMasterKey::derive(b"shared seed", b"tenant-A");
        let m2 = TenantMasterKey::derive(b"shared seed", b"tenant-B");
        assert_ne!(m1.as_bytes(), m2.as_bytes());
    }

    #[test]
    fn nonce_advances_per_seal() {
        // Two successive seals of the same plaintext at the same
        // address with the same master must NOT produce identical
        // ciphertexts — the nonce is fresh per seal. Without this,
        // an observer who saw two seals could detect equal plaintexts
        // (a known AEAD failure mode).
        let master = fixture_master(0x44);
        let addr = fixture_address();
        let pt = b"same-plaintext-twice";
        with_deterministic_entropy([23u8; 32], || {
            let s1 = cell_seal(&master, &addr, pt);
            let s2 = cell_seal(&master, &addr, pt);
            assert_ne!(s1, s2,
                "fresh nonce per seal must produce distinct sealed envelopes");
            // Both still decrypt cleanly under the same address.
            assert_eq!(cell_open(&master, &addr, &s1).unwrap(), pt);
            assert_eq!(cell_open(&master, &addr, &s2).unwrap(), pt);
        });
    }

    #[test]
    fn canonical_bytes_disambiguate_field_boundaries() {
        // (scope = "ab", domain = "c") must not produce the same
        // canonical bytes as (scope = "a", domain = "bc") — the
        // length prefixes are what guarantee this. A regression here
        // would silently let two cells share an HKDF salt.
        let a = CellAddress::new("ab", "c", "n", 0);
        let b = CellAddress::new("a", "bc", "n", 0);
        assert_ne!(a.canonical_bytes(), b.canonical_bytes());
    }

    #[test]
    fn debug_redacts_master_bytes() {
        // The Debug impl must NOT print the underlying bytes — they
        // would leak into tracing spans / serial logs / panic reports
        // and undo every other guarantee in this module.
        let m = fixture_master(0x55);
        let s = format!("{:?}", m);
        assert!(s.contains("redacted"), "Debug must redact: got {s}");
        assert!(!s.contains("85"), "Debug must not surface byte 0x55 (= 85)");
    }

    // ── Tenant master rotation (#662) ───────────────────────────────

    /// Test 1.1 — `rotate_cell` round-trip. Seal under master A,
    /// rotate to B, open with B → original plaintext recovered.
    #[test]
    fn rotate_cell_round_trips_to_new_master() {
        let master_a = fixture_master(0xA1);
        let master_b = fixture_master(0xB2);
        let addr = fixture_address();
        let plaintext = b"rotation round-trip payload";
        with_deterministic_entropy([29u8; 32], || {
            let sealed_a = cell_seal(&master_a, &addr, plaintext);
            let sealed_b = rotate_cell(&master_a, &master_b, &addr, &sealed_a)
                .expect("rotate_cell must succeed when old master can open");
            // The new envelope is structurally a fresh AEAD output —
            // distinct nonce + tag from the old (no shared nonce by
            // construction; rotation does NOT reuse the inbound nonce).
            assert_ne!(sealed_a, sealed_b,
                "rotate_cell must produce a fresh envelope, not echo the input");
            // New master opens cleanly; recovered plaintext matches.
            let recovered = cell_open(&master_b, &addr, &sealed_b)
                .expect("post-rotation open under new master must succeed");
            assert_eq!(recovered.as_slice(), plaintext);
        });
    }

    /// Test 1.2 — opening original sealed bytes with B (without
    /// rotation) MUST fail. Sanity check that proves the rotation
    /// step is doing real work — without `rotate_cell`, master B
    /// has no path into A's ciphertext.
    #[test]
    fn untouched_envelope_fails_under_new_master() {
        let master_a = fixture_master(0xA1);
        let master_b = fixture_master(0xB2);
        let addr = fixture_address();
        let plaintext = b"isolation invariant";
        with_deterministic_entropy([31u8; 32], || {
            let sealed_a = cell_seal(&master_a, &addr, plaintext);
            // Without rotation, master B cannot read master A's bytes.
            assert_eq!(
                cell_open(&master_b, &addr, &sealed_a),
                Err(AeadError::Auth),
                "master B must not open master A's untouched envelope",
            );
        });
    }

    /// Test 1.3 — multi-cell `rotate_tenant` walk. Seal N cells under
    /// A; rotate; iterate the report; every `rotated` entry opens
    /// cleanly under B. Belt-and-braces: also verify each rotated
    /// entry now FAILS under master A (the rotation step is one-way
    /// at the per-cell envelope level — master A still works on the
    /// untouched original bytes, but the freshly-sealed-under-B bytes
    /// are A-opaque).
    #[test]
    fn rotate_tenant_walk_re_seals_every_cell() {
        let master_a = fixture_master(0xA1);
        let master_b = fixture_master(0xB2);
        let addresses: alloc::vec::Vec<CellAddress> = (0u64..5)
            .map(|i| CellAddress::new("acme", "orders", format!("Order#{i}"), i + 1))
            .collect();
        let plaintexts: alloc::vec::Vec<alloc::vec::Vec<u8>> = (0u8..5)
            .map(|i| {
                let mut v = alloc::vec::Vec::new();
                v.extend_from_slice(b"payload-");
                v.push(b'A' + i);
                v
            })
            .collect();
        with_deterministic_entropy([37u8; 32], || {
            // Seal every cell under master A.
            let sealed_a: alloc::vec::Vec<(CellAddress, alloc::vec::Vec<u8>)> = addresses
                .iter()
                .zip(plaintexts.iter())
                .map(|(addr, pt)| (addr.clone(), cell_seal(&master_a, addr, pt)))
                .collect();

            // Drive the rotation walk.
            let report = rotate_tenant(
                &master_a,
                &master_b,
                sealed_a.iter().map(|(a, s)| (a.clone(), s.clone())),
            );
            assert!(report.is_complete(),
                "no failures expected on an all-clean input; got {} failures",
                report.failures.len());
            assert_eq!(report.rotated.len(), addresses.len(),
                "every cell must appear in `rotated`");

            // Every rotated entry opens under B with the original plaintext.
            for ((addr, new_sealed), expected) in report.rotated.iter().zip(plaintexts.iter()) {
                let recovered = cell_open(&master_b, addr, new_sealed)
                    .expect("rotated cell must open under new master");
                assert_eq!(recovered.as_slice(), expected.as_slice());
                // And master A canNOT open the freshly-sealed-under-B bytes.
                assert_eq!(
                    cell_open(&master_a, addr, new_sealed),
                    Err(AeadError::Auth),
                    "rotated bytes must be opaque to the OLD master",
                );
            }
        });
    }

    /// Test 1.4 — failure path: corrupt one entry; the walk reports
    /// it in `failures` without aborting the rest. A single bad cell
    /// must not hold the whole tenant hostage to the old master.
    #[test]
    fn rotate_tenant_isolates_per_cell_failures() {
        let master_a = fixture_master(0xA1);
        let master_b = fixture_master(0xB2);
        let addresses: alloc::vec::Vec<CellAddress> = (0u64..3)
            .map(|i| CellAddress::new("acme", "orders", format!("Order#{i}"), i + 1))
            .collect();
        with_deterministic_entropy([41u8; 32], || {
            // Seal every cell under master A.
            let mut sealed_a: alloc::vec::Vec<(CellAddress, alloc::vec::Vec<u8>)> = addresses
                .iter()
                .map(|addr| {
                    let pt = format!("payload-for-{}", addr.cell_name);
                    (addr.clone(), cell_seal(&master_a, addr, pt.as_bytes()))
                })
                .collect();

            // Corrupt the middle cell's tag so its open() fails Auth.
            let mid = 1;
            let last = sealed_a[mid].1.len() - 1;
            sealed_a[mid].1[last] ^= 0xFF;

            // Also include a structurally-truncated cell to exercise
            // the `Truncated` arm of the failure list.
            let truncated_addr = CellAddress::new("acme", "orders", "Order#truncated", 99);
            sealed_a.push((truncated_addr.clone(), alloc::vec::Vec::from(&[0u8; 5][..])));

            let report = rotate_tenant(
                &master_a,
                &master_b,
                sealed_a.iter().map(|(a, s)| (a.clone(), s.clone())),
            );

            assert!(!report.is_complete(),
                "corrupted + truncated cells must surface as failures");
            assert_eq!(report.failures.len(), 2,
                "exactly the corrupted cell + the truncated cell fail; \
                 got {} failures", report.failures.len());
            // The 3 clean cells (indices 0 and 2 — index 1 was tampered)
            // still rotate.
            assert_eq!(report.rotated.len(), addresses.len() - 1);

            // Failure addresses match what we corrupted.
            let failure_names: alloc::vec::Vec<&str> = report
                .failures
                .iter()
                .map(|(addr, _)| addr.cell_name.as_str())
                .collect();
            assert!(failure_names.contains(&"Order#1"),
                "tampered cell must appear in failures");
            assert!(failure_names.contains(&"Order#truncated"),
                "truncated cell must appear in failures");
            // And the failure kinds are distinguished — Auth for the
            // tampered envelope, Truncated for the short one.
            for (addr, kind) in report.failures.iter() {
                if addr.cell_name == "Order#1" {
                    assert_eq!(*kind, AeadError::Auth);
                } else if addr.cell_name == "Order#truncated" {
                    assert_eq!(*kind, AeadError::Truncated);
                }
            }

            // Sanity: rotated entries open under B, untouched under A.
            for (addr, new_sealed) in report.rotated.iter() {
                assert!(cell_open(&master_b, addr, new_sealed).is_ok(),
                    "rotated cell {:?} must open under master B", addr);
            }

            // Sanity: the cells that FAILED to rotate are still
            // readable under master A — the rotation walk left them
            // alone, so the operator can retry / zeroize / replicate.
            let untampered_addr = &addresses[0];
            let untampered_bytes = &sealed_a[0].1;
            assert!(
                cell_open(&master_a, untampered_addr, untampered_bytes).is_ok(),
                "old master must still open the original (un-rotated) bytes",
            );
        });
    }

    // ── Process-global tenant master slot (#663) ────────────────────

    /// `current_tenant_master` must return `None` when nothing has
    /// been installed. The boot path uses this to fail loudly rather
    /// than panicking on a missing master.
    #[test]
    fn current_tenant_master_is_none_until_installed() {
        let _guard = entropy::TEST_LOCK.lock();
        reset_tenant_master_for_test();
        assert!(current_tenant_master().is_none(),
            "uninstalled slot must read as None");
    }

    /// `install_tenant_master` followed by `current_tenant_master`
    /// must hand back the exact bytes — same `as_bytes()` value as
    /// went in. This is the core read-after-write contract the host
    /// CLI boot path relies on.
    #[test]
    fn install_then_current_returns_same_bytes() {
        let _guard = entropy::TEST_LOCK.lock();
        reset_tenant_master_for_test();
        let bytes = [0x42u8; CELL_KEY_LEN];
        install_tenant_master(TenantMasterKey::from_bytes(bytes));
        let got = current_tenant_master().expect("after install, slot must be Some");
        assert_eq!(got.as_bytes(), &bytes);
        reset_tenant_master_for_test();
    }

    /// `Once` semantics: a second install is a no-op (first install
    /// wins). Production paths install exactly once at boot, so this
    /// pins behaviour for the "boot script accidentally calls install
    /// twice" scenario — the second call must NOT silently swap the
    /// master out from under in-flight cell_seal callers.
    #[test]
    fn second_install_is_a_no_op() {
        let _guard = entropy::TEST_LOCK.lock();
        reset_tenant_master_for_test();
        let first = [0x11u8; CELL_KEY_LEN];
        let second = [0x99u8; CELL_KEY_LEN];
        install_tenant_master(TenantMasterKey::from_bytes(first));
        install_tenant_master(TenantMasterKey::from_bytes(second));
        let got = current_tenant_master().unwrap();
        assert_eq!(got.as_bytes(), &first,
            "Once::call_once: first install must win, second is silently dropped");
        reset_tenant_master_for_test();
    }

    // ── Cross-tier fixture (#660) ─────────────────────────────────────
    //
    // Lock the wire format byte-for-byte against an external (TS /
    // Worker) opener. The Worker test in `src/cell-encryption.test.ts`
    // hard-codes the same hex the function below asserts; that
    // double-pin is what proves the wire shape is identical across
    // tiers — neither side can drift without breaking the other.
    //
    // The Rust direction here uses a deterministic entropy source so
    // the nonce is reproducible; the Worker side then opens the same
    // bytes via `cell_open_wasm` and recovers the same plaintext.
    // The reverse direction (TS seals, Rust opens) doesn't need a
    // matching Rust fixture: any envelope the TS side produces with
    // the same canonical address bytes + master deserialises through
    // exactly the function the Rust side already exercises in
    // `round_trip_recovers_plaintext`.

    /// Hex of the canonical fixture: master = 0xAA × 32, address =
    /// (scope = "worker", domain = "Order", cell_name = "ord-42",
    /// version = 1), entropy seed = [0x42; 32], plaintext =
    /// b"cross-tier round-trip payload". The TS test mirrors these
    /// inputs verbatim. Bumping any of the four fields below requires
    /// regenerating the hex (run this test with `--nocapture` and
    /// copy the printed bytes into both this constant and the TS
    /// fixture).
    pub(super) const CROSS_TIER_FIXTURE_PLAINTEXT: &[u8] =
        b"cross-tier round-trip payload";
    pub(super) const CROSS_TIER_FIXTURE_MASTER: [u8; CELL_KEY_LEN] = [0xAAu8; CELL_KEY_LEN];
    pub(super) const CROSS_TIER_FIXTURE_ENTROPY_SEED: [u8; 32] = [0x42u8; 32];

    fn cross_tier_fixture_address() -> CellAddress {
        CellAddress::new("worker", "Order", "ord-42", 1)
    }

    /// Hex of the sealed envelope produced by the deterministic seed
    /// + master + address + plaintext above. If the AEAD format ever
    /// drifts (different HKDF info string, different AAD layout,
    /// different ChaCha20 round count, etc.) this constant breaks
    /// AND the Worker test breaks — the two together pin the
    /// cross-tier contract.
    pub(super) const CROSS_TIER_FIXTURE_SEALED_HEX: &str =
        "bb3017b93796dc709e4aad59713c2b04138686ca33f233bd4ef1ff4088d3aed5607d457afa05ed72dfec9c6482d4092ab7f25574ae1112c989";

    /// Decode a hex string with no separators — small no_std-clean
    /// helper specific to this fixture; the test crate already pulls
    /// `hex` for some suites but `cell_aead` itself doesn't depend on
    /// it, and pulling a transitive crate just for one test is
    /// overkill.
    fn decode_hex(s: &str) -> Vec<u8> {
        assert!(s.len() % 2 == 0, "hex string length must be even");
        let mut out = Vec::with_capacity(s.len() / 2);
        let bytes = s.as_bytes();
        for pair in bytes.chunks(2) {
            let hi = char::from(pair[0]).to_digit(16).expect("valid hex digit") as u8;
            let lo = char::from(pair[1]).to_digit(16).expect("valid hex digit") as u8;
            out.push((hi << 4) | lo);
        }
        out
    }

    /// Cross-tier wire-format pin (#660). The Rust seal MUST produce
    /// exactly the bytes the Worker test consumes. Two reasons this
    /// is a fully-deterministic fixture rather than a round-trip
    /// only:
    ///   * Catches accidental format drift the moment it lands —
    ///     a swap of the HKDF info string or AAD layout would still
    ///     pass a self-round-trip on either side, but breaks the
    ///     cross-tier contract.
    ///   * Documents the bytes both sides expect, so a future tier
    ///     (FPGA, mobile) has a known-good test vector to validate
    ///     its own implementation against without booting either of
    ///     the existing tiers.
    #[test]
    fn cross_tier_fixture_seal_matches_constant() {
        let master = TenantMasterKey::from_bytes(CROSS_TIER_FIXTURE_MASTER);
        let addr = cross_tier_fixture_address();
        with_deterministic_entropy(CROSS_TIER_FIXTURE_ENTROPY_SEED, || {
            let sealed = cell_seal(&master, &addr, CROSS_TIER_FIXTURE_PLAINTEXT);
            // Print the actual bytes when the assertion fails — caller
            // can copy them straight into the constant + TS fixture.
            let actual_hex: String = sealed
                .iter()
                .map(|b| format!("{:02x}", b))
                .collect();
            assert_eq!(
                actual_hex, CROSS_TIER_FIXTURE_SEALED_HEX,
                "cross-tier sealed envelope drifted from the locked-in fixture; \
                 update both this constant AND the matching TS fixture in \
                 src/cell-encryption.test.ts"
            );

            // Sanity: the bytes we just produced still open under the
            // same master/address (a basic round-trip on the fixture).
            let recovered = cell_open(&master, &addr, &sealed)
                .expect("fixture must round-trip on the Rust side");
            assert_eq!(recovered, CROSS_TIER_FIXTURE_PLAINTEXT);
        });
    }

    /// Inverse direction: take the locked-in hex (the same string the
    /// TS test seals + sends back) and prove `cell_open` recovers the
    /// fixture plaintext. This is the path a future tier's seal
    /// implementation would need to satisfy — produce these bytes,
    /// any AEAD-correct opener (Rust or TS) reads them back.
    #[test]
    fn cross_tier_fixture_opens_from_locked_hex() {
        let master = TenantMasterKey::from_bytes(CROSS_TIER_FIXTURE_MASTER);
        let addr = cross_tier_fixture_address();
        let sealed = decode_hex(CROSS_TIER_FIXTURE_SEALED_HEX);
        // Open does NOT touch the entropy source (no nonce draw), so
        // it runs without `with_deterministic_entropy` — proves the
        // opener is a pure function of (master, addr, sealed).
        let recovered = cell_open(&master, &addr, &sealed)
            .expect("locked-hex envelope must open under fixture master/address");
        assert_eq!(recovered, CROSS_TIER_FIXTURE_PLAINTEXT);
    }
}

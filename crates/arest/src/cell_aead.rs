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

// ── Cell address ───────────────────────────────────────────────────

/// Canonical four-tuple identifying a cell across the engine. Acts as
///   * HKDF salt (per-cell key separation)
///   * AEAD AAD (re-target detection — same ciphertext at a different
///     address fails decrypt)
///   * replay defence vector (the version field is per-cell monotonic
///     per #558, so an older ciphertext at the same scope/domain/name
///     fails decrypt against a newer key by construction)
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
    ///     [u32 LE scope_len  | scope_bytes]
    ///     [u32 LE domain_len | domain_bytes]
    ///     [u32 LE name_len   | name_bytes]
    ///     [u64 LE version]
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
}

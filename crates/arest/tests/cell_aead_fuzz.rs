// crates/arest/tests/cell_aead_fuzz.rs
//
// Property-fuzz harness for the cell-level AEAD primitive (#669).
//
// Pins three invariants under proptest's randomised input generation,
// on top of the hand-written cases already in `crate::cell_aead::tests`:
//
//   1. Round-trip: cell_open(addr, cell_seal(addr, pt)) == pt for any
//      address shape and any plaintext up to 1 KiB.
//
//   2. Cross-tenant rejection: a sealed envelope under tenant A's
//      master MUST NOT decrypt under tenant B's master, for any pair
//      of distinct masters.
//
//   3. Tamper detection: any single-byte XOR mutation anywhere in the
//      sealed envelope (nonce / ciphertext / tag) MUST fail Auth on
//      open.
//
//   4. Address-mismatch (AAD-binding) rejection: sealing under one
//      address and opening under a one-field-modified address MUST
//      fail Auth.
//
// ## Cap on plaintext size
//
// Each proptest case must stay sub-second; 1 KiB is plenty to cover
// the AEAD's per-block path while keeping shrinking cheap. Address
// strings cap at 32 bytes per field for the same reason — beyond that,
// the AEAD is just being asked to hash more salt bytes, which is not
// what these properties are exercising.
//
// ## Process-global entropy + master slot
//
// `cell_seal` draws its nonce from `arest::csprng`, which requires a
// process-global entropy source to be installed. The cell-AEAD module
// itself does NOT touch the process-global tenant-master slot — it
// takes a `&TenantMasterKey` directly — so we construct masters per-
// case via `TenantMasterKey::from_bytes` without needing
// `install_tenant_master`. The entropy install happens once per
// process behind a `OnceLock`, guarded by a local mutex so the
// proptest runner's parallelism cannot tear the global state.

use std::sync::{Mutex, OnceLock};

use arest::cell_aead::{
    cell_open, cell_seal, AeadError, CellAddress, TenantMasterKey, NONCE_LEN, TAG_LEN,
};
use arest::entropy::{self, DeterministicSource};

use proptest::prelude::*;

/// Process-wide guard so the proptest runner's threads can't race on
/// the entropy source / csprng state. The cell-AEAD path itself is
/// thread-safe under the spin locks inside `entropy` + `csprng`, but
/// we serialise here too so a tampered fixture can't be observed
/// mid-install. One mutex covers all four proptest blocks.
fn global_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// Install a deterministic entropy source exactly once per process —
/// every subsequent test case shares it. The source itself is
/// stateful (a counter), so successive `cell_seal` calls draw distinct
/// nonces; we don't need a fresh source per case. The OnceLock keeps
/// the install cheap (one-shot, no re-entry).
fn ensure_entropy_installed() {
    static INSTALLED: OnceLock<()> = OnceLock::new();
    INSTALLED.get_or_init(|| {
        entropy::install(Box::new(DeterministicSource::new([0x5Au8; 32])));
    });
}

/// Strategy for a CellAddress with bounded field widths. Keeping
/// strings ASCII-printable (and short) makes shrunk counterexamples
/// readable in failure output without restricting the AEAD path —
/// `canonical_bytes()` is byte-exact regardless of UTF-8 content.
fn cell_address_strategy() -> impl Strategy<Value = CellAddress> {
    (
        "[a-zA-Z0-9_-]{0,32}",
        "[a-zA-Z0-9_-]{0,32}",
        "[a-zA-Z0-9_#-]{0,32}",
        any::<u64>(),
    )
        .prop_map(|(scope, domain, name, version)| {
            CellAddress::new(scope, domain, name, version)
        })
}

// Master keys are generated inline via `any::<[u8; 32]>()` and
// wrapped at use site with `TenantMasterKey::from_bytes` — pulling a
// dedicated strategy out as a helper would be one extra layer for no
// readability win when the only thing it does is `.prop_map(from_bytes)`.

proptest! {
    #![proptest_config(ProptestConfig {
        // 256 cases per property × 4 properties × sub-second per case
        // = ~few-second harness. Default settings would over-shrink
        // on the larger inputs (1 KiB plaintext); cap shrink iters
        // so failures still surface a small counterexample without
        // dominating the runtime.
        cases: 256,
        max_shrink_iters: 1024,
        .. ProptestConfig::default()
    })]

    /// Property 1 — round-trip.
    ///
    /// For any plaintext up to 1 KiB and any cell address, the
    /// envelope produced by `cell_seal` opens back to the same
    /// plaintext. Sealed envelope length is exactly
    /// `plaintext.len() + NONCE_LEN + TAG_LEN`.
    #[test]
    fn round_trip_recovers_plaintext(
        addr in cell_address_strategy(),
        master_bytes in any::<[u8; 32]>(),
        plaintext in proptest::collection::vec(any::<u8>(), 0..=1024),
    ) {
        let _guard = global_lock().lock().unwrap();
        ensure_entropy_installed();

        let master = TenantMasterKey::from_bytes(master_bytes);
        let sealed = cell_seal(&master, &addr, &plaintext);
        prop_assert_eq!(
            sealed.len(),
            plaintext.len() + NONCE_LEN + TAG_LEN,
            "envelope = nonce + ciphertext + tag",
        );
        let recovered = cell_open(&master, &addr, &sealed)
            .expect("round-trip open must succeed");
        prop_assert_eq!(recovered, plaintext);
    }

    /// Property 2 — cross-tenant rejection.
    ///
    /// Sealing under master A and opening under master B (any pair of
    /// distinct 32-byte masters) MUST fail with `AeadError::Auth`.
    /// Sanity-checks that master A still opens its own bytes — a
    /// regression that broke the seal path without breaking the open
    /// path would otherwise still pass this property.
    ///
    /// Note on the "global master slot" simulation: the cell-AEAD API
    /// takes the master as a parameter (it does NOT consult the
    /// process-global slot installed by `install_tenant_master`). So
    /// we don't need to re-install between halves — both masters
    /// coexist as local values. Documented here because the task
    /// brief mentions the simulate-via-reinstall workaround as a
    /// fallback.
    #[test]
    fn cross_tenant_open_rejected(
        addr in cell_address_strategy(),
        master_a_bytes in any::<[u8; 32]>(),
        master_b_bytes in any::<[u8; 32]>(),
        plaintext in proptest::collection::vec(any::<u8>(), 0..=1024),
    ) {
        prop_assume!(master_a_bytes != master_b_bytes);
        let _guard = global_lock().lock().unwrap();
        ensure_entropy_installed();

        let master_a = TenantMasterKey::from_bytes(master_a_bytes);
        let master_b = TenantMasterKey::from_bytes(master_b_bytes);
        let sealed = cell_seal(&master_a, &addr, &plaintext);
        prop_assert_eq!(
            cell_open(&master_b, &addr, &sealed),
            Err(AeadError::Auth),
            "master B must not open master A's envelope",
        );
        // Sanity: A still opens its own bytes.
        let recovered = cell_open(&master_a, &addr, &sealed)
            .expect("master A must still open its own envelope");
        prop_assert_eq!(recovered, plaintext);
    }

    /// Property 3 — tamper detection.
    ///
    /// Pick any single byte position in the sealed envelope and XOR
    /// it with any non-zero mask. The resulting envelope MUST fail
    /// `cell_open` with `AeadError::Auth`. Index is constrained via
    /// `prop_assume!` to land inside the actual envelope; the XOR
    /// mask is forced non-zero so the mutation is real.
    ///
    /// Cap plaintext at 256 B here (rather than 1 KiB) — proptest
    /// generates a fresh `index` per case independent of plaintext
    /// length, and a too-large envelope wastes shrink budget on
    /// "shrink the plaintext" rather than "shrink the index".
    #[test]
    fn tamper_detection_fails_open(
        addr in cell_address_strategy(),
        master_bytes in any::<[u8; 32]>(),
        plaintext in proptest::collection::vec(any::<u8>(), 0..=256),
        index in any::<usize>(),
        xor_mask in 1u8..=255,
    ) {
        let _guard = global_lock().lock().unwrap();
        ensure_entropy_installed();

        let master = TenantMasterKey::from_bytes(master_bytes);
        let sealed = cell_seal(&master, &addr, &plaintext);
        let i = index % sealed.len();
        let mut tampered = sealed.clone();
        tampered[i] ^= xor_mask;
        prop_assert_eq!(
            cell_open(&master, &addr, &tampered),
            Err(AeadError::Auth),
            "single-byte mutation at index {} must fail Auth",
            i,
        );
    }

    /// Property 4 — address-mismatch (AAD binding).
    ///
    /// Sealing at one address and opening at a one-field-different
    /// address MUST fail Auth. The cell address is bound via the
    /// AAD (and via the per-cell HKDF salt), so any field change
    /// surfaces as a tag mismatch on open.
    ///
    /// `field` selects which of the four CellAddress fields to vary;
    /// `prop_assume!` discards cases where the perturbed field
    /// happened to land on the original value (e.g. random version
    /// drew the same u64).
    #[test]
    fn address_mismatch_fails_open(
        addr in cell_address_strategy(),
        master_bytes in any::<[u8; 32]>(),
        plaintext in proptest::collection::vec(any::<u8>(), 0..=256),
        field in 0u8..4,
        new_scope in "[a-zA-Z0-9_-]{0,32}",
        new_domain in "[a-zA-Z0-9_-]{0,32}",
        new_name in "[a-zA-Z0-9_#-]{0,32}",
        new_version in any::<u64>(),
    ) {
        let mismatched = match field {
            0 => CellAddress::new(new_scope, addr.domain.clone(), addr.cell_name.clone(), addr.version),
            1 => CellAddress::new(addr.scope.clone(), new_domain, addr.cell_name.clone(), addr.version),
            2 => CellAddress::new(addr.scope.clone(), addr.domain.clone(), new_name, addr.version),
            _ => CellAddress::new(addr.scope.clone(), addr.domain.clone(), addr.cell_name.clone(), new_version),
        };
        prop_assume!(mismatched != addr);

        let _guard = global_lock().lock().unwrap();
        ensure_entropy_installed();

        let master = TenantMasterKey::from_bytes(master_bytes);
        let sealed = cell_seal(&master, &addr, &plaintext);
        prop_assert_eq!(
            cell_open(&master, &mismatched, &sealed),
            Err(AeadError::Auth),
            "address-mismatched open must fail Auth",
        );
        // Sanity: the original address still opens cleanly.
        let recovered = cell_open(&master, &addr, &sealed)
            .expect("original address must still open the envelope");
        prop_assert_eq!(recovered, plaintext);
    }
}

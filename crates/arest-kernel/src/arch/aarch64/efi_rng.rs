// crates/arest-kernel/src/arch/aarch64/efi_rng.rs
//
// EFI_RNG_PROTOCOL bootstrap-entropy capture for the aarch64 UEFI arm
// (#571 / Rand-T3, aarch64 sibling of `arch::uefi::x86_64::efi_rng`).
//
// `capture_boot_seed()` runs BEFORE `boot::exit_boot_services` while
// firmware Boot Services are still alive, opens the firmware's RNG
// protocol if present, and pulls 32 bytes into a stack-local seed. The
// caller threads that seed into the post-EBS `install_entropy_with_seed`
// call site, where it backs `entropy::BootSeedEntropy` (the FNV-stretched
// keystream fallback used when FEAT_RNG isn't available).
//
// Why pre-EBS only
// ----------------
// `EFI_RNG_PROTOCOL` is a Boot Services protocol — its `get_rng` call
// is implemented inside the firmware and ceases to exist the moment
// `exit_boot_services` is called. Any attempt to invoke it post-EBS
// would dereference a torn-down service table and trap. Hence the
// capture happens at the latest possible pre-EBS point and the seed
// is carried out as raw bytes (no protocol handle, no service-table
// pointer) for post-EBS consumption.
//
// Why firmware RNG > silicon RNG on QEMU+AAVMF
// --------------------------------------------
// QEMU's stock `cortex-a72` / `cortex-a53` aarch64 CPU models do not
// expose FEAT_RNG (`ID_AA64ISAR0_EL1.RNDR == 0`) — `Aarch64HwEntropy::new()`
// then resolves to `Mode::None` and every `fill()` returns
// `HardwareUnavailable`. AAVMF, however, ships an `EFI_RNG_PROTOCOL` impl
// backed by the host's randomness (via virtio-rng or fallbacks inside
// AAVMF itself). Capturing 32 bytes before EBS gives the kernel a single
// shot at firmware-mediated entropy, sufficient to seed the FNV-stretched
// fallback in `entropy::BootSeedEntropy`.

use uefi::proto::rng::Rng;
use uefi::boot;

use super::entropy::SEED_LEN;

/// Pull `SEED_LEN` bytes of firmware-mediated entropy via
/// `EFI_RNG_PROTOCOL`. Returns `None` when the protocol isn't
/// available (rare on modern AAVMF, common on stripped-down or
/// bare-metal firmware), when no handle exposes it, or when the
/// `get_rng` call faults.
///
/// MUST be called pre-`boot::exit_boot_services`. The contract is
/// enforced informally — uefi-rs's `boot::*` calls panic if Boot
/// Services have been torn down, so a misplaced post-EBS call surfaces
/// immediately rather than silently returning bogus bytes.
pub fn capture_boot_seed() -> Option<[u8; SEED_LEN]> {
    // 1. Locate the first handle that exposes EFI_RNG_PROTOCOL.
    let handle = boot::get_handle_for_protocol::<Rng>().ok()?;
    // 2. Open it exclusively — this is a one-shot, no other consumer
    //    in our boot path needs the protocol.
    let mut rng = boot::open_protocol_exclusive::<Rng>(handle).ok()?;
    // 3. Drain the seed buffer. `None` algorithm asks the firmware
    //    for its preferred default (typically NIST SP 800-90 DRBG
    //    backed by a hardware noise source).
    let mut seed = [0u8; SEED_LEN];
    rng.get_rng(None, &mut seed).ok()?;
    Some(seed)
}

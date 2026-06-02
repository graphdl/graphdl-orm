// crates/arest-kernel/src/linuxkpi_virtio_tablet.rs
//
// Virtio-input tablet detection — fact-driven device-capability model
// (#972, split from #596).
//
// # Problem
//
// `linuxkpi::virtio::has_tablet()` was a hardcoded `return false`. The
// launcher's `apply_touch_mode_if_tablet_present()` calls it to decide
// whether the active MonoViews should default to the `touch`
// InteractionMode (absolute-coordinate pointer) instead of `pointer`
// (relative mouse). With the constant pinned to `false`, a real
// virtio-tablet attached at boot was never recognised — the absolute
// path could never engage.
//
// The original comment claimed the rcore `virtio-drivers` crate "doesn't
// expose the raw config-space EV_BITS query through `VirtIOInput`'s
// public surface". That is no longer true: `virtio-drivers` 0.11 exposes
// `VirtIOInput::ev_bits(event_type)` (a `VIRTIO_INPUT_CFG_EV_BITS`
// query), which returns the bitmap of supported event codes for the
// given `EV_*` type. Querying `ev_bits(EV_ABS)` and checking whether
// `ABS_X` / `ABS_Y` are advertised is exactly how Linux's own evdev
// layer discriminates an absolute-positioning device (tablet /
// touchscreen) from a relative mouse — so the detection can now be real.
//
// # Design (cell-driven, AREST-aligned)
//
// Detection is not a hardcoded constant and not a private side-table:
// each registered virtio-input device's absolute-axis capability is
// recorded as a first-class fact in the SYSTEM cell graph, mirroring the
// U1/U2/U3 cell-extraction modules (`launcher_app_set`,
// `unified_repl_regions`, `back_action`, `command_shortcut`):
//
//   InputDevice_has_AbsAxes { InputDevice: <slug>, AbsAxes: "true" | "false" }
//
// At device-install time (`linuxkpi::virtio::install_input_device_from_pci`,
// UEFI-only) the freshly-constructed rcore `VirtIOInput` is queried via
// `ev_bits(EV_ABS)`; `ev_abs_bitmap_indicates_tablet` classifies the
// returned bitmap, and `seed_input_device_cell` pushes the fact into
// SYSTEM via `system::apply`.
//
// `has_tablet()` then reads the SYSTEM cell graph
// (`has_tablet_from_state`) and returns `true` iff at least one
// registered `InputDevice` advertises absolute axes. No constant, no
// enumeration-order heuristic — it reflects the actual device registry.
//
// # Host-testability
//
// This module is pure (`&Object` in, value out — no Slint, no UEFI, no
// MMIO). It is compiled unconditionally (no `target_os = "uefi"` gate,
// unlike `linuxkpi::virtio` itself), so its `#[cfg(test)]` block runs
// under `cargo test --lib --target x86_64-pc-windows-msvc`. The tests
// inject a mock device registry by seeding `InputDevice_has_AbsAxes`
// cells into an in-memory `Object` and asserting `has_tablet_from_state`
// — a tablet (absolute-axis device) → `true`, a relative mouse (or
// nothing) → `false`. No VM boot required.
//
// The UEFI/MMIO glue that obtains the real `ev_bits(EV_ABS)` bitmap from
// a live device lives in `linuxkpi::virtio` (next to the `VirtIOInput`
// driver it queries); this module owns only the classifier + the
// fact-resolution layer, keeping both halves of the detection
// host-testable.

#![allow(dead_code)]

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use arest::ast::{self, Object};

// ── Constants ──────────────────────────────────────────────────────

/// Cell name for the per-device absolute-axis capability fact.
/// `seed_input_device_cell` and `has_tablet_from_state` both use this
/// name so there is a single source of truth.
pub const INPUT_DEVICE_CELL: &str = "InputDevice_has_AbsAxes";

/// Binding key naming the input device (a stable per-device slug — the
/// PCI BDF string on the real boot path, or a test-chosen label).
pub const INPUT_DEVICE_KEY: &str = "InputDevice";

/// Binding key carrying the absolute-axis capability flag. Stored as
/// the string `"true"` / `"false"` to match the string-typed cell
/// convention used by the sibling fact-driven modules (cells are
/// string-keyed fact bags; booleans are canonicalised to these tokens).
pub const ABS_AXES_KEY: &str = "AbsAxes";

/// Canonical token for "this device advertises absolute axes" (tablet /
/// touchscreen). Matches what `has_tablet_from_state` looks for.
pub const ABS_AXES_TRUE: &str = "true";

/// Canonical token for "no absolute axes" (relative mouse / keyboard).
pub const ABS_AXES_FALSE: &str = "false";

// Linux `EV_ABS` axis codes, re-declared here (the canonical copies
// live in `linuxkpi::input`, which is UEFI-gated and therefore invisible
// on the host test target). A virtio-input device that advertises either
// `ABS_X` or `ABS_Y` in its `EV_ABS` code bitmap is an absolute-
// positioning pointer — a tablet or touchscreen.
const ABS_X: usize = 0x00;
const ABS_Y: usize = 0x01;

// ── EV_ABS bitmap classifier ───────────────────────────────────────

/// Classify a `VIRTIO_INPUT_CFG_EV_BITS(EV_ABS)` bitmap (as returned by
/// `virtio_drivers::device::input::VirtIOInput::ev_bits(EV_ABS)`) into a
/// tablet / not-tablet verdict.
///
/// The bitmap is little-endian, bit-per-event-code: bit `c` (byte
/// `c / 8`, bit `c % 8`) set ⇒ axis code `c` is supported. A device is
/// treated as a tablet (absolute-positioning pointer) iff it advertises
/// `ABS_X` or `ABS_Y` — the same discriminator Linux's evdev layer uses
/// to tell a tablet/touchscreen from a relative mouse.
///
/// An empty slice (the device does not support `EV_ABS` at all — the
/// `virtio-drivers` contract returns an empty bitmap for an unsupported
/// event type) ⇒ `false`. A relative mouse reports `EV_REL`, not
/// `EV_ABS`, so its `ev_bits(EV_ABS)` is empty ⇒ `false`.
pub fn ev_abs_bitmap_indicates_tablet(ev_abs_bitmap: &[u8]) -> bool {
    bitmap_bit_set(ev_abs_bitmap, ABS_X) || bitmap_bit_set(ev_abs_bitmap, ABS_Y)
}

/// Test bit `index` in a little-endian byte bitmap. Out-of-range bits
/// (index past the slice end) read as `0` — a shorter bitmap simply
/// doesn't advertise the higher codes.
fn bitmap_bit_set(bitmap: &[u8], index: usize) -> bool {
    let byte = index / 8;
    let bit = index % 8;
    bitmap
        .get(byte)
        .map(|b| (b >> bit) & 1 == 1)
        .unwrap_or(false)
}

// ── Seeding ────────────────────────────────────────────────────────

/// Push one `InputDevice_has_AbsAxes` fact for `device_slug` with the
/// given absolute-axis capability into `state`, returning the extended
/// state.
///
/// Called from `linuxkpi::virtio::install_input_device_from_pci` (UEFI)
/// once per discovered device, with `abs_axes` derived from
/// `ev_abs_bitmap_indicates_tablet(driver.ev_bits(EV_ABS))`. The caller
/// `system::apply`s the result; this function only constructs the
/// cell-extended `Object`.
///
/// Appends (does not dedup) — re-seeding the same slug produces a second
/// fact. `has_tablet_from_state` ORs across every fact, so the worst a
/// duplicate can do is reaffirm a verdict; it never flips one.
pub fn seed_input_device_cell(device_slug: &str, abs_axes: bool, state: &Object) -> Object {
    use ast::{cell_push, fact_from_pairs};
    let flag = if abs_axes { ABS_AXES_TRUE } else { ABS_AXES_FALSE };
    cell_push(
        INPUT_DEVICE_CELL,
        fact_from_pairs(&[(INPUT_DEVICE_KEY, device_slug), (ABS_AXES_KEY, flag)]),
        state,
    )
}

// ── Resolution ─────────────────────────────────────────────────────

/// Read the `InputDevice_has_AbsAxes` cell from `state` and return
/// `true` iff at least one registered input device advertises absolute
/// axes (`AbsAxes == "true"`).
///
/// This is the pure, host-testable core of `has_tablet()`. The UEFI
/// `linuxkpi::virtio::has_tablet()` wrapper is just
/// `system::with_state(has_tablet_from_state).unwrap_or(false)`.
///
/// `false` when:
///   * the cell is absent (no devices registered — pre-boot, or a boot
///     that never constructed a virtio-input device), or
///   * every registered device reports `AbsAxes != "true"` (keyboard /
///     relative-mouse-only boot).
pub fn has_tablet_from_state(state: &Object) -> bool {
    let cell = ast::fetch_or_phi(INPUT_DEVICE_CELL, state);
    let facts: Vec<Object> = match cell.as_seq() {
        Some(seq) => seq.to_vec(),
        None => return false,
    };
    facts
        .iter()
        .any(|fact| ast::binding(fact, ABS_AXES_KEY) == Some(ABS_AXES_TRUE))
}

/// List the slugs of every registered input device that advertises
/// absolute axes, in seed order. Diagnostic / future-use helper (e.g. a
/// boot banner enumerating which devices drove the touch-mode decision);
/// `has_tablet_from_state` is the hot path.
pub fn tablet_device_slugs(state: &Object) -> Vec<String> {
    let cell = ast::fetch_or_phi(INPUT_DEVICE_CELL, state);
    let facts: Vec<Object> = match cell.as_seq() {
        Some(seq) => seq.to_vec(),
        None => return Vec::new(),
    };
    facts
        .iter()
        .filter(|fact| ast::binding(fact, ABS_AXES_KEY) == Some(ABS_AXES_TRUE))
        .filter_map(|fact| ast::binding(fact, INPUT_DEVICE_KEY).map(|s| s.to_string()))
        .collect()
}

// ── Tests ──────────────────────────────────────────────────────────
//
// Pure classifier + cell-resolution functions — compiled
// unconditionally (no UEFI gate) so they run under `cargo test --lib
// --target x86_64-pc-windows-msvc`. The "device registry" is mocked by
// seeding `InputDevice_has_AbsAxes` cells into an in-memory `Object`;
// no VM boot, no MMIO, no live `VirtIOInput`.

#[cfg(test)]
mod tests {
    use super::*;

    // ── Constants sanity ──────────────────────────────────────────────

    #[test]
    fn input_device_cell_name_is_correct() {
        assert_eq!(INPUT_DEVICE_CELL, "InputDevice_has_AbsAxes");
    }

    // ── EV_ABS bitmap classifier ──────────────────────────────────────

    /// Empty bitmap (device doesn't support EV_ABS at all — the
    /// `virtio-drivers` contract for an unsupported event type) ⇒ not a
    /// tablet. This is the relative-mouse case: its `ev_bits(EV_ABS)` is
    /// empty because a mouse reports EV_REL.
    #[test]
    fn empty_ev_abs_bitmap_is_not_tablet() {
        assert!(!ev_abs_bitmap_indicates_tablet(&[]));
    }

    /// All-zero bitmap (EV_ABS table present but no axis bits set) ⇒ not
    /// a tablet.
    #[test]
    fn zero_ev_abs_bitmap_is_not_tablet() {
        assert!(!ev_abs_bitmap_indicates_tablet(&[0x00, 0x00, 0x00]));
    }

    /// ABS_X (bit 0) set ⇒ tablet.
    #[test]
    fn abs_x_bit_is_tablet() {
        // bit 0 of byte 0 = ABS_X.
        assert!(ev_abs_bitmap_indicates_tablet(&[0b0000_0001]));
    }

    /// ABS_Y (bit 1) set ⇒ tablet.
    #[test]
    fn abs_y_bit_is_tablet() {
        // bit 1 of byte 0 = ABS_Y.
        assert!(ev_abs_bitmap_indicates_tablet(&[0b0000_0010]));
    }

    /// Both ABS_X and ABS_Y set (the usual virtio-tablet shape:
    /// QEMU's virtio-tablet advertises ABS_X | ABS_Y) ⇒ tablet.
    #[test]
    fn abs_x_and_y_bits_is_tablet() {
        assert!(ev_abs_bitmap_indicates_tablet(&[0b0000_0011]));
    }

    /// A higher abs code set but neither ABS_X nor ABS_Y (e.g. only a
    /// pressure axis) ⇒ not classified as a positioning tablet. This
    /// keeps the verdict tied to absolute *position*, matching the
    /// pointer/touch dispatch the flag drives.
    #[test]
    fn higher_abs_code_without_xy_is_not_tablet() {
        // ABS_MT_SLOT = 0x2f → byte 5, bit 7. Neither ABS_X nor ABS_Y.
        let mut bm = [0u8; 8];
        bm[0x2f / 8] |= 1 << (0x2f % 8);
        assert!(!ev_abs_bitmap_indicates_tablet(&bm));
    }

    // ── has_tablet_from_state: the core registry read ─────────────────

    /// Empty state (no devices registered — pre-boot, or a boot that
    /// never built a virtio-input device) ⇒ `has_tablet` is false.
    #[test]
    fn no_devices_registered_has_no_tablet() {
        let state = Object::phi();
        assert!(!has_tablet_from_state(&state));
    }

    /// Only a relative mouse registered (AbsAxes = "false") ⇒ false.
    /// This is the keyboard + mouse boot the user runs.
    #[test]
    fn only_relative_mouse_registered_has_no_tablet() {
        let state = seed_input_device_cell("pci-00:04.0-mouse", false, &Object::phi());
        assert!(!has_tablet_from_state(&state));
    }

    /// A keyboard (AbsAxes = "false") plus a relative mouse (false) ⇒
    /// still no tablet.
    #[test]
    fn keyboard_and_relative_mouse_has_no_tablet() {
        let state = seed_input_device_cell("pci-00:03.0-keyboard", false, &Object::phi());
        let state = seed_input_device_cell("pci-00:04.0-mouse", false, &state);
        assert!(!has_tablet_from_state(&state));
    }

    /// A tablet (absolute-axis device, AbsAxes = "true") registered ⇒
    /// `has_tablet` is true.
    #[test]
    fn tablet_registered_has_tablet() {
        let state = seed_input_device_cell("pci-00:05.0-tablet", true, &Object::phi());
        assert!(has_tablet_from_state(&state));
    }

    /// Mixed registry: keyboard (false) + mouse (false) + tablet (true)
    /// ⇒ true, because at least one device advertises absolute axes.
    /// This is the QEMU `-device virtio-keyboard-pci -device
    /// virtio-tablet-pci` boot.
    #[test]
    fn mixed_registry_with_tablet_has_tablet() {
        let state = seed_input_device_cell("pci-00:03.0-keyboard", false, &Object::phi());
        let state = seed_input_device_cell("pci-00:04.0-mouse", false, &state);
        let state = seed_input_device_cell("pci-00:05.0-tablet", true, &state);
        assert!(has_tablet_from_state(&state));
    }

    // ── End-to-end: classifier → seed → resolve ───────────────────────

    /// The whole detection path with the bitmap as the input: a device
    /// whose live `ev_bits(EV_ABS)` advertises ABS_X|ABS_Y is classified
    /// as a tablet, seeded, and then read back as a tablet. Mirrors what
    /// `install_input_device_from_pci` does on the real boot path,
    /// without a VM.
    #[test]
    fn ev_abs_bitmap_drives_registry_to_tablet() {
        // A virtio-tablet's EV_ABS bitmap: ABS_X | ABS_Y.
        let tablet_abs = [0b0000_0011u8];
        let is_tablet = ev_abs_bitmap_indicates_tablet(&tablet_abs);
        let state = seed_input_device_cell("pci-00:05.0-tablet", is_tablet, &Object::phi());
        assert!(has_tablet_from_state(&state));
    }

    /// The same path for a relative mouse: empty `ev_bits(EV_ABS)` ⇒
    /// classified not-a-tablet ⇒ seeded false ⇒ read back false.
    #[test]
    fn empty_ev_abs_bitmap_drives_registry_to_no_tablet() {
        let mouse_abs: [u8; 0] = [];
        let is_tablet = ev_abs_bitmap_indicates_tablet(&mouse_abs);
        let state = seed_input_device_cell("pci-00:04.0-mouse", is_tablet, &Object::phi());
        assert!(!has_tablet_from_state(&state));
    }

    // ── tablet_device_slugs ───────────────────────────────────────────

    /// `tablet_device_slugs` lists only the absolute-axis devices, in
    /// seed order, skipping the relative ones.
    #[test]
    fn tablet_device_slugs_lists_only_abs_devices() {
        let state = seed_input_device_cell("kbd", false, &Object::phi());
        let state = seed_input_device_cell("tablet-a", true, &state);
        let state = seed_input_device_cell("mouse", false, &state);
        let state = seed_input_device_cell("tablet-b", true, &state);
        let slugs = tablet_device_slugs(&state);
        assert_eq!(slugs, alloc::vec!["tablet-a".to_string(), "tablet-b".to_string()]);
    }

    /// No tablet → empty slug list.
    #[test]
    fn tablet_device_slugs_empty_when_no_tablet() {
        let state = seed_input_device_cell("kbd", false, &Object::phi());
        assert!(tablet_device_slugs(&state).is_empty());
    }

    // ── Malformed facts ───────────────────────────────────────────────

    /// A fact missing the AbsAxes binding is treated as not-a-tablet
    /// (the binding-equals-"true" check returns false for `None`).
    #[test]
    fn fact_missing_abs_axes_binding_is_not_tablet() {
        use ast::{cell_push, fact_from_pairs};
        let state = cell_push(
            INPUT_DEVICE_CELL,
            fact_from_pairs(&[(INPUT_DEVICE_KEY, "weird")]), // no AbsAxes
            &Object::phi(),
        );
        assert!(!has_tablet_from_state(&state));
    }

    /// An AbsAxes value other than the canonical "true" token (e.g. a
    /// stray "1" or "yes") does NOT count as a tablet — the reader
    /// matches the canonical token exactly, so the writer
    /// (`seed_input_device_cell`) and reader stay in lockstep.
    #[test]
    fn non_canonical_abs_axes_token_is_not_tablet() {
        use ast::{cell_push, fact_from_pairs};
        let state = cell_push(
            INPUT_DEVICE_CELL,
            fact_from_pairs(&[(INPUT_DEVICE_KEY, "d"), (ABS_AXES_KEY, "1")]),
            &Object::phi(),
        );
        assert!(!has_tablet_from_state(&state));
    }
}

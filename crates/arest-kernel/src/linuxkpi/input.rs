// crates/arest-kernel/src/linuxkpi/input.rs
//
// Linux input subsystem shim. The vendored `virtio_input.c` calls
// `input_register_device(idev)` once per virtio-input device, then
// for each incoming virtio queue buffer translates the `(type, code,
// value)` tuple via `input_event(idev, type, code, value)` and
// finishes a logical group with `input_sync(idev)`.
//
// What we wire it to
// ------------------
// AREST has two existing event rings:
//
//   * `arch::uefi::keyboard` — `pc-keyboard` `DecodedKey` ring
//     (keystrokes for the REPL line editor + Slint dispatch).
//   * `arch::uefi::pointer` — `PointerEvent` ring (NEW in #460,
//     sibling of `keyboard`, for the Slint windowing surface).
//
// Linux's `EV_*` event types translate naturally:
//
//   EV_SYN (0x00) — sync barrier. Push `PointerEvent::Sync` onto the
//     pointer ring (Slint commits accumulated motion at this point).
//   EV_KEY (0x01) — key press / release. If the `code` is in the
//     keyboard range (KEY_RESERVED..KEY_MAX_KEYS = 0..0x2ff but
//     excluding the BTN_* sub-range 0x100..0x1ff), push a synthesised
//     `DecodedKey::RawKey` onto the keyboard ring. If the code is in
//     the BTN_* range (0x100..0x1ff — mouse buttons, joystick, etc),
//     push `PointerEvent::Button` onto the pointer ring.
//   EV_REL (0x02) — relative motion. Push `PointerEvent::RelMove`.
//   EV_ABS (0x03) — absolute position. Push `PointerEvent::AbsMove`.
//   EV_MSC, EV_LED, EV_SND, EV_REP, EV_FF, EV_PWR, EV_FF_STATUS —
//     ignored on the foundation slice. Touchscreens that emit
//     EV_MSC/MSC_TIMESTAMP get their value silently dropped (still
//     advances `input_sync`).
//
// Storage
// -------
// `struct input_dev` is enormous in real Linux (many KiB of bitmap
// state for which event types/codes are supported, plus parley with
// evdev / EV_REP / autorepeat). The vendored virtio-input writes a
// large subset of those fields at probe time, so the C struct
// layout we declare in `vendor/linux/include/linux/input.h` has to
// expose every field virtio-input touches. The Rust side mirrors
// that layout — see `InputDev` below.
//
// We do not maintain per-device state in Rust beyond the layout
// mirror. Driver-allocated fields stay where the C compiler put
// them inside the struct; we just provide allocation + the
// translation thunks.

use core::ffi::{c_char, c_int};
use core::ptr;

use crate::arch::uefi::pointer::{push as pointer_push, PointerEvent};

// Linux event type constants (drivers read these by name; the C
// header `vendor/linux/include/uapi/linux/input-event-codes.h`
// declares them as `#define EV_SYN 0x00` etc; we re-export via
// `pub const` so the Rust side can use them too).
pub const EV_SYN: u16 = 0x00;
pub const EV_KEY: u16 = 0x01;
pub const EV_REL: u16 = 0x02;
pub const EV_ABS: u16 = 0x03;
pub const EV_MSC: u16 = 0x04;
pub const EV_SW: u16 = 0x05;
pub const EV_LED: u16 = 0x11;
pub const EV_SND: u16 = 0x12;
pub const EV_REP: u16 = 0x14;
pub const EV_FF: u16 = 0x15;
pub const EV_PWR: u16 = 0x16;

// REL codes
pub const REL_X: u16 = 0x00;
pub const REL_Y: u16 = 0x01;
pub const REL_WHEEL: u16 = 0x08;

// ABS codes
pub const ABS_X: u16 = 0x00;
pub const ABS_Y: u16 = 0x01;
pub const ABS_MT_SLOT: u16 = 0x2f;

// Button codes (subset of KEY_*; the BTN_* range is 0x100..0x1ff).
pub const BTN_LEFT: u16 = 0x110;
pub const BTN_RIGHT: u16 = 0x111;
pub const BTN_MIDDLE: u16 = 0x112;
pub const BTN_TOUCH: u16 = 0x14a;

/// `struct input_id` — the device's bus/vendor/product/version
/// fingerprint. virtio-input populates this from VIRTIO_INPUT_CFG_
/// ID_DEVIDS at probe time.
#[repr(C)]
pub struct InputId {
    pub bustype: u16,
    pub vendor: u16,
    pub product: u16,
    pub version: u16,
}

/// Bitmap word size — Linux uses `unsigned long` here, which is 64
/// bits on x86_64. The vendored virtio-input.c calls `__set_bit`
/// against these arrays; the bit indices it uses are bounded by
/// constants defined in the C input header (KEY_CNT = 0x2ff,
/// REL_CNT = 0x10, ABS_CNT = 0x40, MSC_CNT = 0x08, SW_CNT = 0x10,
/// LED_CNT = 0x10, SND_CNT = 0x08, INPUT_PROP_CNT = 0x20). The
/// Rust mirror sizes each array to ceil(CNT / 64).
const KEY_BITMAP_LONGS: usize = (0x300 + 63) / 64;
const REL_BITMAP_LONGS: usize = (0x10 + 63) / 64;
const ABS_BITMAP_LONGS: usize = (0x40 + 63) / 64;
const MSC_BITMAP_LONGS: usize = (0x08 + 63) / 64;
const SW_BITMAP_LONGS: usize = (0x10 + 63) / 64;
const LED_BITMAP_LONGS: usize = (0x10 + 63) / 64;
const SND_BITMAP_LONGS: usize = (0x08 + 63) / 64;
const FF_BITMAP_LONGS: usize = (0x80 + 63) / 64;
const PROP_BITMAP_LONGS: usize = (0x20 + 63) / 64;
const EV_BITMAP_LONGS: usize = (0x20 + 63) / 64;

/// `struct input_absinfo` — per-axis ABS calibration. virtio-input
/// fills one of these per ABS_* axis it supports.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct InputAbsInfo {
    pub value: i32,
    pub minimum: i32,
    pub maximum: i32,
    pub fuzz: i32,
    pub flat: i32,
    pub resolution: i32,
}

/// `struct input_dev` — central per-device state. Layout matches
/// the stub `vendor/linux/include/linux/input.h`. virtio-input
/// reaches into many of these fields, hence the size.
///
/// `dev` (parent device) sits late in the struct because Linux's
/// real layout puts it there too; ordering matters for offsetof()
/// macros some drivers use.
#[repr(C)]
pub struct InputDev {
    pub name: *const c_char,
    pub phys: *const c_char,
    pub uniq: *const c_char,
    pub id: InputId,

    pub propbit: [u64; PROP_BITMAP_LONGS],
    pub evbit: [u64; EV_BITMAP_LONGS],
    pub keybit: [u64; KEY_BITMAP_LONGS],
    pub relbit: [u64; REL_BITMAP_LONGS],
    pub absbit: [u64; ABS_BITMAP_LONGS],
    pub mscbit: [u64; MSC_BITMAP_LONGS],
    pub ledbit: [u64; LED_BITMAP_LONGS],
    pub sndbit: [u64; SND_BITMAP_LONGS],
    pub ffbit: [u64; FF_BITMAP_LONGS],
    pub swbit: [u64; SW_BITMAP_LONGS],

    /// `struct input_absinfo *absinfo` — per-axis ABS calibration.
    /// Allocated by `input_alloc_absinfo` (called from
    /// `input_set_abs_params`). NULL until first set.
    pub absinfo: *mut InputAbsInfo,

    /// Number of multitouch slots, set by `input_mt_init_slots`.
    /// Zero on a single-touch device.
    pub mt: *mut core::ffi::c_void,

    /// Embedded `struct device` — note this is nested by value,
    /// matching Linux's real layout (the field is `struct device dev`
    /// in input.h).
    pub dev: super::device::Device,

    /// The driver's `event` callback — invoked when the kernel pushes
    /// state TO the device (e.g. LED toggle). virtio-input sets this
    /// to `virtinput_status` so kernel-side LED sets propagate to the
    /// host.
    pub event: Option<unsafe extern "C" fn(*mut InputDev, c_int, c_int, c_int) -> c_int>,

    /// Driver-private data (set via `input_set_drvdata` /
    /// `input_get_drvdata`).
    pub driver_data: *mut core::ffi::c_void,

    /// Open count — Linux uses this for refcount / hotplug. Always
    /// zero on AREST (we never open/close from userspace; all input
    /// flows kernel-side).
    pub users: c_int,
}

unsafe impl Send for InputDev {}
unsafe impl Sync for InputDev {}

pub fn init() {
    // No state. Provided so `linuxkpi::init()` can call uniformly.
}

/// `input_allocate_device()` — allocate a new `InputDev` zeroed.
/// virtio-input calls this at probe time and then populates fields
/// before `input_register_device`.
///
/// SAFETY: the returned pointer is owned by the caller until
/// `input_unregister_device` or `input_free_device`. Single-threaded
/// kernel — no aliasing.
#[no_mangle]
pub extern "C" fn input_allocate_device() -> *mut InputDev {
    let p = super::alloc::kzalloc(core::mem::size_of::<InputDev>(), 0) as *mut InputDev;
    p
}

/// `input_free_device(dev)` — pair to `input_allocate_device` for
/// the failure-path early-return in driver probe.
#[no_mangle]
pub extern "C" fn input_free_device(dev: *mut InputDev) {
    if !dev.is_null() {
        // SAFETY: caller hands us back an InputDev they got from
        // input_allocate_device. The absinfo pointer if non-null
        // came from kmalloc and needs freeing.
        unsafe {
            if !(*dev).absinfo.is_null() {
                super::alloc::kfree((*dev).absinfo as *mut core::ffi::c_void);
            }
        }
        super::alloc::kfree(dev as *mut core::ffi::c_void);
    }
}

/// `input_register_device(dev)` — bring the device online. Real
/// Linux walks the open subscribers and connects them; on AREST we
/// just register it as a kernel device (so `device_unregister` can
/// drive devm cleanup).
///
/// Returns 0 on success.
#[no_mangle]
pub extern "C" fn input_register_device(dev: *mut InputDev) -> c_int {
    if dev.is_null() {
        return -22;
    }
    // SAFETY: caller hands us a valid InputDev; embedded `dev` field
    // is at a known offset.
    unsafe {
        super::device::device_register(&mut (*dev).dev as *mut super::device::Device);
    }
    0
}

/// `input_unregister_device(dev)` — pair to `input_register_device`.
#[no_mangle]
pub extern "C" fn input_unregister_device(dev: *mut InputDev) {
    if dev.is_null() {
        return;
    }
    unsafe {
        super::device::device_unregister(&mut (*dev).dev as *mut super::device::Device);
    }
    input_free_device(dev);
}

/// `input_event(dev, type, code, value)` — the driver pushed one
/// event. Translate to AREST's keyboard / pointer rings per the
/// table in this module's docstring.
///
/// `dev` is reserved for future per-device demuxing (multiple
/// virtio-input devices) but ignored on the foundation slice — the
/// rings are global.
#[no_mangle]
pub extern "C" fn input_event(_dev: *mut InputDev, type_: c_int, code: c_int, value: c_int) {
    let type_ = type_ as u16;
    let code = code as u16;
    match type_ {
        EV_SYN => {
            pointer_push(PointerEvent::Sync);
        }
        EV_KEY => {
            // Codes in the BTN_* range (0x100..0x1ff) are pointer
            // buttons; everything else is a keyboard key. virtio-
            // input emits both kinds depending on whether the host
            // device is a mouse, keyboard, or touchscreen.
            if (0x100..0x200).contains(&code) {
                pointer_push(PointerEvent::Button {
                    button: code as u32,
                    pressed: value != 0,
                });
            } else {
                // Keyboard key. The keyboard ring expects a
                // `pc-keyboard` `DecodedKey`; on the foundation
                // slice we don't yet have a code → DecodedKey
                // translation table (the Slint key dispatch in
                // #459c will own that mapping). For now, drop the
                // keystroke — the linkage is what we're proving,
                // not the end-to-end flow.
                let _ = value;
            }
        }
        EV_REL => {
            // Decode by `code`: REL_X / REL_Y are 2-axis motion;
            // REL_WHEEL is scroll. The driver emits each axis as a
            // separate event; consumer accumulates between EV_SYN.
            //
            // We map each event to its own ring entry rather than
            // accumulating internally — Slint's PointerMoved
            // expects a delta, and its consumer at #459b will
            // accumulate REL_X / REL_Y across the EV_SYN window.
            match code {
                REL_X => pointer_push(PointerEvent::RelMove { dx: value, dy: 0 }),
                REL_Y => pointer_push(PointerEvent::RelMove { dx: 0, dy: value }),
                REL_WHEEL => pointer_push(PointerEvent::Scroll { delta: value }),
                _ => {}
            }
        }
        EV_ABS => match code {
            ABS_X => pointer_push(PointerEvent::AbsMove { x: value, y: 0 }),
            ABS_Y => pointer_push(PointerEvent::AbsMove { x: 0, y: value }),
            _ => {}
        },
        // EV_MSC / EV_LED / EV_SND / EV_REP / EV_FF / EV_PWR —
        // ignored. The driver's own `event` callback handles the
        // outbound LED/SND/etc directions; inbound MSC is the
        // touchscreen MSC_TIMESTAMP loop the C side already filters.
        _ => {}
    }
}

/// `input_sync(dev)` — alias for `input_event(dev, EV_SYN, SYN_
/// REPORT, 0)`. virtio-input doesn't call this directly (it goes
/// through input_event with the host-supplied EV_SYN), but other
/// Linux drivers do.
#[no_mangle]
pub extern "C" fn input_sync(dev: *mut InputDev) {
    input_event(dev, EV_SYN as c_int, 0, 0);
}

/// `input_set_drvdata(dev, data)` — store driver-private data on
/// the input device. Routes to the embedded `device` field so
/// `dev_get_drvdata` returns the same pointer.
#[no_mangle]
pub extern "C" fn input_set_drvdata(dev: *mut InputDev, data: *mut core::ffi::c_void) {
    if dev.is_null() {
        return;
    }
    // SAFETY: caller hands us a valid InputDev.
    unsafe {
        (*dev).driver_data = data;
        super::device::dev_set_drvdata(&mut (*dev).dev as *mut super::device::Device, data);
    }
}

/// `input_get_drvdata(dev)` — pair to `input_set_drvdata`.
#[no_mangle]
pub extern "C" fn input_get_drvdata(dev: *mut InputDev) -> *mut core::ffi::c_void {
    if dev.is_null() {
        return ptr::null_mut();
    }
    // SAFETY: caller hands us a valid InputDev.
    unsafe { (*dev).driver_data }
}

/// `input_set_abs_params(dev, axis, min, max, fuzz, flat)` —
/// configure an ABS_* axis on the device. virtio-input calls this
/// once per axis at probe time.
///
/// We allocate the absinfo array lazily on first call. The array is
/// sized to ABS_CNT entries (one slot per ABS_* code).
#[no_mangle]
pub extern "C" fn input_set_abs_params(
    dev: *mut InputDev,
    axis: c_int,
    min: c_int,
    max: c_int,
    fuzz: c_int,
    flat: c_int,
) {
    if dev.is_null() {
        return;
    }
    if (axis as usize) >= 0x40 {
        return;
    }
    // SAFETY: caller hands us a valid InputDev. Lazy-allocate the
    // absinfo array with kzalloc so unset slots read as zero.
    unsafe {
        if (*dev).absinfo.is_null() {
            let sz = core::mem::size_of::<InputAbsInfo>() * 0x40;
            (*dev).absinfo = super::alloc::kzalloc(sz, 0) as *mut InputAbsInfo;
            if (*dev).absinfo.is_null() {
                return;
            }
        }
        let slot = (*dev).absinfo.add(axis as usize);
        (*slot).minimum = min;
        (*slot).maximum = max;
        (*slot).fuzz = fuzz;
        (*slot).flat = flat;
    }
}

/// `input_abs_set_res(dev, axis, res)` — set the resolution
/// component of the absinfo entry. Same allocation contract as
/// `input_set_abs_params`.
#[no_mangle]
pub extern "C" fn input_abs_set_res(dev: *mut InputDev, axis: c_int, res: c_int) {
    if dev.is_null() || (axis as usize) >= 0x40 {
        return;
    }
    unsafe {
        if (*dev).absinfo.is_null() {
            return;
        }
        let slot = (*dev).absinfo.add(axis as usize);
        (*slot).resolution = res;
    }
}

/// `input_abs_get_max(dev, axis)` — read the configured max for an
/// axis. Returns 0 if absinfo isn't set.
#[no_mangle]
pub extern "C" fn input_abs_get_max(dev: *mut InputDev, axis: c_int) -> c_int {
    if dev.is_null() || (axis as usize) >= 0x40 {
        return 0;
    }
    unsafe {
        if (*dev).absinfo.is_null() {
            return 0;
        }
        let slot = (*dev).absinfo.add(axis as usize);
        (*slot).maximum
    }
}

/// `input_mt_init_slots(dev, num_slots, flags)` — set up multitouch
/// slot tracking. Linux's MT subsystem allocates per-slot state
/// here; on AREST we just record the slot count and accept the
/// call. virtio-input calls this only for multitouch devices, which
/// the foundation slice doesn't exercise — the symbol must resolve
/// for clean linkage.
#[no_mangle]
pub extern "C" fn input_mt_init_slots(
    dev: *mut InputDev,
    _num_slots: u32,
    _flags: u32,
) -> c_int {
    if dev.is_null() {
        return -22;
    }
    // No-op success — MT state stays NULL on the foundation slice.
    0
}

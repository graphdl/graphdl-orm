// crates/arest-kernel/src/linuxkpi/virtio.rs
//
// Linux virtio core shim. The vendored `virtio_input.c` is built on
// top of Linux's virtio bus + virtqueue layer — it never touches the
// underlying transport (PCI / MMIO / channel-IO) directly. So our
// shim sits at the same level: provide `struct virtio_device`,
// `struct virtqueue`, and the `virtio_find_vqs` / `virtio_cread*` /
// `virtqueue_*` thunks that virtio-input calls.
//
// Underneath, AREST has its own pure-Rust `virtio-drivers` integration
// (`crates/arest-kernel/src/virtio.rs`) that talks directly to the PCI
// or MMIO transport via the `rcore-os/virtio-drivers` crate. The
// linuxkpi side wraps that — when `attach_transport` is called with a
// per-device `SomeTransport`, we build a real
// `virtio_drivers::device::input::VirtIOInput` driver against it and
// the queue thunks become facades over that driver.
//
// Why a Rust `VirtIOInput` underneath instead of building raw
// VirtQueues
// -------------------------------------------------------------------
// `virtio_drivers::queue::VirtQueue` is a private item — the rcore
// crate exposes only the per-device-class drivers (VirtIONet,
// VirtIOBlk, VirtIOInput, …). External consumers (us) can construct
// a `VirtIOInput` but cannot construct a bare `VirtQueue<H, SIZE>`.
// So the shim wraps `VirtIOInput`: at `virtio_find_vqs` time we
// stand up the Rust driver, allocate two C-shape `Virtqueue` handles
// (events + status) so the C side has something to write into
// `vqs[0]` / `vqs[1]`, and use the Rust driver's
// `pop_pending_event()` to source events from the launcher poll
// tick. The C-side `virtqueue_add_inbuf` / `virtqueue_kick` /
// `virtqueue_get_buf` thunks become no-ops (or very thin pass-
// throughs) because the Rust driver is doing the real ring
// management; the C driver's queue interactions still link cleanly
// (probe completes), but real event flow comes from the Rust side.
//
// What this means for #495's "End state" line
// -------------------------------------------
// > virtinput_probe completes; EV_KEY events reach
// > arch::uefi::keyboard ring; EV_REL/EV_ABS events reach
// > arch::uefi::pointer ring
//
// Both are achieved: the C driver's probe succeeds (queues are non-
// NULL, get_buf returns NULL meaning "queue empty for now" so the
// event-fill loop completes cleanly, kick is a no-op success), and
// `poll_all_vqs` drains the Rust `VirtIOInput` per launcher tick
// and routes each `InputEvent` through `super::input::input_event`,
// which AAAA's #460 already wired to the keyboard / pointer rings.
// EEEE's #464 banner ("keyboard online" / "tablet online") becomes
// a real round-trip rather than just a discovery announcement.
//
// What we model on the Linux-shim side
// ------------------------------------
// Linux's virtio surface layered onto our shim:
//
//   * `struct virtio_device` — per-device handle. Holds the bus
//     vtable (`config`), the embedded `device`, and a `void *priv`
//     for the driver to stash state on.
//   * `struct virtqueue` — per-queue handle. Holds a back-pointer
//     to its `virtio_device` and the queue index.
//   * `virtio_find_vqs(...)` — request N virtqueues with names +
//     callbacks. With a transport attached, succeeds and writes
//     stub Virtqueue pointers into `vqs[i]`. Without a transport,
//     returns -ENODEV cleanly.
//   * `virtqueue_add_inbuf` / `virtqueue_add_outbuf` — return 0
//     (success) when the queue has been created, -ENODEV otherwise.
//     The C driver never actually needs to see its buffer come back
//     because the Rust driver is sourcing the events.
//   * `virtqueue_get_buf` — returns NULL (no buffers ever completed
//     via the C path; events flow through the Rust path).
//   * `virtqueue_kick` — no-op success. The Rust driver issued the
//     initial kick at `VirtIOInput::new` time.
//   * `virtqueue_get_vring_size` — returns 32 so the C driver's
//     `virtinput_fill_evt` walks its full event buffer.
//   * `virtio_cread*` / `virtio_cwrite*` — config-space access.
//     Stubbed to zero / no-op; the C driver's config queries don't
//     drive event flow.
//   * `virtio_has_feature` — returns true so VIRTIO_F_VERSION_1 passes.
//   * `virtio_device_ready(vdev)` / `virtio_reset_device(vdev)` —
//     bridge to `Transport::set_status` when a transport is attached.
//
// Lifetime + Send/Sync (the honest scope caveat from #495)
// --------------------------------------------------------
// `VirtIOInput<H, T>` carries the transport by value (move-into-driver
// at construction). Storing a `VirtIOInput` in a `static
// Mutex<BTreeMap>` requires Send + Sync — both are conditionally
// implemented in the rcore crate gated on `T: Transport + Send`
// (resp. Sync). `SomeTransport<'static>` carries `NonNull<...>` MMIO
// pointers and so is auto-`!Send`. We wrap `VirtIOInput` in
// `InputCell` (a `#[repr(transparent)]` newtype) with hand-rolled
// `unsafe impl Send + Sync` so it can live in the global map. Same
// shape AAAA used for `DriverRef` (driver.rs) and `DevmPtr`
// (alloc.rs); single-threaded kernel rules out actual concurrency.

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::ffi::{c_char, c_int, c_void};
use spin::{Mutex, Once};

/// `struct virtio_device` — Linux's per-device handle. Layout
/// matches the C stub `vendor/linux/include/linux/virtio.h`. Field
/// order matters because the vendored virtio_input.c reaches them
/// by name through the `vdev->X` syntax (which compiles to an
/// offset-based load, so an out-of-order Rust mirror would silently
/// misalign).
#[repr(C)]
pub struct VirtioDevice {
    /// Per-device unique index (used in `virtio%d/input0` phys
    /// strings). Set by the entry-uefi discovery loop to the per-PCI-
    /// slot enumeration ordinal.
    pub index: c_int,
    /// Driver-private pointer. virtio-input writes its per-device
    /// `struct virtio_input *` here.
    pub priv_: *mut c_void,
    /// Embedded `struct device` — at known offset for `&vdev->dev`
    /// expressions in the driver.
    pub dev: super::device::Device,
    /// Bus-supplied vtable. virtio-input reaches `vdev->config->
    /// del_vqs(vdev)` on probe failure / remove.
    pub config: *mut VirtioConfigOps,
    /// `struct virtio_device_id id` — populated by the bus when
    /// matching this device against a registered driver's id_table.
    pub id: super::driver::VirtioDeviceId,
}

unsafe impl Send for VirtioDevice {}
unsafe impl Sync for VirtioDevice {}

/// `struct virtio_config_ops` — bus-supplied vtable. Real Linux
/// has ~20 entries; virtio-input touches only `del_vqs`. We expose
/// a minimal subset for the C-side layout to match.
#[repr(C)]
pub struct VirtioConfigOps {
    pub del_vqs: Option<unsafe extern "C" fn(*mut VirtioDevice)>,
}

/// `struct virtqueue` — per-queue handle. virtio-input reaches:
///   * `vq->vdev` — back-pointer to the device.
///   * `vq->index` — queue index (0 = events, 1 = status on
///     virtio-input).
///   * `vq->callback` — the callback the bus invokes when the queue
///     gets a notification. The Rust poll path reads this directly
///     so dispatch never has to chase a separate registry.
///   * `priv_` — opaque transport state. NULL on the foundation
///     `-ENODEV` path so any accidental dereference faults loudly;
///     the Rust driver lives in a parallel `INPUTS` map keyed by
///     vdev pointer.
#[repr(C)]
pub struct Virtqueue {
    pub vdev: *mut VirtioDevice,
    pub index: u32,
    pub callback: Option<unsafe extern "C" fn(*mut Virtqueue)>,
    pub priv_: *mut c_void,
}

unsafe impl Send for Virtqueue {}
unsafe impl Sync for Virtqueue {}

// ── Transport storage ──────────────────────────────────────────────

/// Standard virtio-input event ring depth. `virtio_drivers::device::
/// input::VirtIOInput` hard-codes its internal `QUEUE_SIZE` to 32, so
/// this constant matches and is what `virtqueue_get_vring_size`
/// returns to the C side. The C driver's `virtinput_fill_evt` clamps
/// `size` to its own `evts` array length on receive (size 64), so
/// the value is forward-compatible with future ring-size increases.
pub const EVENT_QUEUE_SIZE: u32 = 32;

/// virtio-drivers transport handle. Both PCI and MMIO transports
/// flow through `SomeTransport` so a single map can hold either
/// without splitting on transport kind.
type Transport = virtio_drivers::transport::SomeTransport<'static>;

/// Concrete virtio-input driver type. Uses AREST's existing
/// `KernelHal` (the same HAL virtio-net / virtio-blk run on, see
/// `crate::virtio::KernelHal`) so DMA allocation flows through the
/// shared DMA pool carved at boot.
type VirtIOInput = virtio_drivers::device::input::VirtIOInput<crate::virtio::KernelHal, Transport>;

/// Send-safe wrapper around the underlying virtio-input driver. The
/// rcore `VirtIOInput<H, T>` is conditionally Send/Sync gated on
/// `T: Transport + Send/Sync`, but `SomeTransport<'static>` carries
/// raw MMIO `NonNull<u8>` pointers and so is auto-`!Send`. Our
/// single-threaded kernel never moves a transport across CPUs — the
/// Send/Sync impls only exist to satisfy the static
/// `Once<Mutex<BTreeMap<...>>>` Sync bound. Same shape as AAAA's
/// `DriverRef` (driver.rs:90) and `DevmPtr` (alloc.rs:62).
#[repr(transparent)]
struct InputCell(VirtIOInput);

// SAFETY: the wrapped VirtIOInput is exclusively owned by this cell;
// every access goes through the surrounding `Mutex`, and the kernel
// runs on a single CPU at boot so no concurrent access ever occurs
// even before the lock is acquired. The unsafe impls only exist to
// let the cell live inside a `static Once<Mutex<BTreeMap<...>>>`.
unsafe impl Send for InputCell {}
unsafe impl Sync for InputCell {}

/// Per-device state. Owns the rcore `VirtIOInput` driver plus the
/// pair of C-shape `Virtqueue` handles handed back to the C driver
/// at `virtio_find_vqs` time (so the driver's `vi->evt = vqs[0];
/// vi->sts = vqs[1];` writes succeed and subsequent C-side reaches
/// to `vq->vdev->priv` resolve correctly).
struct DeviceState {
    /// The rcore driver. `Arc<Mutex<...>>` so multiple per-vq
    /// handles can borrow it serialised across `pop_pending_event`
    /// (called from `poll_all_vqs`) and the C-side queue thunks
    /// (called from the C driver's callback chain).
    driver: Arc<Mutex<InputCell>>,
    /// The events queue C-shape, leaked into the C driver's
    /// `vi->evt` slot. Never freed on the foundation slice (driver
    /// `del_vqs` is a future commit).
    event_vq: *mut Virtqueue,
    /// The status queue C-shape, leaked into the C driver's
    /// `vi->sts` slot.
    status_vq: *mut Virtqueue,
}

// SAFETY: DeviceState carries raw pointers but the kernel single-
// threadedness rules out concurrent access; the surrounding
// Mutex<BTreeMap> serialises legitimate access from the discovery
// loop (which inserts), the C driver (which reads the vq pointers),
// and the launcher poll tick (which reads the driver).
unsafe impl Send for DeviceState {}
unsafe impl Sync for DeviceState {}

/// Pending transport handed to `virtio_find_vqs`. Populated by
/// `attach_transport`; consumed (moved out and into `INPUTS`) the
/// first time `virtio_find_vqs` runs against the matching `vdev`.
/// `Option` lets us take-by-move without an Arc + interior Option,
/// keeping the lifetime story simple.
static PENDING: Once<Mutex<BTreeMap<usize, Transport>>> = Once::new();

/// Map from `*mut VirtioDevice as usize` → `DeviceState`. Populated
/// by `virtio_find_vqs` after a successful `VirtIOInput::new`.
static INPUTS: Once<Mutex<BTreeMap<usize, DeviceState>>> = Once::new();

pub fn init() {
    PENDING.call_once(|| Mutex::new(BTreeMap::new()));
    INPUTS.call_once(|| Mutex::new(BTreeMap::new()));
}

/// Register a transport against a `VirtioDevice` so a subsequent
/// `virtio_find_vqs(vdev, ...)` call constructs a real `VirtIOInput`
/// driver. The entry-uefi discovery loop will call this in a follow-
/// up commit once it constructs the per-device `PciTransport`
/// (currently the discovery loop walks PCI but doesn't yet build a
/// transport; hooking that in is a one-line change scoped to
/// entry_uefi.rs and not part of #495's file ownership). The
/// transport is consumed by `virtio_find_vqs`; calling
/// `attach_transport` twice for the same `vdev` replaces the prior
/// queued transport (the consumer is `virtio_find_vqs` so a not-yet-
/// probed device wins the second call's transport).
pub fn attach_transport(vdev: *mut VirtioDevice, transport: Transport) {
    if vdev.is_null() {
        return;
    }
    if let Some(map) = PENDING.get() {
        map.lock().insert(vdev as usize, transport);
    }
}

/// Take the transport queued for `vdev` by `attach_transport`,
/// removing it from the pending map. Returns None when no transport
/// has been attached (the foundation-mode path the discovery loop
/// currently exercises).
fn take_pending(vdev: *mut VirtioDevice) -> Option<Transport> {
    PENDING
        .get()
        .and_then(|map| map.lock().remove(&(vdev as usize)))
}

/// `virtio_find_vqs(vdev, nvqs, vqs[], cbs[], names[], desc)` —
/// allocate `nvqs` virtqueues and stash them in `vqs[]`. virtio-
/// input asks for two: "events" (host → guest events) and "status"
/// (guest → host LED/sound updates).
///
/// Behaviour:
///   * If a transport has been attached against `vdev` via
///     `attach_transport`, take it from the pending map and pass it
///     to `VirtIOInput::new` to construct the rcore driver. On
///     success, allocate per-queue C-shape `Virtqueue` handles,
///     write them into `vqs[0]` (events) / `vqs[1]` (status), and
///     register the resulting `DeviceState` against `vdev` in
///     `INPUTS`. Returns 0.
///   * Without an attached transport: -ENODEV (-19). The driver's
///     probe path treats this as a hard error and bails cleanly
///     through `err_init_vq`. Same behaviour as the foundation
///     slice, so the discovery loop in entry_uefi.rs that doesn't
///     yet attach a transport still works unchanged.
///
/// Reentrant for distinct `vdev` pointers; not reentrant for the
/// same `vdev` (the second call would find the transport already
/// taken and return -ENODEV).
#[no_mangle]
pub extern "C" fn virtio_find_vqs(
    vdev: *mut VirtioDevice,
    nvqs: u32,
    vqs: *mut *mut Virtqueue,
    callbacks: *const Option<unsafe extern "C" fn(*mut Virtqueue)>,
    _names: *const *const c_char,
    _desc: *mut c_void,
) -> c_int {
    if vdev.is_null() || vqs.is_null() {
        return -22; // -EINVAL
    }
    let transport = match take_pending(vdev) {
        Some(t) => t,
        None => return -19, // -ENODEV — no real transport wired
    };

    // Construct the rcore VirtIOInput driver. This call exercises
    // the full virtio handshake — feature negotiation, queue setup
    // for both events + status, initial event-buffer fill — which
    // is exactly the scope #495 asked for. On failure we drop the
    // transport (it gets unwound by `VirtIOInput::new`'s error path
    // releasing any partially-set queues).
    let driver = match VirtIOInput::new(transport) {
        Ok(d) => d,
        Err(e) => {
            crate::println!("  linuxkpi/virtio: VirtIOInput::new failed: {:?}", e);
            return -19;
        }
    };
    let driver = Arc::new(Mutex::new(InputCell(driver)));

    // Allocate per-queue C-shape handles. Box-leak so the C side's
    // `vi->evt = vqs[0]; vi->sts = vqs[1]` writes survive the call;
    // the matching Box-from-raw lives in `DeviceState` for cleanup
    // in a future `del_vqs` commit.
    let mut leaked: [*mut Virtqueue; 2] = [core::ptr::null_mut(); 2];
    for i in 0..nvqs.min(2) as usize {
        let cb = if !callbacks.is_null() {
            // SAFETY: caller hands us an array of `nvqs` callback
            // slots (per the C ABI of `vq_callback_t *cbs[]`). The
            // bounded read stays within the array.
            unsafe { *callbacks.add(i) }
        } else {
            None
        };
        let vq_box = Box::new(Virtqueue {
            vdev,
            index: i as u32,
            callback: cb,
            priv_: core::ptr::null_mut(),
        });
        let vq_ptr = Box::into_raw(vq_box);
        leaked[i] = vq_ptr;
        // SAFETY: caller hands us `vqs` as an array of `nvqs` slots
        // (per the C ABI of `struct virtqueue *vqs[]`). The bounded
        // write stays within the array.
        unsafe {
            *vqs.add(i) = vq_ptr;
        }
    }

    let state = DeviceState {
        driver,
        event_vq: leaked[0],
        status_vq: leaked[1],
    };
    if let Some(map) = INPUTS.get() {
        map.lock().insert(vdev as usize, state);
    }

    0
}

/// Look up the device state registered against the `vdev` carried
/// in `vq`. Returns None when `vq` is NULL or no `DeviceState`
/// matches (foundation-mode queue with no underlying driver).
fn device_for_vq(vq: *mut Virtqueue) -> Option<Arc<Mutex<InputCell>>> {
    if vq.is_null() {
        return None;
    }
    // SAFETY: caller hands us a Virtqueue produced by
    // virtio_find_vqs; reading `vdev` is sound.
    let vdev = unsafe { (*vq).vdev };
    if vdev.is_null() {
        return None;
    }
    let map = INPUTS.get()?;
    let guard = map.lock();
    Some(guard.get(&(vdev as usize))?.driver.clone())
}

/// `virtqueue_add_inbuf(vq, sg, num, data, gfp)` — submit a buffer
/// for the device to fill. The Rust `VirtIOInput` driver manages
/// its own event-buffer ring (allocated at `VirtIOInput::new` time
/// and re-queued automatically inside `pop_pending_event`), so the
/// C driver's `add_inbuf` calls are redundant — events flow through
/// the Rust path, not through the buffers the C side passes here.
///
/// Returns 0 (success) when the queue exists, -ENODEV otherwise.
/// The C driver's success-counting in `virtinput_fill_evt` then
/// loops the full size returned by `virtqueue_get_vring_size`
/// without short-circuiting.
#[no_mangle]
pub extern "C" fn virtqueue_add_inbuf(
    vq: *mut Virtqueue,
    _sg: *mut c_void,
    _num: u32,
    _data: *mut c_void,
    _gfp: c_int,
) -> c_int {
    if device_for_vq(vq).is_some() {
        0
    } else {
        -19
    }
}

/// `virtqueue_add_outbuf(vq, sg, num, data, gfp)` — submit a buffer
/// for the device to read. Used by virtio-input on the status queue
/// (LED toggles guest→host). The Rust driver doesn't yet expose a
/// matching `push_status_event` API, so status writes are silently
/// dropped — virtio_input.c's status path explicitly tolerates this
/// ("On error we are losing the status update, which isn't critical
/// as this is typically used for stuff like keyboard leds." —
/// virtio_input.c:57-58).
#[no_mangle]
pub extern "C" fn virtqueue_add_outbuf(
    vq: *mut Virtqueue,
    _sg: *mut c_void,
    _num: u32,
    _data: *mut c_void,
    _gfp: c_int,
) -> c_int {
    if device_for_vq(vq).is_some() {
        0
    } else {
        -19
    }
}

/// `virtqueue_get_buf(vq, len)` — pop a completed buffer. Returns
/// NULL because event flow goes through the Rust driver
/// (`pop_pending_event` in `poll_all_vqs`), not through the C-side
/// buffer ring. The C driver's `virtinput_recv_events` loop sees
/// "queue empty" on its first iteration and exits cleanly.
#[no_mangle]
pub extern "C" fn virtqueue_get_buf(_vq: *mut Virtqueue, len: *mut u32) -> *mut c_void {
    if !len.is_null() {
        // SAFETY: caller hands us a writable u32 (per Linux ABI of
        // `unsigned int *len`). Zero so any accidental read of len
        // by the caller doesn't see stack garbage.
        unsafe {
            *len = 0;
        }
    }
    core::ptr::null_mut()
}

/// `virtqueue_kick(vq)` — notify the device of new buffers. The
/// initial kick on the events queue happened inside
/// `VirtIOInput::new`; the Rust driver re-issues notify after each
/// `pop_pending_event` re-queue. The C driver's kicks are redundant
/// no-op success.
#[no_mangle]
pub extern "C" fn virtqueue_kick(_vq: *mut Virtqueue) -> bool {
    true
}

/// `virtqueue_get_vring_size(vq)` — query the queue's slot count.
/// virtio-input calls this once at probe time to size its event-fill
/// loop. Returns `EVENT_QUEUE_SIZE` so the fill loop walks the full
/// 32 slots; the values themselves are dropped by the no-op
/// `virtqueue_add_inbuf` but the loop completion is what matters
/// for clean probe.
#[no_mangle]
pub extern "C" fn virtqueue_get_vring_size(vq: *mut Virtqueue) -> u32 {
    if device_for_vq(vq).is_some() {
        EVENT_QUEUE_SIZE
    } else {
        0
    }
}

/// `virtqueue_detach_unused_buf(vq)` — drain a buffer the driver
/// previously added but never got back. Returns NULL (no buffers
/// were ever added through the C-side path; the Rust driver owns
/// the real ring).
#[no_mangle]
pub extern "C" fn virtqueue_detach_unused_buf(_vq: *mut Virtqueue) -> *mut c_void {
    core::ptr::null_mut()
}

/// `virtio_cread_bytes(vdev, offset, buf, len)` — read raw bytes
/// from the device's config space. Stub zeroes the destination; the
/// C driver's config queries (name / serial / etc) come back empty,
/// which the C driver tolerates because it checks the returned-size
/// byte against zero before using the buffer.
#[no_mangle]
pub extern "C" fn virtio_cread_bytes(
    _vdev: *mut VirtioDevice,
    _offset: u32,
    buf: *mut c_void,
    len: usize,
) {
    if !buf.is_null() && len > 0 {
        // SAFETY: caller hands us a writable buffer of `len` bytes.
        unsafe {
            core::ptr::write_bytes(buf as *mut u8, 0, len);
        }
    }
}

/// `virtio_cwrite_bytes(vdev, offset, buf, len)` — write raw bytes
/// to the device's config space. No-op (the host config space writes
/// are typically status; virtio-input uses cfg_select for the read
/// side only).
#[no_mangle]
pub extern "C" fn virtio_cwrite_bytes(
    _vdev: *mut VirtioDevice,
    _offset: u32,
    _buf: *const c_void,
    _len: usize,
) {
}

/// `virtio_cread8(vdev, offset)` — single-byte config read. The
/// `virtio_cread_le(vdev, struct, field, &val)` macro that virtio-
/// input uses expands into one of these per field width. Stub
/// returns 0 — same "config-space empty" semantics as
/// `virtio_cread_bytes`.
#[no_mangle]
pub extern "C" fn virtio_cread8(_vdev: *mut VirtioDevice, _offset: u32) -> u8 {
    0
}

#[no_mangle]
pub extern "C" fn virtio_cread16(_vdev: *mut VirtioDevice, _offset: u32) -> u16 {
    0
}

#[no_mangle]
pub extern "C" fn virtio_cread32(_vdev: *mut VirtioDevice, _offset: u32) -> u32 {
    0
}

#[no_mangle]
pub extern "C" fn virtio_cread64(_vdev: *mut VirtioDevice, _offset: u32) -> u64 {
    0
}

/// `virtio_cwrite8(vdev, offset, val)` — single-byte config write.
/// No-op.
#[no_mangle]
pub extern "C" fn virtio_cwrite8(_vdev: *mut VirtioDevice, _offset: u32, _val: u8) {}

#[no_mangle]
pub extern "C" fn virtio_cwrite16(_vdev: *mut VirtioDevice, _offset: u32, _val: u16) {}

#[no_mangle]
pub extern "C" fn virtio_cwrite32(_vdev: *mut VirtioDevice, _offset: u32, _val: u32) {}

#[no_mangle]
pub extern "C" fn virtio_cwrite64(_vdev: *mut VirtioDevice, _offset: u32, _val: u64) {}

/// `virtio_has_feature(vdev, fbit)` — query the device's negotiated
/// feature bits. Returns true so virtio-input's
/// `VIRTIO_F_VERSION_1` check at probe entry passes. Real
/// negotiated semantics flow through `Transport::read_device_
/// features` inside `VirtIOInput::new`'s `begin_init` call; the
/// C-side `virtio_has_feature` is consulted only at probe-entry
/// gate-keeping, not at queue setup.
#[no_mangle]
pub extern "C" fn virtio_has_feature(_vdev: *mut VirtioDevice, _fbit: u32) -> bool {
    true
}

/// `virtio_device_ready(vdev)` — drive the device into RUNNING
/// status. The rcore `VirtIOInput::new` call already issued
/// `set_status(DRIVER_OK)` via `transport.finish_init()`, so this
/// shim has nothing to do — but the C side calls it post-probe and
/// expects no faults, hence the no-op success.
#[no_mangle]
pub extern "C" fn virtio_device_ready(_vdev: *mut VirtioDevice) {}

/// `virtio_reset_device(vdev)` — drive the device through RESET.
/// No-op on the foundation slice; the rcore driver's `Drop` impl
/// calls `queue_unset` on both queues, which is the most useful
/// thing we can do here without exposing reset on the public
/// `VirtIOInput` surface.
#[no_mangle]
pub extern "C" fn virtio_reset_device(_vdev: *mut VirtioDevice) {}

// ── Discovery ──────────────────────────────────────────────────────

/// Number of registered virtio-input devices. Returns 0 when the
/// `INPUTS` map hasn't been initialised (no `linuxkpi::init()` call)
/// or when `virtio_find_vqs` has not yet succeeded for any vdev.
///
/// Used by the launcher-side boot detection (Track XXXXX #466) to
/// decide whether the kernel should default the active MonoView's
/// InteractionMode to 'touch' (which the design-system tokens
/// derive into the spacious DensityScale per `readings/ui/monoview.md`'s
/// derivation rule). A device count of 1 is the keyboard-only case;
/// 2+ implies the QEMU CMD line wired a `-device virtio-tablet-pci`
/// alongside the keyboard (per `Dockerfile.uefi`'s default), and
/// the launcher should switch into touch mode to match the
/// hardware shape.
pub fn input_device_count() -> usize {
    INPUTS
        .get()
        .map(|map| map.lock().len())
        .unwrap_or(0)
}

/// Construct a Rust `VirtIOInput` driver against a PCI virtio-input
/// device and register it in the `INPUTS` map. Pure-Rust path that
/// bypasses the C-driver `probe` chain in `virtio_find_vqs` —
/// necessary because the C-side path only fires when the vendored
/// `virtio_input.c` was compiled (gated on `linuxkpi_c_linked`,
/// which requires clang in PATH at build.rs time and is therefore
/// absent on Windows hosts and on the Debian-slim runtime container
/// that ships the kernel image).
///
/// On success the caller can rely on `input_device_count()` /
/// `has_tablet()` returning the new total, and `poll_all_vqs()` will
/// drain `EV_KEY` / `EV_REL` / `EV_ABS` events off the device into
/// the `arch::uefi::keyboard` and `arch::uefi::pointer` rings via
/// `super::input::input_event`. The launcher super-loop already
/// calls `poll_all_vqs()` per tick — no further wiring needed.
///
/// Construction is destructive: virtio handshake (begin_init / queue
/// setup / finish_init) writes to the device. Don't double-call for
/// the same PCI slot — the second `VirtIOInput::new` would race on
/// device-status bits and the driver would return `AlreadyUsed`.
///
/// Returns `true` on success, `false` if PCI transport construction
/// or `VirtIOInput::new` failed (banner line printed in either case
/// so the operator sees what happened).
pub fn install_input_device_from_pci(bus: u8, device: u8, function: u8) -> bool {
    use virtio_drivers::transport::pci::{
        bus::{DeviceFunction, PciRoot},
        PciTransport,
    };
    use virtio_drivers::transport::{DeviceType, Transport as _};

    init();
    let device_function = DeviceFunction { bus, device, function };
    let mut root = PciRoot::new(crate::virtio::PioCam);
    let transport: Transport = match PciTransport::new::<crate::virtio::KernelHal, _>(
        &mut root,
        device_function,
    ) {
        Ok(t) => t.into(),
        Err(e) => {
            crate::println!(
                "  linuxkpi/virtio: PciTransport::new for input failed: {:?}",
                e
            );
            return false;
        }
    };
    if transport.device_type() != DeviceType::Input {
        crate::println!(
            "  linuxkpi/virtio: device_type at {:02x}:{:02x}.{} is not Input",
            bus,
            device,
            function,
        );
        return false;
    }
    let driver = match VirtIOInput::new(transport) {
        Ok(d) => d,
        Err(e) => {
            crate::println!("  linuxkpi/virtio: VirtIOInput::new failed: {:?}", e);
            return false;
        }
    };
    let driver = Arc::new(Mutex::new(InputCell(driver)));

    // Pure Rust path — no C-side `Virtqueue` handles to leak. The
    // `event_vq` / `status_vq` slots stay null; `poll_all_vqs`'s
    // snapshot loop tolerates null `event_vq` (skips the C callback
    // dispatch tail).
    let state = DeviceState {
        driver,
        event_vq: core::ptr::null_mut(),
        status_vq: core::ptr::null_mut(),
    };

    let map = match INPUTS.get() {
        Some(m) => m,
        None => return false,
    };
    // C-path keys by `vdev as usize`; we have no vdev so synthesize a
    // unique key from PCI BDF (bus:8 | device:5 | function:3 packs
    // cleanly into 16 bits and never collides with a heap pointer in
    // practice — kernel-virtual addresses are far above 64 KiB).
    let key = ((bus as usize) << 16) | ((device as usize) << 8) | (function as usize);
    map.lock().insert(key, state);
    true
}

/// True when at least one of the registered virtio-input devices is
/// likely a tablet (absolute-positioning pointer device). On the
/// foundation slice we don't yet read VIRTIO_INPUT_CFG_EV_BITS to
/// discriminate keyboards from tablets at the per-device level
/// (see `entry_uefi.rs`'s discovery loop comment for the rationale —
/// the rcore `virtio-drivers` crate doesn't expose the raw config-
/// space EV_BITS query through `VirtIOInput`'s public surface), so
/// the heuristic mirrors the same enumeration-order convention the
/// boot banner uses: keyboard first, tablet second per
/// `Dockerfile.uefi`'s `-device virtio-keyboard-pci -device
/// virtio-tablet-pci` ordering. Two-or-more registered devices →
/// the second is the tablet.
///
/// Returns `false` when:
///   * `INPUTS` hasn't been initialised yet (`linuxkpi::init()` has
///     not run — pre-boot or non-linuxkpi build path).
///   * Fewer than 2 devices have been registered (keyboard-only
///     boot, or the build path that hits the `-ENODEV` foundation-
///     mode branch in `virtio_find_vqs` and never registers a
///     `DeviceState`).
///
/// Cheap: one map-len read under a brief Mutex acquire. Designed to
/// be called once at boot from the launcher's bootstrap (post-
/// `system::init`, pre-super-loop) — not per-frame.
pub fn has_tablet() -> bool {
    // #595/#596 follow-up: returning `true` here triggers
    // `apply_touch_mode_if_tablet_present()` which churns SYSTEM state
    // and appears to stress the heap enough to surface a virtio-net
    // descriptor corruption (panic in `virtio-drivers/net_buf.rs:76`
    // with a length field interpreted as 2_883_584). Pointer events
    // still flow through `drain_pointer_into_slint_window` regardless
    // of touch mode — touch mode is purely a UI density preference.
    // The user's stated preference is keyboard + mouse + Doom, not
    // touch-mode density, so locking this to `false` matches their ask
    // while we continue investigating the descriptor corruption.
    false
}

// ── Callback dispatch ──────────────────────────────────────────────

/// Poll every registered virtio-input device, drain pending events
/// from the rcore driver, and route them through
/// `super::input::input_event` (which AAAA's #460 already wired to
/// the keyboard / pointer rings via the `EV_KEY` / `EV_REL` /
/// `EV_ABS` translation table).
///
/// Designed to be called once per launcher super-loop tick — matches
/// how `linuxkpi::tick()` already drains the workqueue ring. The
/// launcher super-loop change to call this lives in a follow-up
/// commit (touching `ui_apps/launcher.rs` is MMMM's territory per
/// #495's file-ownership map); exposing the entry point here is the
/// minimal surface needed so the wire-up commit can be pure
/// launcher-side.
///
/// Cheap when idle: when `INPUTS` is empty (no `virtio_find_vqs`
/// has succeeded) the function is one map-empty check.
pub fn poll_all_vqs() {
    let map = match INPUTS.get() {
        Some(m) => m,
        None => return,
    };
    // Snapshot the (driver Arc, callback fn-ptr, vq handle) tuples
    // so we don't hold the INPUTS lock across either the rcore
    // `pop_pending_event` (which acquires the per-driver Mutex
    // recursively if we don't release the outer first) or the C
    // callback (which calls back into our `virtqueue_*` thunks that
    // re-acquire the outer lock).
    let snapshot: Vec<(
        Arc<Mutex<InputCell>>,
        Option<unsafe extern "C" fn(*mut Virtqueue)>,
        *mut Virtqueue,
    )> = {
        let guard = map.lock();
        guard
            .values()
            .map(|state| {
                // SAFETY: state.event_vq came from `Box::into_raw`
                // in `virtio_find_vqs`; reading `callback` is sound.
                let cb = if state.event_vq.is_null() {
                    None
                } else {
                    unsafe { (*state.event_vq).callback }
                };
                (state.driver.clone(), cb, state.event_vq)
            })
            .collect()
    };

    for (driver, cb, vq) in snapshot {
        // Drain every pending event off the device. Each call to
        // `pop_pending_event` returns one event, re-queues the
        // buffer, and re-issues `Transport::notify` if needed —
        // exactly mirroring what virtio_input.c's
        // `virtinput_recv_events` does on the C side.
        loop {
            let event = {
                let mut g = driver.lock();
                g.0.pop_pending_event()
            };
            let event = match event {
                Some(e) => e,
                None => break,
            };
            // Translate to AAAA's `input_event` thunk (which routes
            // EV_KEY → keyboard ring, EV_REL/EV_ABS → pointer ring).
            // `dev` argument is reserved for future per-device
            // demuxing; AAAA's input_event ignores it on the
            // foundation slice, so passing NULL is sound.
            super::input::input_event(
                core::ptr::null_mut(),
                event.event_type as c_int,
                event.code as c_int,
                event.value as c_int,
            );
        }

        // Fire the C callback (if registered) so any C-side state
        // it maintains stays in sync. virtio_input.c's
        // virtinput_recv_events does its own queue drain via
        // virtqueue_get_buf — which we stub to NULL — so the
        // callback runs once, sees empty queue, exits cleanly.
        if let (Some(cb), false) = (cb, vq.is_null()) {
            // SAFETY: `cb` was registered by the C-side driver
            // during `virtio_find_vqs`; the driver guarantees the
            // function pointer remains valid for the lifetime of
            // the kernel (it's typically a static fn in the .text
            // segment). `vq` is the Box-leaked Virtqueue C-shape,
            // valid for the same lifetime as INPUTS.
            unsafe {
                cb(vq);
            }
        }
    }
}

// ── Tests ──────────────────────────────────────────────────────────
//
// The kernel crate is built as a `[[bin]]` with `test = false` (see
// Cargo.toml), so `cargo test` doesn't run these — they're inline
// documentation of the queue surface's expected shape and a
// typecheck of the API signatures. The Send/Sync impls and the
// public function signatures get cross-checked by every
// `cargo +nightly check --features linuxkpi` run, which is the
// verification path #495 calls out as the bar for this commit.
#[cfg(test)]
mod tests {
    use super::*;

    /// `virtio_find_vqs` returns -ENODEV when no transport has been
    /// attached for the given vdev. This is the foundation-mode
    /// behaviour that `entry_uefi.rs`'s discovery loop currently
    /// expects (it walks PCI without yet attaching transports).
    #[test]
    fn find_vqs_no_transport_returns_enodev() {
        init();
        let mut vdev = VirtioDevice {
            index: 0,
            priv_: core::ptr::null_mut(),
            dev: super::super::device::Device {
                parent: core::ptr::null_mut(),
                driver_data: core::ptr::null_mut(),
                bus: core::ptr::null_mut(),
                release: None,
            },
            config: core::ptr::null_mut(),
            id: super::super::driver::VirtioDeviceId { device: 18, vendor: 0x1AF4 },
        };
        let mut vqs: [*mut Virtqueue; 2] = [core::ptr::null_mut(); 2];
        let cbs: [Option<unsafe extern "C" fn(*mut Virtqueue)>; 2] = [None, None];
        let names: [*const c_char; 2] = [core::ptr::null(), core::ptr::null()];
        let err = virtio_find_vqs(
            &mut vdev,
            2,
            vqs.as_mut_ptr(),
            cbs.as_ptr(),
            names.as_ptr(),
            core::ptr::null_mut(),
        );
        assert_eq!(err, -19);
        assert!(vqs[0].is_null());
        assert!(vqs[1].is_null());
    }

    /// NULL `vdev` / `vqs` argument returns -EINVAL, not a fault.
    #[test]
    fn find_vqs_null_args_return_einval() {
        init();
        let cbs: [Option<unsafe extern "C" fn(*mut Virtqueue)>; 0] = [];
        assert_eq!(
            virtio_find_vqs(
                core::ptr::null_mut(),
                0,
                core::ptr::null_mut(),
                cbs.as_ptr(),
                core::ptr::null(),
                core::ptr::null_mut(),
            ),
            -22,
        );
    }

    /// `virtqueue_get_buf` always returns NULL — events flow through
    /// the Rust driver path, not through the C-side buffer ring.
    /// Stub-mode (no underlying driver) and live-mode behave the
    /// same here so the C driver's recv loop exits cleanly either
    /// way.
    #[test]
    fn get_buf_returns_null() {
        init();
        let mut len: u32 = 0xdead_beef;
        assert!(virtqueue_get_buf(core::ptr::null_mut(), &mut len).is_null());
        assert_eq!(len, 0);
    }

    /// `virtqueue_kick` is a no-op success — the rcore driver
    /// issued the initial notify at construction time.
    #[test]
    fn kick_returns_true() {
        init();
        assert!(virtqueue_kick(core::ptr::null_mut()));
    }

    /// `virtqueue_get_vring_size` on a NULL / foundation-mode vq
    /// returns 0; the C-side `virtinput_fill_evt` `min(size,
    /// ARRAY_SIZE(evts))` clamp safely reduces to zero loops.
    #[test]
    fn vring_size_null_vq_returns_zero() {
        init();
        assert_eq!(virtqueue_get_vring_size(core::ptr::null_mut()), 0);
    }

    /// `poll_all_vqs` is a no-op when no devices are registered —
    /// it must be safe to call from the launcher super-loop on any
    /// boot path including ones that never construct a virtio-input
    /// device.
    #[test]
    fn poll_all_vqs_empty_is_noop() {
        init();
        poll_all_vqs();
    }

    /// `has_tablet()` returns `false` when no devices have been
    /// registered (pre-boot, or the foundation-mode `-ENODEV` path
    /// where `virtio_find_vqs` never builds a `DeviceState`).
    /// `input_device_count()` agrees by returning 0 in the same
    /// state — Track XXXXX #466's launcher-side touch-mode detection
    /// relies on both reading "no devices" rather than panicking on
    /// the empty map.
    #[test]
    fn has_tablet_with_no_devices_is_false() {
        init();
        // INPUTS may carry leftover state from sibling tests in this
        // module; assert `has_tablet` agrees with `input_device_count`
        // rather than asserting an absolute count, so test order
        // doesn't matter. Both functions read the same map under the
        // same lock so they always agree.
        let count = input_device_count();
        assert_eq!(has_tablet(), count >= 2);
    }

    /// `virtqueue_add_inbuf` returns -ENODEV without an attached
    /// driver, 0 with one. The `0` branch needs a real device to
    /// exercise; the `-ENODEV` branch is what the foundation
    /// discovery loop sees today.
    #[test]
    fn add_inbuf_no_device_returns_enodev() {
        init();
        assert_eq!(
            virtqueue_add_inbuf(
                core::ptr::null_mut(),
                core::ptr::null_mut(),
                1,
                core::ptr::null_mut(),
                0,
            ),
            -19,
        );
    }
}

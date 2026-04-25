// crates/arest-kernel/src/allocator.rs
//
// Global allocator for the BIOS-path kernel. Uses `talc`: a modern
// slab-style allocator that holds up under realloc churn (e.g. wasmi
// `Memory::grow`) where `linked_list_allocator-0.10` trips a
// "Freed node aliases existing hole" assertion (#440 / #376-post).
// Backed by a fixed static buffer at boot — sized to comfortably
// cover Doom's WASM instance + framebuffer chain (see HEAP_SIZE
// doc-comment).
//
// When the bootloader memory-map plumbing lands (#180) this switches
// to a dynamic region carved from BootInfo.memory_regions so the
// heap can grow to whatever the firmware reports as usable RAM.
//
// Each UEFI entry harness (`entry_uefi*.rs`) carries its own
// `#[global_allocator]` against a `boot::allocate_pages`-derived
// heap; this file is gated to `cfg(not(target_os = "uefi"))` in
// `main.rs` so the two declarations don't collide. Migrating those
// to talc is tracked separately under the same per-arch entry
// owner so the Cargo.toml keeps `linked_list_allocator` for now.
//
// The allocator unblocks every `alloc::*` type: Box, Vec, String,
// BTreeMap, format!, etc. — without it, the kernel is restricted to
// core primitives and stack-allocated buffers only.
//
// API shape preserved from the prior `LockedHeap` setup so existing
// callers don't move:
//   * `ALLOCATOR.lock()` returns a guard with `.size()` and
//     `.free()` for the REPL `heap` command (`repl.rs`).
//   * `init()` is a `pub fn` taking no arguments, called once from
//     `main.rs::kernel_main` before any allocating type is touched.
// The wrapper newtype `KernelHeap` is what makes both surfaces
// possible on top of `talc`'s `Talck` (which itself only exposes
// allocation primitives via `GlobalAlloc`, not byte-counter
// accessors): every accounting operation pokes a pair of `AtomicUsize`
// totals incremented in `alloc` / decremented in `dealloc`. Cheap
// (one relaxed atomic per call) and matches what the REPL command
// actually wants to print.

use core::alloc::{GlobalAlloc, Layout};
use core::mem::MaybeUninit;
use core::sync::atomic::{AtomicUsize, Ordering};
use talc::{ClaimOnOom, Span, Talc, Talck};

/// Heap size for the static-buffer bootstrap allocator.
///
/// Sized to comfortably hold:
///   * Object graphs + DEFS table for the baked metamodel (~64 KiB).
///   * Two back-buffers for the framebuffer driver — at the
///     bootloader-default 1280x720x24bpp that's 2.7 MiB each, so
///     ~5.4 MiB just for the buffer chain (#269 triple-buffering).
///   * wasmi runtime instances + parsed modules (#270) — single
///     digit MiB for typical guest workloads.
///   * Vec / String / BTreeMap churn from command parsing, HTTP
///     bodies, syscall copy_in/out, and freeze/thaw cycles.
///
/// 8 MiB headroom comfortably covers everything we ship today and
/// stays well below the bootloader 0.11 BIOS-stage's static-frame-
/// allocator headroom (16 MiB BSS pushed past it once the wasmi
/// `.text` got pulled in alongside; combined LOAD-segment sizes
/// were too large for the early-boot region). When the dynamic
/// memory plumbing (#180 follow-up) lands, this gets carved out of
/// `BootInfo.memory_regions` instead of the static BSS.
const HEAP_SIZE: usize = 8 * 1024 * 1024;

/// Backing storage for the heap. `MaybeUninit` avoids zero-filling
/// the buffer in the binary — the BSS pattern handles that on
/// bootloader entry. `#[used]` keeps the symbol from being dropped
/// by LTO.
#[used]
static mut HEAP_STORAGE: MaybeUninit<[u8; HEAP_SIZE]> = MaybeUninit::uninit();

/// Underlying talc allocator. Wrapped in `Talck` (talc's
/// `lock_api`-backed `GlobalAlloc` adapter) over a `spin::Mutex<()>`
/// — the `spin` crate is already in the kernel's deps for the
/// `SerialPort` singleton, so we don't pick up a new transitive.
///
/// The `ClaimOnOom` handler attaches `HEAP_STORAGE` to the heap on
/// the first OOM. `init()` below also issues an explicit `claim` so
/// the heap is alive before any allocation hits, mirroring the
/// previous `LockedHeap::init` semantics.
static TALC: Talck<spin::Mutex<()>, ClaimOnOom> = Talc::new(unsafe {
    // SAFETY: `HEAP_STORAGE` is a `static mut` we own for the
    // lifetime of the kernel. `Span::from_array` records its base
    // pointer + length; nothing reads through this `Span` until the
    // first `claim` (issued either from `init()` below or via
    // `ClaimOnOom` on the first alloc — whichever fires first).
    ClaimOnOom::new(Span::from_array(
        core::ptr::addr_of!(HEAP_STORAGE) as *mut [u8; HEAP_SIZE],
    ))
})
.lock();

/// Total bytes currently allocated. Kept as an atomic so the REPL
/// `heap` command can read it without taking the allocator lock
/// (and risking a deadlock if the print path itself alloc'd).
/// Incremented in `alloc`, decremented in `dealloc`.
static ALLOCATED: AtomicUsize = AtomicUsize::new(0);

/// Wrapper newtype that owns the `#[global_allocator]` slot.
/// Delegates `GlobalAlloc` to the inner `Talck`, with a side-channel
/// to keep `ALLOCATED` in sync. Also publishes a `lock()` accessor
/// returning a guard exposing `.size()` / `.free()` so the REPL
/// `heap` command keeps its existing call-site shape.
pub struct KernelHeap;

unsafe impl GlobalAlloc for KernelHeap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = TALC.alloc(layout);
        if !p.is_null() {
            ALLOCATED.fetch_add(layout.size(), Ordering::Relaxed);
        }
        p
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        TALC.dealloc(ptr, layout);
        ALLOCATED.fetch_sub(layout.size(), Ordering::Relaxed);
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let p = TALC.alloc_zeroed(layout);
        if !p.is_null() {
            ALLOCATED.fetch_add(layout.size(), Ordering::Relaxed);
        }
        p
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let p = TALC.realloc(ptr, layout, new_size);
        if !p.is_null() {
            // realloc returns either the old ptr or a fresh one; either
            // way, the accounting delta is just the size change. If the
            // realloc failed (null return), the original allocation is
            // still live, so leave the counter alone.
            ALLOCATED.fetch_add(new_size.wrapping_sub(layout.size()), Ordering::Relaxed);
        }
        p
    }
}

impl KernelHeap {
    /// Acquire a stats-only handle. Mirrors the old
    /// `LockedHeap::lock()` shape — the returned guard is `Copy`
    /// because it carries no real lock (the byte counters are
    /// atomics), but the "lock"-like API matches the REPL call site
    /// in `repl.rs::"heap"` unchanged.
    pub fn lock(&self) -> KernelHeapStats {
        KernelHeapStats
    }
}

/// Snapshot of heap stats. Returned by `KernelHeap::lock()`. The
/// `size()` / `free()` methods recover the previous
/// `linked_list_allocator::Heap` accessor names.
#[derive(Copy, Clone)]
pub struct KernelHeapStats;

impl KernelHeapStats {
    /// Total backing-buffer size in bytes.
    pub fn size(&self) -> usize {
        HEAP_SIZE
    }

    /// Bytes currently free. `HEAP_SIZE - allocated` — does not
    /// account for talc's per-allocation header overhead (small
    /// constant per live allocation), so the printed number is a
    /// best-effort upper bound on the real free-byte count.
    pub fn free(&self) -> usize {
        HEAP_SIZE.saturating_sub(ALLOCATED.load(Ordering::Relaxed))
    }
}

/// The global allocator. See `KernelHeap` for delegation strategy.
#[global_allocator]
pub static ALLOCATOR: KernelHeap = KernelHeap;

/// Initialise the allocator. Must be called exactly once, before
/// any allocating type is touched (Box, Vec, format!, etc.).
///
/// `talc::ClaimOnOom` would also pick up the heap on first OOM,
/// but doing the claim eagerly here matches the existing call-site
/// contract (`allocator::init()` is the first thing `kernel_main`
/// does) and keeps the failure mode of "alloc before init" as a
/// clean panic at the call site rather than an OOM-handler dive.
pub fn init() {
    // SAFETY: `HEAP_STORAGE` is exclusively owned by this module
    // (single `static mut`, no other writers). `claim` records the
    // span as a heap region the allocator can subdivide; the
    // returned `Span` describes the now-aligned usable subregion
    // (we discard it — subsequent allocations route through
    // `ClaimOnOom` for any further regions, of which there are
    // none in the static-buffer model).
    unsafe {
        let span = Span::from_array(core::ptr::addr_of!(HEAP_STORAGE) as *mut [u8; HEAP_SIZE]);
        let _ = TALC.lock().claim(span);
    }
}

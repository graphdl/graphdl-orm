// crates/arest-kernel/src/allocator.rs
//
// Global allocator for the kernel. Using `linked_list_allocator`:
// simple free-list allocator behind a spin lock, suitable for a
// single-address-space kernel. Backed by a fixed static buffer at
// boot — 1 MiB is enough for MVP (Object graphs, Vec<String> for
// command parsing, the compiled DEFS table for a single tenant).
//
// When the bootloader memory-map plumbing lands (#180) this switches
// to a dynamic region carved from BootInfo.memory_regions so the
// heap can grow to whatever the firmware reports as usable RAM.
//
// The allocator unblocks every `alloc::*` type: Box, Vec, String,
// BTreeMap, format!, etc. — without it, the kernel is restricted to
// core primitives and stack-allocated buffers only.

use core::mem::MaybeUninit;
use linked_list_allocator::LockedHeap;

/// Heap size for the static-buffer bootstrap allocator. 1 MiB is
/// overkill for hello-world but comfortably covers a parsed Domain
/// plus its compiled DEFS table for a single-tenant REPL.
const HEAP_SIZE: usize = 1024 * 1024;

/// Backing storage for the heap. `MaybeUninit` avoids zero-filling
/// the 1 MiB buffer in the binary — the BSS pattern handles that on
/// bootloader entry. `#[used]` keeps the symbol from being dropped
/// by LTO.
#[used]
static mut HEAP_STORAGE: MaybeUninit<[u8; HEAP_SIZE]> = MaybeUninit::uninit();

/// The global allocator. `LockedHeap::empty()` yields an unusable
/// allocator; `init()` populates it with the static buffer above.
/// Any attempt to allocate before `allocator::init()` runs will
/// panic inside the allocator itself — we call it as the first step
/// in `kernel_main` to avoid that window.
#[global_allocator]
pub static ALLOCATOR: LockedHeap = LockedHeap::empty();

/// Initialise the allocator. Must be called exactly once, before
/// any allocating type is touched (Box, Vec, format!, etc.). The
/// `unsafe` is because we hand the allocator a raw pointer and a
/// length and promise we own the backing storage for the rest of
/// the kernel's lifetime.
pub fn init() {
    unsafe {
        let ptr = core::ptr::addr_of_mut!(HEAP_STORAGE) as *mut u8;
        ALLOCATOR.lock().init(ptr, HEAP_SIZE);
    }
}

// crates/arest-kernel/src/entry_uefi.rs
//
// UEFI entry point (#344). Only compiled for `target_os = "uefi"` —
// the BIOS path (`x86_64-unknown-none`) keeps its `bootloader_api`
// entry in `main.rs`'s top-level gated body.
//
// Step 1 scaffold: bring the kernel up under UEFI far enough to
// prove `uefi-rs` links, `#[entry]` wires a `_start` symbol the
// firmware picks up, and `ConOut` reaches the serial console. The
// real arch-neutral `kernel_run(BootInfo)` lands after step 2 of the
// pivot (arch trait extraction) — this file stays tiny until then.
//
// What this gives us today:
//   * `cargo build --target x86_64-unknown-uefi --release` produces
//     an `EFI` executable.
//   * Boot under QEMU-OVMF:
//       qemu-system-x86_64 -bios OVMF.fd -kernel arest-kernel.efi
//     prints the AREST scaffold banner via firmware ConOut.
//   * BIOS path is untouched — existing x86_64-unknown-none build
//     still produces the same kernel image.
//   * `println!` (#344 step 3) routes through `arch::_print`, whose
//     UEFI implementation (`arch::uefi::serial::_print`) writes via
//     ConOut. Same macro the BIOS path uses — no UEFI-specific
//     printing call sites in shared kernel code.
//
// What this does not do yet (tracked in #344 follow-up commits):
//   * ExitBootServices + hand-off to `kernel_run` (step 4).
//   * Real arch serial driver post-ExitBootServices (16550 on
//     x86_64-uefi → COM1 in QEMU; PL011 on aarch64-uefi → virt
//     pl011 in QEMU). Until then `_print` writes silently no-op
//     after firmware services tear down.
//   * Populate a `BootInfo` from UEFI GetMemoryMap + Graphics Output
//     Protocol, so `memory::init` / the framebuffer work the same
//     way the BIOS path does (step 4).
//   * aarch64-unknown-uefi — this entry is target-agnostic, but the
//     kernel body below the arch trait doesn't exist yet (step 5).

#![cfg(target_os = "uefi")]

use core::cell::UnsafeCell;
use linked_list_allocator::LockedHeap;
use uefi::prelude::*;
use uefi::boot::MemoryType;

use crate::println;

// Global allocator. Uses a static .bss-backed `LockedHeap` rather
// than `uefi::allocator::Allocator` so the heap SURVIVES
// ExitBootServices — uefi-rs's allocator is backed by
// `BootServices::allocate_pool`, which faults after EBS.
//
// The BIOS arm uses the same crate for the same reason (see
// `allocator.rs`); this keeps the kernel's Box/Vec/String codepaths
// identical on both boot targets.
//
// Size: 8 MiB is a conservative pick that matches the BIOS heap
// (see `allocator.rs`'s HEAP_SIZE). UEFI firmware loaders typically
// pre-allocate BSS when mapping the PE32+ image, so 8 MiB of kernel
// BSS just reserves that many pages up-front. QEMU+OVMF with the
// default 128 MiB guest accommodates this comfortably. The .bss
// bytes themselves are zeroed by the firmware before _start runs,
// so `LockedHeap::empty()` + a single init() call is enough.
//
// The init() call runs at the TOP of efi_main — before ANY
// alloc-using code (println! transcodes args via a String on the
// UEFI serial path). Must NOT move later without switching to a
// crate that supports delayed init.
const HEAP_SIZE: usize = 8 * 1024 * 1024;

// SAFETY wrapper: static mut arrays aren't directly Sync-safe. The
// heap is only touched via `ALLOCATOR.lock()` (single-CPU kernel,
// no preemption), and the init() below happens before any concurrent
// use. UnsafeCell documents the interior mutability to the borrow
// checker without requiring `static mut`.
#[repr(C, align(16))]
struct HeapBytes(UnsafeCell<[u8; HEAP_SIZE]>);
unsafe impl Sync for HeapBytes {}

static HEAP: HeapBytes = HeapBytes(UnsafeCell::new([0u8; HEAP_SIZE]));

#[global_allocator]
static ALLOCATOR: LockedHeap = LockedHeap::empty();

/// UEFI entry point. `uefi-rs`'s `#[entry]` expands this into the
/// PE32+ `_start` symbol the firmware invokes after loading the
/// image.
///
/// Boot pipeline (#344 step 4 — partial):
///   1. ConOut online (firmware-managed, init_console no-op on UEFI).
///   2. Pre-EBS banner via `println!` → ConOut.
///   3. `boot::exit_boot_services` — firmware tears down. After
///      this, the system table is invalidated and `with_stdout`
///      silently no-ops.
///   4. `arch::switch_to_post_ebs_serial` flips `_print` onto the
///      direct-I/O 16550 path. Same COM1 line QEMU's `-serial
///      stdio` is wired to, so the banner survives the hand-off
///      unbroken on the host terminal.
///   5. Post-EBS banner via `println!` → 16550. Proves the cutover
///      works end-to-end.
///   6. `arch::init_memory(memory_map)` (step 4c) — consume the
///      firmware memory map, install the OffsetPageTable + frame
///      allocator singletons behind the same accessor surface the
///      BIOS arm publishes, and print a post-init banner proving
///      the page-table singleton is live.
///   7. Halt. Step 4d (kernel_run handoff) wires the arch-neutral
///      kernel body once its subsystems (virtio / net / blk / repl)
///      drop their `cfg(not(target_os = "uefi"))` gates.
#[entry]
fn efi_main() -> Status {
    // Heap init MUST be the first thing — the global allocator is an
    // empty LockedHeap; the first alloc call before init() would
    // panic. Subsequent `println!` and any uefi-rs internal alloc
    // work (transcoding format args to UCS-2, for example) all route
    // through this heap.
    //
    // SAFETY: HEAP is a static, zero-initialized byte array. No code
    // has run that could be holding a pointer into it yet — we're
    // literally the first line of efi_main. The cast to *mut u8 is
    // trivially safe on a `static HeapBytes` with `UnsafeCell`
    // interior; single-threaded kernel means no racing initialisation.
    unsafe {
        let heap_start = HEAP.0.get() as *mut u8;
        ALLOCATOR.lock().init(heap_start, HEAP_SIZE);
    }

    crate::arch::init_console();

    // Pre-EBS SSE enable (step 4d prep). Does not depend on boot
    // services, so firing it up front keeps the f32/f64-emitting
    // codegen in kernel body callers (notably wasmi once #270/#271
    // land) from tripping #UD. The BIOS arm fires this from
    // `kernel_main` for the same reason; doing it here means the
    // shared `kernel_run` body can assume SSE is live regardless of
    // which entry path got us here.
    crate::arch::enable_sse();

    // ASCII hyphens — keeps the line printable on bare COM1, which
    // most OVMF builds downcode UCS-2 -> ASCII on. The kernel itself
    // happily transcodes BMP glyphs through ConOut, but the smoke
    // harness reads stdout via QEMU's `-serial stdio`, where the
    // round-trip survives only if the banner is ASCII.
    println!("AREST kernel - UEFI scaffold (#344)");
    println!("  step 4 of 8: ExitBootServices + post-EBS serial");
    println!("  pre-EBS:  ConOut active (firmware-managed), SSE enabled");

    // SAFETY: `boot::exit_boot_services` walks the current memory
    // map, gets the firmware's signature lock, and tears down
    // BootServices. The returned `MemoryMapOwned` is a stable copy
    // of the map the firmware handed us. We hand it straight into
    // `arch::init_memory` (step 4c) which flattens the CONVENTIONAL
    // regions into a frame allocator and stands up the page-table
    // singleton.
    let memory_map = unsafe { boot::exit_boot_services(MemoryType::LOADER_DATA) };

    // Firmware ConOut is now invalid. Switch `_print` onto the
    // direct-I/O 16550 path BEFORE the next println! so the
    // banner doesn't disappear into a no-op.
    crate::arch::switch_to_post_ebs_serial();

    println!("  post-EBS: 16550 COM1 active (kernel-managed)");

    // Step 4c: consume the firmware memory map, install the paging
    // + frame-allocator singletons. `init_memory` returns the
    // physical-memory offset (always 0 on UEFI — firmware identity-
    // maps RAM), matching the shape of the BIOS arm's facade.
    let _phys_offset = crate::arch::init_memory(memory_map);

    // Proves the page-table singleton is live post-EBS: going
    // through `memory::usable_frame_count()` forces a `FRAME_ALLOCATOR.lock()`
    // + a pass over the descriptor iterator, so a hung lock or a
    // malformed memory map surfaces here rather than silently at
    // first allocation inside kernel_run.
    let frame_count = crate::arch::memory::usable_frame_count();
    let usable_mib = (frame_count * 4096) / (1024 * 1024);
    println!(
        "  mem:      {frame_count} frames usable ({usable_mib} MiB) (UEFI memory map)"
    );

    // Post-EBS heap smoke (step 4d wave 3, 5b74f2a). `uefi::allocator`
    // would fault here because BootServices is gone; our static-BSS
    // `LockedHeap` keeps serving allocations. Building a Vec and
    // summing it proves both the heap init from pre-EBS carried
    // through AND `format!` on the post-EBS 16550 path still works.
    // Sum of 0..16 is 120 — the host-side smoke asserts that exact
    // number.
    let test_vec: alloc::vec::Vec<u32> = (0..16u32).collect();
    let sum: u32 = test_vec.iter().sum();
    println!("  alloc:    post-EBS heap live (sum 0..16 = {sum})");

    println!("  next:        kernel_run handoff (step 4d)");

    // Scaffold halt — via the facade so the call site is identical
    // to the BIOS arm's bottom-of-kernel_run. Step 4d wires
    // `kernel_run(phys_offset)` once the shared body subsystems are
    // UEFI-capable; until then the entry parks here after proving
    // the page-table + frame-allocator singletons are live.
    crate::arch::halt_forever()
}

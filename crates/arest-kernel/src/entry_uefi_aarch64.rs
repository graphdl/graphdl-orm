// crates/arest-kernel/src/entry_uefi_aarch64.rs
//
// aarch64-unknown-uefi entry point (#344 cross-arch). Sibling of
// `entry_uefi.rs` (x86_64-unknown-uefi). Split into two files because
// the two arms diverge on the panic handler (x86_64 uses raw port I/O
// to COM1; aarch64 uses raw MMIO to PL011 at 0x0900_0000) and because
// the pre-EBS heap + SSE init sequence diverges (x86_64 flips CR0/CR4
// for SSE; aarch64 has NEON on by default under UEFI).
//
// Scope of THIS commit chain (#366 + #367):
//   * Static-BSS `LockedHeap` heap (post-EBS-safe, parallels x86_64).
//   * `efi_main`
//       - Initialise the heap (before any `println!`).
//       - Print pre-EBS banner via PL011.
//       - `boot::exit_boot_services` — firmware tears down.
//       - `arch::init_memory(memory_map)` — consume the firmware
//         memory map, install the UefiFrameAllocator singleton AND
//         carve the 2 MiB DMA pool for a future virtio-mmio bring-up.
//       - Print post-EBS banner: frame count, usable MiB, DMA pool status.
//       - Halt via `wfi` loop.
//   * `panic` — print a one-line fault marker via PL011, then `wfi` loop.
//
// Deliberately NOT here yet (matching the x86_64 arm's step-by-step
// progression, and tracked by #368 / #369):
//   * virtio-mmio transport + find_virtio_net / find_virtio_blk (#368).
//   * virtio-net + virtio-blk drivers online + MAC / sector banners (#369).
//
// Gated on `cfg(all(target_os = "uefi", target_arch = "aarch64"))`
// and lives behind a `mod entry_uefi_aarch64;` in `main.rs` guarded
// by the same cfg so a `cargo check --target x86_64-unknown-uefi`
// ignores it entirely.

#![cfg(all(target_os = "uefi", target_arch = "aarch64"))]

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
// Mirrors the x86_64-UEFI arm's pattern (`entry_uefi.rs`) byte-for-
// byte: same crate, same size (16 MiB), same `static` + UnsafeCell
// wrapping, same init-at-top-of-efi_main discipline.
//
// Size: 16 MiB. Generous for the aarch64 UEFI bring-up path — the
// smoke is banner-only so the main alloc pressure is `format_args!`
// transcoding, plus the `MemoryMapOwned` firmware descriptor buffer
// temporarily during init. QEMU virt with the default 256 MiB guest
// accommodates this comfortably.
const HEAP_SIZE: usize = 16 * 1024 * 1024;

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

/// UEFI entry point for the aarch64 target. `uefi-rs`'s `#[entry]`
/// expands this into the PE32+ `_start` symbol the firmware invokes.
///
/// Boot pipeline (#344 cross-arch, #366 memory bring-up):
///   1. Heap init — static-BSS LockedHeap gets a fixed byte range.
///   2. Pre-EBS banner via `println!` → PL011 MMIO.
///   3. `boot::exit_boot_services` — firmware tears down. The
///      returned `MemoryMapOwned` is the snapshot we feed to
///      `arch::init_memory`.
///   4. `arch::init_memory(memory_map)` — consume the firmware
///      memory map, install the UefiFrameAllocator singleton behind
///      the same accessor API the x86_64-UEFI arm publishes.
///   5. Post-EBS banner: frame count, usable MiB.
///   6. Halt via `wfi` loop. #367-#369 follow-ups grow the banner
///      with DMA pool + virtio-mmio + virtio-net/blk lines.
#[entry]
fn efi_main() -> Status {
    // Heap init MUST be the first thing — the global allocator is an
    // empty LockedHeap; the first alloc call before init() would
    // panic. Subsequent `println!` and any uefi-rs internal alloc
    // work all route through this heap.
    //
    // SAFETY: HEAP is a static, zero-initialized byte array. No code
    // has run that could be holding a pointer into it yet — we're
    // literally the first line of efi_main.
    unsafe {
        let heap_start = HEAP.0.get() as *mut u8;
        ALLOCATOR.lock().init(heap_start, HEAP_SIZE);
    }

    crate::arch::init_console();

    // Pre-EBS banner. ASCII-only (carries cleanly through QEMU's
    // `-serial stdio` PL011 UARTDR straight through to the host
    // terminal without any transcoding).
    println!("AREST kernel - aarch64-UEFI scaffold");
    println!("  target: aarch64-unknown-uefi");
    println!("  pre-EBS:  PL011 MMIO active at 0x0900_0000");

    // SAFETY: `boot::exit_boot_services` walks the current memory
    // map, gets the firmware's signature lock, and tears down
    // BootServices. The returned `MemoryMapOwned` is a stable copy
    // of the map the firmware handed us. We hand it straight into
    // `arch::init_memory` which flattens CONVENTIONAL regions into
    // a frame allocator and stands up the singleton.
    let memory_map = unsafe { boot::exit_boot_services(MemoryType::LOADER_DATA) };

    // Firmware BootServices is now gone. Our `println!` writes to
    // PL011 MMIO directly (not via ConOut), so it survives EBS with
    // no cutover needed — the PL011 register at 0x0900_0000 stays
    // firmware-identity-mapped.
    println!("  post-EBS: PL011 MMIO survives (no ConOut cutover needed)");

    // #366: consume the firmware memory map, install the
    // UefiFrameAllocator singleton. `init_memory` returns the
    // physical-memory offset (always 0 on UEFI — AAVMF identity-
    // maps RAM), matching the shape of the x86_64-UEFI arm's facade.
    let _phys_offset = crate::arch::init_memory(memory_map);

    // Proves the frame-allocator singleton is live post-EBS: going
    // through `memory::usable_frame_count()` forces a `FRAME_ALLOCATOR.lock()`
    // + a pass over the descriptor iterator, so a hung lock or a
    // malformed memory map surfaces here rather than silently later.
    let frame_count = crate::arch::memory::usable_frame_count();
    let usable_mib = (frame_count * 4096) / (1024 * 1024);
    println!(
        "  mem:      {frame_count} frames usable ({usable_mib} MiB) (UEFI memory map)"
    );

    // #367: DMA pool carve smoke. `arch::init_memory` on aarch64 now
    // mirrors the x86_64-UEFI arm: carves a 2 MiB contiguous region
    // out of the firmware memory map and reserves it for a future
    // virtio-mmio bring-up (#368/#369). This line proves the carve
    // landed at runtime -- `with_dma_pool` returns `Some` only when
    // the pool was built, which in turn only happens when
    // `dma::carve_dma_region` found a big-enough CONVENTIONAL region.
    // A `NONE` here (on a 256 MiB QEMU guest with 60+ MiB usable)
    // would indicate a regression in the carve logic.
    let dma_ok = crate::arch::memory::with_dma_pool(|_| true).unwrap_or(false);
    println!(
        "  dma:      pool {} (2 MiB UEFI memory-map carve for virtio)",
        if dma_ok { "live" } else { "NONE (carve failed)" }
    );

    println!("  next:   ExitBootServices + memory map (follow-ups)");

    // Halt via wfi loop. Returns `!`, so the `Status` return on the
    // `#[entry]` fn is unreachable — uefi-rs's macro expands the
    // signature check anyway; halt_forever's divergence satisfies
    // both the compiler and the firmware's caller convention.
    crate::arch::halt_forever();
}

/// Panic handler for the aarch64 UEFI path. The x86_64 arm's
/// `entry_uefi.rs` panic handler raw-I/Os COM1 at 0x3F8; here we do
/// the same thing against PL011 MMIO at 0x0900_0000 so a fault
/// surfaces as a visible "!! UEFI kernel panic !!" marker rather
/// than a silent hang.
///
/// Uses a stack-local writer targeting the same PL011 UARTDR the
/// banner writes to — no alloc dependency (so a panic inside an
/// allocator hook can't deadlock), no singleton (so a panic
/// mid-mutation of a future serial-state struct can't fight it).
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    use core::fmt::Write;

    /// QEMU virt PL011 UARTDR address. Duplicated from
    /// `arch::aarch64::serial` so the panic path has zero module
    /// dependencies — if an import is what caused the panic, the
    /// fault marker still gets out.
    const UARTDR: *mut u8 = 0x0900_0000 as *mut u8;

    struct RawPl011;
    impl Write for RawPl011 {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            for b in s.bytes() {
                // SAFETY: UARTDR is the QEMU virt PL011 data register,
                // identity-mapped by firmware. Writes are stateless
                // MMIO with no memory-safety impact.
                unsafe { UARTDR.write_volatile(b) };
            }
            Ok(())
        }
    }

    let mut w = RawPl011;
    let _ = w.write_str("\r\n!! UEFI kernel panic (aarch64) !!\r\n");
    let _ = writeln!(w, "{info}");

    loop {
        // SAFETY: `wfi` is unprivileged in EL1 and has no side
        // effects beyond pausing until the next interrupt. `nomem` /
        // `nostack` / `preserves_flags` describe it accurately.
        unsafe {
            core::arch::asm!(
                "wfi",
                options(nomem, nostack, preserves_flags),
            );
        }
    }
}

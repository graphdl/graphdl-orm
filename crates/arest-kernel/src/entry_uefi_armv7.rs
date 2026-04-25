// crates/arest-kernel/src/entry_uefi_armv7.rs
//
// armv7-unknown-uefi entry point (#346d / #389 cross-arch). Sibling of
// `entry_uefi_aarch64.rs` (aarch64 UEFI) and `entry_uefi.rs` (x86_64
// UEFI). Mirrors the aarch64 arm wave-for-wave on the 32-bit ARM
// silicon: same pre-EBS banner, same ExitBootServices handoff, same
// arch::init_memory + DMA pool carve, same virtio-mmio scan + driver
// bring-up, same wfi halt loop. The only divergent piece is the
// pointer width — armv7 is 32-bit, so `arch::armv7::init_memory`
// returns `u32` rather than `u64`. The MMIO transport's `init_offset`
// expects `u64`, so we widen at that boundary.
//
// Scope of THIS commit (#389):
//   * Static-BSS `Talck` heap (post-EBS-safe, parallels aarch64
//     and x86_64 UEFI arms — same crate, same size, same UnsafeCell
//     wrapping, same init-at-top discipline).
//   * `efi_main`
//       - Initialise the heap (before any `println!`).
//       - Print pre-EBS banner via PL011 MMIO (firmware's identity
//         mapping covers QEMU virt-armv7's 0x0900_0000 PL011 just like
//         the aarch64 virt machine).
//       - `boot::exit_boot_services` — firmware tears down.
//       - `arch::init_memory(memory_map)` — install the
//         UefiFrameAllocator singleton + carve the 2 MiB DMA pool.
//         Returns `u32` (32-bit pointer width on armv7).
//       - virtio_mmio scan — find virtio-net / virtio-blk slots in
//         the QEMU-virt 32-slot MMIO window at 0x0a00_0000.
//       - Drive each discovered device: read MAC, read sector count
//         + read-only flag, print a driver-online banner.
//       - Halt via `wfi` loop.
//   * `panic` — print a one-line fault marker via PL011, then `wfi` loop.
//
// Replaces the runtime stubs in `arch::armv7::runtime_stub`. That
// scaffold module's own header explicitly anticipates this hand-off
// ("Both items will be REPLACED by `entry_uefi_armv7.rs` when #346d
// lands"); we keep its source file in place to minimise the per-arch
// touch surface in this commit but elide the `mod runtime_stub;`
// declaration in `arch::armv7::mod` so its `#[global_allocator]`,
// `#[panic_handler]`, and `efi_main` items don't collide with the
// ones declared below. A follow-up cleanup commit can delete the file.
//
// Gated on `cfg(all(target_os = "uefi", target_arch = "arm"))` and
// lives behind a `mod entry_uefi_armv7;` in `main.rs` guarded by the
// same cfg so a `cargo check --target aarch64-unknown-uefi` (and the
// other UEFI / unknown-none targets) ignores it entirely.

#![cfg(all(target_os = "uefi", target_arch = "arm"))]

use core::cell::UnsafeCell;
use talc::{ClaimOnOom, Span, Talc, Talck};
use uefi::prelude::*;
use uefi::boot::MemoryType;

use crate::println;

// Global allocator. Uses a static .bss-backed `Talck<spin::Mutex<()>,
// ClaimOnOom>` rather than `uefi::allocator::Allocator` so the heap
// SURVIVES ExitBootServices — uefi-rs's allocator is backed by
// `BootServices::allocate_pool`, which faults after EBS.
//
// Mirrors the aarch64-UEFI arm's pattern (`entry_uefi_aarch64.rs`):
// same crate (talc, swapped from linked_list_allocator under #440 /
// #443 to survive wasmi `Memory::grow` realloc churn during Doom's
// `Z_Init`), same UnsafeCell-wrapped static heap region, same init-
// at-top-of-efi_main discipline.
//
// Size: 16 MiB. Generous for the armv7 UEFI bring-up path — the
// smoke is banner-only so the main alloc pressure is `format_args!`
// transcoding, plus the `MemoryMapOwned` firmware descriptor buffer
// temporarily during init. QEMU virt with the default 256 MiB guest
// accommodates this comfortably (armv7 has 32-bit address space but
// QEMU virt-armv7 ships up to ~3 GiB usable RAM under the firmware's
// LPAE-based identity map, plenty of room).
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

// `ClaimOnOom::empty()` starts the allocator with no backing region;
// the explicit `claim(Span::from_base_size(...))` below in `efi_main`
// attaches the static `HEAP` array once the kernel is running. Wrapped
// in `Talck` (talc's `lock_api`-backed `GlobalAlloc` adapter) over a
// `spin::Mutex<()>` — `spin` is already in the kernel's deps so this
// picks up no new transitive.
#[global_allocator]
static ALLOCATOR: Talck<spin::Mutex<()>, ClaimOnOom> =
    Talc::new(unsafe { ClaimOnOom::new(Span::empty()) }).lock();

/// UEFI entry point for the armv7 target. `uefi-rs`'s `#[entry]`
/// expands this into the PE32+ `efi_main` symbol the firmware invokes
/// (the custom `arest-kernel-armv7-uefi.json` target's
/// `pre-link-args /entry:efi_main` resolves against the
/// `#[export_name = "efi_main"]` the macro emits).
///
/// Boot pipeline (mirrors `entry_uefi_aarch64::efi_main` wave-for-
/// wave; the only divergence is the `as u64` widening on the memory
/// offset to satisfy `virtio_mmio::init_offset`):
///   1. Heap init — static-BSS Talck gets a fixed byte range claimed.
///   2. Pre-EBS banner via `println!` → PL011 MMIO.
///   3. `boot::exit_boot_services` — firmware tears down. The
///      returned `MemoryMapOwned` is the snapshot we feed to
///      `arch::init_memory`.
///   4. `arch::init_memory(memory_map)` — consume the firmware
///      memory map, install the UefiFrameAllocator singleton behind
///      the same accessor API the aarch64-UEFI arm publishes. Returns
///      `u32` (armv7's 32-bit pointer width).
///   5. Post-EBS banner: frame count, usable MiB, DMA pool status.
///   6. virtio_mmio scan + driver bring-up.
///   7. Halt via `wfi` loop.
#[entry]
fn efi_main() -> Status {
    // Heap init MUST be the first thing — the global allocator is an
    // empty Talck; the first alloc call before claim() would panic
    // (ClaimOnOom would then trigger but with an empty span).
    // Subsequent `println!` and any uefi-rs internal alloc work all
    // route through this heap.
    //
    // SAFETY: HEAP is a static, zero-initialized byte array. No code
    // has run that could be holding a pointer into it yet — we're
    // literally the first line of efi_main.
    unsafe {
        let heap_start = HEAP.0.get() as *mut u8;
        ALLOCATOR
            .lock()
            .claim(Span::from_base_size(heap_start, HEAP_SIZE))
            .expect("heap claim");
    }

    crate::arch::init_console();

    // Pre-EBS banner. ASCII-only (carries cleanly through QEMU's
    // `-serial stdio` PL011 UARTDR straight through to the host
    // terminal without any transcoding).
    println!("AREST kernel - armv7-UEFI scaffold");
    println!("  target: armv7-unknown-uefi (custom target JSON)");
    println!("  pre-EBS:  PL011 MMIO active at 0x0900_0000");

    // SAFETY: `boot::exit_boot_services` walks the current memory
    // map, gets the firmware's signature lock, and tears down
    // BootServices. The returned `MemoryMapOwned` is a stable copy
    // of the map the firmware handed us. We hand it straight into
    // `arch::init_memory` which flattens reclaimable regions into a
    // frame allocator and stands up the singleton.
    let memory_map = unsafe { boot::exit_boot_services(MemoryType::LOADER_DATA) };

    // Firmware BootServices is now gone. Our `println!` writes to
    // PL011 MMIO directly (not via ConOut), so it survives EBS with
    // no cutover needed — the PL011 register at 0x0900_0000 stays
    // firmware-identity-mapped under ArmVirtPkg the same way AAVMF
    // identity-maps it on aarch64.
    println!("  post-EBS: PL011 MMIO survives (no ConOut cutover needed)");

    // Consume the firmware memory map, install the UefiFrameAllocator
    // singleton + carve a 2 MiB DMA pool. `arch::armv7::init_memory`
    // returns `u32` (32-bit pointer width on armv7); we widen to `u64`
    // at the `virtio_mmio::init_offset` boundary below since that
    // module's signature is shared with the aarch64 arm. The value is
    // 0 in both cases (UEFI firmware identity-maps RAM under
    // ArmVirtPkg) so the widening is a no-op semantically.
    let phys_offset_u32 = crate::arch::init_memory(memory_map);
    let phys_offset = phys_offset_u32 as u64;

    // Proves the frame-allocator singleton is live post-EBS: going
    // through `memory::usable_frame_count()` forces a `FRAME_ALLOCATOR.lock()`
    // + a pass over the descriptor iterator, so a hung lock or a
    // malformed memory map surfaces here rather than silently later.
    let frame_count = crate::arch::memory::usable_frame_count();
    let usable_mib = (frame_count * 4096) / (1024 * 1024);
    println!(
        "  mem:      {frame_count} frames usable ({usable_mib} MiB) (UEFI memory map)"
    );

    // DMA pool carve smoke. `arch::armv7::init_memory` mirrors the
    // aarch64 arm: carves a 2 MiB contiguous region out of the
    // firmware memory map and reserves it for the virtio-mmio bring-
    // up below. This line proves the carve landed at runtime --
    // `with_dma_pool` returns `Some` only when the pool was built,
    // which in turn only happens when `dma::carve_dma_region` found
    // a big-enough reclaimable region. A `NONE` here (on a 256 MiB
    // QEMU guest with 60+ MiB usable) would indicate a regression
    // in the carve logic.
    let dma_ok = crate::arch::memory::with_dma_pool(|_| true).unwrap_or(false);
    println!(
        "  dma:      pool {} (2 MiB UEFI memory-map carve for virtio)",
        if dma_ok { "live" } else { "NONE (carve failed)" }
    );

    // virtio-mmio slot scan. Seed the HAL's phys_offset (= 0 under
    // ArmVirtPkg identity mapping) and walk the 32 virtio-mmio slots
    // QEMU's armv7 virt machine exposes at 0x0a00_0000 + 0x200 * n.
    // Same MMIO layout as the aarch64 virt machine — QEMU's
    // `hw/arm/virt.c` exposes VIRT_MMIO at the same physical address
    // on both the 32-bit and 64-bit variants of the virt machine, so
    // the scanner walks the identical address window on both arms.
    // Without `-device virtio-*-device` in the QEMU args, both
    // lookups return None; with them, the slot indices appear here.
    crate::virtio_mmio::init_offset(phys_offset);
    let virtio_net_slot = crate::virtio_mmio::find_virtio_net();
    let virtio_blk_slot = crate::virtio_mmio::find_virtio_blk();
    println!(
        "  virtio-mmio: walk OK (virtio-net: {}, virtio-blk: {})",
        match &virtio_net_slot {
            Some(s) => alloc::format!(
                "slot {} @ {:#010x}", s.index, s.base_paddr
            ),
            None => alloc::string::String::from("none"),
        },
        match &virtio_blk_slot {
            Some(s) => alloc::format!(
                "slot {} @ {:#010x}", s.index, s.base_paddr
            ),
            None => alloc::string::String::from("none"),
        },
    );

    // Actually drive the virtio devices the MMIO scanner found. Both
    // `try_init_*` functions return None when no matching slot is
    // present (they internally repeat the MMIO scan), so this block
    // is safe even if QEMU was launched without the
    // `-device virtio-*-device` pair. When virtio-net is present we
    // read + report the MAC; when virtio-blk is present we report
    // capacity + read-only flag. Mirrors the banner the aarch64 arm
    // emits — same shape, different ISA running the same MMIO transport.
    let virtio_net_dev = crate::virtio_mmio::try_init_virtio_net();
    let virtio_net_mac = virtio_net_dev.as_ref().map(|d| d.mac_address());
    match &virtio_net_mac {
        Some(m) => println!(
            "  virtio-net: driver online, MAC {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            m[0], m[1], m[2], m[3], m[4], m[5],
        ),
        None => println!("  virtio-net: no device / init failed"),
    }

    // #450: hand the discovered virtio-net device to smoltcp. Mirrors
    // the wiring DDD landed for x86_64 UEFI in entry_uefi.rs (#359),
    // now that #450 has widened the `mod net;` gate in main.rs to
    // include aarch64 + armv7 UEFI. `crate::virtio_mmio::VirtioPhy`
    // (HHH's #449 parallel of `crate::virtio::VirtioPhy` for the MMIO
    // transport) is the `smoltcp::phy::Device` adapter the cfg-gated
    // `net::KernelDevice::Virtio` arm consumes. When no virtio-net is
    // present (`virtio_net_dev = None`), `net::init` falls back to a
    // Loopback device bound to 127.0.0.1/8 so in-guest smoke still
    // has a reachable address.
    crate::net::init(virtio_net_dev.map(crate::virtio_mmio::VirtioPhy::new));
    println!("  net:      smoltcp interface live (DHCPv4 pending)");

    let virtio_blk_dev = crate::virtio_mmio::try_init_virtio_blk();
    match &virtio_blk_dev {
        Some(d) => {
            let sectors = d.capacity();
            let ro = d.readonly();
            // virtio-blk spec sector size = 512 bytes. Same constant
            // the x86_64 / aarch64 arms carry (512 per virtio 1.1
            // §5.2.3); hard-coded here because `crate::block` is
            // `cfg(target_arch = "x86_64")` gated and unreachable
            // from armv7 UEFI.
            const SECTOR_SIZE: u64 = 512;
            let cap_kib = (sectors * SECTOR_SIZE) / 1024;
            let mode = if ro { "read-only" } else { "read-write" };
            println!(
                "  virtio-blk: driver online, {sectors} sectors ({cap_kib} KiB), {mode}"
            );
        }
        None => println!("  virtio-blk: no device / init failed"),
    }

    println!("  next:   ExitBootServices + memory map (follow-ups)");

    // Halt via wfi loop. Returns `!`, so the `Status` return on the
    // `#[entry]` fn is unreachable — uefi-rs's macro expands the
    // signature check anyway; halt_forever's divergence satisfies
    // both the compiler and the firmware's caller convention.
    crate::arch::halt_forever();
}

/// Panic handler for the armv7 UEFI path. The aarch64 arm's
/// `entry_uefi_aarch64.rs` panic handler raw-MMIOs PL011 at
/// 0x0900_0000; here we do the same thing on the 32-bit ARM ISA so
/// a fault surfaces as a visible "!! UEFI kernel panic !!" marker
/// rather than a silent hang.
///
/// Uses a stack-local writer targeting the same PL011 UARTDR the
/// banner writes to — no alloc dependency (so a panic inside an
/// allocator hook can't deadlock), no singleton (so a panic mid-
/// mutation of a future serial-state struct can't fight it).
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    use core::fmt::Write;

    /// QEMU virt PL011 UARTDR address. Duplicated from
    /// `arch::armv7::serial` so the panic path has zero module
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
    let _ = w.write_str("\r\n!! UEFI kernel panic (armv7) !!\r\n");
    let _ = writeln!(w, "{info}");

    loop {
        // SAFETY: `wfi` is unprivileged in PL1 on armv7 and has no
        // side effects beyond pausing until the next interrupt.
        // `nomem` / `nostack` / `preserves_flags` describe it
        // accurately to the compiler.
        unsafe {
            core::arch::asm!(
                "wfi",
                options(nomem, nostack, preserves_flags),
            );
        }
    }
}

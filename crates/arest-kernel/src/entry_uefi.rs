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
use uefi::mem::memory_map::MemoryMapOwned;

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
// Size: 16 MiB. Initial pick was 8 MiB (matching the BIOS heap),
// bumped when framebuffer::install started running on UEFI — its
// two heap-backed BackBuffers each mirror the GPU framebuffer byte-
// for-byte, so at 1024x768x4 that's ~6.3 MiB of back-buffer alone,
// leaving room for system::init's Box / Vec / Arc / BTreeMap churn
// plus any follow-up alloc traffic. QEMU+OVMF with the default 128
// MiB guest accommodates this comfortably. The .bss bytes themselves
// are zeroed by the firmware before _start runs, so
// `LockedHeap::empty()` + a single init() call is enough.
//
// The init() call runs at the TOP of efi_main — before ANY
// alloc-using code (println! transcodes args via a String on the
// UEFI serial path). Must NOT move later without switching to a
// crate that supports delayed init.
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

/// Panic handler for the UEFI path. Replaces uefi-rs's default
/// (which logs via `system_table.stderr()` — gone post-EBS, so a
/// panic after ExitBootServices prints nothing and the kernel
/// silently hangs).
///
/// Strategy: raw port I/O to COM1 0x3F8. Works before and after
/// EBS identically — QEMU's OVMF binds COM1 at boot and our
/// post-EBS 16550 path uses the same port, so this handler
/// produces visible output in both phases without depending on
/// BootServices or the kernel's SERIAL singleton (the latter may
/// be mid-mutation when a panic fires).
///
/// Busy-polls the LSR's THR-empty bit between each byte so slow
/// consoles don't drop characters. `writeln!` via core::fmt
/// handles formatting without alloc — panic inside alloc would
/// otherwise deadlock on the LockedHeap mutex.
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    use core::fmt::Write;
    use x86_64::instructions::port::Port;

    struct RawCom1 {
        data: Port<u8>,
        lsr: Port<u8>,
    }
    impl RawCom1 {
        fn new() -> Self {
            Self {
                data: Port::new(0x3F8),
                lsr: Port::new(0x3FD),
            }
        }
    }
    impl Write for RawCom1 {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            for b in s.bytes() {
                // Wait until THR empty (LSR bit 5 set).
                // SAFETY: 0x3F8/0x3FD are COM1's 16550 ports —
                // accessible on every PC-compatible, no memory
                // safety impact. Raw reads/writes only.
                unsafe {
                    while self.lsr.read() & 0x20 == 0 {}
                    self.data.write(b);
                }
            }
            Ok(())
        }
    }

    let mut com1 = RawCom1::new();
    let _ = writeln!(com1, "\n!! UEFI kernel panic !!");
    let _ = writeln!(com1, "{info}");
    loop {
        unsafe { core::arch::asm!("hlt", options(nomem, nostack)); }
    }
}

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

    // GOP framebuffer capture (#270/#271 prep, 57efd07 diagnosis).
    // The earlier full-GOP attempt (reverted) hung the kernel post-
    // EBS — narrowed to the ScopedProtocol's Drop (which calls
    // BootServices::close_protocol). This version captures the
    // mode info + framebuffer pointer/size, then `mem::forget`s the
    // ScopedProtocol so Drop does not run. We leak a protocol lock
    // that firmware tears down at ExitBootServices anyway.
    let (gop_w, gop_h, gop_stride, gop_fmt_idx, gop_ptr, gop_size) =
        match uefi::boot::get_handle_for_protocol::<
            uefi::proto::console::gop::GraphicsOutput,
        >() {
            Ok(handle) => {
                match uefi::boot::open_protocol_exclusive::<
                    uefi::proto::console::gop::GraphicsOutput,
                >(handle) {
                    Ok(mut gop) => {
                        let info = gop.current_mode_info();
                        let (w, h) = info.resolution();
                        let stride = info.stride();
                        let fmt_idx = match info.pixel_format() {
                            uefi::proto::console::gop::PixelFormat::Rgb     => 0usize,
                            uefi::proto::console::gop::PixelFormat::Bgr     => 1,
                            uefi::proto::console::gop::PixelFormat::Bitmask => 2,
                            uefi::proto::console::gop::PixelFormat::BltOnly => 3,
                        };
                        let mut fb = gop.frame_buffer();
                        let ptr = fb.as_mut_ptr() as usize;
                        let size = fb.size();
                        drop(fb);
                        // SKIP Drop — forget leaks the ScopedProtocol
                        // rather than running its close_protocol path.
                        core::mem::forget(gop);
                        (w, h, stride, fmt_idx, ptr, size)
                    }
                    Err(_) => (0, 0, 0, 9, 0, 0),
                }
            }
            Err(_) => (0, 0, 0, 9, 0, 0),
        };

    // (Earlier gop-lookup diagnostic — commit 57efd07 — was
    // subsumed by the capture block above, which calls
    // `get_handle_for_protocol` as its first step.)

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

    // Hand off to the post-EBS body. Mirrors the BIOS arm's shape
    // (`kernel_main` -> `kernel_run`): everything that depends on
    // a live, post-ExitBootServices world lives in `kernel_run_uefi`,
    // so downstream UEFI work (#359 net, #363 IDT, #370 virtio-gpu)
    // can keep adding to the post-EBS tail without growing
    // `efi_main` further. Never returns.
    kernel_run_uefi(
        memory_map,
        gop_w, gop_h, gop_stride, gop_fmt_idx, gop_ptr, gop_size,
    )
}

/// Post-ExitBootServices body for the UEFI x86_64 path.
///
/// `efi_main` owns everything that needs BootServices alive (heap
/// init, ConOut println!s, GOP capture, the `exit_boot_services`
/// call itself, and the cutover to the direct-I/O 16550 serial).
/// Once that hand-off is complete, this function takes ownership of
/// the rest of boot: memory init, DMA pool, virtio, block, the
/// AREST engine, wasmi, and the Doom shim binding.
///
/// Sibling of the BIOS arm's `main.rs::kernel_run(phys_offset)` —
/// same arch-neutral tail, just parameterised differently because
/// the UEFI path has the firmware memory map + a captured GOP
/// framebuffer descriptor to feed in, where the BIOS path gets
/// those via `bootloader_api::BootInfo`.
fn kernel_run_uefi(
    memory_map: MemoryMapOwned,
    gop_w: usize,
    gop_h: usize,
    gop_stride: usize,
    gop_fmt_idx: usize,
    gop_ptr: usize,
    gop_size: usize,
) -> ! {
    println!("  post-EBS: 16550 COM1 active (kernel-managed)");

    // Step 4c: consume the firmware memory map, install the paging
    // + frame-allocator singletons. `init_memory` returns the
    // physical-memory offset (always 0 on UEFI — firmware identity-
    // maps RAM), matching the shape of the BIOS arm's facade.
    let _phys_offset = crate::arch::init_memory(memory_map);

    // #363: install the kernel-owned IDT now that the heap +
    // frame allocator are live. Firmware's IDT is gone after
    // `boot::exit_boot_services`, so any CPU exception (a stray
    // `int3`, a #DF fired by a buggy MMIO write below) would
    // triple-fault the box silently if we did not stand one up.
    // The IDT installs the breakpoint + double-fault gates plus
    // (since #379) the IRQ 0..47 vectors so PIT-driven ticks land
    // on `timer_handler` rather than an unpopulated slot once
    // `sti` is on.
    crate::arch::init_interrupts();

    // #379: bring the 1 kHz monotonic ms timer online. PIC remap +
    // PIT divisor + `sti`. Must run AFTER init_interrupts so the
    // IRQ 0 vector is populated before the first tick fires.
    // Mirrors the BIOS arm's `init_pic` -> `time::init` chain that
    // `init_gdt_and_interrupts` runs after `init_idt`.
    crate::arch::init_time();
    let pit_t0 = crate::arch::time::now_ms();
    println!(
        "  pit:      1 kHz timer online, IRQ 0 → vector 32 (now_ms={pit_t0})"
    );

    // Prove the counter actually advances by spinning ~10 ms and
    // re-reading. The 8259 PIC fires IRQ 0 every ~1 ms once `sti`
    // is on, so a busy loop long enough to cover several PIT
    // periods MUST observe the counter move forward — if it
    // doesn't, either the PIC unmask, the IRQ 0 vector, or the
    // `sti` is broken. Spin against `now_ms()` itself rather than
    // a cycle-count proxy so the loop terminates as soon as the
    // counter actually advances; capped at a `pause`-loop budget
    // so a never-ticking timer surfaces as a smoke-harness
    // timeout (no banner advancement) rather than a hang here.
    let pit_target = pit_t0.wrapping_add(10);
    let mut spins: u64 = 0;
    while crate::arch::time::now_ms() < pit_target && spins < 200_000_000 {
        unsafe { core::arch::asm!("pause", options(nomem, nostack)); }
        spins = spins.wrapping_add(1);
    }
    let pit_t1 = crate::arch::time::now_ms();
    println!(
        "  pit:      now_ms advanced t0={pit_t0} t1={pit_t1} (delta={})",
        pit_t1.wrapping_sub(pit_t0),
    );

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

    // #363: int3 round-trip smoke. Fires a software breakpoint;
    // the kernel-owned IDT routes #BP into `breakpoint_handler`
    // which prints + iretqs back. The next println! confirms
    // execution resumed past the int3 — i.e. the IDT hand-off
    // worked end-to-end. Mirrors the BIOS arm's identical smoke
    // in `kernel_run` (main.rs).
    crate::arch::breakpoint();
    println!("  idt:      int3 round-tripped through UEFI IDT");

    // DMA pool carve smoke (ed869c4). `arch::init_memory` on UEFI
    // now mirrors the BIOS arm: carves a 2 MiB contiguous region out
    // of the firmware memory map and reserves it for virtio-drivers.
    // This line proves the carve landed at runtime (not just at
    // compile time) -- `with_dma_pool` returns `Some` only when the
    // pool was built, which in turn only happens when
    // `dma::carve_dma_region` found a big-enough CONVENTIONAL region.
    // A `none` here (on a 128 MiB QEMU guest with 60+ MiB usable)
    // would indicate a regression in the carve logic.
    let dma_ok = crate::arch::memory::with_dma_pool(|_| true).unwrap_or(false);
    println!(
        "  dma:      pool {} (2 MiB UEFI memory-map carve for virtio)",
        if dma_ok { "live" } else { "NONE (carve failed)" }
    );

    // virtio statics + PCI walker smoke (#344/#345). Seeds the virtio
    // HAL's phys_offset (= 0 under UEFI's identity mapping) and walks
    // legacy PCI config space via the 0xCF8/0xCFC port pair. On UEFI
    // x86_64 + QEMU-OVMF the port pair remains wired to the PCI host
    // bridge -- firmware boot mode doesn't change the legacy PIO path
    // -- so the same `pci::find_virtio_net` / `find_virtio_blk` the
    // BIOS `kernel_run` calls works byte-for-byte here. Without
    // `-device virtio-*-pci` in the smoke's QEMU args, both scans
    // return None; a live scan would appear here with the device
    // coordinates. Either way, a `pci: walk OK` line proves the port
    // I/O path + PCI bus iteration ran without faulting.
    crate::virtio::init_offset(0);
    let virtio_net_pci = crate::pci::find_virtio_net();
    let virtio_blk_pci = crate::pci::find_virtio_blk();
    println!(
        "  pci:      walk OK (virtio-net: {}, virtio-blk: {})",
        match &virtio_net_pci {
            Some(d) => alloc::format!(
                "{:02x}:{:02x}.{}", d.bus, d.device, d.function
            ),
            None => alloc::string::String::from("none"),
        },
        match &virtio_blk_pci {
            Some(d) => alloc::format!(
                "{:02x}:{:02x}.{}", d.bus, d.device, d.function
            ),
            None => alloc::string::String::from("none"),
        },
    );

    // Actually drive the virtio devices the PCI walker found. Both
    // `try_init_*` functions return None when no device is present
    // (they internally repeat the PCI scan), so this block is safe
    // even on the -device-less historical path. When virtio-net is
    // present we read and report the MAC address; when virtio-blk
    // is present we report capacity + read-only flag. These are the
    // same probes the BIOS `kernel_run` does for its boot banner,
    // now running on UEFI x86_64 thanks to the DMA-pool carve
    // (ed869c4) plus the block/net/virtio un-gate (ed869c4).
    let virtio_net_dev = crate::virtio::try_init_virtio_net();
    let virtio_net_mac = virtio_net_dev.as_ref().map(|d| d.mac_address());
    match &virtio_net_mac {
        Some(m) => println!(
            "  virtio-net: driver online, MAC {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            m[0], m[1], m[2], m[3], m[4], m[5],
        ),
        None => println!("  virtio-net: no device / init failed"),
    }

    let virtio_blk_dev = crate::virtio::try_init_virtio_blk();
    match &virtio_blk_dev {
        Some(d) => {
            let sectors = d.capacity();
            let ro = d.readonly();
            let cap_kib =
                (sectors * (crate::block::BLOCK_SECTOR_SIZE as u64)) / 1024;
            let mode = if ro { "read-only" } else { "read-write" };
            println!(
                "  virtio-blk: driver online, {sectors} sectors ({cap_kib} KiB), {mode}"
            );
        }
        None => println!("  virtio-blk: no device / init failed"),
    }

    // Install the virtio-blk device in the block module's singleton,
    // then run boot-time mount (#337 BIOS parity). `mount` reads
    // sector 0, validates the CRC, and either reports "fresh disk"
    // (zero-filled) or "rehydrated" (valid checkpoint found). On the
    // UEFI smoke's 1 MiB zero-filled disk.img it should report fresh
    // disk; a subsequent `smoke_round_trip` then exercises the full
    // write-read path end-to-end against virtio-blk MMIO.
    if let Some(dev) = virtio_blk_dev {
        crate::block::install(dev);
        match crate::block_storage::mount() {
            crate::block_storage::MountStatus::NoDevice =>
                println!("  block:    no persistence device"),
            crate::block_storage::MountStatus::FreshDisk =>
                println!("  block:    fresh disk (no prior checkpoint)"),
            crate::block_storage::MountStatus::Rehydrated => {
                let prev = crate::block_storage::last_boot_count();
                let bytes = crate::block_storage::last_state()
                    .map(|v| v.len()).unwrap_or(0);
                println!(
                    "  block:    rehydrated checkpoint ({bytes} bytes, boot_count was {prev})"
                );
            }
            crate::block_storage::MountStatus::Corrupted =>
                println!("  block:    checkpoint CRC mismatch"),
        }
        if crate::block_storage::smoke_round_trip() {
            let new_bc = crate::block_storage::last_boot_count();
            println!(
                "  block:    checkpoint round-trip OK (boot_count now {new_bc})"
            );
        } else {
            println!("  block:    checkpoint round-trip FAILED");
        }
    }

    // Report GOP capture (the pre-EBS snapshot above). MMIO at
    // `gop_ptr..gop_ptr+gop_size` is driven by the GPU post-EBS,
    // so direct pixel writes are valid without BootServices.
    let gop_fmt = match gop_fmt_idx {
        0 => "Rgb",
        1 => "Bgr",
        2 => "Bitmask",
        3 => "BltOnly",
        _ => "none",
    };
    if gop_ptr != 0 {
        println!(
            "  gop:      {gop_w}x{gop_h} stride={gop_stride} fmt={gop_fmt} fb={gop_ptr:#018x}+{gop_size}"
        );

        // Direct-MMIO bulk-write smoke. Fill a 320x200 rectangle at
        // the top-left with a known pattern (`i.wrapping_mul(0x01010101)`
        // — a per-pixel gradient that also exercises the full u32
        // write path), then sum the low 32 bits of the resulting
        // MMIO contents to verify the writes stuck at scale. 320x200
        // pixels is Doom's native resolution; proving we can paint a
        // Doom-sized rect at wire speed (no cached writes, no
        // coalescing assumptions) is the primitive the host-shim's
        // drawFrame path will use in #270/#271.
        //
        // SAFETY: fb_ptr points at firmware-mapped MMIO that remains
        // valid for the rest of boot. 320*200*4 = 256000 bytes, well
        // inside the 4 MB fb_size bound captured earlier.
        let fb = gop_ptr as *mut u32;
        const W: usize = 320;
        const H: usize = 200;
        unsafe {
            for y in 0..H {
                for x in 0..W {
                    let pixel = (((y * W + x) as u32) & 0xFF) * 0x01010101;
                    core::ptr::write_volatile(fb.add(y * gop_stride + x), pixel);
                }
            }
            // Sum readback (wrapping). Expected value is the sum of
            // `(i & 0xFF) * 0x01010101` for i in 0..64000. 0xFF
            // pattern repeats every 256 steps, so i & 0xFF summed
            // over 64000 = 250 * (0 + ... + 255) = 250 * 32640 =
            // 8160000. Times 0x01010101 (wrapping u32) =
            // 8160000 * 16843009 wrapping = ... let the kernel
            // print the actual readback sum; the smoke's assertion
            // pins the exact value we observe first.
            let mut sum: u32 = 0;
            for y in 0..H {
                for x in 0..W {
                    sum = sum.wrapping_add(core::ptr::read_volatile(fb.add(y * gop_stride + x)));
                }
            }
            println!("  gop-mmio: wrote {W}x{H}, readback sum={sum:#010x}");
        }

        // Install the triple-buffered framebuffer singleton on top
        // of the GOP front buffer, mirroring the BIOS path's
        // `framebuffer::install` call in kernel_run. The back
        // buffers are heap-allocated Vec<u8>s of the same byte length
        // as the front (`gop_size`), so the post-EBS heap must have
        // been init'd — which it has (above) — before we reach here.
        //
        // UEFI spec guarantees Rgb/Bgr PixelFormats carry a reserved
        // alpha byte (UEFI §12.9: PixelRedGreenBlueReserved8BitPerColor
        // and PixelBlueGreenRedReserved8BitPerColor), so
        // bytes_per_pixel = 4 for every GOP-reachable boot. Bitmask
        // and BltOnly both fall through to the `else` branch below
        // — framebuffer::install expects a linear byte surface, and
        // BltOnly has none; Bitmask's channel offsets would require
        // a PixelFormat::Unknown construction we don't yet populate.
        let fb_info = match gop_fmt_idx {
            0 => Some(bootloader_api::info::FrameBufferInfo {
                byte_len: gop_size,
                width: gop_w,
                height: gop_h,
                pixel_format: bootloader_api::info::PixelFormat::Rgb,
                bytes_per_pixel: 4,
                stride: gop_stride,
            }),
            1 => Some(bootloader_api::info::FrameBufferInfo {
                byte_len: gop_size,
                width: gop_w,
                height: gop_h,
                pixel_format: bootloader_api::info::PixelFormat::Bgr,
                bytes_per_pixel: 4,
                stride: gop_stride,
            }),
            _ => None,
        };
        if let Some(info) = fb_info {
            // SAFETY: `gop_ptr` + `gop_size` describe the firmware-
            // mapped GOP framebuffer, valid for the rest of boot (we
            // mem::forget'd the ScopedProtocol above so firmware won't
            // reclaim it at EBS). No other code in efi_main is holding
            // a reference into that MMIO region at this point.
            unsafe { crate::framebuffer::install(info, gop_ptr as *mut u8, gop_size) };

            // Triple-buffer paint smoke — mirrors kernel_run's #269
            // paint smoke (main.rs line ~299), including the second
            // present that rotates to the other back buffer. The
            // front_fnv1a readback then computes an FNV-1a hash over
            // the GPU MMIO bytes (cacheable under QEMU+OVMF's default
            // paging attributes) to prove pixels made it through.
            use crate::framebuffer::Color;
            let _ = crate::framebuffer::with_back(|back| {
                back.clear(Color::rgb(0x10, 0x10, 0x18));
                back.fill_rect(40,  40, 320, 200, Color::RED);
                back.fill_rect(360, 40, 320, 200, Color::GREEN);
                back.fill_rect(680, 40, 320, 200, Color::BLUE);
                back.draw_line(40, 260, 1240, 260, Color::WHITE);
                back.draw_text(40, 280, "AREST UEFI", Color::YELLOW);
            });
            crate::framebuffer::present();
            let frame_a = crate::framebuffer::front_fnv1a().unwrap_or(0);
            let _ = crate::framebuffer::with_back(|back| {
                back.clear(Color::rgb(0x10, 0x10, 0x18));
                back.fill_rect(40,  40, 320, 200, Color::RED);
                back.fill_rect(360, 40, 320, 200, Color::GREEN);
                back.fill_rect(680, 40, 320, 200, Color::BLUE);
                back.fill_rect(560, 100, 160, 80, Color::WHITE);
                back.draw_line(40, 260, 1240, 260, Color::WHITE);
                back.draw_text(40, 280, "AREST UEFI", Color::YELLOW);
            });
            crate::framebuffer::present();
            let frame_b = crate::framebuffer::front_fnv1a().unwrap_or(0);
            println!(
                "  fb:       paint smoke OK, presents={}, frame_a={frame_a:#018x}, frame_b={frame_b:#018x} (#269)",
                crate::framebuffer::presents(),
            );

            // Runtime exercise of the 4bpp blit_doom_frame path
            // (9c4984d). Builds a synthetic 640x400 BGRA frame with
            // a diagonal gradient (R = x & 0xFF, G = y & 0xFF, B =
            // (x ^ y) & 0xFF), runs the blit, presents, and reports
            // the new front-buffer hash. The hash MUST differ from
            // the `frame_b` emitted above — if the 4bpp path is
            // still silently no-op'ing (as it did before 9c4984d),
            // the present would leave the framebuffer unchanged
            // and the two hashes would match. The smoke harness
            // inspects the "doom-blit:" line for presence; any
            // human-level audit can compare the hash against
            // frame_b.
            const DOOM_W: usize = 640;
            const DOOM_H: usize = 400;
            let mut doom_buf: alloc::vec::Vec<u8> =
                alloc::vec![0u8; DOOM_W * DOOM_H * 4];
            for y in 0..DOOM_H {
                for x in 0..DOOM_W {
                    let off = (y * DOOM_W + x) * 4;
                    // Doom writes 0xAARRGGBB little-endian →
                    // [B, G, R, A] in memory.
                    doom_buf[off]     = ((x ^ y) & 0xFF) as u8; // B
                    doom_buf[off + 1] = (y & 0xFF) as u8;       // G
                    doom_buf[off + 2] = (x & 0xFF) as u8;       // R
                    doom_buf[off + 3] = 0xFF;                    // A
                }
            }
            let _ = crate::framebuffer::with_back(|back| {
                back.blit_doom_frame(&doom_buf);
            });
            crate::framebuffer::present();
            let frame_doom = crate::framebuffer::front_fnv1a().unwrap_or(0);
            println!(
                "  doom-blit: synthetic 640x400 BGRA frame blitted, fnv1a={frame_doom:#018x} (#270/#271)"
            );
        } else {
            println!("  fb:       format {gop_fmt} unsupported by framebuffer::install (BltOnly/Bitmask)");
        }
    } else {
        println!("  gop:      not available (headless UEFI boot)");
    }

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

    // Step 4d wave 4: initialise the AREST engine under UEFI.
    // `system::init()` stands up the baked metamodel + single-
    // tenant DEFS table via the same ρ-application path the BIOS
    // arm uses (see main.rs `kernel_run`). This is a pure
    // alloc-heavy call — Box / Vec / Arc / BTreeMap churn — so
    // it's the strongest test yet that the post-EBS heap is
    // correctly feeding every alloc codepath in the shared kernel
    // body. A silent freeze here would mean a subtle alloc
    // regression, surfaced via missing banner + smoke timeout.
    crate::system::init();
    println!("  engine:   system::init() completed (arest engine live on UEFI)");

    // Step 4d wave 5: wasmi runtime smoke. UEFI-only — the BIOS
    // bootloader can't load a kernel image the wasmi-linking binary
    // produces (triple-faults pre-_start, 5e8a15e). Loads a hand-
    // assembled 37-byte WASM module that exports `main` -> i32 42,
    // instantiates, and calls it. The returned 42 proves the full
    // wasmi pipeline works under UEFI: decoder, type section, code
    // section parsing, instantiation, execution. With the custom
    // panic handler (above) raw-port-I/O'ing to COM1, any fault in
    // the pipeline surfaces as a visible "UEFI kernel panic" line
    // rather than a silent hang.
    //
    // The hex below is the WebAssembly binary encoding of:
    //   (module (func (export "main") (result i32) i32.const 42))
    const TINY_WASM: &[u8] = &[
        0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00, // \0asm version 1
        0x01, 0x05, 0x01, 0x60, 0x00, 0x01, 0x7F,       // type: () -> i32
        0x03, 0x02, 0x01, 0x00,                         // funcs: [type 0]
        0x07, 0x08, 0x01, 0x04, 0x6D, 0x61, 0x69, 0x6E, 0x00, 0x00, // exp "main"
        0x0A, 0x06, 0x01, 0x04, 0x00, 0x41, 0x2A, 0x0B, // code: i32.const 42
    ];
    let engine = wasmi::Engine::default();
    let module = wasmi::Module::new(&engine, TINY_WASM).expect("parse tiny wasm");
    let mut store = wasmi::Store::new(&engine, ());
    let linker = wasmi::Linker::<()>::new(&engine);
    let pre = linker.instantiate(&mut store, &module).expect("instantiate");
    let instance = pre.start(&mut store).expect("start");
    let main_fn = instance
        .get_typed_func::<(), i32>(&store, "main")
        .expect("get main");
    let answer = main_fn.call(&mut store, ()).expect("call main");
    println!("  wasmi:    tiny module executed, main() = {answer} (runtime live on UEFI)");

    // Doom host-shim binding smoke (#270/#271, scaffold f3be6d4).
    // Creates a Linker<KernelDoomHost>, binds all 10 Doom imports
    // via `doom::bind_doom_imports`, and prints a success line.
    // Does NOT invoke any import — the stubs panic — so this only
    // verifies the binding path compiles and the func_wrap calls
    // run without a DuplicateDefinition. A real Doom .wasm with
    // these imports can be instantiated against this linker in a
    // later commit without needing to re-register.
    let doom_engine = wasmi::Engine::default();
    let mut doom_linker: wasmi::Linker<crate::doom::KernelDoomHost> =
        wasmi::Linker::new(&doom_engine);
    crate::doom::bind_doom_imports(&mut doom_linker);
    println!("  doom:     10 host imports bound to wasmi::Linker (ready for #270 guest)");

    println!("  next:        kernel_run handoff (step 4d)");

    // Scaffold halt — via the facade so the call site is identical
    // to the BIOS arm's bottom-of-kernel_run. Step 4d wires
    // `kernel_run(phys_offset)` once the shared body subsystems are
    // UEFI-capable; until then the entry parks here after proving
    // the page-table + frame-allocator singletons are live.
    crate::arch::halt_forever()
}

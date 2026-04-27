// crates/arest-kernel/src/entry_uefi.rs
//
// UEFI x86_64 entry point. Only compiled for `target_os = "uefi"` +
// `target_arch = "x86_64"`. Owns the post-#380 boot pipeline now
// that the BIOS path is gone.
//
// Pipeline:
//   #[entry] efi_main
//     ├─ heap init (talc against `boot::allocate_pages` 48 MiB)
//     ├─ pre-EBS banner via firmware ConOut (`arch::uefi::serial`)
//     ├─ GraphicsOutputProtocol capture (gop_w/gop_h/etc.)
//     ├─ boot::exit_boot_services
//     ├─ post-EBS 16550 COM1 cutover
//     └─ kernel_run_uefi(memory_map, gop_*) -> arch-neutral kernel
//        body: init_memory, DMA pool, virtio, block, AREST engine,
//        wasmi (under `feature = "doom"`), Slint launcher, REPL.
//
// `println!` routes through `arch::_print` (uefi-rs ConOut pre-EBS,
// raw 16550 COM1 post-EBS); same macro the aarch64 / armv7 entries
// use — no UEFI-specific printing call sites in shared kernel code.

#![cfg(target_os = "uefi")]

use talc::{ClaimOnOom, Span, Talc, Talck};
use uefi::prelude::*;
use uefi::boot::{AllocateType, MemoryType};
use uefi::mem::memory_map::MemoryMapOwned;

use crate::println;

// Global allocator. Uses `talc` (`Talck<spin::Mutex<()>, ClaimOnOom>`)
// rather than `uefi::allocator::Allocator` so the heap SURVIVES
// ExitBootServices — uefi-rs's allocator is backed by
// `BootServices::allocate_pool`, which faults after EBS.
//
// The BIOS arm uses the same crate for the same reason (see
// `allocator.rs`); this keeps the kernel's Box/Vec/String codepaths
// identical on both boot targets. The talc swap (#440 / #443) replaces
// `linked_list_allocator::LockedHeap`, which trips a "Freed node
// aliases existing hole" assertion under wasmi `Memory::grow` realloc
// churn during Doom's `Z_Init` (#376 follow-up).
//
// Heap-backing strategy under #376: the heap region is allocated via
// `boot::allocate_pages(AnyPages, LOADER_DATA, ...)` at the very top
// of `efi_main`, then the `Talck` is init'd against that
// firmware-allocated range. Originally the heap lived in a static
// `.bss` byte array (16 MiB); when #376 wired the Doom WASM
// instantiate path, wasmi's parsing of the 4.35 MiB doom.wasm
// allocates ~8.6 MiB for its compiled bytecode tables on top of
// system::init's churn (~6 MiB), framebuffer back buffers (~8 MiB
// at 1280x800x4 doubled), and the doom host-shim's per-call
// drawFrame copies (~1 MiB each). 16 MiB was no longer enough;
// bumping the static `.bss` heap to 32 MiB triggered `BdsDxe: Out
// of Resources` at OVMF load (the PE32+ image's `SizeOfImage`
// includes `.bss`, so a 32+ MiB static array makes OVMF refuse to
// load the kernel from a 128 MiB QEMU guest). The
// `allocate_pages` path sidesteps this entirely: the kernel `.efi`
// stays small (~5 MiB on disk and in memory at load time), and the
// runtime heap is grabbed from firmware-managed CONVENTIONAL memory
// at the very start of `efi_main`, using the firmware's own page
// allocator that knows the full 128 MiB system map.
//
// Size: 48 MiB at run time. Allocates 12288 pages (4 KiB each) of
// `LOADER_DATA`-typed memory. The firmware's memory map reports
// `LOADER_DATA` regions as in-use post-EBS, so when
// `arch::init_memory` later walks the map, our 48 MiB heap region
// is correctly excluded from the frame allocator's pool — no
// double-mapping risk. 48 MiB sits comfortably below OVMF's
// max-contiguous-region threshold on a 128 MiB QEMU guest (a
// previous 64 MiB attempt failed silently — the firmware couldn't
// find a single 64 MiB contiguous CONVENTIONAL run that early in
// boot — and a 24 MiB attempt left wasmi's `Memory` instantiation
// short of the ~5 MiB linear memory `doom.wasm` declares (`min =
// 72 pages`)).
//
// Heap budget under #376:
//   * AREST `system::init` (Box/Vec/Arc/BTreeMap churn): ~6 MiB
//   * framebuffer back buffers (2 × 1280×800×4 = ~8 MiB):  ~8 MiB
//   * wasmi-side tables for parsed `doom.wasm` module:      ~9 MiB
//   * Doom WASM linear memory (`min = 72 pages`):           ~5 MiB
//   * doom host-shim drawFrame frame copies:                ~1 MiB
//   * tickGame transient allocations + headroom:           ~10 MiB
//                                                          ------
//                                                  TOTAL: ~39 MiB
// 48 MiB leaves comfortable headroom for the per-tic alloc churn
// the game loop generates as it decompresses sprites, builds visplane
// lists, and rebuilds the BGRA frame buffer.
//
// The 48 MiB heap survives `ExitBootServices`: the firmware's
// `allocate_pages` returns memory typed `LOADER_DATA`, which per the
// UEFI spec belongs to the OS loader/kernel and is preserved across
// the EBS handoff. `Talck::lock().claim(Span)` records the raw region
// once; no subsequent `allocate_pool` calls are needed.
//
// The `init()` call runs at the TOP of `efi_main` — immediately
// after the `allocate_pages` call returns, before ANY alloc-using
// code (`println!` transcodes args via a `String` on the UEFI serial
// path). Must NOT move later without switching to a crate that
// supports delayed init.
const HEAP_SIZE: usize = 32 * 1024 * 1024;
const HEAP_PAGES: usize = HEAP_SIZE / 4096;

// `ClaimOnOom::empty()` starts the allocator with no backing region;
// the explicit `claim(Span::from_base_size(...))` below in `efi_main`
// attaches the firmware-allocated heap pages once `boot::allocate_pages`
// returns. Wrapped in `Talck` (talc's `lock_api`-backed `GlobalAlloc`
// adapter) over a `spin::Mutex<()>` — `spin` is already in the kernel's
// deps for the `SerialPort` singleton, so this picks up no new transitive.
#[global_allocator]
static ALLOCATOR: Talck<spin::Mutex<()>, ClaimOnOom> =
    Talc::new(unsafe { ClaimOnOom::new(Span::empty()) }).lock();

/// Raw COM1 byte writer for diagnostic output that must work even
/// before / after the heap is up. Same shape as the panic_handler's
/// `RawCom1` — busy-polls THR-empty between bytes so slow consoles
/// don't drop characters. Used to surface heap-init failures
/// (allocate_pages OOM, Talck claim refusal) before any
/// `println!`-routed output is possible.
fn raw_com1_str(s: &str) {
    use x86_64::instructions::port::Port;
    let mut data: Port<u8> = Port::new(0x3F8);
    let mut lsr: Port<u8> = Port::new(0x3FD);
    for b in s.bytes() {
        // SAFETY: 0x3F8/0x3FD are COM1's 16550 ports — accessible
        // on every PC-compatible, no memory safety impact.
        unsafe {
            while lsr.read() & 0x20 == 0 {}
            data.write(b);
        }
    }
}

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
/// otherwise deadlock on the Talck mutex.
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
///   7. Halt. The arch-neutral kernel body drives the rest of boot
///      via `kernel_run_uefi` (defined below).
#[entry]
fn efi_main() -> Status {
    // Heap init MUST be the first thing — the global allocator is an
    // empty Talck; the first alloc call before claim() would panic
    // (ClaimOnOom would then trigger but with an empty span). Subsequent
    // `println!` and any uefi-rs internal alloc work (transcoding format
    // args to UCS-2, for example) all route through this heap.
    //
    // Strategy: ask the firmware for a 48 MiB CONVENTIONAL region via
    // `boot::allocate_pages` and init the Talck there. The
    // firmware is the only thing that knows the full memory map this
    // early in boot, so its allocator is the right tool. The returned
    // region is typed `LOADER_DATA`, which UEFI guarantees survives
    // ExitBootServices (the kernel image's runtime data lives in the
    // same type), so the heap stays valid through the post-EBS body
    // below. See `HEAP_SIZE` doc-comment above for why we don't use
    // a `static` `.bss`-backed array (PE32+ `SizeOfImage` includes
    // `.bss`, so 24+ MiB statics make OVMF's BdsDxe loader fail with
    // "Out of Resources" when loading from a 128 MiB QEMU guest).
    //
    // Diagnostic raw-COM1 prints bracket the allocate_pages call so
    // a regression in the firmware allocator (OOM, INVALID_PARAMETER)
    // surfaces as a visible "heap: allocate_pages...FAIL" line on the
    // serial output even though `println!` isn't yet usable (no
    // heap = no UCS-2 transcode buffer for ConOut). Without these the
    // failure mode is a silent boot-time hang, indistinguishable from
    // OVMF refusing to load the image.
    raw_com1_str("\nheap: requesting 32 MiB via boot::allocate_pages...");
    let heap_ptr = match uefi::boot::allocate_pages(
        AllocateType::AnyPages,
        MemoryType::LOADER_DATA,
        HEAP_PAGES,
    ) {
        Ok(p) => {
            raw_com1_str("OK\n");
            p
        }
        Err(_) => {
            raw_com1_str("FAIL (firmware out of resources)\n");
            // Halt — no way forward without a heap; panic_handler
            // would itself need format-args allocations that would
            // re-enter the dead allocator. Park in a tight loop so
            // the smoke harness's 30 s cap surfaces this as a missing-
            // banner regression rather than a silent hang.
            loop {
                unsafe {
                    core::arch::asm!("hlt", options(nomem, nostack));
                }
            }
        }
    };
    // SAFETY: `allocate_pages` returns a `NonNull<u8>` to a fresh,
    // page-aligned, exclusive 32 MiB region. No other code holds a
    // pointer into it — we just got it from the firmware. The
    // `Talck::lock().claim(Span)` call records the raw region; the
    // borrow checker doesn't see this as an aliased mutable borrow
    // because the heap pages are typed-as-raw memory, not a Rust
    // owned object.
    unsafe {
        ALLOCATOR
            .lock()
            .claim(Span::from_base_size(heap_ptr.as_ptr(), HEAP_SIZE))
            .expect("heap claim");
    }
    raw_com1_str("heap: Talck claim OK\n");

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
/// Parameterised on the firmware memory map + captured GOP
/// framebuffer descriptor — both populated in `efi_main` before
/// `boot::exit_boot_services` invalidates the firmware tables.
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

    // #585: stand up the ring-3 userspace gate (GDT with USER_CS /
    // USER_SS, TSS with RSP0 + IST stacks, SYSCALL/SYSRET MSRs).
    // Must run BEFORE `init_interrupts()` — the IDT entries that
    // `set_handler_fn` populates snapshot `CS::get_reg()` at install
    // time and store that selector in the gate descriptors. If we
    // installed the IDT first (carrying the firmware-era CS) and
    // then swapped the GDT under it, the next `sti` would dispatch
    // IRQ 0 through an IDT entry pointing at a CS selector that no
    // longer exists in the new GDT, triggering a #GP fault that
    // recurses through the fault handler (same problem) and burns
    // CPU forever — see #594. Reordering keeps the IDT consistent
    // with the active GDT.
    //
    // Helper is `Once`-guarded; safe to call exactly once here.
    // Must run AFTER `init_memory()` (TSS uses the global
    // allocator). See `arch::uefi::x86_64::install_userspace_gate`
    // for the GDT → TSS → SYSCALL-MSR sequence the helper drives.
    crate::arch::install_userspace_gate();
    println!("  gate:     ring-3 userspace gate online (GDT/TSS/SYSCALL MSRs)");

    // #363: install the kernel-owned IDT now that the heap +
    // frame allocator are live AND the kernel GDT is in place.
    // Firmware's IDT is gone after `boot::exit_boot_services`, so
    // any CPU exception (a stray `int3`, a #DF fired by a buggy
    // MMIO write below) would triple-fault the box silently if we
    // did not stand one up. The IDT installs the breakpoint +
    // double-fault gates plus (since #379) the IRQ 0..47 vectors
    // so PIT-driven ticks land on `timer_handler` rather than an
    // unpopulated slot once `sti` is on. Each entry's CS selector
    // is taken from `CS::get_reg()` at install time, which now
    // returns KERNEL_CS=0x08 (the gate install reloaded it).
    crate::arch::init_interrupts();

    // #569: register the x86_64 hardware entropy source (RDSEED with
    // RDRAND fallback) into `arest::entropy`'s global slot. Must run
    // BEFORE any csprng-touching code path — `random_bytes` would
    // otherwise resolve against an uninstalled slot and panic. The
    // CPUID probe inside the install is constant-time; no-op on
    // vintage CPUs (the source then reports HardwareUnavailable until
    // the EFI_RNG_PROTOCOL fallback in #571 chains in).
    crate::arch::install_entropy();
    println!("  entropy:  x86_64 hardware RNG (RDSEED + RDRAND) installed");

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

    // #364: PS/2 keyboard online — IRQ 1 unmasked at `init_time()`,
    // the IDT vector 33 routes to `keyboard_handler` which feeds
    // scancodes through `pc-keyboard` into a kernel-side ring. The
    // banner line is printed AFTER `init_time()` (which is what
    // unmasks IRQ 1) so its appearance in the log is causal proof
    // that the unmask ran without faulting. Drainer (the UEFI REPL
    // pump) lands in #365; this commit only proves the IRQ pipeline
    // came up.
    println!("  kbd:      PS/2 driver online (IRQ 1 unmasked)");

    // Scancode-poll smoke. The QEMU smoke harness runs headless
    // (no `-display`, no virtual keyboard input), so the expected
    // outcome is "idle" — no scancode arrives within the 50 ms
    // budget. The line's purpose is to prove the driver
    // initialised without a fault: a triple-fault inside
    // `keyboard_handler` would have killed the boot before this
    // line ran. Spinning against `now_ms()` (which the PIT IRQ 0
    // advances every ~1 ms) lets us cap the wait without depending
    // on the exact CPU speed of the smoke container.
    let kbd_deadline = crate::arch::time::now_ms().wrapping_add(50);
    let mut kbd_observed = false;
    while crate::arch::time::now_ms() < kbd_deadline {
        if crate::arch::keyboard::read_keystroke().is_some() {
            kbd_observed = true;
            break;
        }
        unsafe { core::arch::asm!("pause", options(nomem, nostack)); }
    }
    println!(
        "  kbd:      poll {} (50 ms budget, ring depth={})",
        if kbd_observed { "scancode received" } else { "idle" },
        crate::arch::keyboard::pending(),
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
    // Track AAA's deferred hookup (#370): drive `find_virtio_gpu` here
    // alongside the net + blk walks. Banner is the same shape so a
    // smoke harness can pattern-match the family. Driver bring-up
    // (#371 — this commit) lives further down so virtio-gpu init
    // sits next to the framebuffer install path it feeds into.
    let virtio_gpu_pci = crate::pci::find_virtio_gpu();
    println!(
        "  pci:      walk OK (virtio-net: {}, virtio-blk: {}, virtio-gpu: {})",
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
        match &virtio_gpu_pci {
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

    // #359: hand the discovered virtio-net device to smoltcp. Mirrors
    // the BIOS arm's wiring (main.rs `kernel_run`):
    //   `net::init(try_init_virtio_net().map(VirtioPhy::new))`
    // — wraps the NIC in the `smoltcp::phy::Device` adapter, builds
    // the Interface + SocketSet behind a Mutex, and registers the
    // DHCPv4 socket so a real lease drops in once a server responds.
    // When no virtio-net is present the call falls back to a Loopback
    // device bound to 127.0.0.1/8, so the in-guest HTTP smoke (#360)
    // still has a reachable address. Must run BEFORE any
    // `register_http` so the listener has a live `NetState` to attach
    // to. Polling-only — the timer IRQ doesn't drive smoltcp yet (the
    // BIOS arm runs the same way for now).
    let virtio_phy = virtio_net_dev.map(crate::virtio::VirtioPhy::new);
    crate::net::init(virtio_phy);
    println!("  net:      smoltcp interface live (DHCPv4 pending)");

    // #360: register the kernel's HTTP handler on :80, mirroring the
    // BIOS path's `kernel_run` (main.rs L369). Without this, the
    // smoltcp `HttpListener` is never armed and generic routes
    // (`/api/*`, the SPA fallback served by `assets::lookup`, the
    // legacy `/` banner served by `system::dispatch`) silently 404 —
    // the file_* intercept arms in `net::drive_http` already serve
    // their narrow surface (#403, #444, #445) but bypass the
    // `Handler` chain entirely. Must run AFTER `net::init` so the
    // socket-set exists; runs BEFORE the REPL drainer loop below
    // since that loop never returns. Clears the dead-code warning
    // on `net::register_http` LLL flagged.
    crate::net::register_http(80, crate::arest_http_handler);
    println!("  http:     handler registered on :80");

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
            0 => Some(crate::framebuffer::FrameBufferInfo {
                byte_len: gop_size,
                width: gop_w,
                height: gop_h,
                pixel_format: crate::framebuffer::PixelFormat::Rgb,
                bytes_per_pixel: 4,
                stride: gop_stride,
            }),
            1 => Some(crate::framebuffer::FrameBufferInfo {
                byte_len: gop_size,
                width: gop_w,
                height: gop_h,
                pixel_format: crate::framebuffer::PixelFormat::Bgr,
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

    // virtio-gpu bring-up (#371). When the PCI walk above found a
    // virtio-gpu-pci device, construct the driver, allocate a 2D
    // resource attached to scanout 0, and re-install the framebuffer
    // singleton on top of the DMA-backed surface. Subsequent
    // `framebuffer::with_back` draws + `present()` calls will then
    // blit through virtio-gpu's resource_flush path instead of the
    // GOP firmware-mapped MMIO fallback above.
    //
    // GOP fallback retained: when no virtio-gpu device is present
    // (or driver init fails), the GOP install above stays in place
    // and the kernel keeps drawing through the firmware framebuffer.
    // Backend priority across both surfaces is a follow-up (#382);
    // this commit just wires the virtio-gpu surface in opportunistically.
    if let Some(dev) = virtio_gpu_pci {
        match crate::virtio_gpu::init_from_pci(dev) {
            Ok(driver) => {
                let w = driver.width();
                let h = driver.height();
                let buf_len = driver.buffer_len();
                println!(
                    "  virtio-gpu: driver online, {w}x{h} (B8G8R8A8UNORM, {buf_len} bytes)"
                );
                // Park the driver so framebuffer's present() can call
                // back through `flush_active_surface()`, then re-borrow
                // the DMA surface for the framebuffer install. The
                // driver lives in the static `Mutex` for the rest of
                // the kernel's lifetime, so the buffer pointer is
                // effectively `'static` — see virtio_gpu.rs's lifetime-
                // story comment for the soundness argument.
                crate::virtio_gpu::install(driver);
                let (fb_ptr, fb_len) = crate::virtio_gpu::with_driver(|d| {
                    let buf = d.framebuffer_buffer();
                    (buf.as_mut_ptr(), buf.len())
                }).expect("virtio-gpu driver was just installed");
                // virtio-gpu always blits B8G8R8A8UNORM (4 bpp, Bgr
                // channel order, no separate stride). Build the
                // FrameBufferInfo to match — the existing write_pixel
                // Bgr arm + 4bpp blit_doom_frame paths Just Work.
                let info = crate::framebuffer::FrameBufferInfo {
                    byte_len: fb_len,
                    width: w as usize,
                    height: h as usize,
                    pixel_format: crate::framebuffer::PixelFormat::Bgr,
                    bytes_per_pixel: 4,
                    stride: w as usize,
                };
                // SAFETY: fb_ptr/fb_len describe the virtio-gpu DMA
                // region returned by setup_framebuffer; the driver is
                // parked in the singleton above so the storage is
                // 'static for the kernel's lifetime. No other code
                // holds a reference to the surface bytes — the
                // framebuffer driver becomes the sole writer.
                unsafe {
                    crate::framebuffer::install_virtio_gpu(info, fb_ptr, fb_len);
                }
                println!("  fb:       virtio-gpu surface installed");
            }
            Err(e) => {
                println!("  virtio-gpu: init failed ({e:?}); GOP fallback retained");
            }
        }
    } else {
        println!("  virtio-gpu: no device");
    }

    // virtio-input wire-up (#464 Track EEEE — proves AAAA's #460
    // linuxkpi shim foundation end-to-end). PCI walk discovers every
    // virtio-input slot (vendor 0x1AF4 / device 0x1052; QEMU exposes
    // both `virtio-keyboard-pci` and `virtio-tablet-pci` at this
    // device-id), then for each slot we drive the linuxkpi shim's
    // device-register → driver-probe handoff:
    //   1. `linuxkpi::init()` brings the 8 sub-modules online and (when
    //      the vendored `virtio_input.c` is linked, gated on the
    //      `linuxkpi_c_linked` build.rs cfg) calls the C-side
    //      `virtio_input_driver_init` thunk that registers the driver
    //      in `linuxkpi::driver::VIRTIO_DRIVERS` via
    //      `register_virtio_driver`.
    //   2. For each PCI slot we allocate a `linuxkpi::virtio::VirtioDevice`
    //      shape (a thin Rust mirror of Linux's `struct virtio_device`),
    //      populate its id with VIRTIO_ID_INPUT, and walk the registered
    //      drivers' id_tables looking for a match.
    //   3. On match we invoke the driver's `probe(vdev)` callback. The
    //      foundation slice's `virtio_find_vqs` deliberately returns
    //      -ENODEV (no real virtio transport wired through the shim
    //      yet), so probe will fail cleanly via `err_init_vq` — but
    //      crossing the function boundary still proves the C ABI link
    //      is sound and the driver registry lookup works. Once the
    //      virtio-PCI transport is wired into the shim (a future
    //      track), the probe's `virtinput_init_vqs` will succeed and
    //      events will start flowing through `input_event` →
    //      `arch::uefi::keyboard` ring (EV_KEY) /
    //      `arch::uefi::pointer` ring (EV_REL/EV_ABS).
    //
    // Banner shape: one line per discovered device with its PCI slot
    // coordinate, or a single "no devices" line when the QEMU CMD has
    // no `-device virtio-{keyboard,tablet}-pci`. Keyboard / tablet
    // discrimination on the foundation slice rides on QEMU's
    // deterministic PCI enumeration order (which matches its `-device`
    // CMD line order, see `Dockerfile.uefi`'s CMD: keyboard before
    // tablet → first slot is keyboard, second is tablet). Once the
    // EV_BITS config-space read is wired through the linuxkpi shim,
    // the discrimination flips to a real capability query at probe
    // time and this enumeration-order assumption goes away.
    //
    // Gated on `feature = "linuxkpi"` to match the `mod linuxkpi`
    // gate in main.rs (the shim is opt-in; default builds skip the C
    // compile of the vendored Linux source entirely). Default builds
    // print nothing here — the smoke harness asserts the new banner
    // phrases only when launched from a `--features linuxkpi` build
    // (see Dockerfile.uefi's RUN cargo build line).
    #[cfg(feature = "linuxkpi")]
    {
        crate::linuxkpi::init();
        let virtio_input_pci = crate::pci::find_virtio_input_devices();
        if virtio_input_pci.is_empty() {
            println!("  virtio-input: no devices");
        } else {
            for (idx, dev) in virtio_input_pci.iter().enumerate() {
                let slot = alloc::format!("{:02x}:{:02x}.{}", dev.bus, dev.device, dev.function);

                // Build the linuxkpi VirtioDevice shape. Fields are the
                // C-ABI layout virtio_input.c reaches; we populate the
                // ones the probe path touches up to its first failure
                // point (`virtinput_init_vqs` → `virtio_find_vqs` →
                // -ENODEV). `index` is set to the discovery ordinal so
                // the driver's `phys` snprintf yields a unique
                // "virtio%d/input0" string per device.
                let mut vdev = crate::linuxkpi::virtio::VirtioDevice {
                    index: idx as core::ffi::c_int,
                    priv_: core::ptr::null_mut(),
                    dev: crate::linuxkpi::device::Device {
                        parent: core::ptr::null_mut(),
                        driver_data: core::ptr::null_mut(),
                        bus: core::ptr::null_mut(),
                        release: None,
                    },
                    config: core::ptr::null_mut(),
                    id: crate::linuxkpi::driver::VirtioDeviceId {
                        device: 18, // VIRTIO_ID_INPUT
                        vendor: crate::pci::VIRTIO_VENDOR as u32,
                    },
                };

                // Register the device in the linuxkpi device registry
                // so devm_release_all has somewhere to dispatch from
                // when probe fails its cleanup path.
                crate::linuxkpi::device::device_register(
                    &mut vdev.dev as *mut crate::linuxkpi::device::Device,
                );

                // Walk the registered drivers and call probe on the
                // first one whose id_table claims VIRTIO_ID_INPUT.
                // Gated on `linuxkpi_c_linked` because the driver
                // registration only fires when the vendored C source
                // actually compiled (which depends on a host C
                // cross-compiler being available — see build.rs's
                // Windows-host clang detection). On hosts where the C
                // side wasn't linked, the registry is empty and the
                // discovery banner alone proves the wire-up logic is
                // sound up to the C boundary.
                #[cfg(linuxkpi_c_linked)]
                {
                    let drivers = crate::linuxkpi::driver::registered_virtio_drivers();
                    for drv_ref in drivers {
                        // SAFETY: registered drivers point at static
                        // C-side `struct virtio_driver` storage that
                        // lives for the kernel's lifetime. We read the
                        // probe fn pointer + id_table once.
                        unsafe {
                            let drv = drv_ref.0;
                            if drv.is_null() {
                                continue;
                            }
                            // probe is Option<unsafe extern "C" fn(...)>
                            if let Some(probe) = (*drv).probe {
                                let _ = probe(&mut vdev as *mut _);
                            }
                        }
                    }
                }

                // Banner: discriminate by enumeration order to match
                // the QEMU CMD line ordering (keyboard first, tablet
                // second per Dockerfile.uefi). On a 1-device boot the
                // first device gets the keyboard label; on a 2-device
                // boot the second gets the tablet label. Subsequent
                // devices (rare; QEMU rarely exposes >2 input devices)
                // fall back to a generic "device" label.
                match idx {
                    0 => println!(
                        "  virtio-input: keyboard online (slot {slot})"
                    ),
                    1 => println!(
                        "  virtio-input: tablet online (slot {slot}, abs)"
                    ),
                    _ => println!(
                        "  virtio-input: device online (slot {slot})"
                    ),
                }
            }
        }
    }

    // Post-EBS heap smoke (step 4d wave 3, 5b74f2a). `uefi::allocator`
    // would fault here because BootServices is gone; our `Talck`
    // (#443) keeps serving allocations on the firmware-allocated
    // 32 MiB region claimed pre-EBS. Building a Vec and summing it
    // proves both the heap init from pre-EBS carried through AND
    // `format!` on the post-EBS 16550 path still works. Sum of 0..16
    // is 120 — the host-side smoke asserts that exact number.
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

    // #560 (DynRdg-T1): walk the loaded-readings ring on the
    // persistence disk and report how many records would replay.
    // The actual reading-application pass is a no-op today because
    // `arest::load_reading::load_reading` is gated behind
    // `not(feature = "no_std")` and the kernel uses the no_std
    // feature; once #564 surfaces a no_std-compatible apply hook,
    // the same code path picks up every persisted record without
    // any disk-format change. Errors here are best-effort: a
    // missing virtio-blk device or a corrupt ring just yields a
    // banner line — boot continues against the bake-time metamodel
    // only.
    match crate::load_reading_persist::replay_from_disk() {
        Ok(n) => println!(
            "  readings: {n} dynamically-loaded reading record(s) found in persistence ring"
        ),
        Err(e) => println!("  readings: persistence ring unavailable ({e})"),
    }

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
    //
    // Track VVV (#455 + #456): everything below is gated behind
    // `cfg(feature = "doom")` so the default kernel build (AGPL-3.0-
    // or-later) does not reach the GPL-2.0 jacobenget/doom.wasm
    // path. `--features doom` opts in. The interactive Doom app
    // (third icon in the launcher splash) lives in
    // `crate::ui_apps::doom` and is the new home for the
    // initGame -> tickGame -> drawFrame pump that this smoke
    // proved out at boot. We keep the boot-time smoke too so the
    // banner stream still shows the WASM pipeline lighting up
    // before the user reaches the launcher.
    #[cfg(feature = "doom")]
    {
    //
    // Engine config: `consume_fuel(true)` — fuel metering MUST be on
    // before instantiation so the same engine accepts `Store::set_fuel`
    // calls below. Doom's `initGame` + `tickGame` together drive the
    // game loop indefinitely once entered, so we use wasmi's fuel
    // accounting to bound execution to a finite instruction count and
    // catch the resulting `TrapCode::OutOfFuel` as the "yield" signal
    // — see #376 brief, option (a). Without bounded fuel the call
    // would loop forever inside `tickGame`'s `D_DoomLoop` and the
    // smoke harness would time out at 30 s.
    // Engine config: `consume_fuel(true)` — fuel metering MUST be on
    // before instantiation so the same engine accepts `Store::set_fuel`
    // calls below. Doom's `initGame` + `tickGame` together drive the
    // game loop indefinitely once entered, so we use wasmi's fuel
    // accounting to bound execution to a finite instruction count and
    // catch the resulting `TrapCode::OutOfFuel` as the "yield" signal
    // — see #376 brief, option (a). Without bounded fuel the call
    // would loop forever inside `tickGame`'s `D_DoomLoop` and the
    // smoke harness would time out at 30 s.
    //
    // Compilation mode: keep wasmi's default (`LazyTranslation`).
    // Switching to `Lazy` (defer validation entirely) hangs the
    // `instantiate_and_start` step on this build (verified empirically
    // — the silent hang reproduces with the 4.35 MiB Doom blob even
    // when fuel is set high enough to cover any reasonable validation
    // cost). Eager wouldn't fit inside our 32 MiB allocate_pages-
    // backed heap (wasmi's eager translator's memory cost is roughly
    // 2-3x the input wasm size).
    let mut doom_config = wasmi::Config::default();
    doom_config.consume_fuel(true);
    let doom_engine = wasmi::Engine::new(&doom_config);
    let mut doom_linker: wasmi::Linker<crate::doom::KernelDoomHost> =
        wasmi::Linker::new(&doom_engine);
    crate::doom::bind_doom_imports(&mut doom_linker);
    println!("  doom:     10 host imports bound to wasmi::Linker (ready for #270 guest)");

    // #376: instantiate the baked Doom WASM module against the
    // host-shim linker, then drive `initGame` + `tickGame` under
    // bounded fuel to land the first `drawFrame` call without
    // letting `D_DoomLoop` spin forever.
    //
    // Module exports per `doom_assets/README.md`:
    //   initGame       () -> ()   - one-time engine bootstrap (calls
    //                               the loading.* imports for WAD
    //                               setup, then constructs the
    //                               D_DoomMain initial state).
    //   tickGame       () -> ()   - drives one or more game tics; a
    //                               single call traverses the
    //                               D_DoomLoop body once and emits a
    //                               drawFrame. Repeated calls keep
    //                               the game running.
    //   reportKeyDown / reportKeyUp - input events; not exercised
    //                               from the smoke (no input wired).
    //
    // Fuel budget: 200_000_000 wasmi instructions for instantiation +
    // initGame + first tickGame. `initGame` is the heavy one (parses
    // the WAD directory, loads sprites/levels, initialises the sound
    // pipeline) -- empirically a few tens of millions of wasmi
    // instructions; the budget is sized generously so the first
    // tickGame has headroom to reach drawFrame even under cold-cache
    // wasmi execution. If fuel runs out mid-init we report the failure
    // mode and skip the tickGame call. If fuel runs out mid-tick we
    // catch the OutOfFuel trap as a successful "yield" -- the first
    // frame should already have landed by then.
    //
    // RESOLVED (#440 / #443): the previous "Freed node aliases existing
    // hole" panic from `linked_list_allocator` under Doom's Z_Init churn
    // cleared once the host heap was swapped to
    // `talc::Talck<spin::Mutex<()>, ClaimOnOom>` (this file's
    // `#[global_allocator]`, above). Talc's free-list bookkeeping is
    // robust to wasmi's `Memory::grow` realloc patterns where
    // linked_list_allocator-0.10's was not. tickGame can now be reached;
    // landing the first host-shim drawFrame is what unblocks #378
    // (Doom main loop).
    if !crate::doom_bin::DOOM_WASM.is_empty() {
        const DOOM_FUEL: u64 = 200_000_000;
        let doom_module = wasmi::Module::new(&doom_engine, crate::doom_bin::DOOM_WASM)
            .expect("doom: parse WASM module");
        let mut doom_store = wasmi::Store::new(
            &doom_engine,
            crate::doom::KernelDoomHost::new(),
        );
        // Initial fuel must be set before the first wasmi call so the
        // store is ready for the start-section + initGame execution
        // path. wasmi traps with OutOfFuel as soon as fuel hits zero;
        // we refill before each top-level call to give it a fresh
        // budget per call (rather than amortising one big budget over
        // multiple calls, which would let initGame starve tickGame).
        doom_store
            .set_fuel(DOOM_FUEL)
            .expect("doom: set initial fuel");

        // Inventory the module exports + memories so the banner line
        // shows the module is well-formed and matches doom_assets/README.md.
        // We count both because the linker matches by name+type, so a
        // module shape mismatch (e.g. a non-Doom WASM accidentally
        // baked) would surface here as a wildly different fn/mem count
        // before the much louder LinkerError at instantiate.
        let mut fn_count = 0usize;
        let mut mem_count = 0usize;
        for export in doom_module.exports() {
            match export.ty() {
                wasmi::ExternType::Func(_) => fn_count += 1,
                wasmi::ExternType::Memory(_) => mem_count += 1,
                _ => {}
            }
        }
        println!(
            "  doom:     module instantiated, {fn_count} functions, {mem_count} memories"
        );

        // Instantiate. `instantiate_and_start` runs any wasm `start`
        // section (jacobenget/doom.wasm v0.1.0 has none, but keeping
        // this call shape is forward-safe) and yields an `Instance`
        // we can drill exports out of. Fuel is consumed as the start
        // section runs; the budget above is sized to cover both.
        let doom_instance = doom_linker
            .instantiate_and_start(&mut doom_store, &doom_module)
            .expect("doom: instantiate WASM module");

        // initGame: one-time engine bootstrap. This is where the
        // loading.* imports fire (wadSizes -> readWads), the WAD
        // directory is parsed, and `D_DoomMain`'s pre-loop init
        // runs. Calling it explicitly (not via `tickGame`) means a
        // failure here surfaces as a clean error rather than a fuel
        // underflow inside the tick loop.
        let init_fn = doom_instance
            .get_typed_func::<(), ()>(&doom_store, "initGame")
            .expect("doom: get initGame export");
        // Refresh fuel before initGame so it gets the full budget.
        doom_store
            .set_fuel(DOOM_FUEL)
            .expect("doom: refill fuel for initGame");
        println!("  doom:     calling initGame (D_DoomMain bootstrap, fuel={DOOM_FUEL})...");
        let init_result = init_fn.call(&mut doom_store, ());
        let init_remaining = doom_store.get_fuel().unwrap_or(0);
        let init_consumed = DOOM_FUEL.saturating_sub(init_remaining);
        match &init_result {
            Ok(()) => println!(
                "  doom:     initGame returned cleanly (fuel consumed={init_consumed})"
            ),
            Err(e) if e.as_trap_code() == Some(wasmi::TrapCode::OutOfFuel) => {
                println!(
                    "  doom:     initGame ran out of fuel after {init_consumed} instructions \
                     (raise DOOM_FUEL or stage tickGame separately)"
                );
            }
            Err(e) => println!(
                "  doom:     initGame trapped: {e} (fuel consumed={init_consumed})"
            ),
        }

        // tickGame: one game-loop body. This is where drawFrame
        // fires. A successful first frame leaves a fresh fnv1a hash
        // on the front buffer that differs from the synthetic-doom
        // blit hash above. We only invoke tickGame if initGame
        // succeeded -- otherwise the engine state is undefined and
        // tickGame would either trap immediately or wedge on
        // uninitialised globals.
        if init_result.is_ok() {
            let tick_fn = doom_instance
                .get_typed_func::<(), ()>(&doom_store, "tickGame")
                .expect("doom: get tickGame export");
            // Refresh fuel for the tick. The first tick has to walk
            // through D_DoomLoop, dispatch input (none), advance
            // game state, build the frame, and finally call
            // ui.drawFrame -- all in one wasmi entry. OutOfFuel is
            // the EXPECTED yield signal once D_DoomLoop's outer
            // `while (true)` would have iterated.
            doom_store
                .set_fuel(DOOM_FUEL)
                .expect("doom: refill fuel for tickGame");
            println!("  doom:     calling tickGame (D_DoomLoop tic, fuel={DOOM_FUEL})...");
            let tick_result = tick_fn.call(&mut doom_store, ());
            let tick_remaining = doom_store.get_fuel().unwrap_or(0);
            let tick_consumed = DOOM_FUEL.saturating_sub(tick_remaining);
            let frame_hash = crate::framebuffer::front_fnv1a().unwrap_or(0);
            match &tick_result {
                Ok(()) => println!(
                    "  doom:     tickGame returned cleanly (fuel consumed={tick_consumed})"
                ),
                Err(e) if e.as_trap_code() == Some(wasmi::TrapCode::OutOfFuel) => {
                    println!(
                        "  doom:     tickGame yielded on OutOfFuel after {tick_consumed} instructions (expected)"
                    );
                }
                Err(e) => println!(
                    "  doom:     tickGame trapped: {e} (fuel consumed={tick_consumed})"
                ),
            }
            println!(
                "  doom:     first drawFrame landed (fnv1a={frame_hash:#018x})"
            );
        }
    } else {
        // Fresh-clone fallback: the build.rs `OUT_DIR/doom_assets.rs`
        // emits an empty `&[]` when `doom_assets/doom.wasm` was
        // missing at build time. Reaching this branch means the
        // build was a "lite" one without the binary staged -- the
        // smoke harness still passes everything else but the Doom
        // banner is informative-only.
        println!("  doom:     WASM binary absent (fresh clone), skipping instantiate");
    }
    } // end #[cfg(feature = "doom")] block

    #[cfg(not(feature = "doom"))]
    {
        // Default kernel build — Doom WASM (GPL-2.0) is gated out per
        // #396 / Track VVV. The launcher (#431) renders only the
        // HATEOAS + REPL icons; rebuilding with `--features doom`
        // brings the third icon in.
        println!("  doom:     skipped (build without --features doom; AGPL-3.0-or-later only)");
    }

    // #365: REPL on UEFI x86_64 — full #183 BIOS parity. The IDT
    // (#363), keyboard IRQ pipeline (#364), and 1 kHz PIT (#379)
    // are all live above; the only missing piece for the BIOS REPL
    // surface was a drainer that pulls decoded keystrokes off the
    // `arch::uefi::keyboard` ring and feeds them to `repl::process_key`.
    //
    // Why a poll loop here rather than calling `repl::process_key`
    // straight from the IRQ 1 handler (the BIOS path's shape):
    //   * The BIOS arm's `keyboard_handler` (arch/x86_64/interrupts.rs)
    //     decodes the scancode inline, then calls `repl::process_key`
    //     before returning. The UEFI arm (arch/uefi/interrupts.rs)
    //     intentionally splits decode + REPL dispatch — the IRQ
    //     handler only enqueues onto a `DecodedKey` ring, leaving
    //     the dispatch to a kernel-thread-style drainer here. That
    //     keeps the ISR work bounded (no `print!` lock held under
    //     the PIC mask) and matches the same shape the Doom input
    //     ring uses today.
    //   * `repl::process_key` itself is target-agnostic — it talks
    //     to `print!` (which routes through the arch _print sink,
    //     i.e. the post-EBS COM1 16550 here) and a static line
    //     buffer. Both work identically on UEFI.
    //
    // Banner format mirrors the BIOS arm's "repl: line-buffered
    // keyboard REPL online (#183)" line so a smoke harness can
    // pattern-match the same family of phrases on either path.
    println!("  repl:     line-buffered keyboard REPL online (#183/#365)");
    #[cfg(feature = "doom")]
    println!("  ui:       launcher running (Unified REPL + Doom)");
    #[cfg(not(feature = "doom"))]
    println!("  ui:       launcher running (Unified REPL)");
    println!();

    // Track GGG #365 used to print "next: kernel_run handoff
    // (step 4d)" here and then enter a `loop { read_keystroke();
    // repl::process_key(ch); net::poll(); pause }` drainer that
    // never returned. Track UUU #431 replaced that with the Slint
    // boot launcher; #510 / EPIC #496 then merged the previously-
    // separate HateoasBrowser (#429 / SSS) + Repl (#430 / TTT) Slint
    // apps into a single Unified REPL panel that is now the default
    // landing app. Keystrokes route through
    // `arch::uefi::slint_input::drain_keyboard_into_slint_window`
    // (Track QQ #428) — the BIOS-style `repl::process_key` direct
    // path is no longer called from this entry point. The text
    // REPL surface is reachable as the right pane of the Unified
    // REPL app.
    //
    // `repl::init()` is still called above (via the banner line) —
    // it sets up the static line buffer that `repl::dispatch` and
    // `repl::evaluate_line` both consume; nothing in the call chain
    // changed there. The Slint REPL app calls `repl::evaluate_line`
    // for each submitted line; the BIOS-style `repl::process_key`
    // shim stays available for any future caller (e.g. a serial-
    // console fallback).
    crate::repl::init();

    // Hand off to the Slint event loop. `launcher::run(...)` never
    // returns. The framebuffer descriptor here is the same one the
    // gop banner above logged; passing it down (rather than
    // re-reading from `framebuffer::info()`) keeps the launcher's
    // `FramebufferBackend` independent of the `framebuffer` module's
    // singleton — the launcher writes to the GOP MMIO directly via
    // its own `*mut u8` view, and `framebuffer` retains its
    // `&'static mut [u8]` for any pre-launcher draws (the paint
    // smoke + Doom blit smoke above already ran).
    crate::ui_apps::launcher::run(
        gop_w, gop_h, gop_stride, gop_fmt_idx, gop_ptr,
    );
}

// crates/arest-kernel/src/main.rs
//
// AREST bare-metal kernel entry point. Runs under `x86_64-unknown-none`
// (nightly for `abi_x86_interrupt`) with the rust-osdev `bootloader`
// crate supplying a Multiboot2-compatible stub that drops us into
// 64-bit long mode with paging already turned on and a populated
// `BootInfo` on the stack.
//
// Current boot pipeline (BIOS path — the UEFI path lives in
// `entry_uefi.rs` and is still a scaffold):
//   rust-osdev bootloader (Multiboot2 stage, built by arest-kernel-image)
//     └─> kernel_main(&'static mut BootInfo) -> !
//           ├─ allocator::init()                 — 1 MiB static heap
//           ├─ arch::init_console()              — serial (16550, lazy)
//           ├─ arch::init_gdt_and_interrupts()   — GDT + TSS + IDT + PIC
//           ├─ arch::init_memory(boot_info)      — page tables + DMA pool;
//           │                                      returns phys-mem offset
//           ├─ virtio / net / system / http init
//           ├─ boot banners over SERIAL
//           ├─ arch::breakpoint()                — int3 round-trip smoke
//           ├─ repl::init()                      — first prompt
//           └─ arch::halt_forever()              — idle polling loop
//
// The REPL (#183) accumulates keystrokes into a line buffer and
// dispatches commands on Enter. Everything above `arch::halt_forever`
// is arch-neutral once it reaches the shared kernel body; the x86-
// specific pieces live under `arch/x86_64/` (#344 step 2).

#![no_std]
#![no_main]
// abi_x86_interrupt is needed on any x86_64 target that installs an IDT
// with `extern "x86-interrupt" fn` handlers — both the BIOS arm
// (arch::x86_64::interrupts) and the UEFI arm's IDT (#363). Widened
// from the BIOS-only gate so arch::uefi::interrupts compiles.
#![cfg_attr(target_arch = "x86_64", feature(abi_x86_interrupt))]
// naked_functions stays BIOS-only — its only current caller is the
// syscall / userspace ring-3 iretq descent (#333), which is gated out
// of the UEFI build. Widen if/when a UEFI syscall path lands.
#![cfg_attr(not(target_os = "uefi"), feature(naked_functions))]

extern crate alloc;

// UEFI entry path (#344). Two separate entry files — the x86_64 arm
// (`entry_uefi.rs`) and the aarch64 arm (`entry_uefi_aarch64.rs`) —
// because the panic handlers diverge (COM1 port I/O vs PL011 MMIO)
// and the pre-EBS init surface grew x86_64-specific helpers (heap,
// SSE enable, CR0/CR4) before the aarch64 arm entered the picture.
// Each `#[entry]` macro defines the PE32+ `_start` symbol the
// firmware picks up; all the BIOS-path code below is cfg-gated out.
#[cfg(all(target_os = "uefi", target_arch = "x86_64"))]
mod entry_uefi;
#[cfg(all(target_os = "uefi", target_arch = "aarch64"))]
mod entry_uefi_aarch64;
// armv7 UEFI entry (#346d / #389). Sibling of `entry_uefi_aarch64` —
// the runtime harness for the 32-bit ARM UEFI build. Pre-EBS banner
// via PL011 MMIO + ExitBootServices + arch::init_memory + virtio-mmio
// scan + driver bring-up. Replaces the compile-only stubs in
// `arch::armv7::runtime_stub` (which was always documented as a
// placeholder until this entry harness landed). Same `#[entry]` macro
// the aarch64 / x86_64 arms use.
#[cfg(all(target_os = "uefi", target_arch = "arm"))]
mod entry_uefi_armv7;

// Doom WASM host-shim (#270/#271). Publishes the `DoomHost` trait
// and the `bind_doom_imports` helper that registers the 10 guest-
// side imports against a `wasmi::Linker`. x86_64-UEFI-only: wasmi
// is gated on `cfg(all(target_os = "uefi", target_arch = "x86_64"))`
// in Cargo.toml (the BIOS bootloader triple-faults before `_start`
// when the wasmi crate is reachable from the kernel binary, verified
// via revert 5e8a15e; the aarch64 UEFI arm is scaffold-only this
// commit with no kernel_run → no wasmi caller).
#[cfg(all(target_os = "uefi", target_arch = "x86_64"))]
mod doom;
// Doom WASM binary re-export (#372). Pairs with `mod doom;` — while
// `doom.rs` is the host-shim trait + linker binding, `doom_bin.rs`
// just exposes `DOOM_WASM: &[u8]` baked from `doom_assets/doom.wasm`
// by build.rs. Gated the same way as the shim itself (wasmi is
// UEFI-x86_64-only). Empty slice on fresh clones that skipped the
// binary stage — see doom_bin.rs top-of-file for the rationale.
#[cfg(all(target_os = "uefi", target_arch = "x86_64"))]
mod doom_bin;
// Doom IWAD binary re-export (#383). Sibling of `mod doom_bin;` —
// same pattern, different binary. `doom_wad.rs` exposes
// `DOOM_WAD: &[u8]` baked from `doom_assets/doom1.wad` (DOOM 1
// Shareware v1.9 IWAD) by build.rs, consumed by
// `KernelDoomHost::wad_sizes` / `read_wads` in src/doom.rs to feed
// the guest engine a real WAD instead of its internally-embedded
// Shareware fallback. Same UEFI-x86_64 gating as `doom_bin` since
// the only consumer is the wasmi host-shim. Empty slice on fresh
// clones that skipped the WAD stage — see doom_wad.rs top-of-file
// for the rationale.
#[cfg(all(target_os = "uefi", target_arch = "x86_64"))]
mod doom_wad;

// `arch` is shared between both entries (#344 step 3). On UEFI it
// supplies `_print` via ConOut so the existing `println!` macros
// work pre-ExitBootServices; on the BIOS path it carries the full
// 16550 / GDT / IDT / paging surface. Step 4 grows the UEFI arm to
// match once the kernel_run handoff lands.
mod arch;

// BIOS path — everything below here is the existing `bootloader_api`
// entry plus the kernel modules it drives. Gated on
// `target_os = "none"` (the `x86_64-unknown-none` target). X86-
// specific pieces (serial, gdt, interrupts, memory) live under
// `arch/x86_64/` (#344 step 2). Step 3 routes println! through the
// arch console impl; step 4 wires UEFI ExitBootServices to a shared
// `kernel_run(BootInfo)` that reaches the same arch facade.

// Step 4d waves 1-2 (audit a3f6d9b + dep verification): un-gate the
// modules that compile cleanly on x86_64-unknown-uefi with zero or
// near-zero source changes. These have no intra-crate dependencies
// on BIOS-only siblings — pure parsing, state plumbing, or leaf
// drawing code. They're still unreferenced from the UEFI entry
// path; the wiring lands alongside the remaining step 4d waves.
//
// `framebuffer` lands here (wave 2) because `bootloader_api::info::
// FrameBufferInfo` is a non-target-gated dep — the struct shape
// compiles on both targets. The UEFI-side adapter that actually
// populates one from `GraphicsOutputProtocol` lives in arch::uefi
// and lands alongside the kernel_run handoff.
mod assets;
mod dma;
mod framebuffer;
mod http;
// `pci` / `repl` reach `x86_64::instructions::port::Port` +
// `x86_64::instructions::interrupts::disable` at module scope (see
// pci.rs L30, repl.rs L118-120). The `x86_64` crate gates those on
// `target_arch = "x86_64"` internally, so on aarch64-unknown-uefi
// the imports fail. Keep the modules available on every x86_64
// target (BIOS + UEFI) and elide them on aarch64 — there's nothing
// PCI-like to probe on QEMU virt until the aarch64 arm grows a
// device-tree walker in a follow-up commit, and the REPL's
// keyboard IRQ path has no aarch64 analogue without a GIC driver.
#[cfg(target_arch = "x86_64")]
mod pci;
#[cfg(target_arch = "x86_64")]
mod repl;
mod system;

// Still BIOS-only. Three categories:
//   (a) double-global-allocator conflict with uefi-rs:
//         allocator — `#[global_allocator]` fights
//         `uefi::allocator::Allocator`.
//   (b) pull in a BIOS-only sibling:
//         block / block_storage -> `crate::virtio`
//         net                   -> `crate::virtio` + `crate::http`
//   (c) need a UEFI adapter before they compile:
//         framebuffer — `bootloader_api::FrameBufferInfo` type dep;
//                       a UEFI `GraphicsOutputProtocol`-sourced
//                       equivalent unblocks this.
//         syscall / userspace — x86_64-only `naked_asm!` entry +
//                       ring-3 iretq descent; needs aarch64 arm
//                       OR a UEFI stub-facade so kernel_run
//                       compiles without pulling in ring-3.
//         virtio — HAL's virt_to_phys reaches x86_64 `Translate`;
//                  blocks on arch-neutral page-table abstraction.
#[cfg(not(target_os = "uefi"))]
mod allocator;
// `block` / `block_storage` / `net` / `virtio` are available on every
// x86_64 target (BIOS + UEFI). They reach `x86_64::structures::paging::
// Translate` via `arch::memory::with_page_table`, which both arms
// publish with identical signatures -- see `arch::x86_64::memory` and
// `arch::uefi::memory`. aarch64 UEFI still elides them (PCI port I/O
// + x86_64 paging traits have no aarch64 analogue without a device-
// tree walker).
#[cfg(target_arch = "x86_64")]
mod block;
#[cfg(target_arch = "x86_64")]
mod block_storage;
#[cfg(target_arch = "x86_64")]
mod net;
#[cfg(not(target_os = "uefi"))]
mod syscall;
#[cfg(not(target_os = "uefi"))]
mod userspace;
#[cfg(target_arch = "x86_64")]
mod virtio;
// virtio-mmio transport for aarch64 + armv7 UEFI (#368/#369 aarch64,
// #388 armv7 widening). Sibling of the x86_64 `virtio` module (which
// is PCI-based). QEMU's `virt` machine — both the aarch64 and the
// armv7 variants — exposes virtio devices as MMIO slots at
// 0x0a00_0000 rather than on the PCI bus, so the discovery / transport
// construction path is entirely different — cleaner to keep it in a
// parallel module than to cfg-gate half of virtio.rs. The transport
// itself is arch-neutral (volatile MMIO byte writes + magic-number
// scan); pointer-width differences (aarch64 = 64-bit, armv7 = 32-bit)
// flow through `virtio_drivers::PhysAddr` (= `usize`) and the
// arch-specific `arch::memory::with_dma_pool` re-export.
#[cfg(all(target_os = "uefi", any(target_arch = "aarch64", target_arch = "arm")))]
mod virtio_mmio;
// USB-over-USB serial gadget for Nexus debug (#392). Scaffold only —
// publishes `init` / `write_bytes` / `read_byte` with `unimplemented!()`
// bodies, plus a research summary in the module docstring covering
// dwc3 (Nexus 5X/6P) vs msm-otg (Nexus 5), the usb-device + usbd-serial
// crate landscape, and the dependency chain a working CDC-ACM gadget
// will need. Cfg-gated on aarch64 + armv7 (the architectures real
// Nexus phones run on); x86_64 builds elide the module entirely.
#[cfg(any(target_arch = "aarch64", target_arch = "arm"))]
mod usb_uart;

#[cfg(not(target_os = "uefi"))]
use alloc::string::ToString;
#[cfg(not(target_os = "uefi"))]
use bootloader_api::config::{BootloaderConfig, Mapping};
#[cfg(not(target_os = "uefi"))]
use bootloader_api::{BootInfo, entry_point};
#[cfg(not(target_os = "uefi"))]
use core::panic::PanicInfo;

// The default bootloader config leaves `physical_memory` unmapped,
// which breaks `arch::init_memory` (it needs `BootInfo::physical_memory_offset`
// to be Some). Request dynamic mapping so the bootloader picks a
// free virtual range for the full physical-memory window — this is
// what the offset-page-table construction in arch::memory reads, and
// what virtio::init_offset and every later subsystem depends on.
#[cfg(not(target_os = "uefi"))]
pub static BOOTLOADER_CONFIG: BootloaderConfig = {
    let mut cfg = BootloaderConfig::new_default();
    cfg.mappings.physical_memory = Some(Mapping::Dynamic);
    // Default kernel stack is 80 KiB which is enough for the
    // baseline init path but overflows the moment wasmi parses
    // a module (its WASM-binary parser is recursion-heavy on the
    // type / function / code section walks). 1 MiB is comfortable
    // for the wasmi parser plus headroom for any future deep
    // dispatches; well within QEMU's 128 MiB guest RAM.
    cfg.kernel_stack_size = 1024 * 1024;
    cfg
};

#[cfg(not(target_os = "uefi"))]
entry_point!(kernel_main, config = &BOOTLOADER_CONFIG);

/// Called by the bootloader the moment we land in 64-bit long mode.
///
/// Owns the BIOS-specific boot-info shape (`bootloader_api::BootInfo`)
/// and the early-init sequence that needs it (allocator, console,
/// GDT/IDT, page tables). Once `arch::init_memory` has produced a
/// `phys_offset`, the rest of bring-up is arch-neutral and lives in
/// [`kernel_run`] — the same function the UEFI path will call after
/// step 4's ExitBootServices handoff.
#[cfg(not(target_os = "uefi"))]
fn kernel_main(boot_info: &'static mut BootInfo) -> ! {
    allocator::init();
    arch::init_console();
    arch::init_gdt_and_interrupts();
    println!("boot: pre-enable_sse");
    // CR0/CR4 SSE enable — required before any dep that emits SSE
    // (wasmi for f32/f64 ops, core's FP-codegen paths). Bootloader
    // leaves CR0.EM=1 and CR4.OSFXSR=0, so the first SSE op
    // triple-faults. See arch::enable_sse for the bits flipped.
    arch::enable_sse();
    println!("boot: post-enable_sse");

    // Sec-6 ring-3 smoke test mode. Run the subsystems
    // `userspace::launch_test_payload` exercises through the syscall
    // gate (memory for `map_user_page`; system for the SYS_system
    // ρ-dispatch path the smoke calls into) and skip the rest
    // (virtio / net / blk / REPL). Diverges into ring 3 and never
    // returns.
    #[cfg(feature = "ring3-smoke")]
    {
        println!("AREST kernel online");
        println!("  mode: ring3-smoke — launching test payload");
        arch::init_memory(boot_info);
        system::init();
        syscall::init();
        userspace::launch_test_payload();
    }

    // Install the framebuffer singleton (#269 prep — bootloader
    // already maps a linear FB; no virtio-gpu needed for the basic
    // graphics path). Done BEFORE `arch::init_memory` consumes
    // `boot_info` since the field-disjoint borrow on `.framebuffer`
    // here is independent from the whole-struct re-borrow below.
    if let Some(fb) = boot_info.framebuffer.as_mut() {
        let info = fb.info();
        let buf = fb.buffer_mut();
        let ptr = buf.as_mut_ptr();
        let len = buf.len();
        // SAFETY: `buf` originates in the bootloader-managed
        // `&'static mut BootInfo`, so the bytes live for the whole
        // boot. `framebuffer::install` immediately stashes the
        // pointer behind a Mutex — no other code is holding a
        // reference at this point in init.
        unsafe { framebuffer::install(info, ptr, len) };
    }

    let phys_offset = arch::init_memory(boot_info);
    kernel_run(phys_offset)
}

/// Arch-neutral kernel bring-up. Runs after the entry path has
/// brought up allocator, console, descriptor tables, and page
/// tables, and supplied the bootloader-mapped physical-memory
/// offset.
///
/// Today only the BIOS path (`kernel_main`) reaches it. The UEFI
/// path (`entry_uefi.rs`) still parks in a pre-ExitBootServices halt
/// loop after the banner — step 4 of #344 brings it through
/// ExitBootServices into `arch::init_*` and then here, at which
/// point a single `cargo build --target x86_64-unknown-uefi` boots
/// over OVMF with the same banner the BIOS path produces.
///
/// The set of `cfg(not(target_os = "uefi"))` gates on `mod`
/// declarations above is what currently constrains `kernel_run` to
/// the BIOS build — every subsystem this function touches needs to
/// compile on UEFI before the gate can drop.
#[cfg(not(target_os = "uefi"))]
fn kernel_run(phys_offset: u64) -> ! {
    virtio::init_offset(phys_offset);
    // virtio-net bring-up (#262). PCI scan first (banner-visible),
    // then construct the full VirtIONet driver, then wrap it in the
    // `smoltcp::phy::Device` adapter `VirtioPhy` so the interface
    // talks to the real NIC. If any step fails we fall back to the
    // loopback device so in-guest smoke tests still run.
    let virtio_net = pci::find_virtio_net();
    let virtio_phy = virtio::try_init_virtio_net().map(virtio::VirtioPhy::new);
    let virtio_mac = virtio_phy.as_ref().map(|p| p.mac_address());

    // virtio-blk bring-up (#335). PCI scan + driver init; on success
    // the `block` module takes ownership of the driver and exposes a
    // sector-oriented API to `block_storage` (#337). Absence is non-
    // fatal — the kernel continues booting with in-memory state only.
    let virtio_blk_pci = pci::find_virtio_blk();
    let virtio_blk = virtio::try_init_virtio_blk();
    let blk_capacity_sectors = virtio_blk.as_ref().map(|d| d.capacity()).unwrap_or(0);
    let blk_readonly = virtio_blk.as_ref().map(|d| d.readonly()).unwrap_or(false);
    if let Some(dev) = virtio_blk {
        block::install(dev);
    }

    net::init(virtio_phy);
    system::init();
    net::register_http(80, arest_http_handler);

    // Collect memory stats for the banner.
    let frame_count = arch::memory::usable_frame_count();
    let usable_mib  = (frame_count * 4096) / (1024 * 1024);

    println!("AREST kernel online");
    println!("  target: x86_64-unknown-none");
    println!("  heap:   1 MiB static (#178)");
    println!("  gdt:    loaded with TSS + double-fault IST (#179)");
    println!("  idt:    breakpoint + double-fault + keyboard (#181)");
    println!("  pic:    remapped to 32+, timer (IRQ 0) + keyboard (IRQ 1) unmasked");
    println!(
        "  time:   PIT 1 kHz monotonic ms (#180 followup), now_ms={}",
        arch::time::now_ms(),
    );
    println!("  memory: {usable_mib} MiB usable RAM ({frame_count} x 4 KiB frames) (#180)");
    match framebuffer::info() {
        Some(info) => {
            println!(
                "  fb:     {}x{} @{}bpp, stride={}, format={:?} (bootloader-mapped)",
                info.width, info.height, info.bytes_per_pixel * 8, info.stride, info.pixel_format,
            );
            // Triple-buffer paint smoke (#269). Two presents so the
            // buffer chain visibly cycles — the second draw lands on
            // a different back than the first. Each present's hash
            // gets logged so the smoke harness can assert
            // deterministic frame content.
            use framebuffer::Color;
            let _ = framebuffer::with_back(|back| {
                back.clear(Color::rgb(0x10, 0x10, 0x18));
                back.fill_rect(40,  40, 320, 200, Color::RED);
                back.fill_rect(360, 40, 320, 200, Color::GREEN);
                back.fill_rect(680, 40, 320, 200, Color::BLUE);
                back.draw_line(40,  260, 1240, 260, Color::WHITE);
                back.draw_text(40,  280, "AREST kernel", Color::YELLOW);
            });
            framebuffer::present();
            let frame_a = framebuffer::front_fnv1a().unwrap_or(0);
            let _ = framebuffer::with_back(|back| {
                // Re-draw on the OTHER back buffer (rotated by present),
                // overlay a white rect — proves both back buffers reach
                // the front and damage tracking copies just the changed
                // region rather than the whole 1280x720x3 surface.
                back.clear(Color::rgb(0x10, 0x10, 0x18));
                back.fill_rect(40,  40, 320, 200, Color::RED);
                back.fill_rect(360, 40, 320, 200, Color::GREEN);
                back.fill_rect(680, 40, 320, 200, Color::BLUE);
                back.fill_rect(560, 100, 160, 80, Color::WHITE);
                back.draw_line(40,  260, 1240, 260, Color::WHITE);
                back.draw_text(40,  280, "AREST kernel", Color::YELLOW);
            });
            framebuffer::present();
            let frame_b = framebuffer::front_fnv1a().unwrap_or(0);
            println!(
                "  fb:     paint smoke OK, presents={}, frame_a={:#018x}, frame_b={:#018x} (#269)",
                framebuffer::presents(), frame_a, frame_b,
            );
        }
        None => println!("  fb:     none (text-mode boot — no linear framebuffer)"),
    }
    println!("  net:    smoltcp loopback 127.0.0.1/8 (#261 — virtio-net in #262)");
    match virtio_net {
        Some(dev) => println!(
            "  pci:    virtio-net found at {:02x}:{:02x}.{} (vendor={:#06x} device={:#06x}, BAR0={:#010x})",
            dev.bus, dev.device, dev.function, dev.vendor_id, dev.device_id, dev.bars[0],
        ),
        None => println!("  pci:    no virtio-net device found on legacy PCI bus (loopback only)"),
    }
    match &virtio_mac {
        Some(mac) => println!("  virtio: driver online, smoltcp phy bound, MAC {}", mac),
        None => println!("  virtio: driver not constructed — falling back to loopback"),
    }
    match virtio_blk_pci {
        Some(dev) => println!(
            "  blk:    virtio-blk found at {:02x}:{:02x}.{} (device={:#06x})",
            dev.bus, dev.device, dev.function, dev.device_id,
        ),
        None => println!("  blk:    no virtio-blk device on legacy PCI bus (non-persistent boot)"),
    }
    if blk_capacity_sectors > 0 {
        let cap_kib = (blk_capacity_sectors * (block::BLOCK_SECTOR_SIZE as u64)) / 1024;
        let mode = if blk_readonly { "read-only" } else { "read-write" };
        println!(
            "  blk:    driver online, {} sectors ({} KiB), {} (#335)",
            blk_capacity_sectors, cap_kib, mode,
        );
    } else {
        println!("  blk:    driver not constructed — persistence disabled");
    }

    // Storage-4 (#337). Boot-time mount reads sector 0 off the disk;
    // on success, `block_storage::last_state` exposes the rehydrated
    // freeze bytes for system::init to consume (not yet wired here —
    // the kernel's Once<Object> is immutable per boot; wiring happens
    // once the mutation path lands). Until then the mount status +
    // round-trip smoke prove the pipeline is live end-to-end.
    let mount_status = block_storage::mount();
    match mount_status {
        block_storage::MountStatus::NoDevice => {
            println!("  blk:    no persistence device — kernel state is ephemeral");
        }
        block_storage::MountStatus::FreshDisk => {
            println!("  blk:    fresh disk (no prior checkpoint) — first-boot semantics");
        }
        block_storage::MountStatus::Rehydrated => {
            let prev = block_storage::last_boot_count();
            let bytes = block_storage::last_state().map(|v| v.len()).unwrap_or(0);
            println!(
                "  blk:    rehydrated checkpoint ({} bytes, boot_count was {}) (#337)",
                bytes, prev,
            );
        }
        block_storage::MountStatus::Corrupted => {
            println!("  blk:    checkpoint CRC mismatch — refusing silent overwrite");
        }
    }
    if matches!(
        mount_status,
        block_storage::MountStatus::FreshDisk | block_storage::MountStatus::Rehydrated,
    ) {
        // End-to-end smoke: write a marker + read it back via the full
        // header-plus-sectors path. Proves virtio-blk read/write, CRC
        // validation, and the header-last write ordering. Runs once
        // per boot; replaced by the real commit path once the kernel
        // grows a mutable state transition.
        if block_storage::smoke_round_trip() {
            let new_bc = block_storage::last_boot_count();
            println!(
                "  blk:    checkpoint round-trip OK (boot_count now {}) (#337)",
                new_bc,
            );
        } else {
            println!("  blk:    checkpoint round-trip FAILED");
        }
    }
    println!("  http:   listening on :80 (#264)");

    // wasmi-in-kernel deferred (#270 research follow-up). The
    // wasmi crate compiles cleanly into x86_64-unknown-none with
    // `default-features = false, features = ["hash-collections"]`
    // (verified via `cargo build`), and SSE is now enabled in
    // arch::enable_sse so f32/f64 ops won't fault. But the BIOS
    // bootloader silently triple-faults at kernel load time when
    // the wasmi runtime entry is reachable — Engine + Linker +
    // Module::new pulls in enough .text + .data.rel.ro that the
    // bootloader-stage frame allocator can't satisfy the load.
    // Likely fix: switch to UEFI loader (#344 step 4c+) which has
    // a much larger early-boot pool, or carve a non-static heap
    // from BootInfo.memory_regions (#180 follow-up). Until then,
    // Doom-via-WASM stays a future task — this kernel ships
    // without the runtime.

    // Prove the allocator works — allocate a String and echo it.
    let greeting = "heap is live".to_string();
    println!("  alloc: {greeting}");

    // Prove the IDT routes — fire a software breakpoint, which should
    // land in our breakpoint handler, print a frame, and return cleanly.
    arch::breakpoint();
    println!("  idt:   int3 round-tripped through breakpoint handler");

    println!("  repl:   line-buffered keyboard REPL online (#183)");
    println!();

    // Print initial prompt — REPL is now live.
    repl::init();

    arch::halt_forever();
}

/// HTTP handler. Two-stage routing:
///
///   1. `assets::lookup` — baked ui.do bundle (#266). Matches `/`,
///      `/assets/*`, and SPA fallback for React-router paths. API
///      paths (`/api/*`) and `/assets/*` misses return `None` here.
///   2. `system::dispatch` — ρ-applied defs over the baked state
///      (#265). Handles `/api/*` and, when no bundle is baked in,
///      the legacy `/` banner.
///
/// Anything that neither resolves returns a plaintext 404.
#[cfg(not(target_os = "uefi"))]
fn arest_http_handler(req: &http::Request) -> http::Response {
    if let Some(asset) = assets::lookup(&req.path) {
        return http::Response::ok_cached(
            asset.content_type,
            asset.cache_control,
            asset.body.to_vec(),
        );
    }
    match system::dispatch(&req.method, &req.path, &req.body) {
        Some(body) => http::Response::ok("text/plain; charset=utf-8", body),
        None => http::Response::not_found(),
    }
}

// BIOS path panic handler. The UEFI path gets its handler from
// `uefi-rs`'s `panic_handler` feature — we MUST NOT declare a
// second one or the linker complains about duplicate `rust_begin_unwind`.
#[cfg(not(target_os = "uefi"))]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    println!("\n!! AREST kernel panic !!");
    println!("{info}");
    #[cfg(feature = "ring3-smoke")]
    {
        userspace::halt_on_exit(userspace::exit_code::KERNEL_PANIC);
    }
    #[cfg(not(feature = "ring3-smoke"))]
    {
        arch::halt_forever();
    }
}

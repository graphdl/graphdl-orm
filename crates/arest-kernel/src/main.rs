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
#![cfg_attr(not(target_os = "uefi"), feature(abi_x86_interrupt))]
#![cfg_attr(not(target_os = "uefi"), feature(naked_functions))]

extern crate alloc;

// UEFI entry path (#344) — compiles only on `x86_64-unknown-uefi` /
// `aarch64-unknown-uefi`. Its `#[entry]` macro defines the PE32+
// `_start` symbol; all the BIOS-path code below is cfg-gated out.
#[cfg(target_os = "uefi")]
mod entry_uefi;

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

#[cfg(not(target_os = "uefi"))]
mod allocator;
#[cfg(not(target_os = "uefi"))]
mod assets;
#[cfg(not(target_os = "uefi"))]
mod block;
#[cfg(not(target_os = "uefi"))]
mod block_storage;
#[cfg(not(target_os = "uefi"))]
mod dma;
#[cfg(not(target_os = "uefi"))]
mod framebuffer;
#[cfg(not(target_os = "uefi"))]
mod http;
#[cfg(not(target_os = "uefi"))]
mod net;
#[cfg(not(target_os = "uefi"))]
mod pci;
#[cfg(not(target_os = "uefi"))]
mod repl;
#[cfg(not(target_os = "uefi"))]
mod syscall;
#[cfg(not(target_os = "uefi"))]
mod system;
#[cfg(not(target_os = "uefi"))]
mod userspace;
#[cfg(not(target_os = "uefi"))]
mod virtio;

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

    // Sec-6 ring-3 smoke test mode. Run only the subsystems needed
    // by userspace::launch_test_payload (memory — so map_user_page
    // can allocate user pages) and skip the rest (virtio / net /
    // system / REPL). Diverges into ring 3 and never returns.
    #[cfg(feature = "ring3-smoke")]
    {
        println!("AREST kernel online");
        println!("  mode: ring3-smoke — launching test payload");
        arch::init_memory(boot_info);
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
    println!("  pic:    remapped to 32+, keyboard (IRQ 1) unmasked");
    println!("  memory: {usable_mib} MiB usable RAM ({frame_count} x 4 KiB frames) (#180)");
    match framebuffer::info() {
        Some(info) => {
            // Smoke a single white-pixel write to prove the byte slice is
            // mapped + writable. Top-left corner so it doesn't disturb
            // any boot-time text-mode region the firmware may still own.
            let _ = framebuffer::with_buffer(|fb| fb.put_pixel(0, 0, 0xFF, 0xFF, 0xFF));
            println!(
                "  fb:     {}x{} @{}bpp, stride={}, format={:?} (bootloader-mapped)",
                info.width, info.height, info.bytes_per_pixel * 8, info.stride, info.pixel_format,
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

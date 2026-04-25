// crates/arest-kernel/src/main.rs
//
// AREST UEFI kernel entry point. Runs under firmware-provided UEFI
// Boot Services on three targets — `x86_64-unknown-uefi` (laptops,
// OVMF / QEMU x86), `aarch64-unknown-uefi` (Raspberry Pi 4, AAVMF /
// QEMU virt-arm), and the custom `arest-kernel-armv7-uefi.json`
// spec (older 32-bit ARM, ArmVirtPkg). The legacy BIOS path
// (`x86_64-unknown-none` + the rust-osdev `bootloader` crate) was
// deprecated in #380 once UEFI reached full parity (#344 series +
// #429/#430/#431 launcher landed).
//
// Each `#[entry]` macro in `entry_uefi*.rs` defines the PE32+
// `_start` symbol the firmware picks up; the per-arch entries are
// otherwise siblings, diverging where the panic handler / pre-EBS
// console differ (COM1 port I/O on x86_64 vs PL011 MMIO on ARM).
// Once each entry has driven `boot::exit_boot_services` + the
// per-arch `arch::init_*` plumbing, the rest of bring-up is
// arch-neutral and converges in the shared kernel body below.

#![no_std]
#![no_main]
// abi_x86_interrupt is needed on any x86_64 UEFI build that installs
// an IDT with `extern "x86-interrupt" fn` handlers — see
// arch::uefi::interrupts (#363).
#![cfg_attr(target_arch = "x86_64", feature(abi_x86_interrupt))]

extern crate alloc;

// UEFI entry path (#344). Three separate entry files — the x86_64
// arm (`entry_uefi.rs`), the aarch64 arm (`entry_uefi_aarch64.rs`),
// and the armv7 arm (`entry_uefi_armv7.rs`) — because the panic
// handlers diverge (COM1 port I/O vs PL011 MMIO) and the pre-EBS
// init surface is arch-specific (heap, SSE enable, CR0/CR4 on x86_64
// vs PL011 + GIC scaffold on ARM). Each `#[entry]` macro defines the
// PE32+ `_start` symbol the firmware picks up.
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
// in Cargo.toml; the aarch64 / armv7 UEFI arms have no kernel_run
// → wasmi caller wired yet.
//
// `feature = "doom"` (#455 + #456 Track VVV): the Doom WASM blob is
// GPL-2.0 (jacobenget/doom.wasm v0.1.0) per #396, so the host shim +
// every reachable instantiation path is gated behind the `doom`
// cargo feature. Default kernel builds ship AGPL-3.0-or-later only;
// `--features doom` opts into the GPL-2.0 inheritance.
#[cfg(all(target_os = "uefi", target_arch = "x86_64", feature = "doom"))]
pub mod doom;
// Doom WASM binary re-export (#372). Pairs with `mod doom;` — while
// `doom.rs` is the host-shim trait + linker binding, `doom_bin.rs`
// just exposes `DOOM_WASM: &[u8]` baked from `doom_assets/doom.wasm`
// by build.rs. Gated the same way as the shim itself (wasmi +
// `feature = "doom"`). Empty slice on fresh clones that skipped the
// binary stage — see doom_bin.rs top-of-file for the rationale.
#[cfg(all(target_os = "uefi", target_arch = "x86_64", feature = "doom"))]
mod doom_bin;
// Doom IWAD binary re-export (#383). Sibling of `mod doom_bin;` —
// same pattern, different binary. `doom_wad.rs` exposes
// `DOOM_WAD: &[u8]` baked from `doom_assets/doom1.wad` (DOOM 1
// Shareware v1.9 IWAD) by build.rs, consumed by
// `KernelDoomHost::wad_sizes` / `read_wads` in src/doom.rs to feed
// the guest engine a real WAD instead of its internally-embedded
// Shareware fallback. Same UEFI-x86_64 + `feature = "doom"` gating
// as `doom_bin` since the only consumer is the wasmi host-shim.
// Empty slice on fresh clones that skipped the WAD stage — see
// doom_wad.rs top-of-file for the rationale.
#[cfg(all(target_os = "uefi", target_arch = "x86_64", feature = "doom"))]
mod doom_wad;

// `arch` is shared across all UEFI entries. On x86_64 UEFI it
// supplies the full 16550 / GDT / IDT / paging / PIT / PS-2 surface
// post-ExitBootServices; on aarch64 / armv7 UEFI it supplies the
// PL011 MMIO console and the WFI idle loop. The `_print` plumbing
// is fed by `print!` / `println!` macros declared in `arch::mod`.
mod arch;

// `framebuffer` exposes a `FrameBufferInfo` + `PixelFormat` shape
// the UEFI entries populate from `GraphicsOutputProtocol`. The
// triple-buffered draw + present pipeline + the `blit_doom_frame`
// adapter live here.
mod assets;
mod dma;
// `fonts` / `icons` (#433 + #434) — vendored design-system bytes. Each
// is a leaf module that exposes `pub static &'static [u8]` slices via
// `include_bytes!` against `assets/fonts/` and `assets/icons/lucide/`,
// plus an `icons::by_name` lookup keyed on `readings/ui/design.md`'s
// IconToken naming. Arch-neutral (no syscalls, no port I/O), so they
// sit alongside the other arch-neutral modules with no cfg gate. Slint
// kernel UI wire-up lands at #436.
pub mod fonts;
pub mod icons;
// Slint apps. First entry: HATEOAS resource browser (#429, Track SSS).
// Each submodule wires `crate::system::with_state` queries to a Slint
// component generated by `slint::include_modules!()` in
// `arch/uefi/slint_backend.rs`. Apps are constructible from any caller
// (including the #431 boot UI launcher). Available on every arch
// the slint surface compiles on (currently UEFI x86_64 only — see
// the `mod ui_apps` import gate in arch::uefi for the cfg story).
#[cfg(all(target_os = "uefi", target_arch = "x86_64"))]
pub mod ui_apps;
mod framebuffer;
// `composer` (#489 Track LLLL). Foreign-toolkit texture compositor
// — the runtime substrate Qt (GGGG #487) and GTK (IIII #488)
// adapters plug into so their widgets render into ForeignSurface
// pixel buffers and Slint composites them per frame as just-
// another-Image source (same primitive VVV's #455 Doom uses to
// push WASM-rendered frames at Slint). The trait surface
// (`ToolkitRenderer`) is the integration seam the toolkit
// adapters fill in once their loader.rs stops returning
// LibraryNotFound (post-#460 + #461). Available unconditionally
// — the abstraction has no toolkit-specific deps and the
// `compositor-test` feature gates the only renderer included
// in the foundation slice (a checkerboard `RustTestRenderer` for
// end-to-end verification). pub so the future Qt/GTK adapters
// and the launcher's super-loop can reach `compose_frame`.
pub mod composer;
// `toolkit_loop` (#490 Track MMMM). Event-loop side of the foreign-
// toolkit Component runtime — sibling of LLLL's #489 `composer`.
// Where the composer owns the texture round-trip (Qt/GTK render into
// `ForeignSurface`, Slint composites), this module owns event-loop
// coordination: Qt's `QEventLoop`, GTK's `GMainLoop`, and Slint's
// own per-tick housekeeping multiplexed inside UUU's #431 launcher
// super-loop. Each registered `ToolkitPump` gets a budgeted time
// slice (≤4ms) per tick; events from AREST's keyboard/pointer rings
// route through `dispatch_key` / `dispatch_pointer` to the
// focused-by-foreign-toolkit pump (existing direct dispatch is the
// fallback when no foreign toolkit owns focus). Available
// unconditionally — the abstraction has no toolkit-specific deps;
// pump impls (qt_adapter::event_loop, gtk_adapter::event_loop) are
// gated by their respective adapter features.
pub mod toolkit_loop;
mod http;
// `pci` / `repl` reach `x86_64::instructions::port::Port` +
// `x86_64::instructions::interrupts::disable` at module scope (see
// pci.rs L30, repl.rs L118-120). The `x86_64` crate gates those on
// `target_arch = "x86_64"` internally, so on aarch64 / armv7 UEFI
// the imports fail. Available on x86_64 UEFI; elided on aarch64 /
// armv7 UEFI — there's nothing PCI-like to probe on QEMU virt
// until the ARM arms grow a device-tree walker, and the REPL's
// keyboard IRQ path has no ARM analogue without a GIC driver.
#[cfg(target_arch = "x86_64")]
mod pci;
#[cfg(target_arch = "x86_64")]
mod repl;
mod system;

// `linuxkpi` (#460 Track AAAA, foundation for the AREST-on-real-
// hardware epic #459). FreeBSD-style Linux kernel API shim that lets
// unmodified Linux C drivers link against AREST primitives. Off by
// default behind the `linuxkpi` cargo feature — same gate shape as
// VVV's #456 doom-feature pattern. Default kernel builds skip the C
// compile of the vendored Linux source (vendor/linux/) entirely, so
// the .efi footprint and AGPL-3.0-or-later license story stay
// unchanged. Opting in via `--features linuxkpi` brings in the
// vendored `drivers/virtio/virtio_input.c` (GPL-2.0-only) which is
// FSF-compatible with AGPL-3.0-or-later. Foundation slice ships
// clean linkage only — actual driver discovery / event flow is
// #459b. Available on x86_64 UEFI only (the C compile for the
// vendored Linux source assumes amd64 calling convention; ARM
// arms can opt in as a future track).
#[cfg(all(target_os = "uefi", target_arch = "x86_64", feature = "linuxkpi"))]
pub mod linuxkpi;

// `qt_adapter` (#487 Track GGGG, second adapter slice in the toolkit
// registry chain). After AAAA's #460 linuxkpi shim landed, this module
// is the LIBRARY-mode equivalent — load `libqt6widgets.so.6` +
// `libqt6core.so.6` as Linux-style shared libraries, expose Qt's widget
// classes as `Component` cells via `ImplementationBinding` facts
// (mirroring the static declarations DDDD #485 emitted in
// `readings/ui/components.md`). Off by default behind the `qt-adapter`
// cargo feature — same gate shape as `linuxkpi` and `doom`. Default
// kernel builds elide the module entirely; --features qt-adapter brings
// in the loader + Component fact registration. The library loader
// degrades to a `LibraryNotFound` stub when linuxkpi has no library-
// loading path yet (foundation slice was driver-mode focused), so the
// Component cells populate with null Symbol pointers; future linuxkpi
// extension fills them in. Selection still picks Slint over Qt because
// the compositor isn't wired (#489).
#[cfg(all(target_os = "uefi", target_arch = "x86_64", feature = "qt-adapter"))]
pub mod qt_adapter;

// `gtk_adapter` (#488 Track IIII, third adapter slice in the toolkit
// registry chain). Symmetric to qt_adapter (#487) — load
// `libgtk-4.so.1` + `libgobject-2.0.so.0` + `libglib-2.0.so.0` as
// Linux-style shared libraries via the linuxkpi shim, expose GTK 4's
// widget classes as `Component` cells via `ImplementationBinding`
// facts (mirroring the static declarations DDDD #485 emitted in
// `readings/ui/components.md`). Off by default behind the
// `gtk-adapter` cargo feature — same gate shape as `linuxkpi`,
// `doom`, and `qt-adapter`. Default kernel builds elide the module
// entirely; --features gtk-adapter brings in the loader + Component
// fact registration. The library loader degrades to a
// `LibraryNotFound` stub when linuxkpi has no library-loading path
// yet (foundation slice was driver-mode focused), so the Component
// cells populate with class-name string Symbols + null GType pointers;
// future linuxkpi extension fills them in. Selection still picks
// Slint over GTK because the compositor isn't wired (#489).
#[cfg(all(target_os = "uefi", target_arch = "x86_64", feature = "gtk-adapter"))]
pub mod gtk_adapter;

// `block` / `block_storage` / `virtio` reach `x86_64::structures::
// paging::Translate` via `arch::memory::with_page_table`, plus the
// PCI transport. All three are x86_64-only today; aarch64 / armv7
// UEFI elide them (PCI port I/O + x86_64 paging traits have no ARM
// analogue without a device-tree walker).
#[cfg(target_arch = "x86_64")]
mod block;
#[cfg(target_arch = "x86_64")]
mod block_storage;
// `file_serve` (#403): HTTP `GET|HEAD /file/{id}/content` route. Reads
// File noun bytes out of the baked SYSTEM state and produces fully-
// serialised HTTP/1.1 wire bytes (including 206 Partial Content for
// `Range` requests). Lives next to `net` because the dispatch path
// reaches both — same x86_64-only gating since the lookup uses
// `block_storage::reserve_region` for region-backed blobs.
#[cfg(target_arch = "x86_64")]
mod file_serve;
// `file_upload` (#444): HTTP `POST /file` route. Sibling write side
// of `file_serve` — accepts a single-part `multipart/form-data` upload
// with a `directory_id` form field, sniffs MIME on the bytes, and
// builds the File noun's five facts (Name / MimeType / ContentRef /
// Size / is_in_Directory). Bodies > 64 KiB return 413 pointing at
// the chunked-PUT route (#445, future track). Same x86_64-only gating
// as `file_serve` for symmetry — the cell-write pipeline mirrors the
// reader's lookup pipeline.
#[cfg(target_arch = "x86_64")]
mod file_upload;
// `net` is available on every UEFI arm (x86_64 / aarch64 / armv7).
// The smoltcp + virtio-drivers deps build cleanly under no_std
// cross-arch. The intra-crate refs in net.rs to `crate::virtio`
// (PCI transport, x86_64-only) and `crate::virtio_mmio` (MMIO
// transport, aarch64+armv7-UEFI-only) are cfg-gated inside net.rs
// so each arm picks the right phy adapter; the file_* intercept
// arms in `drive_http` are cfg-gated to x86_64 because they reach
// `crate::block_storage` which is x86_64-only.
mod net;
#[cfg(target_arch = "x86_64")]
mod virtio;
// virtio-gpu wrapper (#371). Sibling of `virtio` (net + blk) — wraps
// `virtio_drivers::device::gpu::VirtIOGpu` with the same `KernelHal` +
// `PioCam` PCI transport infrastructure. Same x86_64-only gating as
// `virtio` and `pci` because the PCI walker / port I/O backing aren't
// available on aarch64 / armv7 yet.
#[cfg(target_arch = "x86_64")]
mod virtio_gpu;
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

// Each UEFI entry (`entry_uefi*.rs`) carries its own `_start`
// (PE32+ entry symbol the firmware probes), its own
// `#[global_allocator]` wired against a `boot::allocate_pages`-
// derived heap, and its own panic handler — pre-EBS console
// surfaces diverge by arch (COM1 port I/O on x86_64 vs PL011 MMIO
// on ARM), so a shared panic handler isn't possible without a
// runtime branch on something the panic handler can't reach.

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
///
/// Available on every UEFI arm (x86_64 / aarch64 / armv7) — same
/// gate as `mod net;`. The per-arch entries call `net::register_http
/// (80, arest_http_handler)` to make `/api/*` and the SPA fallback
/// reachable through the official `Handler` chain (#360 + #450).
/// The intercept-style file_* routes in `net::drive_http` short-
/// circuit before this handler runs, so the non-x86_64 UEFI arms
/// (no `block_storage` / `file_serve` / `file_upload`) still get the
/// assets + system::dispatch surface without pulling in the storage
/// stack.
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

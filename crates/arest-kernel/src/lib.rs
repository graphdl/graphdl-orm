// crates/arest-kernel/src/lib.rs
//
// AREST UEFI kernel — library facade. Mirror of `main.rs`'s module
// tree, minus the bin-specific `#[no_main]` + `_start` symbol the
// firmware probes. The `[lib]` target gives `cargo test --lib -p
// arest-kernel` a runnable surface for the inline `#[cfg(test)]`
// modules scattered through `process/`, `syscall/`, `synthetic_fs/`,
// `linuxkpi/`, `ui_apps/`, etc. — without it Cargo's bin-only crate
// shape (declared via `[[bin]] test = false`) silently drops every
// inline test on the floor.
//
// Why a separate file rather than `path = "src/main.rs"` for `[lib]`
// ------------------------------------------------------------------
// `main.rs` carries `#![no_main]` (no `fn main`) and includes the
// per-arch `mod entry_uefi*;` declarations that bring in `#[entry]`-
// macro–derived `_start` symbols. That's exactly what the kernel `.efi`
// binary wants — but a Rust *library* has no entry point and `[lib]`
// rejects `#![no_main]`. So the lib carries `#![no_std]` only, declares
// every kernel module, and the bin (`main.rs`) keeps the entry stubs.
// The two files share zero source — `main.rs` `use arest_kernel as _;`
// pulls the lib's compiled module tree through, so the `_start` symbol
// from `entry_uefi.rs` (which lives in the lib now) lands in the linked
// `.efi` image without re-declaring every `mod` line in the bin.
//
// Host-target compatibility
// -------------------------
// The lib compiles on any `target_os` — UEFI for the actual kernel
// build, Windows / Linux / Darwin for `cargo test --lib`. UEFI-specific
// modules (`entry_uefi*`, `arch::uefi`, `arch::aarch64`, `arch::armv7`,
// virtio transports, `block*`, `pci`, `repl`, `ui_apps`, the foreign-
// toolkit adapters, `linuxkpi`, `doom*`) are gated on `target_os =
// "uefi"` so the host build only sees the pure-logic modules
// (`process`, `syscall`, `synthetic_fs`, `composer`, `component_binding`,
// `toolkit_loop`, `assets`, `dma`, `fonts`, `icons`, `framebuffer`,
// `http`, `system`, `net`).
//
// Tests inside UEFI-only modules don't run on the host (their parent
// module isn't compiled). That's expected — the tests gated `#[cfg(all
// (test, target_os = "linux"))]` in `composer.rs` /
// `component_binding.rs` / `synthetic_fs/*.rs` / `toolkit_loop.rs` /
// `gtk_adapter/event_loop.rs` / `qt_adapter/event_loop.rs` already
// document that pattern: they want the host-target test runner.
//
// Why `extern crate alloc` rather than a `use` block
// --------------------------------------------------
// `#![no_std]` strips the `std` prelude, including `Box` / `Vec` /
// `String`. The `alloc` crate is shipped in the sysroot but not auto-
// linked — `extern crate alloc;` brings it in. Mirror of the same
// pattern in `crates/arest/src/lib.rs` (line 23) and the bin's
// `main.rs` line 27.

#![no_std]
// abi_x86_interrupt is needed on any x86_64 UEFI build that installs
// an IDT with `extern "x86-interrupt" fn` handlers — see
// arch::uefi::interrupts (#363). The bin and the lib both carry the
// gate so the lib compiles cleanly under `cargo test --lib` on a host
// stable toolchain (the cfg evaluates to false on non-x86_64 hosts) AND
// under `cargo build --target x86_64-unknown-uefi` on nightly.
#![cfg_attr(target_arch = "x86_64", feature(abi_x86_interrupt))]

extern crate alloc;

// UEFI entry path (#344). Three separate entry files — the x86_64 arm
// (`entry_uefi.rs`), the aarch64 arm (`entry_uefi_aarch64.rs`), and
// the armv7 arm (`entry_uefi_armv7.rs`) — because the panic handlers
// diverge (COM1 port I/O vs PL011 MMIO) and the pre-EBS init surface
// is arch-specific. Each `#[entry]` macro defines the PE32+ `_start`
// symbol the firmware picks up; the lib carries them so the symbol
// lands in the linked `.efi` image once `main.rs` `use`s the lib.
#[cfg(all(target_os = "uefi", target_arch = "x86_64"))]
pub mod entry_uefi;
#[cfg(all(target_os = "uefi", target_arch = "aarch64"))]
pub mod entry_uefi_aarch64;
#[cfg(all(target_os = "uefi", target_arch = "arm"))]
pub mod entry_uefi_armv7;

// Doom WASM host-shim (#270/#271). UEFI x86_64 + `feature = "doom"`-
// gated; same shape as in `main.rs` pre-extract.
#[cfg(all(target_os = "uefi", target_arch = "x86_64", feature = "doom"))]
pub mod doom;
#[cfg(all(target_os = "uefi", target_arch = "x86_64", feature = "doom"))]
pub mod doom_bin;
#[cfg(all(target_os = "uefi", target_arch = "x86_64", feature = "doom"))]
pub mod doom_wad;

// `arch` is shared across all UEFI entries. On x86_64 UEFI it supplies
// the full 16550 / GDT / IDT / paging / PIT / PS-2 surface
// post-ExitBootServices; on aarch64 / armv7 UEFI it supplies the PL011
// MMIO console and the WFI idle loop. On host targets the module
// supplies no-op stubs for `_print` etc. so the `crate::print!` /
// `crate::println!` macros (declared inside `arch::mod`) compile
// cleanly under `cargo test --lib`.
pub mod arch;

// Pure-logic modules — compile on every target (UEFI x86_64 / aarch64
// / armv7 plus host x86_64-pc-windows-msvc / x86_64-unknown-linux-gnu).
// Their inline `#[cfg(test)]` blocks are what `cargo test --lib`
// actually runs.
pub mod assets;
pub mod dma;
pub mod fonts;
pub mod icons;
// `ui_apps` is the Slint-driven boot UI surface (Unified REPL,
// launcher, keyboard, doom). Every submodule that touches the
// runtime imports `slint::*`, and the launcher's `run(...)`
// builder needs `arch::uefi::slint_backend::*` (also gated on
// `feature = "slint"` below). #627 Profile-3: the whole tree
// elides under the headless `--no-default-features --features
// server` profile so the .efi link line drops the launcher
// surface — the only call site is `entry_uefi.rs::kernel_run_uefi
// → ui_apps::launcher::run(...)`, which is itself feature-gated.
#[cfg(all(target_os = "uefi", target_arch = "x86_64", feature = "slint"))]
pub mod ui_apps;
pub mod framebuffer;
pub mod composer;
pub mod component_binding;
pub mod toolkit_loop;
pub mod http;
// `pci` / `repl` reach `x86_64::instructions::port::Port` +
// `x86_64::instructions::interrupts::disable` at module scope. The
// `x86_64` crate gates those on `target_arch = "x86_64"` internally,
// so the host build (which is x86_64-pc-windows-msvc) would still see
// them — but the modules also pull in `crate::arch::memory` /
// `crate::arch::*` UEFI surfaces that don't exist on host. Gate the
// lib's view at the UEFI boundary so host builds elide them entirely.
#[cfg(all(target_os = "uefi", target_arch = "x86_64"))]
pub mod pci;
// `repl` (#628 Profile-4): additionally gated on `feature = "repl"`.
// The only callers are `entry_uefi::kernel_run_uefi` (the static
// line-buffer init, itself feature-gated on `slint`) and
// `ui_apps::unified_repl::submit` (the launcher's REPL panel,
// transitively gated on `slint`). With `slint` composing `repl`
// (Cargo.toml), every default / mini / dev build still pulls the
// module; the headless `--no-default-features --features server`
// build drops it entirely. Mirror of the slint module-decl gates
// the #627 commit added to this file (lines 112 + 126-129).
#[cfg(all(target_os = "uefi", target_arch = "x86_64", feature = "repl"))]
pub mod repl;
pub mod system;

// `process` (#518 Track TTTT). Pure ELF parser + AddressSpace +
// initial-stack builder + privilege-transition trampoline. Available
// unconditionally — the trampoline's actual `iretq` body has per-arch
// `cfg(target_arch = "...")` arms; the host arm returns
// `UnsupportedArch` so the crate compiles + the unit tests run cross-
// platform.
pub mod process;

// `synthetic_fs` (#534 Track HHHHH). POSIX-path → AREST-cell renderer
// table (`/proc/cpuinfo`, `/proc/meminfo` today). Pure byte arithmetic
// — runs on host.
pub mod synthetic_fs;

// `load_reading_persist` (#560 Track PPPPP / DynRdg-T1). Persists
// runtime LoadReading bodies into a virtio-blk-backed ring and
// replays them on boot, after `system::init()` has built the baked
// metamodel state. Pure-logic byte arithmetic on the host side, with
// a UEFI-x86_64-only `VirtioBlkRing` impl that reaches the
// persistence disk via `block_storage::reserve_region`. Tests live
// inline and exercise the host-portable in-memory backend.
pub mod load_reading_persist;

// `syscall` (#507 Track GGGGG). Linux ABI syscall dispatch table.
// SYS_WRITE (1), SYS_EXIT (60), SYS_EXIT_GROUP (231), SYS_OPENAT (257),
// SYS_CLOSE (3) — all pure data marshalling, runs on host with mock
// sinks.
pub mod syscall;

// `linuxkpi` (#460 Track AAAA). UEFI x86_64 + `feature = "linuxkpi"`-
// gated. Same gate as in `main.rs`.
#[cfg(all(target_os = "uefi", target_arch = "x86_64", feature = "linuxkpi"))]
pub mod linuxkpi;

// `qt_adapter` (#487 Track GGGG). UEFI x86_64 + `feature = "qt-adapter"`-
// gated.
#[cfg(all(target_os = "uefi", target_arch = "x86_64", feature = "qt-adapter"))]
pub mod qt_adapter;

// `gtk_adapter` (#488 Track IIII). UEFI x86_64 + `feature = "gtk-adapter"`-
// gated.
#[cfg(all(target_os = "uefi", target_arch = "x86_64", feature = "gtk-adapter"))]
pub mod gtk_adapter;

// `block` / `block_storage` / `virtio` / `virtio_gpu` reach
// `x86_64::structures::paging::Translate` via
// `arch::memory::with_page_table` plus the PCI transport — UEFI x86_64-
// only. Same gate as in `main.rs` pre-extract.
#[cfg(all(target_os = "uefi", target_arch = "x86_64"))]
pub mod block;
#[cfg(all(target_os = "uefi", target_arch = "x86_64"))]
pub mod block_storage;
#[cfg(all(target_os = "uefi", target_arch = "x86_64"))]
pub mod file_serve;
#[cfg(all(target_os = "uefi", target_arch = "x86_64"))]
pub mod file_upload;
pub mod net;
#[cfg(all(target_os = "uefi", target_arch = "x86_64"))]
pub mod virtio;
#[cfg(all(target_os = "uefi", target_arch = "x86_64"))]
pub mod virtio_gpu;
// virtio-mmio transport for aarch64 + armv7 UEFI (#368/#369 aarch64,
// #388 armv7 widening). MMIO-based — different shape from the PCI
// transport in `virtio.rs`.
#[cfg(all(target_os = "uefi", any(target_arch = "aarch64", target_arch = "arm")))]
pub mod virtio_mmio;
// USB-over-USB serial gadget for Nexus debug (#392). Scaffold only —
// aarch64 + armv7 UEFI.
#[cfg(all(target_os = "uefi", any(target_arch = "aarch64", target_arch = "arm")))]
pub mod usb_uart;

/// HTTP handler — three-stage routing:
///
///   1. `assets::lookup_from_state` — ui.do bundle served out of the
///      live SYSTEM File cell graph (#580). Returns `None` for the
///      `/api/*` and `/arest/*` namespaces so dynamic paths reach
///      the dispatch tier instead of being rewritten to `index.html`
///      by the SPA fallback (#610). When the `ui-bundle` feature is
///      ON, `system::init` seeds the cell graph from a build-time
///      `include_bytes!` table so this lookup hits; when OFF (the
///      default + the `server` profile), the cell graph is empty and
///      every asset path falls through to the dispatch tier. The
///      handoff for #581: replace the boot-time seed with a runtime
///      load (HTTP fetch / freeze blob) without touching the wire
///      handler at all.
///   2. `arest::hateoas::handle_arest_read` — engine-less HATEOAS read
///      fallback for `GET /arest/{slug}` and `GET /arest/{slug}/{id}`
///      (#609). Walks the live SYSTEM state via `system::with_state`
///      and emits the `{type, docs, totalDocs}` / `{id, type, ...}`
///      envelopes the apis e2e suite expects. Mirror of the worker's
///      `handleArestReadFallback` so a single contract serves the
///      kernel + the Cloudflare worker.
///   3. `system::dispatch` — ρ-applied defs (`/api/welcome`, `/echo`,
///      …). Final stop before a 404.
///
/// Pub so the per-arch entry harnesses (`entry_uefi*::kernel_run_*`)
/// can register it via `net::register_http(80, arest_http_handler)`.
pub fn arest_http_handler(req: &http::Request) -> http::Response {
    // #580: read the asset out of the live cell graph rather than the
    // build-time `UI_ASSETS` table. The closure runs while the SYSTEM
    // read lock is held, so we capture the materialised `Asset` and
    // drop the guard before building the response. When `init()` has
    // not yet been called, `with_state` returns `None` and we skip the
    // asset tier — same external behaviour as the pre-#580 empty-table
    // build.
    if let Some(Some(asset)) =
        system::with_state(|state| assets::lookup_from_state(state, &req.path))
    {
        return http::Response::ok_cached(
            asset.content_type,
            asset.cache_control,
            asset.body,
        );
    }

    // /arest/parse — registry stats (#611/#612). Mirror of the
    // worker's `handleParseGet` (`src/api/parse.ts`). Sits *before*
    // the generic `/arest/{slug}` fallback because the path shape is
    // different (it's a stats endpoint, not an entity lookup) — the
    // generic handler would otherwise try to resolve "parse" as a
    // slug and 404 it. POST/DELETE remain unhandled here today
    // because the kernel can't yet compile readings at runtime
    // (gated on #588 lifting Stage-2 to no_std).
    if req.method == "GET" && (req.path == "/arest/parse" || req.path.starts_with("/arest/parse?")) {
        if let Some(body) = system::with_state(|s| arest::hateoas::parse_stats(s)) {
            return http::Response::ok("application/json", body);
        }
    }

    // POST /arest/extract — agent verb dispatch (#620 / HATEOAS-6b).
    // The verb is registered as `Func::Platform("extract")` at boot
    // (`system::init`), but the kernel profile installs no body —
    // so `apply` returns `Object::Bottom` and `dispatch_extract`
    // lifts that into the structured 503 envelope with the
    // `Retry-After: <worker-url>` header. Sits *before* the generic
    // HATEOAS read fallback so the path isn't silently treated as a
    // slug ("extract" would resolve to a nonexistent Noun and 404,
    // hiding the introspectable envelope). When a body is installed
    // (e.g. via `externals::install_async_platform_fn` in a future
    // worker-shaped profile), this same branch returns 200 with the
    // serialised result. Branch-free dispatch — same code path runs
    // regardless of body presence; the outcome's status is what
    // varies. See `system::dispatch_extract` for the full envelope
    // shape.
    if req.method == "POST" && req.path == "/arest/extract" {
        let outcome = system::dispatch_extract(&req.body);
        if outcome.status == 200 {
            return http::Response::ok("application/json", outcome.body);
        }
        let retry_after = outcome
            .retry_after
            .unwrap_or_else(|| alloc::string::String::from(system::EXTRACT_WORKER_URL));
        return http::Response::service_unavailable_with_retry_after(
            "application/json",
            retry_after,
            outcome.body,
        );
    }

    // POST /arest/entity — AREST command path (#614/#615), engine-less
    // direct-write fallback. Mirror of the worker's
    // `router.ts::handleArestRoute` POST branch when the engine traps.
    // Body shape: `{noun, fields:{...}, domain?}`. ID generated via
    // `arest::csprng::random_bytes` (16 random bytes hex-encoded), so
    // entropy must be installed before this fires (it is — see
    // `entry_uefi.rs::kernel_run_uefi` which calls `entropy::install`
    // pre-`net::init`).
    if req.method == "POST" && req.path == "/arest/entity" {
        let result = system::with_state(|s| {
            arest::hateoas::handle_arest_create(s, &req.method, &req.path, &req.body)
        });
        if let Some(Some((new_state, body))) = result {
            let _ = system::apply(new_state);
            return http::Response::ok("application/json", body);
        }
    }

    // GET /arest/entities/{slug}/{id}/transitions — list legal
    // next-step events for an entity's current state (#617/#618
    // companion). Mirror of the worker's GET /api/entities/:noun/:id/
    // transitions (router.ts:590). Sits before the generic GET /arest
    // read fallback so the `/transitions` suffix isn't silently
    // treated as a sub-id.
    if req.method == "GET"
        && req.path.starts_with("/arest/entities/")
        && (req.path.ends_with("/transitions") || req.path.contains("/transitions?"))
    {
        let body = system::with_state(|s| {
            arest::hateoas::handle_arest_transitions_for_entity(s, &req.method, &req.path)
        });
        if let Some(Some(body)) = body {
            return http::Response::ok("application/json", body);
        }
        return http::Response::not_found();
    }

    // POST /arest/entities/{slug}/{id}/transition — fire a
    // state-machine transition event (#617/#618). Sits *before* the
    // generic `/arest/entities/{slug}` create handler because the
    // create handler matches on the same prefix; without this guard
    // a transition POST would be silently routed to the create path
    // and rejected for malformed JSON. Mirror of the worker's
    // `router.ts::POST /api/entities/:noun/:id/transition` (line 617).
    if req.method == "POST"
        && req.path.starts_with("/arest/entities/")
        && (req.path.ends_with("/transition") || req.path.contains("/transition?"))
    {
        let result = system::with_state(|s| {
            arest::hateoas::handle_arest_transition(s, &req.method, &req.path, &req.body)
        });
        if let Some(Some((new_state, body))) = result {
            let _ = system::apply(new_state);
            return http::Response::ok("application/json", body);
        }
        // No legal transition (or missing SM / unknown event) — 400
        // with the worker's error envelope shape so the apis e2e
        // suite's `expect(res.status).toBeGreaterThanOrEqual(400)`
        // fallback assertion holds.
        return http::Response::bad_request(
            "{\"errors\":[{\"message\":\"transition rejected\"}]}",
        );
    }

    // POST /arest/entities/{slug} — direct-write fallback (#616).
    // Mirror of the worker's `router.ts::handleEntitiesPost`
    // create-side fallback. Engine path (#613) waits on #588's
    // no_std Stage-2 lift; until then this is the only POST entity
    // surface the kernel honours, sufficient for the apis e2e
    // suite's `POST /arest/entities/Organization with explicit id`
    // (`apis/__e2e__/arest.test.ts:153`) and `POST /arest/entities/
    // Support%20Request with explicit id` (line 240) cases.
    if req.method == "POST" && req.path.starts_with("/arest/entities/") {
        let result = system::with_state(|s| {
            arest::hateoas::handle_arest_create_for_slug(s, &req.method, &req.path, &req.body)
        });
        if let Some(Some((new_state, body))) = result {
            // Commit the new state under the write lock — release of
            // the read lock above and acquisition of the write lock
            // here is *not* atomic against concurrent POSTs, but the
            // smoltcp HTTP path already serves one connection at a
            // time so there's no concurrent writer to race against.
            // When the kernel grows multi-connection HTTP this needs
            // a CAS-style retry loop or a single-writer lane.
            let _ = system::apply(new_state);
            return http::Response::ok("application/json", body);
        }
    }

    // HATEOAS read fallback (#609). Only fires on `/arest/*` paths
    // (`assets::lookup` already excluded them above). Returns
    // `Some(json)` on a recognised slug + entity id, `None` for any
    // miss — including non-GET methods, unknown slugs, and missing
    // entity ids. A miss falls through to `system::dispatch`; a hit
    // short-circuits with `200 application/json`.
    if req.path.starts_with("/arest/") || req.path == "/arest" {
        let read = system::with_state(|s| arest::hateoas::handle_arest_read(s, &req.method, &req.path));
        if let Some(Some(body)) = read {
            return http::Response::ok("application/json", body);
        }
    }

    match system::dispatch(&req.method, &req.path, &req.body) {
        Some(body) => http::Response::ok("text/plain; charset=utf-8", body),
        None => http::Response::not_found(),
    }
}

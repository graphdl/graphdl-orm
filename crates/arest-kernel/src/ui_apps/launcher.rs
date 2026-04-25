// crates/arest-kernel/src/ui_apps/launcher.rs
//
// Boot UI launcher — Rust glue (#431 Track UUU).
//
// Owns the kernel's main event loop after boot. Replaces GGG's REPL
// drainer (#365) at the bottom of `kernel_run_uefi`. Constructs the
// `AppLauncher` Slint window (the splash) plus the previously-built
// `HateoasBrowser` (Track SSS #429) and `Repl` (Track TTT #430)
// windows; tracks which one is currently visible; pumps the keyboard
// ring and the Slint software renderer in a single super loop.
//
// # Why a super loop instead of `Platform::run_event_loop`
//
// Slint v1.16's `Platform::run_event_loop` defaults to
// `Err(NoEventLoopProvider)` and `arch::uefi::slint_backend::
// UefiSlintPlatform` (Track MMM #436 / QQQ #452) takes that default.
// Implementing it would require the Platform impl to know about the
// GOP framebuffer descriptor (so it can build a `FramebufferBackend`
// per draw), tangle it with the live-state singletons in `framebuffer`,
// and recompute width/height/stride/pixel-order from `framebuffer::
// info()` on every frame. The `mcu.md` reference shape Slint
// documents for MCU hosts is the **super-loop** pattern (slint v1.16
// crate `mcu.md` lines 200-245) — drive the loop from the kernel
// `main`, calling `update_timers_and_animations` + `draw_if_needed`
// directly. That's what we do here. `run_event_loop` stays
// unimplemented; nothing in the boot flow calls `Window::run()` (the
// only consumer of the event-loop trait method).
//
// # Multi-window navigation in a single MinimalSoftwareWindow
//
// `UefiSlintPlatform::create_window_adapter` returns a clone of the
// SAME `Rc<MinimalSoftwareWindow>` for every component instantiation
// — Slint's MCU model only has one physical surface. So when the
// user "opens an app" we don't actually open a second OS window: we
// hide the launcher's Slint Window, show the app's Slint Window
// (still backed by the same `MinimalSoftwareWindow`), and route
// keystrokes to the new one. The Slint runtime tracks which
// `slint::Window` is the active "shown" component and routes the
// software renderer's draw at it. This is the multi-window fallback
// the spec calls out as the simpler path on UEFI.
//
// # Esc handling: back-to-launcher
//
// `arch::uefi::slint_input::drain_keyboard_into_slint_window` (Track
// QQ #428) maps `DecodedKey::Unicode('\u{001b}')` to a Slint key
// event and dispatches it to the window. The existing app components
// (HateoasBrowser, Repl) don't consume Esc — their `key-pressed`
// handlers `reject` non-history keys, so Esc is a silent no-op. To
// route Esc to the launcher's "back" path WITHOUT modifying those
// components (which are owned by other tracks per the file ownership
// map), we run our own drain helper that intercepts Esc BEFORE
// dispatch when an app is active. Non-Esc keys go through the
// inline pump (Unicode keystrokes only — see
// `drain_keyboard_with_esc_intercept` for the rationale).
//
// # Background work hook
//
// `crate::net::poll()` is called once per loop iteration so smoltcp's
// DHCPv4 + the registered `:80` HTTP listener (`net::register_http`,
// see `entry_uefi.rs::kernel_run_uefi`) keep ticking. The poll is
// cheap when no socket woke up. A `slint::Timer`-driven approach
// would be more event-loop-idiomatic but Slint v1.16's `Timer` API
// requires a working event loop proxy (`Platform::new_event_loop_
// proxy`), which we deliberately don't implement. Per-frame poll is
// the practical equivalent on the super-loop pattern.
//
// # Safety / lifetime
//
// `run(...)` builds a `FramebufferBackend` from raw GOP coordinates
// passed in from `entry_uefi.rs::kernel_run_uefi`. The same firmware-
// mapped framebuffer is also held by `crate::framebuffer::Driver`
// (installed earlier in `kernel_run_uefi`); the kernel runs single-
// threaded at boot so the two writers coexist by construction (the
// launcher writes whole frames via `render_by_line`, the
// `framebuffer` driver isn't called from this loop). Slint takes
// ownership of the on-screen contents the moment we start drawing.

#![allow(dead_code)]

use alloc::boxed::Box;
use alloc::rc::Rc;
use core::cell::RefCell;

use slint::ComponentHandle;
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};

use crate::arch::uefi::keyboard;
use crate::arch::uefi::slint_backend::{
    AppLauncher, FramebufferBackend, FramebufferPixelOrder, UefiSlintPlatform,
};
use crate::arch::uefi::slint_input::drain_keyboard_into_slint_window;
use crate::toolkit_loop;
use crate::ui_apps::{hateoas, repl};
#[cfg(feature = "doom")]
use crate::ui_apps::doom;

/// Which Slint surface is currently visible. Driven by the
/// open-hateoas / open-repl / open-doom callbacks (forward) and the
/// Esc intercept in `drain_keyboard_with_esc_intercept` (back). The
/// `Doom` variant is unconditional in the enum so the navigation
/// state machine doesn't fork on `cfg`; the actual transition into
/// `Active::Doom` is gated behind `cfg(feature = "doom")` at the
/// callback registration site below — when the feature is off the
/// state can never reach Doom because `open-doom` is never wired.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Active {
    Launcher,
    Hateoas,
    Repl,
    #[cfg(feature = "doom")]
    Doom,
}

/// Shared mutable navigation state. The Rust closures wired into
/// AppLauncher's two callbacks set this; the super loop reads it on
/// each frame to decide which window to drive.
///
/// `Rc<RefCell<...>>` because Slint's callback signatures require
/// `'static` closures and we need both the callback and the loop to
/// mutate the same slot. The kernel's `unsafe-single-threaded` slint
/// feature (Cargo.toml L205) makes the `!Send` `Rc<RefCell<_>>` sound
/// — boot is single-threaded.
type NavState = Rc<RefCell<Active>>;

/// Run the boot UI launcher loop. Never returns.
///
/// Entered once per boot from `entry_uefi.rs::kernel_run_uefi` after
/// every init banner has been printed. Constructs the Slint platform
/// (if not already installed), the launcher splash, and both apps,
/// then enters the per-frame super loop:
///
///   1. Drain the keyboard ring (Esc-intercept when an app is active).
///   2. `slint::platform::update_timers_and_animations()` so any
///      Slint-side animation tick advances on the PIT-backed
///      `arch::time::now_ms` clock.
///   3. `crate::net::poll()` to keep smoltcp + the HTTP listener
///      ticking — same call GGG's drainer made every frame.
///   4. `window.draw_if_needed(|renderer| renderer.render_by_line(
///      &mut backend))` to repaint dirty regions into the GOP MMIO.
///   5. `pause` so the CPU isn't pinned at 100% spin.
///
/// The framebuffer descriptor (width/height/stride/pixel-format/base
/// pointer) is passed in from `kernel_run_uefi` rather than re-read
/// from `crate::framebuffer::info()` — `framebuffer` owns its own
/// `&'static mut [u8]` slice and we need a parallel `*mut u8` view
/// for `FramebufferBackend`. Both writers are sound because the boot
/// is single-threaded; only the launcher writes to the surface after
/// this function takes over.
pub fn run(
    gop_w: usize,
    gop_h: usize,
    gop_stride: usize,
    gop_fmt_idx: usize,
    gop_ptr: usize,
) -> ! {
    // Without a real framebuffer (gop_ptr == 0) the renderer would
    // write into address 0 and fault. The smoke harness under
    // OVMF + QEMU always captures GOP, but a hypothetical headless
    // boot path (or a future cross-firmware variant that fails to
    // open GOP) would land here. Falling back to a halt loop keeps
    // every boot banner above this point observable instead of
    // triple-faulting on the first paint attempt.
    if gop_ptr == 0 {
        crate::println!(
            "  ui:       launcher: no GOP framebuffer captured — entering halt loop"
        );
        loop {
            crate::net::poll();
            unsafe { core::arch::asm!("pause", options(nomem, nostack)); }
        }
    }

    // GOP pixel format → Slint pixel order. `0 = Rgb` and `1 = Bgr`
    // mirror the indexing `kernel_run_uefi` uses when it logs the GOP
    // banner; everything else (Bitmask, BltOnly) falls back to RGBX
    // as the safest default — the framebuffer install step above
    // already filters Bitmask/BltOnly out before installing the
    // backbuffers, so this branch is reachable only on the two real
    // pixel formats UEFI+OVMF emit in practice.
    let pixel_order = match gop_fmt_idx {
        1 => FramebufferPixelOrder::Bgrx,
        _ => FramebufferPixelOrder::Rgbx,
    };

    // Build the shared `MinimalSoftwareWindow` first, then hand a
    // clone to the Slint platform via `with_window`. Both the Rust
    // super-loop here AND Slint's `create_window_adapter` (called
    // when a `Component::new()` runs) end up holding the same `Rc`
    // — the renderer's `draw_if_needed` here paints whatever the
    // currently-shown component dirtied.
    //
    // `RepaintBufferType::ReusedBuffer` matches the choice
    // `UefiSlintPlatform::new` makes for its internal default: the
    // GOP MMIO is a long-lived single surface, so painting only the
    // dirty region per frame is correct (a `NewBuffer` mode would
    // force a full repaint per frame, wasting MMIO bandwidth).
    let window: Rc<MinimalSoftwareWindow> =
        MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    window.set_size(slint::PhysicalSize::new(gop_w as u32, gop_h as u32));

    let platform = Box::new(UefiSlintPlatform::with_window(window.clone()));
    if slint::platform::set_platform(platform).is_err() {
        // `set_platform` returns Err if a platform was already
        // installed in this process. Today nothing else installs a
        // platform on the boot path (smoke tests do, but they run
        // under hosted cargo test, not the .efi build). Keep the
        // notice so a future caller that double-installs surfaces
        // it in the boot log instead of silently winning the race
        // with whichever platform was first.
        crate::println!(
            "  ui:       launcher: slint platform already installed (continuing)"
        );
    }

    // SAFETY: `gop_ptr` is the firmware-mapped GOP framebuffer base
    // captured by `entry_uefi.rs::efi_main` and `mem::forget`'d via
    // its `ScopedProtocol`. The mapping is `'static` for the rest of
    // boot. `framebuffer::install` is the only other writer to that
    // region; we run single-threaded so the two writers coexist by
    // construction (no concurrent writes possible).
    let mut backend = unsafe {
        FramebufferBackend::new(gop_ptr as *mut u8, gop_w, gop_h, gop_stride, pixel_order)
    };

    // Construct every app up front so navigation is just a
    // show/hide swap (no per-click constructor cost). Their
    // callbacks/state stay live for the lifetime of the kernel.
    let launcher = AppLauncher::new()
        .expect("AppLauncher::new() failed under installed Slint platform");
    let hateoas_window = hateoas::build_app()
        .expect("HateoasBrowser construction failed");
    let repl_app = repl::build_app()
        .expect("Repl construction failed");
    // Track VVV (#455 + #456): Doom is feature-gated. When the
    // feature is off, `doom_app` is never constructed and the
    // launcher's `open-doom` callback is never wired (so the Slint
    // side's `show-doom` stays false and the button row is omitted
    // from layout entirely — see `AppLauncher.slint`'s `if
    // root.show-doom` block).
    #[cfg(feature = "doom")]
    let mut doom_app = doom::build_app()
        .expect("Doom construction failed");
    #[cfg(feature = "doom")]
    launcher.set_show_doom(true);

    let nav: NavState = Rc::new(RefCell::new(Active::Launcher));

    // Wire the launcher's two open-* callbacks. Each one swaps the
    // visible Slint Window: hide the launcher, show the chosen app,
    // and update `nav` so the keyboard pump knows where to route Esc.
    {
        let nav = nav.clone();
        let launcher_weak = launcher.as_weak();
        let hateoas_weak = hateoas_window.as_weak();
        launcher.on_open_hateoas(move || {
            let Some(launcher) = launcher_weak.upgrade() else { return };
            let Some(hateoas) = hateoas_weak.upgrade() else { return };
            let _ = launcher.hide();
            let _ = hateoas.show();
            *nav.borrow_mut() = Active::Hateoas;
        });
    }
    {
        let nav = nav.clone();
        let launcher_weak = launcher.as_weak();
        let repl_weak = repl_app.window.as_weak();
        launcher.on_open_repl(move || {
            let Some(launcher) = launcher_weak.upgrade() else { return };
            let Some(repl_window) = repl_weak.upgrade() else { return };
            let _ = launcher.hide();
            let _ = repl_window.show();
            *nav.borrow_mut() = Active::Repl;
        });
    }
    // Track VVV #455: Doom open callback. Only registered when the
    // feature is on; otherwise the Slint side's button row stays
    // hidden (see `set_show_doom(true)` above) and the callback is
    // never invoked. The shape mirrors the Hateoas / Repl wiring.
    #[cfg(feature = "doom")]
    {
        let nav = nav.clone();
        let launcher_weak = launcher.as_weak();
        let doom_weak = doom_app.window.as_weak();
        launcher.on_open_doom(move || {
            let Some(launcher) = launcher_weak.upgrade() else { return };
            let Some(doom_window) = doom_weak.upgrade() else { return };
            let _ = launcher.hide();
            let _ = doom_window.show();
            *nav.borrow_mut() = Active::Doom;
        });
    }
    // Theme toggle is a passive forward — Theme.toggle-mode() has
    // already swapped the global mode inside the Slint handler. No
    // host-side persistence yet; the callback's existence keeps the
    // hook reachable for a future `ThemePref` cell wire-up.
    launcher.on_theme_toggled(|| {});

    // Track MMMM #490: register the kernel-resident SlintPump with
    // the toolkit_loop multiplexer. Qt + GTK pump registrations
    // happen inside their respective adapter `init()` calls
    // (qt_adapter::event_loop::init / gtk_adapter::event_loop::init);
    // those are wired in `entry_uefi.rs::kernel_run_uefi` post-
    // `system::init`, so by the time we reach the super-loop body the
    // pump registry holds 1 + (qt-adapter? 1 : 0) + (gtk-adapter? 1 : 0)
    // entries. This call is idempotent at the singleton level — if
    // some other init path already registered SlintPump it would just
    // append a second copy (which is observable as a benign extra
    // tick per frame; not currently a concern under the boot wiring).
    toolkit_loop::register_default_pumps();

    // Show the launcher first. Until this returns Ok, the
    // MinimalSoftwareWindow has no visible component and
    // `draw_if_needed` would no-op.
    launcher
        .show()
        .expect("AppLauncher::show() failed");

    // Super-loop. Per Slint's mcu.md (lines 200-245), this is the
    // canonical no-event-loop main loop: drain input, advance
    // animations, draw the dirty region, idle. The loop never exits
    // — UEFI has no orderly shutdown path beyond `firmware::reboot`
    // (which we don't reach), and `kernel_run_uefi` is `-> !`.
    loop {
        // 1. Drain the keyboard ring. When an app is active, intercept
        //    Esc for back-to-launcher; otherwise forward all keys to
        //    the active Slint window via the existing
        //    `drain_keyboard_into_slint_window` shape.
        let active_now = *nav.borrow();
        // Track MMMM #490: when any registered foreign-toolkit pump
        // owns keyboard focus, drain the ring through
        // `toolkit_loop::dispatch_key` rather than the
        // direct-to-Slint path below. On the foundation slice every
        // pump's `focused_widget` returns `None` (no Qt / GTK
        // widgets are loaded), so this short-circuits and falls
        // through to the existing per-arm direct dispatch — today's
        // behaviour is preserved exactly. Once a Qt or GTK widget
        // gains focus (post-#491 binding), the ring entries route
        // through the focused pump instead. Esc still flows through
        // the existing arm-specific intercept code below: a foreign
        // toolkit owning focus while an app arm is active is
        // unreachable today and a future concern for #491 to detail.
        if drain_keyboard_into_focused_toolkit_pump() {
            // All ring entries claimed by a foreign toolkit; skip
            // the existing direct-dispatch arm.
        } else {
        match active_now {
            Active::Launcher => {
                // Esc on the launcher is a no-op (we're already at
                // the root). Just forward everything to its window
                // — Slint will silently drop unhandled keys.
                drain_keyboard_into_slint_window(&launcher.window());
            }
            Active::Hateoas => {
                if drain_keyboard_with_esc_intercept(&hateoas_window.window()) {
                    let _ = hateoas_window.hide();
                    let _ = launcher.show();
                    *nav.borrow_mut() = Active::Launcher;
                }
            }
            Active::Repl => {
                if drain_keyboard_with_esc_intercept(&repl_app.window.window()) {
                    let _ = repl_app.window.hide();
                    let _ = launcher.show();
                    *nav.borrow_mut() = Active::Launcher;
                }
            }
            // Track VVV #455: when Doom is active the keystroke
            // path forks. The keyboard ring is single-consumer
            // (`arch::uefi::keyboard::read_keystroke` is `pop_front`,
            // no peek API), so the launcher can't both intercept
            // Esc AND let a Slint dispatch happen — only one
            // consumer wins each ring entry. We give Doom's own
            // drainer (`DoomApp::drain_keystrokes_intercept_esc`)
            // exclusive access to the ring while Active::Doom: it
            // pops every pending entry, drops Esc (returning
            // `true` to signal back-to-launcher), translates
            // every other key via `crate::doom::translate_decoded_key`,
            // and dispatches synthetic press/release pairs against
            // the guest's `reportKeyDown` / `reportKeyUp` exports
            // — same shape `crate::doom::pump_keys_into_guest`
            // uses on the standalone path, but with the Esc check
            // moved here so the launcher can route back. The
            // Doom Window itself receives no Slint key events; it
            // shows whatever the WASM guest renders, no Slint-
            // side input wiring needed (the Window's FocusScope
            // rejects any keystroke that somehow reaches it).
            #[cfg(feature = "doom")]
            Active::Doom => {
                if doom_app.drain_keystrokes_intercept_esc() {
                    let _ = doom_app.window.hide();
                    let _ = launcher.show();
                    *nav.borrow_mut() = Active::Launcher;
                }
            }
        }
        } // end of `else` from `drain_keyboard_into_focused_toolkit_pump`

        // 2. Slint-side timer + animation tick. Slint reads
        //    `arch::time::now_ms()` via `Platform::duration_since_start`
        //    so any animation duration encoded in the .slint files
        //    (e.g. Theme.motion-fast for Button hover transitions)
        //    advances against the kernel's PIT clock.
        slint::platform::update_timers_and_animations();

        // Track MMMM #490: cooperative event-loop pump for every
        // foreign toolkit registered with the multiplexer. Each pump
        // gets a 4ms budget per tick (Qt's
        // `QCoreApplication::processEvents(AllEvents, 4)`, GTK's
        // budget loop over `g_main_context_iteration(NULL, FALSE)`,
        // Slint's no-op observation tick — see `toolkit_loop::pump_all`
        // for the dispatch shape). On the foundation slice every Qt /
        // GTK pump body short-circuits because the loader returned
        // `LibraryNotFound`, so this is one tick-counter bump per
        // registered pump; the call still runs every frame so the
        // wiring is exercised + observable in boot diagnostics.
        //
        // Note: this is placed right after `update_timers_and_animations`
        // per the #490 spec ("between update_timers_and_animations and
        // the keyboard drain"). UUU's #431 super-loop drains the
        // keyboard FIRST (step 1 above), so the spec's intended
        // anchor point is just-after-`update_timers_and_animations`;
        // the keyboard drain has already run by then. Per-frame budget
        // = 4ms × pump_count (3 worst case = 12ms, comfortably under
        // the 16ms 60Hz frame budget).
        let _ = toolkit_loop::pump_all(4);

        // 3. Background work — drive smoltcp + the HTTP listener
        //    every frame. Mirrors GGG's REPL drainer hook (#365 /
        //    entry_uefi.rs L1240): `crate::net::poll()` early-returns
        //    when no socket woke up so this is cheap when idle.
        //    Without it, DHCPv4 leases would never advance and
        //    /api/* routes registered via `net::register_http` would
        //    silently sit in `Listen` forever.
        crate::net::poll();

        // 4. Repaint. `draw_if_needed` is a no-op when the active
        //    window's Slint state hasn't changed (Slint tracks dirty
        //    regions internally), so this is cheap when idle and
        //    bounded to the dirty rect when something updated.
        window.draw_if_needed(|renderer| {
            renderer.render_by_line(&mut backend);
        });

        // 5. Idle. `pause` hints the CPU we're busy-waiting,
        //    reducing power draw and SMT-sibling contention without
        //    blocking IRQs (the PIT IRQ 0 + keyboard IRQ 1 still
        //    fire on schedule). Same shape GGG's drainer used.
        unsafe { core::arch::asm!("pause", options(nomem, nostack)); }
    }
}

/// Track MMMM #490: drain the keyboard ring through the toolkit_loop
/// multiplexer when a foreign-toolkit pump owns keyboard focus.
///
/// Returns `true` if a foreign pump claimed the drain (caller should
/// skip the existing direct-dispatch arm); `false` otherwise (caller
/// falls through to the per-arm direct-dispatch path that was already
/// wired before this helper landed).
///
/// The check `toolkit_loop::dispatch_key` performs is "walk every
/// registered pump, route to the first one whose `focused_widget`
/// returns `Some`". On the foundation slice every Qt / GTK pump returns
/// `None` (no widgets are loaded), so this helper drains zero entries
/// and returns `false` unconditionally — today's behaviour is preserved
/// exactly.
///
/// We translate each `pc-keyboard::DecodedKey` into the kernel-neutral
/// `toolkit_loop::KeyEvent` shape before dispatch:
/// `DecodedKey::Unicode(c)` → `KeyEvent { codepoint: c, pressed: true }`
/// (matching the press-only semantics the existing slint_input drain
/// uses — `pc-keyboard` swallows release scancodes after using them
/// for modifier state). `DecodedKey::RawKey(_)` is dropped (same
/// behaviour as the existing `drain_keyboard_with_esc_intercept`).
///
/// Today's single-pump-with-focus call boundary: when `dispatch_key`
/// returns `false` for an entry (no pump owns focus), we PUT the entry
/// BACK in the ring? No — `read_keystroke` is `pop_front` with no
/// push-front, so once consumed an entry can't go back. The helper's
/// design therefore only consumes entries when a foreign pump WILL
/// claim them: the early-return `if toolkit_loop` check below skips
/// the drain entirely when no pump owns focus, leaving the ring
/// untouched for the per-arm direct-dispatch path to consume.
fn drain_keyboard_into_focused_toolkit_pump() -> bool {
    use pc_keyboard::DecodedKey;

    // Cheap pre-flight: ask the multiplexer whether any registered
    // pump owns focus. If not, leave the ring untouched so the
    // existing per-arm direct-dispatch arm consumes it as before.
    // On the foundation slice no Qt / GTK widget can hold focus
    // (their loaders return `LibraryNotFound` so no widget exists
    // to focus), so this returns `false` immediately — today's
    // direct-dispatch behaviour is preserved exactly.
    if !toolkit_loop::has_foreign_focus() {
        return false;
    }

    // A foreign pump owns focus — drain the entire ring through it.
    // Each ring entry becomes a `KeyEvent { pressed: true }` (the
    // existing slint_input drain uses press-only semantics because
    // `pc-keyboard` swallows release scancodes for modifier-state
    // tracking; we mirror the same shape here so the foreign
    // toolkit's event queue receives the same events Slint would
    // have).
    while let Some(decoded) = keyboard::read_keystroke() {
        match decoded {
            DecodedKey::Unicode(c) => {
                let ev = toolkit_loop::KeyEvent { codepoint: c, pressed: true };
                let _ = toolkit_loop::dispatch_key(ev);
            }
            DecodedKey::RawKey(_) => {
                // Drop. Same behaviour as the existing
                // `drain_keyboard_with_esc_intercept` — the foreign
                // toolkits don't have a kernel-neutral RawKey
                // mapping yet (#491 territory).
            }
        }
    }
    true
}

/// Variant of `drain_keyboard_into_slint_window` that intercepts the
/// Escape keystroke and reports back instead of forwarding it. Returns
/// `true` when Esc was seen (caller should hide the active app and
/// show the launcher); `false` otherwise.
///
/// Non-Esc Unicode keys are forwarded inline as `KeyPressed` +
/// `KeyReleased` pairs (mirrors the shape `slint_input.rs` uses for
/// each ring entry — every drain produces a press+release pair so
/// widgets that look at release don't get stuck holding). We can't
/// re-use `drain_keyboard_into_slint_window` after the Esc check
/// because the ring is FIFO with no push-front; the only way to
/// "filter" Esc is to pop one entry at a time. We can't import
/// `slint_input::decoded_key_to_text` either (it's private to that
/// module, and Track QQ owns slint_input.rs per the ownership map).
///
/// `RawKey` entries (Arrow keys, Function row, modifiers, navigation
/// cluster) are dropped silently here. The REPL app's history walk
/// uses Up/Down which the pc-keyboard US-104 layout decodes to RawKey
/// — so history navigation is broken when the REPL is launched
/// through the launcher (it works fine when the REPL window receives
/// keystrokes directly through `drain_keyboard_into_slint_window`,
/// which has the full RawKey table). Documented limitation; widening
/// this helper would duplicate ~50 lines of mapping code from
/// slint_input.rs. A follow-up that adds a peek API to
/// arch::uefi::keyboard (so we can check for Esc without consuming
/// the entry) lets this helper collapse to a one-line wrapper around
/// `drain_keyboard_into_slint_window`.
fn drain_keyboard_with_esc_intercept(window: &slint::Window) -> bool {
    use pc_keyboard::DecodedKey;
    use slint::SharedString;
    use slint::platform::WindowEvent;

    let mut esc_seen = false;
    while let Some(decoded) = keyboard::read_keystroke() {
        match decoded {
            // Esc is the back-to-launcher signal. Stop dispatching
            // further keys this frame so a burst of input doesn't
            // leak into the about-to-be-hidden window, but keep
            // draining the ring so leftover entries don't queue up
            // for the launcher.
            DecodedKey::Unicode('\u{001b}') => {
                esc_seen = true;
            }
            DecodedKey::Unicode(c) => {
                if !esc_seen {
                    let text: SharedString = c.into();
                    let _ = window.try_dispatch_event(
                        WindowEvent::KeyPressed { text: text.clone() },
                    );
                    let _ = window.try_dispatch_event(
                        WindowEvent::KeyReleased { text },
                    );
                }
            }
            DecodedKey::RawKey(_) => {
                // Drop. See doc comment for the "RawKey forwarding
                // requires the full mapping table from
                // slint_input.rs" rationale.
            }
        }
    }
    esc_seen
}

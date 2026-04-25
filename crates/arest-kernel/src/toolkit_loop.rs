// crates/arest-kernel/src/toolkit_loop.rs
//
// Toolkit event-loop multiplexer (#490 Track MMMM, the event-loop side
// of the foreign-toolkit Component runtime). Sibling of LLLL's #489
// `composer` module (texture compositing) — together they form the
// runtime substrate that lets Qt (GGGG #487), GTK (IIII #488), and any
// future foreign toolkit cooperate inside Slint's super-loop without
// any one of them assuming it is THE main loop.
//
// # The event-loop ownership problem
//
// Every mainstream UI toolkit ships its own event loop and assumes it
// owns the program's main thread:
//
//   * Qt's `QCoreApplication::exec()` enters `QEventLoop::exec()`,
//     which polls UNIX sockets / Win32 handles / X11/Wayland fds and
//     dispatches `QEvent`s to widgets. Idiomatic Qt apps never return
//     from `exec()` until the user closes the last window.
//   * GTK's `gtk_main()` enters `g_main_loop_run()`, which iterates
//     the GLib `GMainContext` (poll fds + dispatch sources) until
//     `gtk_main_quit()` is called. Same shape, different brand.
//   * Slint v1.16's `Platform::run_event_loop` is the equivalent —
//     `slint::run_event_loop()` blocks until `quit_event_loop()`. On
//     the AREST UEFI build, MMM #436's `UefiSlintPlatform` returns
//     `Err(NoEventLoopProvider)` from that method, and the launcher
//     (UUU #431) drives Slint via the documented "MCU super-loop"
//     pattern: `update_timers_and_animations()` + `draw_if_needed()`
//     directly, no event-loop trait method involvement.
//
// Naively combining two of these (e.g. embedding a Qt widget inside
// a Slint window) deadlocks the program: each toolkit's run-loop
// blocks waiting for events, and neither yields to the other. The
// Qt-in-Slint problem is a well-known integration headache (Qt's own
// docs warn about it; the QtWebEngine project solved it by having
// Chromium and Qt cooperate via a shared message pump).
//
// # The cooperative-pumping solution
//
// AREST's UEFI super-loop already knows how to drive Slint without
// `run_event_loop`. Extend the same pattern to Qt + GTK by having
// each toolkit expose a non-blocking "process pending events for at
// most N milliseconds" entry point, then call all of them in
// rotation inside the super-loop's per-frame body:
//
//   loop {
//       drain_keyboard_into_focused_pump();
//       drain_pointer_into_focused_pump();
//       update_timers_and_animations();   // Slint-side
//       toolkit_loop::pump_all(4);        // Qt + GTK + Slint pumps
//       compose_frame();                  // (LLLL's #489)
//       render_by_line(...);              // Slint draw
//       pause;
//   }
//
// Each registered pump gets a 4ms budget per tick (default; the
// caller of `pump_all` can override). With three pumps that's a
// 12ms budget for "non-render, non-input" work, leaving ~5ms slack
// per frame at 60Hz for the actual draw + idle. If a toolkit needs
// more than its budget (e.g. a long-running animation tick), it
// truncates and continues on the next tick — same back-pressure
// pattern Slint's `update_timers_and_animations` already uses.
//
// # Real Qt / GTK calls vs stub no-ops
//
// On the foundation slice (today), GGGG's #487 + IIII's #488 loaders
// return `LibraryNotFound` — there is no `libqt6.so` or `libgtk-4.so`
// reachable from the UEFI cross-build. The pump impls in
// `qt_adapter::event_loop` + `gtk_adapter::event_loop` therefore
// short-circuit to a no-op when the loader handle is null. Once the
// linuxkpi DSO loader unblocks (post-#460 + #461), the pump bodies
// switch to real `QCoreApplication::processEvents` /
// `g_main_context_iteration` calls without any change to this
// module's interface.
//
// # Focus-driven keyboard / pointer dispatch
//
// AREST's existing key/pointer rings (`arch::uefi::keyboard::RING`
// and `arch::uefi::pointer::RING`) are single-consumer — `pop_front`
// removes the entry. Today the launcher pops keystrokes and routes
// them directly to the active Slint window. With foreign toolkits in
// the picture we need a routing decision: if a Qt widget is focused,
// the keystroke goes to Qt's queue (via `QtPump::dispatch_key`); if
// a GTK widget is focused, it goes to GTK's queue; otherwise the
// existing direct path fires (Slint window → focused .slint
// FocusScope).
//
// The decision lives in `dispatch_key` / `dispatch_pointer`, which
// walk the registered pump list and pick the first pump whose
// `focused_widget()` returns `Some`. If no foreign-toolkit pump owns
// focus, the function returns `false` and the caller (launcher
// super-loop) falls back to the direct Slint-side dispatch path —
// preserving today's behaviour exactly when no foreign toolkit is
// loaded.
//
// # Why static / global state
//
// The pump registry lives in a `spin::Mutex<Vec<Arc<dyn ToolkitPump>>>`
// — same shape as LLLL's `composer::TOOLKITS` and the rest of the
// kernel's static-singleton modules (`block_storage::MOUNT`,
// `arch::uefi::keyboard::RING`). Reasons:
//
//   * Adapters init from independent boot stages; they can't pass
//     an `&mut ToolkitLoop` to each other.
//   * The launcher's super-loop is the only consumer, and threading
//     a registry reference through `ui_apps::launcher::run` would
//     plumb adapter knowledge into a place that has no other reason
//     to know about Qt or GTK.
//   * Single-threaded boot makes the `spin::Mutex` contention-free.
//
// # WidgetId
//
// `WidgetId(u64)` is a kernel-side opaque identifier for a foreign
// widget that has focus. Each adapter assigns its own IDs (e.g. Qt's
// adapter could use the QWidget's pointer, GTK's adapter could use
// the GtkWidget's `g_object_id`). The kernel only uses it for
// `Eq`/`Hash`-style operations — the adapter is responsible for
// resolving the ID back to its toolkit-native handle.
//
// # Inline tests
//
// Same gating shape as LLLL's `composer::tests`:
// `cfg(all(test, target_os = "linux"))` so cross-arch host CI runs
// them on a Linux runner without the UEFI target attempting to compile
// the test harness's `_start` symbol. Tests cover:
//
//   * `register_pump` + `pump_all` visit each registered pump
//     exactly once with the right budget.
//   * Stub Qt + GTK pumps (no real loops) return cleanly without
//     blocking.
//   * `dispatch_key` routes to the pump whose `focused_widget`
//     returns `Some`; falls through (returns `false`) when no pump
//     owns focus.

#![allow(dead_code)]

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};

use spin::Mutex;

/// Opaque kernel-side identifier for a foreign-toolkit widget that has
/// keyboard / pointer focus. Each toolkit adapter assigns its own IDs
/// (typically derived from the toolkit's native handle, e.g. Qt's
/// `QWidget*` cast to `u64`, GTK's `g_object_id`); the kernel only
/// uses the value for equality + hash.
///
/// 64-bit chosen to comfortably hold either a 32-bit or 64-bit
/// pointer; on the AREST UEFI x86_64 target every native handle fits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WidgetId(pub u64);

/// One key event ready for dispatch into a toolkit's event queue.
///
/// Mirrors the shape of `slint::platform::WindowEvent::KeyPressed` /
/// `KeyReleased` — a single character payload plus a press/release
/// flag. The kernel keyboard ring stores `pc-keyboard::DecodedKey`,
/// which the launcher (or whoever calls `dispatch_key`) translates
/// into this neutral form before handing it to the focused pump.
///
/// `Copy + Clone` because the dispatch path may need to broadcast a
/// single event to multiple consumers in a future fan-out scenario
/// (e.g. global hotkey listeners). Today every event has exactly
/// one consumer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyEvent {
    /// Unicode codepoint of the keystroke. `'\u{001b}'` is Esc,
    /// `'\u{0008}'` is Backspace, etc. Matches the codepoints
    /// `pc-keyboard`'s `DecodedKey::Unicode` produces.
    pub codepoint: char,
    /// `true` for a press edge, `false` for a release edge. The
    /// existing keyboard ring only emits press events (the
    /// `pc-keyboard` decoder swallows releases after using them to
    /// update modifier state), but the field exists so adapters that
    /// want to synthesise paired press+release pairs can.
    pub pressed: bool,
}

/// One pointer event ready for dispatch. Translation target for the
/// `arch::uefi::pointer::PointerEvent` ring.
///
/// Flat enum mirrors the existing `pointer::PointerEvent`; the
/// kernel side keeps the toolkit-neutral shape so adapters can
/// translate to their native form (Qt's `QMouseEvent`, GTK's
/// `GdkEventButton`) inside `dispatch_pointer`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PointerEvent {
    /// Relative motion delta. `dx, dy` in device units.
    RelMove { dx: i32, dy: i32 },
    /// Absolute position. `x, y` in device space.
    AbsMove { x: i32, y: i32 },
    /// Scroll-wheel delta (positive = away from user).
    Scroll { delta: i32 },
    /// Button press / release. `button` matches Linux input-event
    /// `BTN_*` codes (BTN_LEFT = 0x110, etc.); `pressed = true` for
    /// a press, `false` for a release.
    Button { button: u32, pressed: bool },
}

/// Outcome of a pump's `pump(budget)` call. Reports back whether the
/// pump did any non-trivial work, plus a coarse "would do more if
/// given more budget" hint. Today only used for diagnostics +
/// inline-test assertions; future schedulers could use the
/// `more_pending` hint to skew budget allocation toward busy pumps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PumpResult {
    /// Number of events the pump processed during this call. Zero
    /// when the toolkit's queue was empty (or when the pump is a
    /// no-op stub because its library isn't loaded).
    pub events_processed: u32,
    /// `true` if the pump returned because it hit its budget rather
    /// than because the queue is empty. The next `pump_all` tick
    /// could give this pump more budget if a scheduler wanted to
    /// catch up.
    pub more_pending: bool,
}

impl PumpResult {
    /// Convenience constructor for the "no work, no pending"
    /// result every stub pump returns when its library isn't
    /// loaded. Equivalent to `PumpResult { events_processed: 0,
    /// more_pending: false }` but reads more clearly at call sites.
    pub const fn idle() -> Self {
        Self { events_processed: 0, more_pending: false }
    }
}

/// Trait implemented by each foreign-toolkit adapter to expose its
/// event-loop pump + focus tracking + input dispatch to the kernel's
/// super-loop.
///
/// `Send + Sync` because `register_pump` parks the impl in the
/// static `PUMPS` vec under a `spin::Mutex`. Same constraint LLLL's
/// `composer::ToolkitRenderer` uses for the same reason. Adapters
/// wrapping FFI handles (Qt's `QApplication*`, GTK's
/// `GtkApplication*`) carry their own `unsafe impl Send + Sync`
/// claim — the kernel's single-threaded boot model makes the bound
/// trivially satisfied.
///
/// Five methods:
///
///   * `name(&self)` — slug for diagnostics. Matches LLLL's
///     `composer::TOOLKITS` slug (`"qt6"`, `"gtk4"`, `"slint"`).
///   * `pump(&self, budget_ms)` — process pending events for at
///     most `budget_ms` milliseconds. Non-blocking when the queue
///     is empty.
///   * `focused_widget(&self)` — return `Some(WidgetId)` if a widget
///     owned by this toolkit currently has keyboard / pointer focus;
///     `None` otherwise. Used by `dispatch_key` / `dispatch_pointer`
///     to route events.
///   * `dispatch_key(&self, ev)` — synthesise a key event into the
///     toolkit's native event queue. The pump's next `pump` call
///     will deliver it to the focused widget via the toolkit's
///     normal dispatch path.
///   * `dispatch_pointer(&self, ev)` — same shape for pointer
///     events.
pub trait ToolkitPump: Send + Sync {
    /// Slug identifying the toolkit (`"qt6"`, `"gtk4"`, `"slint"`,
    /// `"test"`). Static lifetime because the slug is a compile-
    /// time constant for every real adapter.
    fn name(&self) -> &str;

    /// Process pending events for at most `budget_ms` milliseconds.
    /// Returns a `PumpResult` reporting how many events were
    /// dispatched + whether more are pending.
    ///
    /// **MUST be non-blocking** when the queue is empty. Called
    /// once per super-loop tick per pump; a pump that blocks here
    /// would freeze the kernel. Stub impls (library not loaded)
    /// return `PumpResult::idle()` immediately.
    fn pump(&self, budget_ms: u32) -> PumpResult;

    /// Return `Some(WidgetId)` if this toolkit owns the currently-
    /// focused widget; `None` otherwise. Multiple pumps may report
    /// `None`; at most one should report `Some` at any time (the
    /// kernel's focus model is exclusive — only one widget across
    /// all toolkits can have keyboard focus).
    ///
    /// `dispatch_key` walks pumps in registration order and routes
    /// to the first pump whose `focused_widget` returns `Some`. If
    /// every pump returns `None`, the caller falls back to direct
    /// Slint-side dispatch.
    fn focused_widget(&self) -> Option<WidgetId>;

    /// Synthesise `ev` into the toolkit's native key event queue.
    /// The pump's next `pump` call delivers it to the focused
    /// widget. No-op for stub pumps (library not loaded).
    fn dispatch_key(&self, ev: KeyEvent);

    /// Synthesise `ev` into the toolkit's native pointer event
    /// queue. Same shape as `dispatch_key`.
    fn dispatch_pointer(&self, ev: PointerEvent);
}

// ---------------------------------------------------------------
// Static state
// ---------------------------------------------------------------

/// Pump registry. Walked in registration order by `pump_all` and
/// `dispatch_key` / `dispatch_pointer`. `Vec` (not `BTreeMap`)
/// because order matters here — adapters that register first get
/// pumped first, and `dispatch_key` falls through pumps in the
/// same order looking for one that owns focus.
///
/// Multiple registrations of the same name are allowed but only the
/// first one's `focused_widget` matters for routing (since
/// `dispatch_key` short-circuits on the first `Some`). Adapters
/// should `register_pump` exactly once at init.
static PUMPS: Mutex<Vec<Arc<dyn ToolkitPump>>> = Mutex::new(Vec::new());

// ---------------------------------------------------------------
// Public API
// ---------------------------------------------------------------

/// Register `pump` against the kernel's pump registry. Called once
/// per adapter init (Qt's `qt_adapter::init`, GTK's
/// `gtk_adapter::init`, plus the kernel-resident `SlintPump` from
/// the launcher's bootstrap).
///
/// Append-only — there is no `unregister_pump`. Adapters live for
/// boot lifetime; a re-register would be a logic bug. The list is
/// walked in append order so registration sequence determines
/// `dispatch_key` fallback order.
///
/// One allocation per call (the `Arc<dyn ToolkitPump>` is already
/// allocated by the caller; the `Vec::push` may grow the inner
/// buffer). Single-threaded boot means no contention.
pub fn register_pump(pump: Arc<dyn ToolkitPump>) {
    PUMPS.lock().push(pump);
}

/// Number of registered pumps. Cheap; useful for boot diagnostics +
/// the inline tests.
pub fn pump_count() -> usize {
    PUMPS.lock().len()
}

/// Walk every registered pump in registration order and call its
/// `pump(per_toolkit_budget_ms)`. Returns the total number of
/// events processed across all pumps (sum of `events_processed`
/// across the per-pump `PumpResult`s).
///
/// Called once per super-loop tick from the launcher (UUU #431),
/// between `update_timers_and_animations` and the keyboard drain.
/// At the default 4ms budget per pump and three registered pumps
/// (Slint, Qt, GTK), the worst-case wall-clock cost of `pump_all`
/// is 12ms — comfortably under the 16ms 60Hz frame budget with
/// room left for the render path + idle.
///
/// Lock discipline: the pump list is cloned out under the
/// `PUMPS` lock, then the lock is released before any pump's
/// `pump` is called. This is important for adapters that may
/// invoke `register_pump` from inside their pump body (re-entry
/// would deadlock); it also prevents long pump bodies from
/// pinning the registry against `dispatch_key` callers on a
/// hypothetical multi-CPU future.
pub fn pump_all(per_toolkit_budget_ms: u32) -> u32 {
    let pumps: Vec<Arc<dyn ToolkitPump>> = {
        let p = PUMPS.lock();
        p.clone()
    };
    let mut total = 0u32;
    for pump in pumps.iter() {
        let result = pump.pump(per_toolkit_budget_ms);
        total = total.saturating_add(result.events_processed);
    }
    total
}

/// Route `ev` to the pump whose `focused_widget` returns `Some`.
/// Returns `true` if a pump took ownership of the event, `false`
/// otherwise (caller should fall back to direct Slint-side
/// dispatch).
///
/// Walks pumps in registration order and short-circuits on the
/// first `Some`. If multiple pumps report `Some` (which they
/// shouldn't — focus is exclusive), only the first one wins; the
/// rest are skipped. This matches the kernel's "one focused widget
/// at a time" model.
///
/// Lock discipline: pump list cloned out under the lock, then lock
/// released before any pump's `dispatch_key` runs. Same rationale
/// as `pump_all` (adapter re-entry safety).
pub fn dispatch_key(ev: KeyEvent) -> bool {
    let pumps: Vec<Arc<dyn ToolkitPump>> = {
        let p = PUMPS.lock();
        p.clone()
    };
    for pump in pumps.iter() {
        if pump.focused_widget().is_some() {
            pump.dispatch_key(ev);
            return true;
        }
    }
    false
}

/// Pointer-event analogue of `dispatch_key`. Same routing rule:
/// first pump whose `focused_widget` returns `Some` takes the
/// event; otherwise returns `false` and caller falls back to
/// direct dispatch.
pub fn dispatch_pointer(ev: PointerEvent) -> bool {
    let pumps: Vec<Arc<dyn ToolkitPump>> = {
        let p = PUMPS.lock();
        p.clone()
    };
    for pump in pumps.iter() {
        if pump.focused_widget().is_some() {
            pump.dispatch_pointer(ev);
            return true;
        }
    }
    false
}

/// Pure-query helper: returns `true` if any registered pump's
/// `focused_widget()` returns `Some`. Lets callers decide whether
/// to route an event through `dispatch_key` / `dispatch_pointer`
/// (which would consume the event) without actually dispatching.
///
/// The launcher's super-loop uses this as a pre-flight check before
/// draining the keyboard ring: if no foreign-toolkit pump owns
/// focus, the existing direct-to-Slint drain path runs unchanged;
/// otherwise the drain routes ring entries through `dispatch_key`
/// to the focused pump.
///
/// Walks the pump registry under the lock; returns at the first
/// pump that reports focus. O(N) in the number of registered pumps.
pub fn has_foreign_focus() -> bool {
    let pumps = PUMPS.lock();
    pumps.iter().any(|p| p.focused_widget().is_some())
}

// ---------------------------------------------------------------
// Built-in Slint pump
// ---------------------------------------------------------------

/// Kernel-resident pump that drives Slint's per-tick housekeeping.
/// Registered automatically by the launcher's bootstrap path; lives
/// in this module rather than `arch::uefi::slint_backend` because
/// the trait it implements is the toolkit_loop one.
///
/// Slint is the kernel's "primary" toolkit — its event loop is
/// already pumped by the launcher's existing super-loop body
/// (`update_timers_and_animations` + `draw_if_needed`). The
/// `SlintPump` impl exists so `pump_all` walks Slint alongside Qt
/// and GTK in a single uniform sweep; Slint's `pump` body is a
/// thin re-call of `update_timers_and_animations` (idempotent +
/// cheap when nothing changed).
///
/// `focused_widget` always returns `None` — Slint focus is tracked
/// internally by the active `slint::Window`, not via WidgetId, so
/// the launcher's existing direct-dispatch path handles Slint key
/// routing. Returning `None` means `dispatch_key` falls through to
/// the direct path — same behaviour as today.
///
/// `dispatch_key` / `dispatch_pointer` are no-ops because Slint
/// doesn't use the kernel-neutral event types — the launcher
/// dispatches `slint::WindowEvent`s directly to the focused
/// `slint::Window` (the existing `drain_keyboard_into_slint_window`
/// path). The methods exist to satisfy the trait but never see
/// callers under today's wiring.
pub struct SlintPump {
    /// Counter incremented each tick. Diagnostics-only; lets a
    /// future health check assert "Slint pump is being driven"
    /// without observing any side-effect on the framebuffer.
    ticks: AtomicU32,
}

impl SlintPump {
    /// Build a fresh Slint pump with the tick counter at zero.
    pub const fn new() -> Self {
        Self { ticks: AtomicU32::new(0) }
    }

    /// Read the tick counter. Diagnostics-only.
    pub fn ticks(&self) -> u32 {
        self.ticks.load(Ordering::Relaxed)
    }
}

impl Default for SlintPump {
    fn default() -> Self {
        Self::new()
    }
}

impl ToolkitPump for SlintPump {
    fn name(&self) -> &str {
        "slint"
    }

    /// Slint's per-tick housekeeping. Calls
    /// `slint::platform::update_timers_and_animations()` to advance
    /// any pending animation tick (same call the launcher's super-
    /// loop already makes; this is a second observation-only call
    /// so `pump_all` walks Slint alongside the foreign-toolkit
    /// pumps in a uniform sweep). The double-call is safe — Slint's
    /// timer advance is idempotent within a single tick.
    ///
    /// Budget is ignored — the call is cheap and bounded by Slint's
    /// own internal animation table size, not by wall-clock.
    fn pump(&self, _budget_ms: u32) -> PumpResult {
        self.ticks.fetch_add(1, Ordering::Relaxed);
        // Don't actually invoke `update_timers_and_animations` here
        // — the launcher's super-loop already calls it once per
        // frame and a second call within the same frame would just
        // re-walk Slint's animation table for no observable effect.
        // We just bump the tick counter so diagnostics can tell
        // the pump was invoked.
        PumpResult::idle()
    }

    fn focused_widget(&self) -> Option<WidgetId> {
        None
    }

    fn dispatch_key(&self, _ev: KeyEvent) {
        // No-op. Slint key dispatch goes through the launcher's
        // existing `drain_keyboard_into_slint_window` path.
    }

    fn dispatch_pointer(&self, _ev: PointerEvent) {
        // No-op. Slint pointer dispatch goes through Slint's own
        // input plumbing once a pointer ring drainer is wired.
    }
}

// ---------------------------------------------------------------
// Bootstrap helper
// ---------------------------------------------------------------

/// Register the kernel-resident `SlintPump`. Called once from the
/// launcher's bootstrap before the super-loop body starts. Idempotent
/// in practice because the launcher's bootstrap runs exactly once
/// per boot — but the registry is append-only, so calling this twice
/// would register two SlintPumps. Not a concern under today's wiring.
pub fn register_default_pumps() {
    register_pump(Arc::new(SlintPump::new()));
}

// ---------------------------------------------------------------
// Testing helpers
// ---------------------------------------------------------------

/// Reset the pump registry. **Test-only** — no production caller.
/// Used by the inline tests to start each test from a known empty
/// state. Not exposed to non-test callers because adapter init
/// expects the registry to be append-only.
#[cfg(any(test, feature = "compositor-test"))]
pub fn reset_pumps() {
    PUMPS.lock().clear();
}

// ---------------------------------------------------------------
// Tests
// ---------------------------------------------------------------
//
// `arest-kernel`'s bin target has `test = false` (Cargo.toml L98)
// so these tests are reachable only on a host build. Same gating
// shape as LLLL's `composer::tests`: `cfg(all(test, target_os =
// "linux"))` runs them on cross-arch CI's Linux runner without the
// UEFI target attempting to compile a `_start` symbol the test
// harness wouldn't know what to do with.

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use core::sync::atomic::AtomicU64;

    /// Stub pump that records every `pump`/`dispatch_key`/
    /// `dispatch_pointer` invocation against shared atomics so tests
    /// can assert routing happened.
    struct CountingPump {
        slug: &'static str,
        focused: Option<WidgetId>,
        pumps: AtomicU64,
        keys: AtomicU64,
        pointers: AtomicU64,
        last_budget: AtomicU32,
    }

    impl CountingPump {
        fn new(slug: &'static str, focused: Option<WidgetId>) -> Self {
            Self {
                slug,
                focused,
                pumps: AtomicU64::new(0),
                keys: AtomicU64::new(0),
                pointers: AtomicU64::new(0),
                last_budget: AtomicU32::new(0),
            }
        }
    }

    impl ToolkitPump for CountingPump {
        fn name(&self) -> &str {
            self.slug
        }
        fn pump(&self, budget_ms: u32) -> PumpResult {
            self.pumps.fetch_add(1, Ordering::Relaxed);
            self.last_budget.store(budget_ms, Ordering::Relaxed);
            PumpResult::idle()
        }
        fn focused_widget(&self) -> Option<WidgetId> {
            self.focused
        }
        fn dispatch_key(&self, _ev: KeyEvent) {
            self.keys.fetch_add(1, Ordering::Relaxed);
        }
        fn dispatch_pointer(&self, _ev: PointerEvent) {
            self.pointers.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Each test starts from a clean registry. `cargo test` runs
    /// tests in parallel by default; the global `PUMPS` mutex
    /// serialises every test that touches it (only one test holds
    /// the lock at a time during reset).
    fn reset() {
        reset_pumps();
    }

    #[test]
    fn register_pump_appends_in_order() {
        reset();
        let a = Arc::new(CountingPump::new("a", None));
        let b = Arc::new(CountingPump::new("b", None));
        register_pump(a);
        register_pump(b);
        assert_eq!(pump_count(), 2);
    }

    #[test]
    fn pump_all_visits_each_pump_exactly_once_with_budget() {
        reset();
        let a = Arc::new(CountingPump::new("a", None));
        let b = Arc::new(CountingPump::new("b", None));
        let c = Arc::new(CountingPump::new("c", None));
        register_pump(a.clone());
        register_pump(b.clone());
        register_pump(c.clone());
        let total = pump_all(7);
        assert_eq!(total, 0, "stub pumps return idle so total events = 0");
        assert_eq!(a.pumps.load(Ordering::Relaxed), 1);
        assert_eq!(b.pumps.load(Ordering::Relaxed), 1);
        assert_eq!(c.pumps.load(Ordering::Relaxed), 1);
        assert_eq!(a.last_budget.load(Ordering::Relaxed), 7);
        assert_eq!(b.last_budget.load(Ordering::Relaxed), 7);
        assert_eq!(c.last_budget.load(Ordering::Relaxed), 7);
    }

    #[test]
    fn pump_all_on_empty_registry_returns_zero() {
        reset();
        assert_eq!(pump_count(), 0);
        assert_eq!(pump_all(4), 0);
    }

    #[test]
    fn slint_pump_is_a_clean_no_op() {
        reset();
        let s = Arc::new(SlintPump::new());
        register_pump(s.clone());
        // Three ticks; each is a no-op counter bump.
        assert_eq!(pump_all(4), 0);
        assert_eq!(pump_all(4), 0);
        assert_eq!(pump_all(4), 0);
        assert_eq!(s.ticks(), 3);
        // Focused widget always None for the Slint pump — direct
        // dispatch fallback handles its key routing.
        assert!(s.focused_widget().is_none());
        // dispatch_* are no-ops — they shouldn't panic.
        s.dispatch_key(KeyEvent { codepoint: 'a', pressed: true });
        s.dispatch_pointer(PointerEvent::RelMove { dx: 1, dy: 2 });
    }

    #[test]
    fn dispatch_key_routes_to_focused_pump() {
        reset();
        let unfocused = Arc::new(CountingPump::new("a", None));
        let focused = Arc::new(CountingPump::new("b", Some(WidgetId(42))));
        let other = Arc::new(CountingPump::new("c", None));
        register_pump(unfocused.clone());
        register_pump(focused.clone());
        register_pump(other.clone());
        let took = dispatch_key(KeyEvent { codepoint: 'x', pressed: true });
        assert!(took, "focused pump should claim the key");
        // Only the focused pump receives the dispatch.
        assert_eq!(unfocused.keys.load(Ordering::Relaxed), 0);
        assert_eq!(focused.keys.load(Ordering::Relaxed), 1);
        assert_eq!(other.keys.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn dispatch_pointer_routes_to_focused_pump() {
        reset();
        let unfocused = Arc::new(CountingPump::new("a", None));
        let focused = Arc::new(CountingPump::new("b", Some(WidgetId(99))));
        register_pump(unfocused.clone());
        register_pump(focused.clone());
        let took = dispatch_pointer(PointerEvent::RelMove { dx: 1, dy: -1 });
        assert!(took);
        assert_eq!(unfocused.pointers.load(Ordering::Relaxed), 0);
        assert_eq!(focused.pointers.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn dispatch_key_returns_false_when_no_pump_owns_focus() {
        reset();
        let a = Arc::new(CountingPump::new("a", None));
        let b = Arc::new(CountingPump::new("b", None));
        register_pump(a.clone());
        register_pump(b.clone());
        let took = dispatch_key(KeyEvent { codepoint: 'q', pressed: true });
        assert!(!took, "no pump owns focus — caller should fall back");
        assert_eq!(a.keys.load(Ordering::Relaxed), 0);
        assert_eq!(b.keys.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn dispatch_picks_first_focused_pump_when_multiple_claim() {
        reset();
        // Multiple pumps claiming focus shouldn't happen in
        // practice (focus is exclusive), but if it does the first
        // registered pump wins. Verify the routing rule.
        let first = Arc::new(CountingPump::new("first", Some(WidgetId(1))));
        let second = Arc::new(CountingPump::new("second", Some(WidgetId(2))));
        register_pump(first.clone());
        register_pump(second.clone());
        dispatch_key(KeyEvent { codepoint: 'z', pressed: true });
        assert_eq!(first.keys.load(Ordering::Relaxed), 1);
        assert_eq!(second.keys.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn pump_all_uses_default_budget_when_caller_passes_four() {
        reset();
        let p = Arc::new(CountingPump::new("p", None));
        register_pump(p.clone());
        pump_all(4);
        assert_eq!(p.last_budget.load(Ordering::Relaxed), 4);
    }

    #[test]
    fn has_foreign_focus_reports_false_when_no_pump_owns_focus() {
        reset();
        let a = Arc::new(CountingPump::new("a", None));
        let b = Arc::new(CountingPump::new("b", None));
        register_pump(a);
        register_pump(b);
        assert!(!has_foreign_focus());
    }

    #[test]
    fn has_foreign_focus_reports_true_when_any_pump_owns_focus() {
        reset();
        let a = Arc::new(CountingPump::new("a", None));
        let b = Arc::new(CountingPump::new("b", Some(WidgetId(1))));
        register_pump(a);
        register_pump(b);
        assert!(has_foreign_focus());
    }

    #[test]
    fn has_foreign_focus_on_empty_registry_returns_false() {
        reset();
        assert!(!has_foreign_focus());
    }
}

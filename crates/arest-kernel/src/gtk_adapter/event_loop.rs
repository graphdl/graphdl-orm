// crates/arest-kernel/src/gtk_adapter/event_loop.rs
//
// GTK event-loop pump — `GtkPump` impl of the kernel's
// `crate::toolkit_loop::ToolkitPump` trait (#490 Track MMMM).
//
// Sibling of GGGG's #487 `qt_adapter::event_loop` and LLLL's #489
// `composer` integration: where the composer owns the texture
// round-trip (GTK renders into a `ForeignSurface` → Slint composites
// it as `Image`), this module owns the GTK-side event-loop drain
// — `g_main_context_iteration(NULL, FALSE)` per super-loop tick when
// libgtk-4 is loaded, no-op when it isn't.
//
// # Why a stub today
//
// IIII's #488 `gtk_adapter::loader` returns `LibraryNotFound`
// unconditionally on the foundation slice — there's no
// `libgtk-4.so.1` reachable from the UEFI cross-build (linuxkpi
// #460 driver-mode focus; library-mode dlopen lands post-#460+#461).
// With no library loaded, the adapter has no `GMainContext*` to
// iterate, so `pump` returns idle immediately.
//
// Once the loader unblocks, the body of `pump` swaps from the stub
// fast-return to:
//
//   1. Look up the default `GMainContext` via `g_main_context_default()`
//      (or hold a cached pointer from `gtk_adapter::init`).
//   2. Loop: call `g_main_context_iteration(ctx, FALSE)` — non-
//      blocking iteration that processes one pending source +
//      returns `TRUE` if work was done, `FALSE` if the queue is
//      empty. Continue until budget exhausted OR `FALSE` returned.
//   3. Return a `PumpResult` with `events_processed = iteration_count`
//      and `more_pending = (last iteration returned TRUE)`.
//
// The dispatch_key / dispatch_pointer methods follow the same
// pattern: stub today, real `gdk_event_put` / `g_signal_emit`
// calls once the library is loaded.
//
// # GMainContext vs QEventLoop semantics
//
// Qt's `processEvents(maxtime)` takes a wall-clock budget directly;
// GTK's `g_main_context_iteration` takes a `gboolean may_block`
// parameter and processes ONE pending source per call. To honour
// the kernel's millisecond budget, we'd loop:
//
//   let start = arch::time::now_ms();
//   let mut count = 0u32;
//   loop {
//       let did_work = g_main_context_iteration(ctx, FALSE);
//       if did_work == 0 { break; }
//       count += 1;
//       if arch::time::now_ms() - start >= budget_ms as u64 { break; }
//   }
//
// For the stub today this all collapses to "increment the tick
// counter and return" — but the loop shape is documented in the
// future-body comment so the swap-in is mechanical.
//
// # Why a single global GtkPump rather than per-window
//
// GTK's `GMainContext` is process-singleton (well — there's a
// notion of per-thread default contexts, but for a single-threaded
// kernel the default context is THE context). One `GtkPump` drives
// the whole GTK subsystem; per-window state (focus tracking) lives
// behind the one pump's `focused_widget` accessor, mirroring the
// QtPump shape.
//
// # Focus tracking
//
// `FOCUSED_WIDGET: spin::Mutex<Option<WidgetId>>` holds the ID of
// whichever GTK widget currently has keyboard focus, or `None` when
// no GTK widget is focused (always today). Future GTK-side wiring:
// `gtk_widget_grab_focus` callers + the `focus-in-event` signal
// handler call `set_focused(Some(WidgetId(<gtk_widget_get_name>'s
// hash, or the GObject pointer cast to u64)))`.

#![allow(dead_code)]

use alloc::sync::Arc;
use spin::{Mutex, Once};

use crate::toolkit_loop::{
    self, KeyEvent, PointerEvent, PumpResult, ToolkitPump, WidgetId,
};

/// Currently-focused GTK widget. `None` when no GTK widget owns
/// keyboard focus (always the case today; the stub pump's
/// `focused_widget` reads from this cell so the kernel's
/// `dispatch_key` short-circuits to direct Slint dispatch).
///
/// `spin::Mutex<Option<WidgetId>>` rather than `AtomicU64` so the
/// `None` state is representable without a sentinel value.
static FOCUSED_WIDGET: Mutex<Option<WidgetId>> = Mutex::new(None);

/// The GTK pump singleton. `Once`-guarded so a second `init` call
/// is a no-op.
static GTK_PUMP: Once<Arc<GtkPump>> = Once::new();

/// GTK event-loop pump implementing `ToolkitPump`. Single instance
/// per kernel boot — `GMainContext` is a process-singleton, so one
/// pump suffices.
///
/// Today's body is a stub: `pump` returns `PumpResult::idle()`,
/// `dispatch_key` / `dispatch_pointer` are no-ops, `focused_widget`
/// reads from `FOCUSED_WIDGET` (always `None` until GTK is loaded
/// and a GTK widget receives focus). Once IIII's
/// `gtk_adapter::loader` returns a real `LibHandle::Loaded`, the
/// body swaps to real GLib API calls without changing the trait
/// surface.
pub struct GtkPump {
    /// Diagnostic counter — bumped each `pump` call. Lets the
    /// inline test assert "pump was driven" without observing any
    /// FFI side effect.
    ticks: spin::Mutex<u64>,
}

impl GtkPump {
    /// Build a fresh GtkPump with the tick counter at zero. The
    /// public constructor is `init`; this exists for the inline
    /// tests.
    pub const fn new() -> Self {
        Self { ticks: Mutex::new(0) }
    }

    /// Read the tick counter. Diagnostics-only.
    pub fn ticks(&self) -> u64 {
        *self.ticks.lock()
    }
}

impl Default for GtkPump {
    fn default() -> Self {
        Self::new()
    }
}

impl ToolkitPump for GtkPump {
    fn name(&self) -> &str {
        "gtk4"
    }

    /// Process pending GTK events for at most `budget_ms`
    /// milliseconds. Stub today (returns idle immediately); future
    /// body loops over `g_main_context_iteration(ctx, FALSE)` until
    /// budget exhausted OR queue drained.
    ///
    /// Budget semantics: GTK's `g_main_context_iteration` doesn't
    /// take a budget directly (it's a "process one source" call
    /// that returns `TRUE`/`FALSE`); we wrap it in a `now_ms`-bounded
    /// loop. See the module docstring for the future-body shape.
    fn pump(&self, _budget_ms: u32) -> PumpResult {
        *self.ticks.lock() += 1;
        // Stub — when libgtk-4 is loaded the body swaps to:
        //
        //   let lib = gtk_adapter::loader::glib();
        //   if lib.base.is_null() { return PumpResult::idle(); }
        //   let ctx_default: extern "C" fn() -> *mut GMainContext
        //       = gtk_adapter::loader::dlsym(&lib,
        //           "g_main_context_default").cast();
        //   let iterate: extern "C" fn(*mut GMainContext, i32) -> i32
        //       = gtk_adapter::loader::dlsym(&lib,
        //           "g_main_context_iteration").cast();
        //   let ctx = ctx_default();
        //   let start = crate::arch::time::now_ms();
        //   let mut count = 0u32;
        //   let mut last_did_work = 0;
        //   loop {
        //       last_did_work = iterate(ctx, 0); // FALSE = non-blocking
        //       if last_did_work == 0 { break; }
        //       count += 1;
        //       if crate::arch::time::now_ms() - start
        //           >= _budget_ms as u64 { break; }
        //   }
        //   PumpResult {
        //       events_processed: count,
        //       more_pending: last_did_work != 0,
        //   }
        //
        // Today the loader returns LibraryNotFound, so we
        // short-circuit. The adapter still gets pumped each tick so
        // `register_pump` is observable in `pump_count()`, but the
        // body is a one-instruction tick counter bump.
        PumpResult::idle()
    }

    /// Currently-focused GTK widget. Always `None` today (no GTK
    /// widgets exist on the foundation slice). When GTK is loaded
    /// and the adapter wires its `focus-in-event` handler, that
    /// handler will `set_focused(Some(WidgetId(<gtk widget ptr>)))`;
    /// this method then returns the current value.
    fn focused_widget(&self) -> Option<WidgetId> {
        *FOCUSED_WIDGET.lock()
    }

    /// Synthesise `ev` into GTK's event queue. Stub today (no-op);
    /// future body posts a `GdkEventKey` via `gdk_display_put_event`
    /// (or for synthetic keystrokes, calls
    /// `gtk_main_do_event(event)` directly).
    fn dispatch_key(&self, _ev: KeyEvent) {
        // Stub — when libgtk is loaded the body swaps to:
        //
        //   let target = current_focus_gtkwidget();
        //   if target.is_null() { return; }
        //   let event = gdk_event_new(GDK_KEY_PRESS);
        //   set_event_keyval(event, ev.codepoint as u32);
        //   set_event_window(event, gtk_widget_get_window(target));
        //   gtk_main_do_event(event);
        //   gdk_event_free(event);
    }

    /// Synthesise `ev` into GTK's event queue. Same shape as
    /// `dispatch_key`. Stub today.
    fn dispatch_pointer(&self, _ev: PointerEvent) {
        // Stub — future body posts a GdkEventMotion / GdkEventButton
        // / GdkEventScroll via gtk_main_do_event.
    }
}

/// Set the currently-focused GTK widget (or clear it with `None`).
/// Called from the GTK-side `focus-in-event` / `focus-out-event`
/// signal handlers (future wiring once GTK is loaded).
///
/// Public so tests + future GTK-side integration can both call it.
pub fn set_focused(widget: Option<WidgetId>) {
    *FOCUSED_WIDGET.lock() = widget;
}

/// Read the currently-focused GTK widget without going through the
/// pump trait. Convenience for diagnostics.
pub fn focused() -> Option<WidgetId> {
    *FOCUSED_WIDGET.lock()
}

/// Initialise the GTK pump singleton and register it with the
/// kernel's toolkit_loop. Called once from `gtk_adapter::init`
/// post-loader bring-up.
///
/// Idempotent at the singleton level — `Once::call_once` short-
/// circuits a second call. The `register_pump` step happens inside
/// the `call_once` closure so re-init doesn't double-register.
///
/// On the foundation slice this still runs even though the loader
/// returned `LibraryNotFound`: the stub pump produces no observable
/// behaviour but the registration makes the kernel's `pump_all`
/// walk GTK alongside Slint and Qt in a uniform sweep.
pub fn init() {
    GTK_PUMP.call_once(|| {
        let pump = Arc::new(GtkPump::new());
        toolkit_loop::register_pump(pump.clone());
        pump
    });
}

/// Read-side accessor for the GtkPump singleton. Returns `None` if
/// `init` hasn't run yet. Currently only used by the inline tests
/// to assert the singleton was registered.
pub fn pump() -> Option<Arc<GtkPump>> {
    GTK_PUMP.get().cloned()
}

// ---------------------------------------------------------------
// Tests
// ---------------------------------------------------------------

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn gtk_pump_returns_idle_without_loaded_library() {
        // Stub pump should never block, never panic, return idle.
        let pump = GtkPump::new();
        let result = pump.pump(4);
        assert_eq!(result, PumpResult::idle());
        // Multiple ticks accumulate the counter.
        let _ = pump.pump(4);
        let _ = pump.pump(4);
        assert_eq!(pump.ticks(), 3);
    }

    #[test]
    fn gtk_pump_focused_widget_starts_none() {
        set_focused(None);
        let pump = GtkPump::new();
        assert!(pump.focused_widget().is_none());
    }

    #[test]
    fn gtk_pump_focused_widget_reflects_set_focused() {
        set_focused(None);
        let pump = GtkPump::new();
        set_focused(Some(WidgetId(0xfeed_face)));
        assert_eq!(pump.focused_widget(), Some(WidgetId(0xfeed_face)));
        set_focused(None);
        assert!(pump.focused_widget().is_none());
    }

    #[test]
    fn gtk_pump_dispatch_key_is_a_clean_no_op() {
        let pump = GtkPump::new();
        // Should not panic, should not block.
        pump.dispatch_key(KeyEvent { codepoint: 'g', pressed: true });
        pump.dispatch_key(KeyEvent { codepoint: '\n', pressed: false });
    }

    #[test]
    fn gtk_pump_dispatch_pointer_is_a_clean_no_op() {
        let pump = GtkPump::new();
        pump.dispatch_pointer(PointerEvent::AbsMove { x: 100, y: 200 });
        pump.dispatch_pointer(PointerEvent::Button { button: 0x111, pressed: false });
        pump.dispatch_pointer(PointerEvent::Scroll { delta: -1 });
    }

    #[test]
    fn gtk_pump_name_is_gtk4() {
        let pump = GtkPump::new();
        assert_eq!(pump.name(), "gtk4");
    }
}

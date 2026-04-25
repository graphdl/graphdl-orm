// crates/arest-kernel/src/qt_adapter/event_loop.rs
//
// Qt event-loop pump — `QtPump` impl of the kernel's
// `crate::toolkit_loop::ToolkitPump` trait (#490 Track MMMM).
//
// Sibling of LLLL's #489 `composer` integration: where the composer
// owns the texture round-trip (Qt renders into a `ForeignSurface` →
// Slint composites it as `Image`), this module owns the Qt-side
// event-loop drain — `QCoreApplication::processEvents(AllEvents,
// budget)` per super-loop tick when libqt6 is loaded, no-op when it
// isn't.
//
// # Why a stub today
//
// GGGG's #487 `qt_adapter::loader` returns `LibraryNotFound`
// unconditionally on the foundation slice — there's no
// `libqt6core.so.6` reachable from the UEFI cross-build (the
// linuxkpi #460 foundation slice was driver-mode focused; library-
// mode dlopen lands in #460+ follow-ups + #461). With no library
// loaded, the adapter has no `QCoreApplication*` to call
// `processEvents` on, so `pump` returns idle immediately.
//
// Once the loader unblocks, the body of `pump` swaps from the stub
// fast-return to:
//
//   1. Look up `QCoreApplication::instance()` via dlsym (or hold a
//      cached pointer from `qt_adapter::init`).
//   2. Call `QCoreApplication::processEvents(QEventLoop::AllEvents,
//      budget_ms)` — Qt's documented "process pending events for
//      at most `budget_ms` ms" entry. Returns when either the queue
//      is empty or the budget is exhausted.
//   3. Translate the return into a `PumpResult` (events_processed
//      from a counter Qt internally maintains, more_pending from
//      `QCoreApplication::hasPendingEvents()`).
//
// The dispatch_key / dispatch_pointer methods follow the same
// pattern: stub today, real `QApplication::postEvent` /
// `QCoreApplication::sendEvent` calls once the library is loaded.
//
// # Why a single global QtPump rather than per-window
//
// Qt's event loop is process-singleton — `QCoreApplication` is a
// global that owns ALL events for ALL widgets. So one `QtPump`
// drives the whole Qt subsystem; per-window state (focus tracking)
// lives behind the one pump's `focused_widget` accessor, which
// reads from a static `FOCUSED_WIDGET` cell that the Qt-side
// signal handlers (eventual `QWidget::focusInEvent` /
// `focusOutEvent` overrides) update.
//
// # Focus tracking
//
// `FOCUSED_WIDGET: spin::Mutex<Option<WidgetId>>` holds the ID of
// whichever Qt widget currently has keyboard focus, or `None` when
// no Qt widget is focused (which is always the case today since no
// Qt widgets exist).
//
// Future Qt-side wiring: when `QWidget::focusInEvent` fires, the
// adapter calls `set_focused(Some(WidgetId(<qwidget ptr>)))`;
// when `focusOutEvent` fires (or the widget is destroyed), it
// calls `set_focused(None)`. The kernel's `dispatch_key` then
// routes to QtPump while focus is `Some`, and to direct Slint
// dispatch while it's `None`.

#![allow(dead_code)]

use alloc::sync::Arc;
use spin::{Mutex, Once};

use crate::toolkit_loop::{
    self, KeyEvent, PointerEvent, PumpResult, ToolkitPump, WidgetId,
};

/// Currently-focused Qt widget. `None` when no Qt widget owns
/// keyboard focus (always the case on the foundation slice; the
/// stub pump's `focused_widget` reads from this cell so the kernel's
/// `dispatch_key` short-circuits to direct Slint dispatch).
///
/// `spin::Mutex<Option<WidgetId>>` rather than `AtomicU64` so the
/// `None` state is representable without a sentinel value (which
/// would conflict with a hypothetical `WidgetId(0)`). Single-
/// threaded boot makes the lock contention-free.
static FOCUSED_WIDGET: Mutex<Option<WidgetId>> = Mutex::new(None);

/// The Qt pump singleton. `Once`-guarded so a second `init` call
/// (unlikely — `qt_adapter::init` runs once at boot) is a no-op.
static QT_PUMP: Once<Arc<QtPump>> = Once::new();

/// Qt event-loop pump implementing `ToolkitPump`. Single instance
/// per kernel boot — Qt's `QCoreApplication` is a process-singleton,
/// so one pump suffices.
///
/// Today's body is a stub: `pump` returns `PumpResult::idle()`,
/// `dispatch_key` / `dispatch_pointer` are no-ops, `focused_widget`
/// reads from `FOCUSED_WIDGET` (always `None` until Qt is loaded
/// and a Qt widget receives focus). Once GGGG's `qt_adapter::loader`
/// returns a real `LibHandle::Loaded`, the body swaps to real Qt
/// API calls without changing the trait surface.
pub struct QtPump {
    /// Diagnostic counter — bumped each `pump` call. Lets the
    /// inline test assert "pump was driven" without observing any
    /// FFI side effect.
    ticks: spin::Mutex<u64>,
}

impl QtPump {
    /// Build a fresh QtPump with the tick counter at zero. The
    /// public constructor is `init` (which goes through the
    /// `Once` so there's only ever one instance per boot); this
    /// `new` exists for the inline tests.
    pub const fn new() -> Self {
        Self { ticks: Mutex::new(0) }
    }

    /// Read the tick counter. Diagnostics-only.
    pub fn ticks(&self) -> u64 {
        *self.ticks.lock()
    }
}

impl Default for QtPump {
    fn default() -> Self {
        Self::new()
    }
}

impl ToolkitPump for QtPump {
    fn name(&self) -> &str {
        "qt6"
    }

    /// Process pending Qt events for at most `budget_ms`
    /// milliseconds. Stub today (returns idle immediately); future
    /// body calls `QCoreApplication::processEvents(QEventLoop::
    /// AllEvents, budget_ms)` via dlsym'd entry once
    /// `qt_adapter::loader::qt_core` returns a non-null base.
    ///
    /// Budget semantics match Qt's `processEvents(maxtime)` — the
    /// call returns when EITHER the queue is drained OR
    /// `budget_ms` ms have elapsed (whichever comes first). Qt's
    /// internal heuristic is "process all events whose timestamp is
    /// <= now + budget"; we rely on Qt to honour the budget.
    fn pump(&self, _budget_ms: u32) -> PumpResult {
        *self.ticks.lock() += 1;
        // Stub — when libqt6 is loaded the body swaps to:
        //
        //   let lib = qt_adapter::loader::qt_core();
        //   if lib.base.is_null() { return PumpResult::idle(); }
        //   let process_events: extern "C" fn(QEventLoopFlags, i32)
        //       = qt_adapter::loader::dlsym(&lib,
        //           "_ZN16QCoreApplication13processEventsE13QEventLoopE..i")
        //           .cast();
        //   process_events(ALL_EVENTS, _budget_ms as i32);
        //   let count = ...;  // from a Qt-side counter we maintain
        //   let pending = ...; // QCoreApplication::hasPendingEvents()
        //   PumpResult { events_processed: count, more_pending: pending }
        //
        // Today the loader returns LibraryNotFound, so we
        // short-circuit. The adapter still gets pumped each tick
        // (so `register_pump` is observable) but the pump body is
        // a one-instruction tick counter bump.
        PumpResult::idle()
    }

    /// Currently-focused Qt widget. Always `None` today (no Qt
    /// widgets exist on the foundation slice). When Qt is loaded
    /// and a `QWidget::focusInEvent` fires, the adapter will
    /// `set_focused(Some(WidgetId(<qwidget ptr>)))`; this method
    /// then returns the current value.
    fn focused_widget(&self) -> Option<WidgetId> {
        *FOCUSED_WIDGET.lock()
    }

    /// Synthesise `ev` into Qt's event queue. Stub today (no-op);
    /// future body calls `QCoreApplication::postEvent(target,
    /// new QKeyEvent(...))` via dlsym'd entries.
    fn dispatch_key(&self, _ev: KeyEvent) {
        // Stub — when libqt6 is loaded the body swaps to:
        //
        //   let target = current_focus_qwidget();
        //   if target.is_null() { return; }
        //   let qkey_new: extern "C" fn(...) -> *mut QKeyEvent
        //       = qt_adapter::loader::dlsym(...).cast();
        //   let event = qkey_new(QEvent::KeyPress, ev.codepoint as i32, ...);
        //   let post_event: extern "C" fn(*mut QObject, *mut QEvent)
        //       = qt_adapter::loader::dlsym(...).cast();
        //   post_event(target, event);
    }

    /// Synthesise `ev` into Qt's event queue. Same shape as
    /// `dispatch_key`. Stub today.
    fn dispatch_pointer(&self, _ev: PointerEvent) {
        // Stub — future body posts QMouseEvent / QWheelEvent via
        // QCoreApplication::postEvent.
    }
}

/// Set the currently-focused Qt widget (or clear it with `None`).
/// Called from the Qt-side `QWidget::focusInEvent` / `focusOutEvent`
/// overrides (future wiring once Qt is loaded).
///
/// Public so tests + future Qt-side integration can both call it.
pub fn set_focused(widget: Option<WidgetId>) {
    *FOCUSED_WIDGET.lock() = widget;
}

/// Read the currently-focused Qt widget without going through the
/// pump trait. Convenience for diagnostics.
pub fn focused() -> Option<WidgetId> {
    *FOCUSED_WIDGET.lock()
}

/// Initialise the Qt pump singleton and register it with the
/// kernel's toolkit_loop. Called once from `qt_adapter::init`
/// post-loader bring-up.
///
/// Idempotent at the singleton level — `Once::call_once` short-
/// circuits a second call. The `register_pump` step happens inside
/// the `call_once` closure so re-init doesn't double-register.
///
/// On the foundation slice this still runs even though the loader
/// returned `LibraryNotFound`: the stub pump produces no observable
/// behaviour but the registration makes the kernel's `pump_all`
/// walk Qt alongside Slint and GTK in a uniform sweep, so the
/// integration is observable in boot diagnostics (`pump_count()`
/// reports the right total).
pub fn init() {
    QT_PUMP.call_once(|| {
        let pump = Arc::new(QtPump::new());
        toolkit_loop::register_pump(pump.clone());
        pump
    });
}

/// Read-side accessor for the QtPump singleton. Returns `None` if
/// `init` hasn't run yet. Currently only used by the inline tests
/// to assert the singleton was registered.
pub fn pump() -> Option<Arc<QtPump>> {
    QT_PUMP.get().cloned()
}

// ---------------------------------------------------------------
// Tests
// ---------------------------------------------------------------

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn qt_pump_returns_idle_without_loaded_library() {
        // Stub pump should never block, never panic, return idle.
        let pump = QtPump::new();
        let result = pump.pump(4);
        assert_eq!(result, PumpResult::idle());
        // Multiple ticks accumulate the counter.
        let _ = pump.pump(4);
        let _ = pump.pump(4);
        assert_eq!(pump.ticks(), 3);
    }

    #[test]
    fn qt_pump_focused_widget_starts_none() {
        // Reset the focus cell to make the test order-independent.
        set_focused(None);
        let pump = QtPump::new();
        assert!(pump.focused_widget().is_none());
    }

    #[test]
    fn qt_pump_focused_widget_reflects_set_focused() {
        set_focused(None);
        let pump = QtPump::new();
        set_focused(Some(WidgetId(0xdead_beef)));
        assert_eq!(pump.focused_widget(), Some(WidgetId(0xdead_beef)));
        set_focused(None);
        assert!(pump.focused_widget().is_none());
    }

    #[test]
    fn qt_pump_dispatch_key_is_a_clean_no_op() {
        let pump = QtPump::new();
        // Should not panic, should not block.
        pump.dispatch_key(KeyEvent { codepoint: 'a', pressed: true });
        pump.dispatch_key(KeyEvent { codepoint: '\u{001b}', pressed: false });
    }

    #[test]
    fn qt_pump_dispatch_pointer_is_a_clean_no_op() {
        let pump = QtPump::new();
        pump.dispatch_pointer(PointerEvent::RelMove { dx: 5, dy: -3 });
        pump.dispatch_pointer(PointerEvent::Button { button: 0x110, pressed: true });
        pump.dispatch_pointer(PointerEvent::Scroll { delta: 2 });
    }

    #[test]
    fn qt_pump_name_is_qt6() {
        let pump = QtPump::new();
        assert_eq!(pump.name(), "qt6");
    }
}

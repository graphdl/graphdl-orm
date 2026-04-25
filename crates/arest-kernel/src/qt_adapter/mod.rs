// crates/arest-kernel/src/qt_adapter/mod.rs
//
// qt_adapter — Qt 6 toolkit adapter (#487, Track GGGG / #484c, the
// second adapter slice in the toolkit registry chain). Goal: load
// `libqt6widgets.so.6` + `libqt6core.so.6` as Linux-style shared
// libraries via the linuxkpi shim's library-loading path, then expose
// each Qt widget class as a `Component` cell via an
// `ImplementationBinding` fact — mirroring DDDD's #485 static
// declarations in `readings/ui/components.md` for the Qt 6 toolkit.
// After this lands, AREST apps can compose Qt widgets just like Slint
// widgets — all at the metamodel level.
//
// Architecture rationale
// ----------------------
// AAAA's #460 linuxkpi shim landed the foundation for loading
// unmodified Linux kernel C drivers (driver-mode). This module is the
// LIBRARY-mode equivalent — Qt 6 ships as `.so.6` userspace shared
// libraries (libqt6widgets.so.6, libqt6core.so.6, etc.) which depend
// on libstdc++ + glibc, but at the symbol-table level they're just
// ELF DSOs with `dlopen`-resolvable C++-mangled symbols (Qt's
// extern-C `staticMetaObject` accessors are wrapped in `extern "C"`
// shims via Qt's MOC). The library-loading + symbol-resolution
// surface needed here is a strict subset of what AAAA's foundation
// slice already established (alloc + io + scatterlist), plus a
// `dlopen`/`dlsym` pair that walks ELF program headers — which the
// foundation slice does NOT yet have. See `loader.rs` for the
// stub-fallback behaviour when the host (Windows + UEFI cross-build)
// has no libqt6 to load.
//
// Today's deliverable
// -------------------
// 1. `loader.rs` — wraps the linuxkpi library-loading path (or stubs
//    to `LibraryNotFound` when that path doesn't exist yet) for
//    libqt6widgets.so.6 + libqt6core.so.6. Each load returns a
//    `LibHandle` carrying the dlopen-equivalent base pointer (or
//    `null` on stub).
// 2. `widgets.rs` — for each of the 11 Qt widget classes DDDD's #485
//    declared (QPushButton / QLineEdit / QListView / QDateEdit /
//    QDialog / QSlider / QComboBox / QProgressBar / QCheckBox /
//    QTabBar / QLabel), holds a `*const QMetaObject` resolved via
//    `dlsym(handle, "<class>::staticMetaObject")` — null when the
//    symbol isn't found (graceful degrade for the Windows-host build).
// 3. `binding.rs` — `register_qt_components()` builds the Component +
//    ImplementationBinding fact set the same way FFFF's #486
//    registry.rs does for Slint, but pointing at Qt's QMetaObject
//    pointers as the Symbol values.
// 4. `marshalling.rs` — `set_property` + `connect_signal` stubs that
//    will eventually reach Qt's `QObject::setProperty` reflection +
//    the `QObject::connect` callback wiring.
//
// Selection still picks Slint over Qt because the compositor isn't
// wired (#489). Real widget rendering lands in #489-#491; this slice's
// job is to populate the cells so the selection rule library has
// somewhere to look.
//
// Lifecycle
// ---------
// `init()` is called once from main.rs after `system::init()` has run
// — Component cells are populated through `system::apply` so they need
// the SYSTEM mutator to be live. On the foundation slice the load
// step is a stub (no libqt6 in the cross-build host), so `init()`
// always falls through to the registration step with null library
// handles. Idempotent — each sub-module's init is `Once`-guarded so a
// second call is a no-op.
//
// Gating
// ------
// The whole subsystem is opt-in behind the `qt-adapter` cargo feature
// (see `Cargo.toml`). Default kernel builds skip every Rust shim
// module + the registration call — the .efi footprint and license
// story are unchanged. Mirrors the gate shape of `linuxkpi` (#460)
// and `doom` (#456).

#![allow(dead_code)]

pub mod binding;
pub mod loader;
pub mod marshalling;
pub mod widgets;

/// One-shot boot-time initialiser. Brings up the qt_adapter slice in
/// dependency order: load the libraries, resolve QMetaObject
/// pointers, then push the Component / ImplementationBinding facts
/// into SYSTEM.
///
/// Each step is graceful-degrade: a missing library on Windows-host
/// builds leaves the symbol table empty; widgets.rs records null
/// pointers; binding.rs still emits the facts (with null Symbol
/// pointer values) so the selection-rule library has stable cell
/// names to query against. The future linuxkpi library-loading
/// extension (out of scope here — it's AAAA's territory) replaces
/// the loader stub with real ELF DSO probing.
///
/// Idempotent — every sub-step is `Once`-guarded internally.
pub fn init() {
    // Step 1: load libqt6core.so.6 + libqt6widgets.so.6. On the
    // foundation slice this is a stub returning `LibraryNotFound`,
    // so `core` and `widgets` end up null. The widgets resolver
    // below handles null bases by recording null QMetaObject
    // pointers — graceful degrade.
    loader::init();

    // Step 2: walk the widget table and dlsym each
    // `<class>::staticMetaObject` symbol against the loaded library
    // bases. Null base → null pointer recorded; the registration
    // step still runs and emits the cells.
    widgets::init();

    // Step 3: build the Component + ImplementationBinding fact set
    // for the 11 Qt 6 widget classes DDDD's #485 declared and apply
    // it to SYSTEM. The Symbol value is the C++ class name string
    // ("QPushButton", "QLineEdit", …) — the same value DDDD's
    // ImplementationBinding facts in components.md carry. The
    // resolved QMetaObject pointer (which is what marshalling.rs
    // reaches for) lives in the widgets table, keyed by the same
    // class name.
    let _ = binding::register_qt_components();
}

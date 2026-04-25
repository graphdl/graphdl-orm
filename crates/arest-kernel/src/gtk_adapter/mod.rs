// crates/arest-kernel/src/gtk_adapter/mod.rs
//
// gtk_adapter — GTK 4 toolkit adapter (#488, Track IIII / #484d, the
// third adapter slice in the toolkit registry chain). Goal: load
// `libgtk-4.so.1` + `libgobject-2.0.so.0` + `libglib-2.0.so.0` as
// Linux-style shared libraries via the linuxkpi shim's library-
// loading path, then expose each GTK widget class as a `Component`
// cell via an `ImplementationBinding` fact — mirroring DDDD's #485
// static declarations in `readings/ui/components.md` for the GTK 4
// toolkit. After this lands, AREST apps can compose GTK widgets just
// like Slint widgets and Qt widgets — all at the metamodel level.
//
// This is the symmetric companion to GGGG's #487 Qt adapter: same
// 5-module shape (`mod`/`loader`/`widgets`/`binding`/`marshalling`),
// same gating pattern (cargo `gtk-adapter` feature + UEFI x86_64
// cfg), same `cell_push_unique` discipline so the shared Component
// cells (Component_has_Role, Component_has_Property, …) emitted by
// FFFF's #486 Slint registration and GGGG's #487 Qt registration
// don't duplicate.
//
// Architecture rationale
// ----------------------
// AAAA's #460 linuxkpi shim landed the foundation for loading
// unmodified Linux kernel C drivers (driver-mode). This module is the
// LIBRARY-mode equivalent — GTK 4 ships as `.so.1` userspace shared
// libraries (libgtk-4.so.1 atop libgobject-2.0.so.0 atop libglib-
// 2.0.so.0) which depend on libcairo + libpango + glibc, but at the
// symbol-table level they're just ELF DSOs with `dlopen`-resolvable
// C-mangled symbols (GTK is a pure-C library — every widget exports
// a `g_<class>_get_type()` accessor function; no C++ name mangling
// to deal with, unlike GGGG's Qt adapter). The library-loading +
// symbol-resolution surface needed here is a strict subset of what
// AAAA's foundation slice already established (alloc + io +
// scatterlist), plus a `dlopen`/`dlsym` pair that walks ELF program
// headers — which the foundation slice does NOT yet have. See
// `loader.rs` for the stub-fallback behaviour when the host (Windows
// + UEFI cross-build) has no libgtk-4 to load — the same situation
// GGGG faced for libqt6.
//
// GObject vs QMetaObject reflection
// ---------------------------------
// Where Qt has `QObject::setProperty(name, QVariant)` driven by the
// MOC-generated `staticMetaObject`, GTK uses `g_object_set_property
// (obj, name, GValue)` driven by the `g_<class>_get_type()`-returned
// `GType` and the property table registered at class-init time via
// `g_object_class_install_property`. The two reflection systems
// solve the same problem (string-name → type-tagged-value setter)
// but the value tagging differs:
//   * Qt's QVariant is a single tagged-union with built-in coercion
//     and a registered-type extension table.
//   * GObject's GValue is a flat (GType, union { gchar*, gint, …})
//     with explicit setters per type (g_value_set_string,
//     g_value_set_int, g_value_set_object, …) and explicit transform
//     callbacks for cross-type coercion (g_value_register_transform_func).
// The AREST-side `ComponentValue` ADT is shared with GGGG's qt_adapter
// (re-exported when both features are enabled, see `marshalling.rs`
// for the cfg-gated sharing); the marshaller's job is to coerce
// `ComponentValue` to either QVariant or GValue depending on which
// adapter receives the call.
//
// Today's deliverable
// -------------------
// 1. `loader.rs` — wraps the linuxkpi library-loading path (or stubs
//    to `LibraryNotFound` when that path doesn't exist yet) for
//    libglib-2.0.so.0 + libgobject-2.0.so.0 + libgtk-4.so.1. Each
//    load returns a `LibHandle` carrying the dlopen-equivalent base
//    pointer (or `null` on stub).
// 2. `widgets.rs` — for each of the 12 GTK widget classes DDDD's
//    #485 declared (GtkButton / GtkEntry / GtkListView / GtkCalendar /
//    GtkDialog / GtkPicture / GtkScale / GtkDropDown / GtkProgressBar /
//    GtkCheckButton / GtkNotebook / GtkBox), holds a `*const GType`
//    resolved via `dlsym(handle, "g_<class>_get_type")` against
//    libgtk-4.so.1 — null when the symbol isn't found (graceful
//    degrade for the Windows-host build).
// 3. `binding.rs` — `register_gtk_components()` builds the Component +
//    ImplementationBinding fact set the same way FFFF's #486
//    registry.rs does for Slint and GGGG's #487 binding.rs does for
//    Qt, but pointing at GType pointers as the Symbol values.
// 4. `marshalling.rs` — `set_property` + `connect_signal` stubs that
//    will eventually reach GObject's `g_object_set_property` reflection
//    + `g_signal_connect` callback wiring.
//
// Selection still picks Slint over GTK because the compositor isn't
// wired (#489). Real widget rendering lands in #489-#491; this slice's
// job is to populate the cells so the selection rule library has
// somewhere to look. (Note: DDDD's #485 ships a "Screen-reader / GTK
// preference" derivation rule at components.md L322-340 that prefers
// GTK 4 when AT-SPI matters — that rule fires once the compositor is
// wired and the runtime has a screen-reader-active fact to query
// against.)
//
// Lifecycle
// ---------
// `init()` is called once from main.rs after `system::init()` has run
// — Component cells are populated through `system::apply` so they need
// the SYSTEM mutator to be live. On the foundation slice the load
// step is a stub (no libgtk-4 in the cross-build host), so `init()`
// always falls through to the registration step with null library
// handles. Idempotent — each sub-module's init is `Once`-guarded so a
// second call is a no-op.
//
// Gating
// ------
// The whole subsystem is opt-in behind the `gtk-adapter` cargo feature
// (see `Cargo.toml`). Default kernel builds skip every Rust shim
// module + the registration call — the .efi footprint and license
// story are unchanged. Mirrors the gate shape of `linuxkpi` (#460),
// `doom` (#456), and `qt-adapter` (#487).

#![allow(dead_code)]

pub mod binding;
pub mod loader;
pub mod marshalling;
pub mod widgets;

/// One-shot boot-time initialiser. Brings up the gtk_adapter slice in
/// dependency order: load the libraries, resolve GType pointers,
/// then push the Component / ImplementationBinding facts into SYSTEM.
///
/// Each step is graceful-degrade: a missing library on Windows-host
/// builds leaves the symbol table empty; widgets.rs records null
/// pointers; binding.rs still emits the facts (with class-name string
/// Symbol values per DDDD's spec) so the selection-rule library has
/// stable cell names to query against. The future linuxkpi library-
/// loading extension (out of scope here — it's AAAA's territory)
/// replaces the loader stub with real ELF DSO probing.
///
/// Idempotent — every sub-step is `Once`-guarded internally.
pub fn init() {
    // Step 1: load libglib-2.0.so.0 + libgobject-2.0.so.0 + libgtk-
    // 4.so.1. On the foundation slice this is a stub returning
    // `LibraryNotFound`, so every handle ends up null. The widgets
    // resolver below handles null bases by recording null GType
    // pointers — graceful degrade.
    loader::init();

    // Step 2: walk the widget table and dlsym each
    // `g_<class>_get_type` symbol against the loaded libgtk-4 base.
    // Null base → null pointer recorded; the registration step still
    // runs and emits the cells.
    widgets::init();

    // Step 3: build the Component + ImplementationBinding fact set
    // for the 12 GTK 4 widget classes DDDD's #485 declared and apply
    // it to SYSTEM. The Symbol value is the C class name string
    // ("GtkButton", "GtkEntry", …) — the same value DDDD's
    // ImplementationBinding facts in components.md carry. The
    // resolved GType pointer (which is what marshalling.rs reaches
    // for) lives in the widgets table, keyed by the same class name.
    let _ = binding::register_gtk_components();
}

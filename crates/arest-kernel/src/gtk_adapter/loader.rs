// crates/arest-kernel/src/gtk_adapter/loader.rs
//
// Library-loading wrapper around the linuxkpi shim's library-loading
// path. Loads `libglib-2.0.so.0`, `libgobject-2.0.so.0`, and
// `libgtk-4.so.1` and exposes `dlopen`-style handles + a `dlsym`-
// style symbol-resolver that `widgets.rs` consumes.
//
// Foundation slice status (#488 Track IIII)
// -----------------------------------------
// AAAA's #460 linuxkpi shim is driver-mode focused — it ships:
//   * `alloc` / `device` / `driver` / `irq` / `workqueue` / `io` /
//     `input` / `virtio` for unmodified Linux kernel C drivers.
// What it does NOT yet ship is a `dlopen`/`dlsym` ELF DSO loader
// (Linux kernel modules are static archives + the kernel module
// loader resolves their symbols against the running kernel's
// exported table — the userspace `dlopen` story is a different
// surface lower in the stack). Wiring that up means walking ELF
// program headers, mapping LOAD segments at `mmap`-equivalent
// offsets, applying relocations, and resolving DT_NEEDED chains —
// substantial work that's a separate track and explicitly out of
// scope here. Same situation GGGG faced for libqt6 in #487.
//
// Until that lands, this module's `try_load` returns
// `LoadStatus::LibraryNotFound` for every requested library. Each
// `LibHandle` stores a null base pointer; `widgets::init` records
// null GType pointers when the base is null; `binding.rs` still
// emits the Component / ImplementationBinding facts so the
// selection rule library has stable cell names to query. The
// future linuxkpi extension (a separate track on AAAA's side)
// replaces the stub here with real ELF DSO probing — the rest of
// gtk_adapter doesn't change.
//
// On a host that DOES have libgtk-4 reachable (a Linux build of
// the kernel that links against the host's dynamic loader, or a
// future UEFI build with an in-kernel ELF loader), the
// `cfg(gtk_dlopen_real)` path could swap the stub for real
// `libloading`-equivalent calls. We don't gate the cfg yet —
// there's no other implementation to gate against.
//
// Library dependency chain
// ------------------------
// libgtk-4 transitively pulls (in DT_NEEDED order):
//   libglib-2.0.so.0    — base data structures (GHashTable, GList,
//                          GSList, GArray) + GMainLoop event loop
//   libgobject-2.0.so.0 — type system (GType, GValue, GObject,
//                          GParamSpec, signal/property reflection
//                          machinery) — depends on libglib
//   libgtk-4.so.1       — every widget class + GdkSurface +
//                          GtkApplication — depends on libgobject
//                          + libgdk-pixbuf-2.0 + libcairo +
//                          libpango-1.0 + libgraphene-1.0 + harfbuzz
// Real loader resolves DT_NEEDED chains automatically; the
// foundation-slice stub doesn't try — every `try_load` returns
// LibraryNotFound regardless of dependency depth.

use core::ffi::c_void;
use spin::Once;

/// Outcome of a library load attempt. `Loaded` carries the dlopen-
/// equivalent base pointer (the address LOAD segments were mapped to,
/// useful as the offset basis for `dlsym`-style symbol lookups).
/// `LibraryNotFound` is the foundation-slice fallback when linuxkpi
/// has no library-loading path yet — the cells still register, with
/// null Symbol pointers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoadStatus {
    /// Library mapped successfully. The base pointer is the offset
    /// to add to a Symbol value table entry to compute the runtime
    /// address. Never null on this variant.
    Loaded(*const c_void),
    /// Library couldn't be located. On the foundation slice this is
    /// the ONLY variant ever returned (linuxkpi has no library-
    /// loading path yet); on a future build with the loader extension
    /// it indicates a real "library not in search path" failure.
    LibraryNotFound,
    /// ELF parse failed (corrupt header, unsupported class, missing
    /// PT_DYNAMIC). Reserved for the future loader extension; the
    /// foundation-slice stub never returns this.
    InvalidElf,
}

// SAFETY: LoadStatus is plain Copy data. The `*const c_void` in the
// Loaded variant is a thread-safe address (we never alias it for
// writes); the kernel runs single-threaded at boot anyway.
unsafe impl Send for LoadStatus {}
unsafe impl Sync for LoadStatus {}

/// dlopen-style handle to a loaded library. `name` is the ELF
/// soname-equivalent (`libgtk-4.so.1`); `status` carries the load
/// outcome; `base` is the convenience-extraction of the base
/// pointer (null on every non-Loaded status).
#[derive(Clone, Copy, Debug)]
pub struct LibHandle {
    pub name: &'static str,
    pub status: LoadStatus,
    pub base: *const c_void,
}

// SAFETY: same reasoning as LoadStatus — Copy data, single-threaded
// at boot, raw pointer is never written through.
unsafe impl Send for LibHandle {}
unsafe impl Sync for LibHandle {}

impl LibHandle {
    /// Construct a not-found handle for `name`. Used by the
    /// foundation-slice stub `try_load` and by tests that want to
    /// exercise the null-handle path without invoking `init`.
    pub const fn not_found(name: &'static str) -> Self {
        Self {
            name,
            status: LoadStatus::LibraryNotFound,
            base: core::ptr::null(),
        }
    }
}

/// Cached library handles, populated once during `init` and read by
/// `widgets::init`. `Once`-guarded so a second `init` call is a
/// no-op.
static GLIB: Once<LibHandle> = Once::new();
static GOBJECT: Once<LibHandle> = Once::new();
static GTK: Once<LibHandle> = Once::new();

/// Load the three GTK 4 libraries in DT_NEEDED order. Each call goes
/// through `try_load`, which on the foundation slice returns
/// `LibraryNotFound` unconditionally (no linuxkpi DSO loader yet —
/// see top-of-file note). The handles are recorded in the static
/// cache for `widgets::init` to consume.
///
/// Idempotent — `Once::call_once` short-circuits a second call.
pub fn init() {
    GLIB.call_once(|| try_load("libglib-2.0.so.0"));
    GOBJECT.call_once(|| try_load("libgobject-2.0.so.0"));
    GTK.call_once(|| try_load("libgtk-4.so.1"));
}

/// Foundation-slice library-load entry point. Mirrors the
/// `dlopen`-style signature a real loader would expose: take the ELF
/// soname, return a handle. On the foundation slice this is a fixed
/// `LibraryNotFound` because linuxkpi has no library-loading path yet
/// — see the top-of-file note. The future linuxkpi extension replaces
/// the body of this function with a real ELF mapper.
///
/// We keep this as a top-level public function (rather than folding
/// it into `init`) so future tests can exercise the load path
/// directly without going through the cached static handles.
pub fn try_load(name: &'static str) -> LibHandle {
    // Stub branch — keep the structure identical to what a real
    // implementation would have, so the future loader extension is a
    // body-swap rather than an interface-swap. A future
    // `extern "C" { fn linuxkpi_dlopen(name: *const c_char) -> *const c_void; }`
    // declaration goes here, gated behind a custom cfg the linuxkpi
    // build script sets when it knows it has the DSO loader.
    LibHandle::not_found(name)
}

/// Look up a symbol by C-string name in a loaded library. The
/// signature matches `dlsym(handle, name) -> void *`. Returns null
/// when:
///   * the handle has a non-Loaded status (foundation-slice default),
///   * the linuxkpi DSO loader has no symbol-table parsing yet
///     (foundation-slice default — there's no DSO loaded so no
///     symbol table to parse),
///   * the symbol genuinely isn't in the library (real-loader
///     future branch).
///
/// On the foundation slice this always returns null because every
/// `LibHandle` carries `LoadStatus::LibraryNotFound`. The
/// `widgets.rs` resolver tolerates null pointers — the registered
/// GType pointer just stays null and the marshalling stubs no-op
/// until a real loader fills in the address.
///
/// GTK exports its `g_<class>_get_type` accessors as plain C
/// symbols (no name mangling — GTK is pure C), so `_symbol` is the
/// straight identifier `g_button_get_type`, `g_entry_get_type`, …
/// — no Itanium-ABI mangling step like Qt's
/// `_ZN11QPushButton16staticMetaObjectE`.
pub fn dlsym(handle: &LibHandle, _symbol: &str) -> *const c_void {
    match handle.status {
        LoadStatus::Loaded(_) => {
            // Real loader path — would walk the .dynsym + .dynstr
            // sections to find `symbol` and return `base + st_value`.
            // Foundation slice never reaches here because `try_load`
            // never returns Loaded. Left as `null` to be explicit
            // about the unimplemented branch rather than panicking
            // — a future loader extension flips this into the real
            // lookup body.
            core::ptr::null()
        }
        LoadStatus::LibraryNotFound | LoadStatus::InvalidElf => core::ptr::null(),
    }
}

/// Read-side accessor for `libglib-2.0.so.0`'s handle. Returns
/// `LibHandle::not_found` if `init` hasn't run; on the foundation
/// slice the cached handle is also `not_found` so callers see the
/// same value either way.
pub fn glib() -> LibHandle {
    GLIB.get()
        .copied()
        .unwrap_or_else(|| LibHandle::not_found("libglib-2.0.so.0"))
}

/// Read-side accessor for `libgobject-2.0.so.0`'s handle. Same
/// shape as `glib()`.
pub fn gobject() -> LibHandle {
    GOBJECT
        .get()
        .copied()
        .unwrap_or_else(|| LibHandle::not_found("libgobject-2.0.so.0"))
}

/// Read-side accessor for `libgtk-4.so.1`'s handle. Same shape as
/// `glib()`. This is the handle every widget-class `g_*_get_type`
/// dlsym targets.
pub fn gtk() -> LibHandle {
    GTK.get()
        .copied()
        .unwrap_or_else(|| LibHandle::not_found("libgtk-4.so.1"))
}

// ── Tests ──────────────────────────────────────────────────────────
//
// `arest-kernel`'s bin target sets `test = false` (Cargo.toml above
// the bin block), so these `#[cfg(test)]` cases run only when the
// crate is re-shaped into a lib for hosted testing — same pattern
// `system.rs` and `file_serve.rs` use. They document the foundation-
// slice behaviour so a future loader extension that lands real
// dlopen has a stable assertion battery to flip from "expect null"
// to "expect non-null".

#[cfg(test)]
mod tests {
    use super::*;

    /// Foundation slice: `try_load` always reports `LibraryNotFound`.
    /// When the linuxkpi DSO loader extension lands, this assertion
    /// flips: `try_load("libgtk-4.so.1")` will return `Loaded(_)`
    /// on hosts where the library is in the search path.
    #[test]
    fn try_load_foundation_slice_is_library_not_found() {
        let h = try_load("libgtk-4.so.1");
        assert_eq!(h.status, LoadStatus::LibraryNotFound);
        assert!(h.base.is_null());
        assert_eq!(h.name, "libgtk-4.so.1");
    }

    /// `dlsym` against a not-found handle returns null. This is the
    /// foundation-slice path every symbol lookup takes.
    #[test]
    fn dlsym_against_not_found_handle_returns_null() {
        let h = LibHandle::not_found("libgtk-4.so.1");
        let p = dlsym(&h, "g_button_get_type");
        assert!(p.is_null());
    }

    /// `init` is idempotent — a second call doesn't panic and the
    /// cached handles remain stable across all three libraries.
    #[test]
    fn init_is_idempotent() {
        init();
        let glib1 = glib();
        let gobject1 = gobject();
        let gtk1 = gtk();
        init(); // second call — must be a no-op
        let glib2 = glib();
        let gobject2 = gobject();
        let gtk2 = gtk();
        assert_eq!(glib1.status, glib2.status);
        assert_eq!(gobject1.status, gobject2.status);
        assert_eq!(gtk1.status, gtk2.status);
        assert_eq!(glib1.name, "libglib-2.0.so.0");
        assert_eq!(gobject1.name, "libgobject-2.0.so.0");
        assert_eq!(gtk1.name, "libgtk-4.so.1");
    }

    /// All three soname strings match the standard GTK 4 packaging
    /// shape (libname-major.minor.so.soversion). Catches a typo in
    /// `init` that would silently load the wrong file once the real
    /// loader lands.
    #[test]
    fn library_names_match_gtk4_soname_convention() {
        init();
        assert_eq!(glib().name, "libglib-2.0.so.0");
        assert_eq!(gobject().name, "libgobject-2.0.so.0");
        assert_eq!(gtk().name, "libgtk-4.so.1");
    }
}

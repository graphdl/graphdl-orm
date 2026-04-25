// crates/arest-kernel/src/qt_adapter/loader.rs
//
// Library-loading wrapper around the linuxkpi shim's library-loading
// path. Loads `libqt6core.so.6` and `libqt6widgets.so.6` and exposes
// `dlopen`-style handles + a `dlsym`-style symbol-resolver that
// `widgets.rs` consumes.
//
// Foundation slice status (#487 Track GGGG)
// -----------------------------------------
// AAAA's #460 linuxkpi shim is driver-mode focused — it ships:
//   * `alloc` / `device` / `driver` / `irq` / `workqueue` / `io` /
//     `input` / `virtio` for unmodified Linux kernel C drivers.
// What it does NOT yet ship is a `dlopen`/`dlsym` ELF DSO loader
// (Linux kernel modules are static archives + the kernel module
// loader resolves their symbols against the running kernel's exported
// table — the userspace `dlopen` story is a different surface lower
// in the stack). Wiring that up means walking ELF program headers,
// mapping LOAD segments at `mmap`-equivalent offsets, applying
// relocations, and resolving DT_NEEDED chains — substantial work
// that's a separate track and explicitly out of scope here.
//
// Until that lands, this module's `load` returns
// `LoadStatus::LibraryNotFound` for every requested library. Each
// `LibHandle` stores a null base pointer; `widgets::init` records
// null QMetaObject pointers when the base is null; `binding.rs` still
// emits the Component / ImplementationBinding facts so the selection
// rule library has stable cell names to query. The future linuxkpi
// extension (a separate track on AAAA's side) replaces the stub here
// with real ELF DSO probing — the rest of qt_adapter doesn't change.
//
// On a host that DOES have libqt6 reachable (a Linux build of the
// kernel that links against the host's dynamic loader, or a future
// UEFI build with an in-kernel ELF loader), the `cfg(qt_dlopen_real)`
// path could swap the stub for real `libloading`-equivalent calls.
// We don't gate the cfg yet — there's no other implementation to
// gate against.

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
/// soname-equivalent (`libqt6widgets.so.6`); `status` carries the
/// load outcome; `base` is the convenience-extraction of the base
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
    /// foundation-slice stub `load` and by tests that want to exercise
    /// the null-handle path without invoking `init`.
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
static QT_CORE: Once<LibHandle> = Once::new();
static QT_WIDGETS: Once<LibHandle> = Once::new();

/// Load both Qt 6 libraries. Each call goes through `try_load`, which
/// on the foundation slice returns `LibraryNotFound` unconditionally
/// (no linuxkpi DSO loader yet — see top-of-file note). The handles
/// are recorded in the static cache for `widgets::init` to consume.
///
/// Idempotent — `Once::call_once` short-circuits a second call.
pub fn init() {
    QT_CORE.call_once(|| try_load("libqt6core.so.6"));
    QT_WIDGETS.call_once(|| try_load("libqt6widgets.so.6"));
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

/// Look up a symbol by C-string name in a loaded library. The signature
/// matches `dlsym(handle, name) -> void *`. Returns null when:
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
/// QMetaObject just stays null and the marshalling stubs no-op until
/// a real loader fills in the address.
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

/// Read-side accessor for `libqt6core.so.6`'s handle. Returns
/// `LibHandle::not_found` if `init` hasn't run; on the foundation
/// slice the cached handle is also `not_found` so callers see the
/// same value either way.
pub fn qt_core() -> LibHandle {
    QT_CORE
        .get()
        .copied()
        .unwrap_or_else(|| LibHandle::not_found("libqt6core.so.6"))
}

/// Read-side accessor for `libqt6widgets.so.6`'s handle. Same shape
/// as `qt_core()`.
pub fn qt_widgets() -> LibHandle {
    QT_WIDGETS
        .get()
        .copied()
        .unwrap_or_else(|| LibHandle::not_found("libqt6widgets.so.6"))
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
    /// flips: `try_load("libqt6core.so.6")` will return `Loaded(_)`
    /// on hosts where the library is in the search path.
    #[test]
    fn try_load_foundation_slice_is_library_not_found() {
        let h = try_load("libqt6core.so.6");
        assert_eq!(h.status, LoadStatus::LibraryNotFound);
        assert!(h.base.is_null());
        assert_eq!(h.name, "libqt6core.so.6");
    }

    /// `dlsym` against a not-found handle returns null. This is the
    /// foundation-slice path every symbol lookup takes.
    #[test]
    fn dlsym_against_not_found_handle_returns_null() {
        let h = LibHandle::not_found("libqt6widgets.so.6");
        let p = dlsym(&h, "_ZN11QPushButton16staticMetaObjectE");
        assert!(p.is_null());
    }

    /// `init` is idempotent — a second call doesn't panic and the
    /// cached handles remain stable.
    #[test]
    fn init_is_idempotent() {
        init();
        let core1 = qt_core();
        let widgets1 = qt_widgets();
        init(); // second call — must be a no-op
        let core2 = qt_core();
        let widgets2 = qt_widgets();
        assert_eq!(core1.status, core2.status);
        assert_eq!(widgets1.status, widgets2.status);
        assert_eq!(core1.name, "libqt6core.so.6");
        assert_eq!(widgets1.name, "libqt6widgets.so.6");
    }
}

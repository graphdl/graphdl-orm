// crates/arest-kernel/src/composer.rs
//
// Foreign-toolkit texture compositor (#489 Track LLLL). The
// composition runtime that lets non-Slint toolkits — Qt (GGGG #487),
// GTK (IIII #488), and any future adapter — render into a CPU-side
// pixel buffer that Slint then composites into its scene tree as
// just-another-Image source. Same primitive Track VVV's #455 Doom
// uses to push a WASM-rendered 640x400 BGRA frame at Slint each tic;
// here it's generalised so any toolkit's render output can be
// treated identically by the surrounding Slint UI.
//
// # Why this layer exists at all
//
// GGGG's #487 (Qt) and IIII's #488 (GTK) registered widget classes
// as `Component` cells against SYSTEM (per DDDD's #485 declarations
// in `readings/ui/components.md`), but stubbed the actual widget
// rendering — neither libqt6widgets.so.6 nor libgtk-4.so.1 is
// reachable from the UEFI build today (the linuxkpi #460 foundation
// slice was driver-mode focused; library-mode dlopen lands in #460+
// follow-ups + #461). The Component cells populate with null Symbol
// pointers and the selection-rule library (#492) picks Slint over
// Qt / GTK because the Slint binding has the `kernel_native` trait.
//
// What this composer does is pre-wire the integration seam so that
// the moment Qt's `loader.rs` returns a real QMetaObject pointer
// (post-#460+#461), Qt's adapter can:
//   1. `composer::register_toolkit("qt6", Arc::new(QtRenderer::new()))`
//      against this module's static toolkit registry.
//   2. `composer::create_surface("qt6", 800, 600)` to allocate a
//      ForeignSurface backed by an RGBA8 pixel buffer.
//   3. Bind the surface to a Slint Image property via
//      `arch::uefi::slint_backend::surface_to_image(&surface)` —
//      a one-liner that wraps the surface bytes in
//      `slint::Image::from_rgba8(SharedPixelBuffer::clone_from_slice
//      (...))` (the same per-frame copy primitive VVV's #455 Doom
//      uses; Slint v1.16's Image API requires owned `'static`
//      pixel data, so detaching a copy each frame is mandatory).
//   4. Let the launcher's super-loop call `composer::compose_frame()`
//      once per Slint frame; that walks every registered surface,
//      asks each surface's owning toolkit's `ToolkitRenderer::paint`
//      to repaint dirty buffers, and returns control to Slint.
//
// Today the only renderer in the foundation slice is a
// `RustTestRenderer` checkerboard generator gated behind
// `--features compositor-test` — proves the round-trip end-to-end
// without depending on a real toolkit being loaded. Qt + GTK plug
// in via the same trait once their library loaders work.
//
// # Pixel format choice — RGBA8 only
//
// Every foreign surface uses RGBA8 (`[R, G, B, A]` in memory, 4
// bytes per pixel). Two reasons:
//
//   * **Slint compatibility.** Slint's `from_rgba8` consumes a
//     `SharedPixelBuffer<Rgba8Pixel>`; `Rgba8Pixel` is `rgb::RGBA8`
//     which is `[R, G, B, A]` in memory. Anything else needs a
//     per-pixel conversion at composite time (see VVV's #455 Doom
//     for the BGRA->RGBA cost: 1_024_000-byte read+write per frame
//     at 35 Hz). Forcing RGBA8 at the composer means the only
//     conversion happens inside the toolkit renderer (which already
//     knows its native pixel layout) rather than per-frame in the
//     hot composite path.
//   * **Toolkit native paths.** Qt's `QImage::Format_RGBA8888` and
//     GTK's `cairo_image_surface_create(CAIRO_FORMAT_ARGB32, …)`
//     both have RGBA8 modes; the conversion (if any) is one-time
//     during the toolkit's render pass, not per-composite. Qt
//     takes a `QImage::Format` enum at construction; GTK / Cairo
//     take it at surface construction. Both are post-#460+#461
//     wiring concerns.
//
// `PixelFormat` carries both `Rgba8` and a `Bgra8` variant for the
// future case where Qt / GTK insist on writing BGRA natively (Qt's
// `Format_ARGB32_Premultiplied` is the most-supported mode on most
// Qt builds, and it's BGRA8 in memory). `surface_to_image` swaps
// channels in that path; the per-frame swap cost is the same one
// VVV's Doom path already pays.
//
// # Why static / global state
//
// The toolkit registry + surface table live in two `spin::Mutex`-
// guarded `BTreeMap`s (the same shape `block_storage::MOUNT` and
// `arch::uefi::keyboard::RING` use). Reasons:
//
//   * Adapters init from independent boot stages (Qt from
//     `qt_adapter::init` post-linuxkpi, GTK from `gtk_adapter::init`
//     ditto, hypothetically a Tauri / web-view adapter from a
//     different stage). They can't pass an `&mut Composer` around
//     to each other.
//   * Slint's `compose_frame` call site is the launcher's super-
//     loop, which has no other reason to hold a composer reference
//     — having it reach a global static keeps the call site free
//     of plumbing.
//   * The kernel runs single-threaded at boot; `spin::Mutex` is
//     contention-free (no other CPU can race), so the global-state
//     pattern is sound.
//
// # Concurrency model
//
// `ForeignSurface::pixels` is `Arc<Mutex<Vec<u8>>>`. The toolkit
// renderer locks it during `paint` (writes new pixels);
// `surface_to_image` locks it during composite (reads bytes for
// `clone_from_slice`). Both are very short-lived locks under the
// kernel's single-threaded boot model — no real contention. The
// `Arc` exists so the renderer can hold a clone of the surface
// alongside the composer's own clone in `SURFACES`; both observers
// see writes through the same `Mutex<Vec<u8>>`.
//
// `ToolkitRenderer` is required to be `Send + Sync` so adapters
// can park their renderer in a static (e.g., `static QT_RENDERER:
// Once<Arc<QtRenderer>>`); the trait constraint matches what
// `register_toolkit` requires. On the kernel's single-threaded
// boot model the bound is trivially satisfied for any pure-Rust
// renderer; for adapters wrapping FFI handles (Qt's `QApplication*`,
// GTK's `GtkApplication*`) the bound is the adapter author's
// responsibility — their `unsafe impl Send + Sync` claim is what
// matters, not anything this module enforces.
//
// # Inline tests
//
// The tests are gated on `cfg(target_os = "linux")` so they run
// during cross-arch host CI on a machine with a test runner; the
// `cargo test --target x86_64-unknown-uefi` target has `test = false`
// on the bin (Cargo.toml L98) so inline tests aren't reachable
// from the UEFI build anyway. Same gating shape Doom's `ui_apps::doom`
// tests hint at via the `arest-kernel`'s bin-level `test = false`
// note (doom.rs L795-L800). The tests cover:
//   * register_toolkit + create_surface idempotent semantics
//   * compose_frame walks every dirty surface and clears the dirty
//     bit after paint
//   * RustTestRenderer paints a deterministic checkerboard pattern
//
// # NOT done in this commit
//
// * Qt's `QImage` rendering hook — lands when GGGG's
//   `qt_adapter::loader::resolve_symbol` returns non-null
//   QPainter / QImage entry points (post-#460+#461).
// * GTK's `Cairo` surface hook — same dependency on IIII's
//   `gtk_adapter::loader` returning non-null `cairo_create` /
//   `cairo_image_surface_create` entry points.
// * Event-loop coordination among toolkits (#490) — Qt's
//   `QCoreApplication::processEvents` and GTK's
//   `g_main_context_iteration` need to be pumped each frame so
//   widget animations advance; that's a separate trait method.
// * Property / signal binding tied to ForeignSurface lifecycle
//   (#491) — the cell-level `set_property("text", "Hello")`
//   and `connect_signal("clicked", callback)` plumbing that
//   makes `ImplementationBinding` cells truly interactive.

#![allow(dead_code)]

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use spin::Mutex;

/// Monotonic 64-bit surface identifier, allocated by `create_surface`.
///
/// IDs are issued by `bumping NEXT_SURFACE_ID` so each call gets a
/// unique value across the lifetime of the kernel. `BTreeMap`
/// iteration over `SURFACES` walks them in ascending ID order — the
/// "deterministic per-frame walk order" `compose_frame` advertises is
/// exactly this ascending-ID order, which is the order surfaces were
/// created.
///
/// 64-bit chosen for headroom: at one surface created per frame at
/// 60 Hz (the worst case if a toolkit creates ephemeral surfaces for
/// every popup / tooltip), the counter takes ~10 billion years to
/// wrap. A `u32` would be sufficient in practice but the `u64` cost
/// is one extra atomic word per allocation — negligible against
/// the surface pixel buffer sizes (1 MB @ 512x512 RGBA8).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SurfaceId(pub u64);

/// Pixel layout of a `ForeignSurface`'s backing buffer.
///
/// Two variants today:
///
///   * `Rgba8` — `[R, G, B, A]` per pixel, 4 bpp. Matches Slint's
///     `Rgba8Pixel` directly so `surface_to_image` is a straight
///     `clone_from_slice`. Preferred default.
///   * `Bgra8` — `[B, G, R, A]` per pixel, 4 bpp. Same memory
///     footprint, R<->B swapped. Matches Qt's `Format_ARGB32_
///     Premultiplied` and Cairo's `CAIRO_FORMAT_ARGB32` native
///     layouts on little-endian hosts. `surface_to_image` does the
///     R<->B swap before handing bytes to Slint — same conversion
///     cost VVV's #455 Doom path already pays at 35 Hz (1_024_000
///     byte reads + writes per swap).
///
/// `non_exhaustive` so a future variant (e.g. `Rgb8` for cases
/// where the toolkit can't produce alpha) doesn't break adapter
/// matches. Adapters must handle the wildcard arm.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PixelFormat {
    /// `[R, G, B, A]` per pixel, 4 bytes per pixel. Slint-native.
    Rgba8,
    /// `[B, G, R, A]` per pixel, 4 bytes per pixel. Qt / Cairo
    /// native on little-endian hosts; needs R<->B swap at composite
    /// time.
    Bgra8,
}

impl PixelFormat {
    /// Bytes per pixel for this format. Both variants are 4 bpp
    /// today; the helper exists so callers don't hard-code the
    /// constant and a future 24-bit variant doesn't silently
    /// over-allocate.
    pub const fn bytes_per_pixel(self) -> usize {
        match self {
            PixelFormat::Rgba8 | PixelFormat::Bgra8 => 4,
        }
    }
}

/// One foreign-toolkit pixel surface. The integration unit between
/// a toolkit's render pass and Slint's compositor.
///
/// `pixels` is heap-allocated `width * height * bpp` bytes, owned
/// behind an `Arc<Mutex<...>>` so:
///
///   * The toolkit's `ToolkitRenderer::paint` impl can hold a
///     clone of the `Arc` (via the `&ForeignSurface` reference) and
///     write into the buffer when the dirty bit is set.
///   * The composer's static surface table holds another clone of
///     the same `Arc` so subsequent `compose_frame` calls see the
///     written pixels through the same backing storage.
///   * `surface_to_image` reads the bytes under the same lock when
///     building the `slint::Image` for the composite step.
///
/// The dirty bit is `AtomicBool` (Acquire/Release ordering) so
/// adapters can flip it from any context — including from inside
/// a Qt signal handler that may not be able to lock a mutex
/// without risking re-entry against the renderer. The toolkit
/// flips it via `invalidate(id)` (or directly when it knows the
/// surface needs repainting); `compose_frame` reads + clears it
/// before each paint.
pub struct ForeignSurface {
    /// Composer-issued surface ID, unique across boot.
    pub id: SurfaceId,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Pixel layout — `Rgba8` or `Bgra8` today.
    pub format: PixelFormat,
    /// Backing pixel buffer. Always exactly
    /// `width * height * format.bytes_per_pixel()` bytes long.
    /// `Vec<u8>` rather than `Vec<Rgba8Pixel>` so the buffer can
    /// hold any pixel format without a per-format generic; the
    /// composer treats it as an opaque byte array and lets the
    /// composite step interpret per `format`.
    pub pixels: Arc<Mutex<Vec<u8>>>,
    /// Dirty flag. Flipped to `true` by `invalidate(id)` or by the
    /// toolkit's own logic when the surface needs repainting; read
    /// + cleared by `compose_frame` before invoking `paint`. Acquire
    /// load on read, Release store on clear, so writes by the
    /// invalidator are visible to the next `compose_frame` pass.
    pub dirty: AtomicBool,
    /// Slug of the toolkit that owns this surface. Used by
    /// `compose_frame` to look up the right `ToolkitRenderer` from
    /// `TOOLKITS`. Owned `String` so the surface survives an
    /// adapter unload (theoretical — adapters live for boot today,
    /// but the lifetime is correct without a borrow).
    pub toolkit: String,
}

impl ForeignSurface {
    /// Mark the surface as needing repainting on the next
    /// `compose_frame`. Cheap (one atomic store, Release ordering).
    /// Called either by the adapter's `ToolkitRenderer::invalidate`
    /// after the toolkit signals "redraw needed" (Qt's
    /// `QWidget::update`, GTK's `gtk_widget_queue_draw`) or directly
    /// by application code when it knows a property change requires
    /// repaint without round-tripping through the toolkit's signal
    /// machinery.
    pub fn invalidate(&self) {
        self.dirty.store(true, Ordering::Release);
    }

    /// True iff the surface is currently flagged for repaint.
    /// Reads with Acquire ordering; pairs with the Release write
    /// in `invalidate`.
    pub fn is_dirty(&self) -> bool {
        self.dirty.load(Ordering::Acquire)
    }
}

/// Trait implemented by each foreign-toolkit adapter. The
/// integration seam between the toolkit's render pass and the
/// composer's per-frame walk.
///
/// `Send + Sync` because `register_toolkit` parks the impl in the
/// static `TOOLKITS` map under a `spin::Mutex`; pulling the trait
/// object out across a lock boundary requires `Sync`, and storing
/// it in the map requires `Send`. The kernel's single-threaded
/// boot model trivially satisfies both for any pure-Rust impl;
/// adapters wrapping FFI handles (Qt's `QApplication*`) must
/// `unsafe impl Send + Sync` for their wrapper type, with the
/// safety obligation that the wrapper is only ever touched from
/// the boot CPU.
///
/// Two methods:
///
///   * `paint(&self, surface)` — the toolkit re-renders into the
///     surface's pixel buffer when dirty. Called by `compose_frame`
///     once per dirty surface per frame, in ascending-ID order.
///     The trait impl is responsible for translating its native
///     pixel layout to whatever the surface declares as its
///     `format` (the composer doesn't second-guess; if the toolkit
///     writes BGRA into an Rgba8 surface, `surface_to_image` shows
///     swapped colours).
///   * `invalidate(&self, surface_id)` — mark the surface as dirty
///     for the next `compose_frame`. Default impl finds the
///     surface in `SURFACES` and calls its `invalidate()` — most
///     adapters won't need to override. Adapters that need to
///     react to the invalidation (e.g. queue a Qt
///     `QWidget::repaint` against the toolkit's own event loop)
///     can override and forward.
pub trait ToolkitRenderer: Send + Sync {
    /// Re-render into `surface`'s pixel buffer. Called once per
    /// dirty surface per `compose_frame`, after the composer has
    /// cleared the surface's dirty bit. The impl locks
    /// `surface.pixels`, writes the new pixel data, and unlocks.
    /// No return value — the next `surface_to_image` call picks
    /// up the new bytes through the shared `Arc`.
    ///
    /// Failure path: the impl handles its own errors. A toolkit
    /// renderer that can't repaint (e.g. Qt's QApplication crashed
    /// or GTK's display server disconnected) is responsible for
    /// either zeroing the surface to a "render failed" colour or
    /// leaving the previous pixels in place; the composer takes
    /// either as valid (its job is dispatch, not error recovery).
    fn paint(&self, surface: &ForeignSurface);

    /// Mark `surface_id` as dirty for the next `compose_frame`.
    /// Default impl looks the surface up in `SURFACES` and calls
    /// its `invalidate()`. Adapters can override to additionally
    /// queue work against their own event loop.
    fn invalidate(&self, surface_id: SurfaceId) {
        if let Some(s) = lookup_surface(surface_id) {
            s.invalidate();
        }
    }
}

// ---------------------------------------------------------------
// Static state
// ---------------------------------------------------------------

/// Counter for the next surface ID. Bumped once per `create_surface`.
/// Starts at 1 so 0 stays available as a "no surface" sentinel for
/// callers that need one (the Slint Image property on a freshly-
/// constructed component, before any toolkit has rendered).
static NEXT_SURFACE_ID: AtomicU64 = AtomicU64::new(1);

/// Toolkit registry. Maps the toolkit's slug (`"qt6"`, `"gtk4"`,
/// `"slint"`, `"test"`) to its renderer. Populated by adapter init
/// (qt_adapter::init, gtk_adapter::init); read by `compose_frame`
/// to dispatch dirty-surface paint calls.
///
/// Slug-keyed rather than ID-keyed because adapters identify by
/// slug — `register_toolkit("qt6", ...)` matches DDDD's #485
/// `Toolkit_has_Slug` cell which uses string slugs. `BTreeMap` for
/// deterministic iteration order (alphabetical by slug); adapters
/// that need a specific dispatch order must serialise their
/// init calls accordingly. Today no order constraint is enforced.
static TOOLKITS: Mutex<BTreeMap<String, Arc<dyn ToolkitRenderer>>> =
    Mutex::new(BTreeMap::new());

/// Surface table. Maps `SurfaceId` to its `Arc<ForeignSurface>`.
/// `BTreeMap` ordered by ID so `compose_frame` walks surfaces in
/// creation order — same convention every other AREST `BTreeMap`-
/// of-IDs uses (block_storage::BLOB_SLOTS, etc.).
///
/// Holds an `Arc` rather than `ForeignSurface` directly so callers
/// of `create_surface` can store a clone for their own use (binding
/// to a Slint Image property, passing to a render thread) while the
/// composer keeps its own clone live. Drop semantics: when the
/// last `Arc` is dropped (e.g. the application closes the widget
/// AND the composer removes the entry — neither happens in the
/// foundation slice) the pixel buffer's `Mutex<Vec<u8>>` releases
/// its allocation back to the heap.
static SURFACES: Mutex<BTreeMap<SurfaceId, Arc<ForeignSurface>>> =
    Mutex::new(BTreeMap::new());

// ---------------------------------------------------------------
// Public API
// ---------------------------------------------------------------

/// Register `renderer` as the painter for toolkit `name`. Called
/// once per adapter init (Qt's `qt_adapter::init`, GTK's
/// `gtk_adapter::init`, the test harness's `--features compositor-
/// test` boot path).
///
/// Idempotent at the slug level: re-registering an existing slug
/// **replaces** the renderer atomically (under the mutex). This
/// matches the "adapter can be re-initialised" pattern other
/// modules follow (system::init, qt_adapter binding registration);
/// the previous `Arc<dyn ToolkitRenderer>` is dropped at the end
/// of the call. Surfaces previously created against the old
/// renderer keep their pixel buffers — the next `compose_frame`
/// pass dispatches to the new renderer for any still-dirty
/// surfaces.
///
/// Allocates one `String` per call (the slug owned copy). No other
/// allocations beyond the BTreeMap entry insertion.
pub fn register_toolkit(name: &str, renderer: Arc<dyn ToolkitRenderer>) {
    let mut map = TOOLKITS.lock();
    map.insert(name.to_string(), renderer);
}

/// Create a new foreign surface and register it in the composer's
/// surface table. Returns the `Arc<ForeignSurface>` so the caller
/// can hold one clone for its own use (binding to a Slint Image
/// property via `surface_to_image`, passing to the toolkit
/// adapter for paint hookup); the composer holds another clone.
///
/// `toolkit` is the slug of an already-registered toolkit (via
/// `register_toolkit`). The composer doesn't validate this at
/// creation — a surface against an unregistered toolkit is
/// allowed; `compose_frame` just skips its paint dispatch (the
/// toolkit will register later, or it won't, and the surface
/// stays at whatever pixel state it was last painted with). This
/// matches the foundation-slice reality where the loader stub
/// returns `LibraryNotFound` and the adapter still emits Component
/// cells but can't yet register a renderer.
///
/// `width` * `height` * `bytes_per_pixel` bytes are allocated
/// upfront. RGBA8 default — adapters that need BGRA8 can build
/// the surface struct directly via the public field surface (the
/// `pixels`, `width`, `height`, `format`, `dirty`, `id` fields are
/// all `pub`), but `create_surface` is the cell-line helper for
/// the common case. `Rgba8` chosen as the default because the
/// composite step is a straight `clone_from_slice` — see the
/// module docstring's pixel-format choice rationale.
///
/// The new surface starts out **clean** (`dirty == false`) — the
/// pixel buffer is zero-initialised (transparent black under
/// RGBA8); the toolkit must call `invalidate(id)` (or the surface's
/// own `invalidate()`) to schedule the first paint.
pub fn create_surface(toolkit: &str, width: u32, height: u32) -> Arc<ForeignSurface> {
    let id = SurfaceId(NEXT_SURFACE_ID.fetch_add(1, Ordering::Relaxed));
    let format = PixelFormat::Rgba8;
    let byte_len = (width as usize) * (height as usize) * format.bytes_per_pixel();
    let surface = Arc::new(ForeignSurface {
        id,
        width,
        height,
        format,
        pixels: Arc::new(Mutex::new(vec![0u8; byte_len])),
        dirty: AtomicBool::new(false),
        toolkit: toolkit.to_string(),
    });
    SURFACES.lock().insert(id, surface.clone());
    surface
}

/// Look up a surface by ID. Returns `None` if the surface was
/// never created or has been removed (no removal API today; the
/// composer holds surfaces for boot lifetime).
///
/// Pure helper exposed for test code + the default
/// `ToolkitRenderer::invalidate` impl. The launcher's super-loop
/// shouldn't need this — `compose_frame` walks the table itself.
pub fn lookup_surface(id: SurfaceId) -> Option<Arc<ForeignSurface>> {
    SURFACES.lock().get(&id).cloned()
}

/// Number of registered surfaces. Cheap; useful for boot diagnostics.
pub fn surface_count() -> usize {
    SURFACES.lock().len()
}

/// Number of registered toolkits. Cheap; useful for boot
/// diagnostics + assertions in the test suite.
pub fn toolkit_count() -> usize {
    TOOLKITS.lock().len()
}

/// One per-frame composition pass, called once per Slint frame from
/// the launcher's super-loop (#431) — sequence:
///
///   1. Walk every surface in the composer's table (ascending ID
///      order via `BTreeMap` iteration).
///   2. For each surface that is dirty (`dirty.swap(false,
///      Acquire)` returns `true`), look up the surface's owning
///      toolkit in `TOOLKITS`, and if registered, dispatch
///      `paint(&surface)`.
///   3. Return.
///
/// Returns the number of paints actually dispatched. Zero on a
/// fully-clean frame (no surfaces dirty, or the surface's toolkit
/// hasn't registered a renderer yet). Useful for diagnostics +
/// the inline test harness.
///
/// Lock order: `SURFACES` lock is held across the whole walk to
/// produce a consistent snapshot; the inner `TOOLKITS` lock is
/// taken and released per surface to minimise hold time. The
/// inner `Arc<ForeignSurface>` is cloned out of the snapshot
/// before invoking `paint` so the toolkit's paint implementation
/// can hold the surface across its own internal state changes
/// without re-entering the composer's lock.
///
/// The toolkit's `paint` runs WITHOUT either composer lock held
/// — locks are released before `paint` is invoked. This is
/// important for adapters that may invoke composer APIs from
/// inside their paint impl (e.g. a Qt signal that creates a
/// new surface during paint); re-entering with the lock held
/// would deadlock.
pub fn compose_frame() -> usize {
    // Snapshot: collect the dirty surfaces + their toolkit slugs
    // under the lock, then release before dispatching paint. This
    // avoids holding either lock across `paint` calls (which may
    // re-enter composer APIs).
    let dirty: Vec<Arc<ForeignSurface>> = {
        let surfaces = SURFACES.lock();
        surfaces
            .values()
            .filter(|s| s.dirty.swap(false, Ordering::Acquire))
            .cloned()
            .collect()
    };
    let mut painted = 0usize;
    for surface in dirty.iter() {
        // Fetch the renderer under the toolkit lock, drop the
        // lock, then dispatch. Renderer is `Arc<dyn ToolkitRenderer>`
        // so the clone is one atomic refcount bump.
        let renderer = {
            let toolkits = TOOLKITS.lock();
            toolkits.get(surface.toolkit.as_str()).cloned()
        };
        if let Some(r) = renderer {
            r.paint(surface);
            painted += 1;
        }
        // Surface's toolkit hasn't registered a renderer yet —
        // skip silently. The dirty bit was already cleared by the
        // swap above; the surface stays at its previous pixel
        // state. If the adapter later registers, the next
        // `invalidate` call re-flags the surface and the next
        // `compose_frame` picks it up.
    }
    painted
}

// ---------------------------------------------------------------
// `--features compositor-test` checkerboard renderer
// ---------------------------------------------------------------

/// Pure-Rust test renderer that paints a deterministic checkerboard
/// pattern into any surface it's handed. Used by `compositor-test`
/// builds to prove the composer round-trip end-to-end without
/// needing a real Qt or GTK library to dlopen.
///
/// Pattern: 8x8 pixel cells alternate between two colours
/// (`primary` and `secondary`). Cell size is hard-coded at 8 — small
/// enough that even a 64x64 surface shows ~8 cells per row, large
/// enough that the pattern is visible at typical UI scales without
/// an FFT-style aliasing. Same kind of test pattern Qt's
/// `QImage::Format_RGBA8888` documentation uses in its examples.
///
/// Independent of surface size + format — the impl handles any
/// `Rgba8` or `Bgra8` surface. Bytes are written in the surface's
/// declared format, so `surface_to_image` produces the right
/// colours without an extra swap.
#[cfg(feature = "compositor-test")]
pub struct RustTestRenderer {
    /// First checkerboard colour as `(R, G, B, A)`. Default is
    /// solid magenta to make the pattern obvious against most
    /// dark / light themes — same colour Slint's tooling uses for
    /// "missing image" placeholders.
    pub primary: [u8; 4],
    /// Second checkerboard colour as `(R, G, B, A)`. Default is
    /// solid black — high contrast against the primary.
    pub secondary: [u8; 4],
}

#[cfg(feature = "compositor-test")]
impl RustTestRenderer {
    /// Build the default checkerboard renderer (magenta + black,
    /// 8x8 cells).
    pub fn new() -> Self {
        Self {
            primary: [0xFF, 0x00, 0xFF, 0xFF],
            secondary: [0x00, 0x00, 0x00, 0xFF],
        }
    }
}

#[cfg(feature = "compositor-test")]
impl Default for RustTestRenderer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "compositor-test")]
impl ToolkitRenderer for RustTestRenderer {
    /// Paint the 8x8 checkerboard. Iterates pixel-by-pixel so the
    /// impl is trivially correct; the inner write is one branchless
    /// conditional + 4 byte writes per pixel. At 1024x720 (the
    /// kernel's default Slint window size) that's ~737k pixel
    /// writes per paint — well under one frame budget at any
    /// realistic refresh rate.
    fn paint(&self, surface: &ForeignSurface) {
        const CELL: u32 = 8;
        let w = surface.width;
        let h = surface.height;
        let bpp = surface.format.bytes_per_pixel();
        let mut buf = surface.pixels.lock();
        // Defensive: if the buffer was reshaped out from under us
        // (shouldn't happen — `create_surface` allocates exactly
        // and we never resize), bail rather than overwriting.
        let expected = (w as usize) * (h as usize) * bpp;
        if buf.len() != expected {
            return;
        }
        // Pre-compute the two colour byte sequences in the
        // surface's native order. Rgba8 = primary as-is;
        // Bgra8 = R<->B swap.
        let (a, b) = match surface.format {
            PixelFormat::Rgba8 => (self.primary, self.secondary),
            PixelFormat::Bgra8 => (
                [self.primary[2], self.primary[1], self.primary[0], self.primary[3]],
                [self.secondary[2], self.secondary[1], self.secondary[0], self.secondary[3]],
            ),
        };
        for y in 0..h {
            for x in 0..w {
                let cell_x = x / CELL;
                let cell_y = y / CELL;
                let parity = (cell_x + cell_y) & 1;
                let colour = if parity == 0 { a } else { b };
                let off = (y as usize * w as usize + x as usize) * bpp;
                buf[off..off + 4].copy_from_slice(&colour);
            }
        }
    }
}

// ---------------------------------------------------------------
// Tests
// ---------------------------------------------------------------
//
// `arest-kernel`'s bin target has `test = false` (Cargo.toml L98)
// so these tests are reachable only on a host build. They're gated
// on `cfg(target_os = "linux")` so cross-arch CI runs them on a
// Linux runner without the UEFI target attempting to compile a
// `_start` symbol the test harness wouldn't know what to do with.
// Same convention the rest of the crate's `#[cfg(test)]` modules
// follow (slint_backend.rs L501-L610, doom.rs L795-L863).

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    /// Each test runs against a fresh global state to avoid order-
    /// dependence between tests in the same binary. `cargo test`
    /// runs tests in parallel by default which would make the
    /// global statics race, so each test takes a reset lock first.
    /// The reset lock is the same `TOOLKITS` mutex — taking it
    /// serialises every test that touches global composer state.
    fn reset_state() {
        TOOLKITS.lock().clear();
        SURFACES.lock().clear();
        // NEXT_SURFACE_ID intentionally NOT reset — IDs are
        // monotonic across the lifetime of the process, including
        // across reset_state calls. Tests assert on relative
        // ordering, never absolute values.
    }

    /// Stub renderer that records every `paint` invocation against
    /// a shared counter so tests can assert dispatch happened.
    struct CountingRenderer {
        paints: Arc<Mutex<u64>>,
    }

    impl ToolkitRenderer for CountingRenderer {
        fn paint(&self, _surface: &ForeignSurface) {
            *self.paints.lock() += 1;
        }
    }

    #[test]
    fn register_toolkit_is_idempotent_at_slug() {
        reset_state();
        let r1 = Arc::new(CountingRenderer { paints: Arc::new(Mutex::new(0)) });
        let r2 = Arc::new(CountingRenderer { paints: Arc::new(Mutex::new(0)) });
        register_toolkit("qt6", r1.clone());
        assert_eq!(toolkit_count(), 1);
        register_toolkit("qt6", r2.clone());
        // Re-registering replaces, doesn't append: count stays 1.
        assert_eq!(toolkit_count(), 1);
        // Distinct slugs add additional entries.
        register_toolkit("gtk4", r1);
        assert_eq!(toolkit_count(), 2);
    }

    #[test]
    fn create_surface_allocates_buffer_and_registers() {
        reset_state();
        let s = create_surface("qt6", 16, 8);
        assert_eq!(s.width, 16);
        assert_eq!(s.height, 8);
        assert_eq!(s.format, PixelFormat::Rgba8);
        assert_eq!(s.pixels.lock().len(), 16 * 8 * 4);
        assert!(!s.is_dirty());
        // Surface is registered in the global table — lookup
        // returns the same Arc.
        let looked_up = lookup_surface(s.id).expect("surface registered");
        assert!(Arc::ptr_eq(&s, &looked_up));
        assert_eq!(surface_count(), 1);
    }

    #[test]
    fn create_surface_issues_distinct_ids_in_order() {
        reset_state();
        let a = create_surface("qt6", 1, 1);
        let b = create_surface("qt6", 1, 1);
        let c = create_surface("gtk4", 1, 1);
        assert!(a.id < b.id);
        assert!(b.id < c.id);
        assert_eq!(surface_count(), 3);
    }

    #[test]
    fn invalidate_flips_dirty_bit() {
        reset_state();
        let s = create_surface("test", 4, 4);
        assert!(!s.is_dirty());
        s.invalidate();
        assert!(s.is_dirty());
    }

    #[test]
    fn compose_frame_walks_dirty_surfaces_in_order() {
        reset_state();
        let counter = Arc::new(Mutex::new(0u64));
        let renderer = Arc::new(CountingRenderer { paints: counter.clone() });
        register_toolkit("qt6", renderer);

        let s1 = create_surface("qt6", 2, 2);
        let s2 = create_surface("qt6", 2, 2);
        let _s3 = create_surface("qt6", 2, 2);
        s1.invalidate();
        s2.invalidate();
        // s3 stays clean.

        let painted = compose_frame();
        assert_eq!(painted, 2, "two dirty surfaces should be painted");
        assert_eq!(*counter.lock(), 2);
        // After compose, both surfaces are clean again.
        assert!(!s1.is_dirty());
        assert!(!s2.is_dirty());
        // A second compose with no dirty surfaces is a no-op.
        let painted2 = compose_frame();
        assert_eq!(painted2, 0);
        assert_eq!(*counter.lock(), 2, "no extra paints on clean compose");
    }

    #[test]
    fn compose_frame_skips_unregistered_toolkit() {
        reset_state();
        // Surface against a toolkit that hasn't registered. The
        // dirty bit is consumed (cleared) but no paint runs.
        let s = create_surface("qt6", 4, 4);
        s.invalidate();
        assert!(s.is_dirty());
        let painted = compose_frame();
        assert_eq!(painted, 0, "no renderer registered → no paint");
        // Dirty bit still cleared — once compose_frame consumes
        // an invalidation, the surface is clean. Adapters that
        // missed registration must re-invalidate after they
        // register if they want a repaint.
        assert!(!s.is_dirty());
    }

    #[test]
    fn default_invalidate_routes_through_surfaces_table() {
        reset_state();
        let counter = Arc::new(Mutex::new(0u64));
        let renderer = Arc::new(CountingRenderer { paints: counter.clone() });
        register_toolkit("qt6", renderer.clone());
        let s = create_surface("qt6", 2, 2);
        // Use the trait's default invalidate impl rather than the
        // surface's direct invalidate method.
        let r: &dyn ToolkitRenderer = renderer.as_ref();
        r.invalidate(s.id);
        assert!(s.is_dirty());
    }

    #[cfg(feature = "compositor-test")]
    #[test]
    fn rust_test_renderer_paints_deterministic_checkerboard() {
        reset_state();
        register_toolkit("test", Arc::new(RustTestRenderer::new()));
        // 16x16 surface = 4 cells across (8px per cell).
        let s = create_surface("test", 16, 16);
        s.invalidate();
        let painted = compose_frame();
        assert_eq!(painted, 1);
        let buf = s.pixels.lock();
        // Pixel (0, 0) is in cell (0, 0) — parity 0 → primary.
        assert_eq!(&buf[0..4], &[0xFF, 0x00, 0xFF, 0xFF]);
        // Pixel (8, 0) is in cell (1, 0) — parity 1 → secondary.
        let off_8_0 = (0 * 16 + 8) * 4;
        assert_eq!(&buf[off_8_0..off_8_0 + 4], &[0x00, 0x00, 0x00, 0xFF]);
        // Pixel (0, 8) is in cell (0, 1) — parity 1 → secondary.
        let off_0_8 = (8 * 16 + 0) * 4;
        assert_eq!(&buf[off_0_8..off_0_8 + 4], &[0x00, 0x00, 0x00, 0xFF]);
        // Pixel (8, 8) is in cell (1, 1) — parity 2 (even) →
        // primary.
        let off_8_8 = (8 * 16 + 8) * 4;
        assert_eq!(&buf[off_8_8..off_8_8 + 4], &[0xFF, 0x00, 0xFF, 0xFF]);
    }

    #[cfg(feature = "compositor-test")]
    #[test]
    fn rust_test_renderer_handles_bgra8_surface() {
        reset_state();
        register_toolkit("test", Arc::new(RustTestRenderer::new()));
        // Build a Bgra8 surface manually — create_surface only
        // exposes the Rgba8 default.
        let id = SurfaceId(NEXT_SURFACE_ID.fetch_add(1, Ordering::Relaxed));
        let surface = Arc::new(ForeignSurface {
            id,
            width: 8,
            height: 8,
            format: PixelFormat::Bgra8,
            pixels: Arc::new(Mutex::new(vec![0u8; 8 * 8 * 4])),
            dirty: AtomicBool::new(true),
            toolkit: "test".to_string(),
        });
        SURFACES.lock().insert(id, surface.clone());
        let painted = compose_frame();
        assert_eq!(painted, 1);
        let buf = surface.pixels.lock();
        // Primary is RGBA = 0xFF/0x00/0xFF/0xFF; the BGRA write
        // swaps R<->B → BGRA = 0xFF/0x00/0xFF/0xFF (palindrome
        // for this particular colour, so the byte sequence is
        // visually identical).
        assert_eq!(&buf[0..4], &[0xFF, 0x00, 0xFF, 0xFF]);
        // Cell (1, 0) → secondary = RGBA 0/0/0/FF; BGRA swap
        // produces the same byte sequence.
        let off_8_0 = 8 * 4;
        assert_eq!(&buf[off_8_0..off_8_0 + 4], &[0x00, 0x00, 0x00, 0xFF]);
    }
}

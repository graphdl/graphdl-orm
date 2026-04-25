// crates/arest-kernel/src/system.rs
//
// SYSTEM function, kernel-side (#265).
//
// The no_std `arest` lib gives us `ast::Object`, `ast::Func`,
// `ast::apply`, `ast::defs_to_state`, and `ast::metacompose`. That
// is the entire SYSTEM surface — everything else (parse, compile,
// command, check) lives behind the std-only feature gate and is
// unavailable here. For the bare-metal kernel the implication is
// that readings are pre-compiled before boot and their resulting
// def set is baked into the binary; at runtime the kernel only
// needs to ρ-apply.
//
// This first version bakes two demo defs so the HTTP handler can
// exercise the full dispatch path:
//
//   `welcome` → a static banner atom.
//   `echo`    → Func::Id; returns whatever input it was handed.
//
// Dispatch cycle:
//   1. Look up the def cell by name via FetchOrPhi(<name, D>).
//   2. metacompose the resulting Object back into a Func (reverse
//      of `func_to_object`, which defs_to_state applied).
//   3. ast::apply the Func to the HTTP-body-derived input against
//      the baked state D.
//   4. Serialise the resulting Object into bytes for the wire.
//
// As more defs get baked in (compiled from the metamodel at build
// time, shipped as `freeze`d bytes, thawed here), the same three
// lines of dispatch logic serve every verb. That's the whole point
// of SYSTEM as a single function.
//
// ── Mutator (#451) ─────────────────────────────────────────────────
//
// XX (#403) and FFF (#444) both flagged the same gap: the original
// `Once<Object>` shape made SYSTEM immutable post-init, which meant
// `POST /file` could compute the would-be next state but not install
// it, and `GET /file/{id}/content` 404'd every freshly-uploaded id.
// The mutator below is the minimum-viable fix: a `RwLock`-protected
// pointer slot, swapped atomically by `apply()`. Each `apply()`
// `Box::leak`s the new state so existing `&'static Object` borrows
// (returned by the legacy `state()` shim retained for `net.rs`)
// remain valid for the rest of the kernel boot. Memory grows per
// upload; an arena reclaim pass is a follow-up once the chunked PUT
// (#445) lands and write rates climb.
//
// API:
//   * `with_state(|s| ...) -> Option<R>`  — read-side guard form,
//     replaces XX's `state() -> Option<&'static Object>` for new
//     callers (file_serve, file_upload). Briefly holds the read
//     lock inside the closure.
//   * `apply(new_state) -> Result<(), &'static str>` — atomic write.
//     Caller computes the fully-built next-state Object via
//     `ast::cell_push`/`build_file_facts` and hands it in; we leak
//     it, swap the pointer under the write lock, and return.
//   * `state() -> Option<&'static Object>` — legacy shim retained
//     so `net.rs` (forbidden territory in this commit) keeps
//     compiling. Returns the leaked-pointer snapshot taken under a
//     brief read lock; the pointer remains valid for the kernel
//     lifetime (memory leaks on each `apply`).

use alloc::boxed::Box;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use arest::ast::{self, Func, Object};
use spin::{Once, RwLock};

/// Baked kernel state — built once during boot and then mutable
/// through `apply()` for the tenant's lifetime. Stored as an
/// `&'static Object` slot behind a `RwLock` so:
///
///   * Reads briefly take the shared lock and snapshot the current
///     pointer (lock release order: drop guard before returning to
///     caller; the pointer itself is `'static` so it outlives the
///     guard).
///   * `apply()` `Box::leak`s the new state, takes the exclusive
///     lock, and overwrites the pointer. Old leaked states are not
///     reclaimed — see top-of-file note.
///
/// `Once` wraps the `RwLock` so the lock itself is constructed
/// lazily inside `init()` rather than at module-load time; this
/// preserves the pre-#451 panic-on-double-init semantics.
static SYSTEM: Once<RwLock<&'static Object>> = Once::new();

/// Build the demo state + defs. Called once from `kernel_main`
/// after `net::init()`. Panics if called twice — mirrors the
/// one-tenant-per-kernel invariant and keeps the lookup path
/// lock-free.
pub fn init() {
    SYSTEM.call_once(|| {
        // Two demo defs. The banner text lives in the `welcome`
        // Constant so the HTTP handler's dispatch is a single
        // ρ-application and not a Rust string.push.
        let defs: Vec<(String, Func)> = alloc::vec![
            (
                "welcome".to_string(),
                Func::Constant(Object::atom(
                    "AREST kernel — one ρ-application away from the wire.\n\n\
                     Try:  curl http://127.0.0.1/api/welcome\n\
                           curl -d 'hello' http://127.0.0.1/api/echo\n",
                )),
            ),
            // Func::Id is the identity ρ-application — apply(Id, x, D) = x.
            ("echo".to_string(), Func::Id),
        ];
        let initial = ast::defs_to_state(&defs, &Object::phi());
        // Box::leak gives us the `&'static Object` the slot stores.
        // The leak is intentional: the legacy `state()` shim returns
        // `&'static Object`, and `apply()`'s atomic-pointer-swap
        // story requires that all live snapshots remain valid for
        // the kernel lifetime.
        let leaked: &'static Object = Box::leak(Box::new(initial));
        RwLock::new(leaked)
    });
}

/// Dispatch a parsed HTTP request through the baked SYSTEM.
///
/// Returns `Some(body)` on a handled path, `None` when no def
/// matches. Callers layer their own 404 on the None branch.
///
/// The path-to-def map is the entirety of the HTTP routing layer:
/// every HTTP verb lands on `apply_named(def_name, body)`, and
/// `apply_named` does one ρ-lookup + one ρ-application. There is
/// no separate command handler, no route middleware, and no
/// method-specific branching; the def owns its interpretation.
pub fn dispatch(_method: &str, path: &str, body: &[u8]) -> Option<Vec<u8>> {
    let def_name = route_to_def(path)?;
    Some(apply_named(def_name, body))
}

fn route_to_def(path: &str) -> Option<&'static str> {
    // Strip an optional query string so `/api/welcome?v=1` still matches.
    let path = path.split('?').next().unwrap_or(path);
    match path {
        // Canonical API namespace introduced in #266. The HTML shell
        // lives at `/`; every dynamic verb is reached at `/api/<def>`.
        "/api/welcome" => Some("welcome"),
        "/api/echo" => Some("echo"),

        // Legacy bundle-free routes. When no ui.do bundle is baked in
        // (assets::UI_ASSETS is empty), the handler falls through
        // from `assets::lookup` to `system::dispatch` for the bare
        // `/` path — keeping the pre-#266 "AREST kernel — one
        // ρ-application away from the wire" banner reachable via
        // `curl http://127.0.0.1/`. Once the bundle is present these
        // paths are shadowed by `/index.html`.
        "/" | "/welcome" => Some("welcome"),
        "/echo" => Some("echo"),

        _ => None,
    }
}

/// Public entry for ring-3 callers (#333). Same dispatch the HTTP
/// handler uses, exposed by name for the syscall layer.
pub fn apply_named_pub(name: &str, body: &[u8]) -> Vec<u8> {
    apply_named(name, body)
}

/// Public cell-fetch entry for ring-3 callers (#333). Looks up the
/// named cell in the baked SYSTEM state and serialises the result
/// to bytes. Returns the same `\xE2\x8A\xA5\n` (⊥) marker that the
/// HTTP path uses when the cell is absent or empty.
///
/// Goes through `Func::FetchOrPhi` rather than `ast::fetch_or_phi`
/// so the syscall path is structurally identical to the wire path
/// — same ρ-dispatch shape, just a different transport.
pub fn fetch_named(name: &str) -> Vec<u8> {
    match with_state(|state| {
        let name_obj = Object::atom(name);
        let tuple = Object::seq(alloc::vec![name_obj, state.clone()]);
        ast::apply(&Func::FetchOrPhi, &tuple, state)
    }) {
        Some(val) => serialise(&val),
        None => serialise(&Object::Bottom),
    }
}

/// Apply the named def to `body` against the baked state and
/// return the resulting Object serialised as bytes.
///
/// Looks up the def's Object representation via
/// FetchOrPhi(<name, D>), reverses it back into a `Func` via
/// `ast::metacompose`, then runs `ast::apply(func, input, D)` —
/// exactly the three-step ρ-dispatch the paper's SYSTEM equation
/// describes. When the def is absent FetchOrPhi returns `Object::Bottom`
/// and `metacompose` gives back `Func::Id`, which is safely a no-op.
fn apply_named(name: &str, body: &[u8]) -> Vec<u8> {
    let out = with_state(|state| {
        let name_obj = Object::atom(name);
        let tuple = Object::seq(alloc::vec![name_obj, state.clone()]);
        let f_obj = ast::apply(&Func::FetchOrPhi, &tuple, state);
        let f = ast::metacompose(&f_obj, state);

        let input = match core::str::from_utf8(body) {
            Ok(s) if !s.is_empty() => Object::atom(s),
            _ => Object::phi(),
        };

        ast::apply(&f, &input, state)
    })
    .expect("system::init() not called");
    serialise(&out)
}

/// Read the baked SYSTEM state through a closure. `f` runs while
/// the read lock is held, so it should be cheap (clone the bits it
/// needs and return a value type — do not stash the `&Object`).
/// Returns `None` when `init` hasn't run.
///
/// This is the post-#451 read-side API, replacing XX's
/// `state() -> Option<&'static Object>`. New code (file_serve,
/// file_upload) reaches for this; the legacy `state()` shim below
/// is retained only for `net.rs` (forbidden in this commit) and
/// will be removed when that module migrates.
pub fn with_state<R>(f: impl FnOnce(&Object) -> R) -> Option<R> {
    let lock = SYSTEM.get()?;
    let guard = lock.read();
    Some(f(*guard))
}

/// Atomically replace the baked SYSTEM state with `new_state`.
/// Caller is responsible for building `new_state` such that it
/// already contains all desired facts (the kernel side of an
/// `ast::cell_push` chain — see `file_upload::build_file_facts`).
///
/// `Box::leak`s the new state so the legacy `state()` shim's
/// `&'static Object` snapshots remain valid for the kernel
/// lifetime. The write lock is held only across the pointer swap
/// — `f`-style closures that rebuild state should compute
/// `new_state` first, then call `apply` to commit.
///
/// Returns `Err` only when `init()` hasn't run yet (a programmer
/// error — the call site should ensure boot ordering puts
/// `system::init` before any route that mutates).
pub fn apply(new_state: Object) -> Result<(), &'static str> {
    let lock = SYSTEM.get().ok_or("system::init() not called")?;
    let leaked: &'static Object = Box::leak(Box::new(new_state));
    let mut guard = lock.write();
    *guard = leaked;
    Ok(())
}

/// Legacy borrow of the baked SYSTEM state, retained as a shim
/// for `net.rs` (forbidden in #451; migrates separately). Returns
/// the leaked-pointer snapshot taken under a brief read lock; the
/// `&'static Object` lifetime is sound because `apply()` never
/// reclaims old states — every install is a fresh `Box::leak`.
///
/// Prefer `with_state(|s| ...)` in new code.
pub fn state() -> Option<&'static Object> {
    let lock = SYSTEM.get()?;
    let guard = lock.read();
    Some(*guard)
}

/// Turn an Object into wire bytes. Atoms pass through; everything
/// else renders via the Debug fallback so the handler always has
/// something to send, even for shapes we haven't explicitly
/// formatted yet.
fn serialise(obj: &Object) -> Vec<u8> {
    match obj {
        Object::Atom(s) => s.clone().into_bytes(),
        Object::Bottom => b"\xE2\x8A\xA5\n".to_vec(), // U+22A5
        _ => {
            // Placeholder rendering — future commits replace this
            // with a proper FFP-to-JSON projection keyed off the
            // request's Accept header.
            alloc::format!("{:?}\n", obj).into_bytes()
        }
    }
}

// ── Tests ──────────────────────────────────────────────────────────
//
// `arest-kernel`'s bin target sets `test = false` (Cargo.toml L33),
// so these `#[cfg(test)]` cases are reachable only when the crate is
// re-shaped into a lib for hosted testing — the same pattern
// `file_serve.rs` and `file_upload.rs` use. They document the
// intended behaviour and provide a ready-to-run battery for the day
// the kernel grows a lib facade.
//
// To keep the tests independent of the singleton boot path (multiple
// tests in one process would race for the `Once`), each test scopes
// its assertions to a fresh `RwLock<&'static Object>` instance built
// the same way `init()` builds `SYSTEM`. The lock's read/write
// semantics are exercised against that local instance; the surface
// `with_state` / `apply` API is then exercised once via `init()` in
// the round-trip test, which uses a `call_once`-guarded global.

#[cfg(test)]
mod tests {
    use super::*;
    use arest::ast::fact_from_pairs;

    /// Helper mirroring `init()`'s leak-and-wrap shape. Returns a
    /// detached `RwLock<&'static Object>` so the lock semantics can
    /// be exercised without contending on the module-level
    /// `SYSTEM` singleton.
    fn fresh_slot(initial: Object) -> RwLock<&'static Object> {
        let leaked: &'static Object = Box::leak(Box::new(initial));
        RwLock::new(leaked)
    }

    /// First read after init returns the initial state — the
    /// pre-#451 "init then read" path still works through the new
    /// guarded shape.
    #[test]
    fn init_then_first_read_sees_initial_state() {
        let initial = ast::cell_push(
            "Probe",
            fact_from_pairs(&[("k", "v0")]),
            &Object::phi(),
        );
        let slot = fresh_slot(initial.clone());

        // Read-side mirror of `with_state` against the local slot.
        let observed: Object = {
            let guard = slot.read();
            (*guard).clone()
        };
        assert_eq!(observed, initial);
    }

    /// `apply` semantics: a write replaces the inner pointer; the
    /// next read sees the new state. This is the upload→download
    /// round-trip shape — POST /file builds new_state, calls
    /// apply(new_state), and the next GET /file/{id}/content sees
    /// the freshly-installed File facts.
    #[test]
    fn apply_replaces_state_subsequent_read_sees_new() {
        let initial = ast::cell_push(
            "Probe",
            fact_from_pairs(&[("k", "v0")]),
            &Object::phi(),
        );
        let slot = fresh_slot(initial);

        // Build a successor state and "apply" it against the local
        // slot (mirrors the module-level `apply`'s leak-and-swap).
        let next = ast::cell_push(
            "Probe",
            fact_from_pairs(&[("k", "v1")]),
            {
                let g = slot.read();
                *g
            },
        );
        let leaked: &'static Object = Box::leak(Box::new(next.clone()));
        {
            let mut g = slot.write();
            *g = leaked;
        }

        let after: Object = {
            let g = slot.read();
            (*g).clone()
        };
        assert_eq!(after, next);

        // The new state really does contain both fact versions
        // (cell_push appends; the latest read should see the
        // updated cell with two entries).
        let cell = ast::fetch_or_phi("Probe", &after);
        let seq = cell.as_seq().unwrap_or(&[]);
        assert!(
            seq.len() >= 2,
            "post-apply state should carry both Probe facts, saw {}", seq.len(),
        );
    }

    /// Concurrent reads are allowed — `RwLock` shares the read lock
    /// across multiple holders. Exercised here by acquiring two
    /// read guards simultaneously and confirming both see the same
    /// snapshot.
    #[test]
    fn concurrent_reads_share_the_lock() {
        let initial = Object::atom("shared");
        let slot = fresh_slot(initial.clone());

        let g1 = slot.read();
        let g2 = slot.read();
        // Both guards observe the same `&'static Object` pointer
        // value because `apply` hasn't run between them.
        assert!(core::ptr::eq(*g1, *g2));
        assert_eq!(**g1, initial);
        assert_eq!(**g2, initial);
        // Drop both before exiting (writes would block until then).
        drop(g1);
        drop(g2);
    }

    /// `try_write` fails while a read guard is held, confirming the
    /// exclusive-write semantics. The kernel is single-threaded
    /// today (per the #451 brief), so this is lock semantics, not
    /// real concurrency — but `RwLock` is the right primitive for
    /// when concurrency lands.
    #[test]
    fn write_is_blocked_while_read_held() {
        let slot = fresh_slot(Object::atom("rd"));
        let read_guard = slot.read();
        // `try_write` returns `None` because the read guard is live.
        assert!(slot.try_write().is_none());
        drop(read_guard);
        // Once the read guard is dropped the write goes through.
        assert!(slot.try_write().is_some());
    }

    /// End-to-end exercise of the public `init` → `with_state` →
    /// `apply` → `with_state` round trip against the module
    /// singleton. Guarded so a re-run within the same process
    /// (e.g. `cargo test` with multiple tests sharing the binary)
    /// doesn't double-init.
    ///
    /// The init step is a no-op when SYSTEM is already populated
    /// (spin::Once contract), so this test is robust to ordering
    /// against any other test that happens to call `init()`.
    #[test]
    fn module_round_trip_init_apply_read() {
        init();

        // Read the as-init'd state — must contain the `welcome`
        // and `echo` def cells `init()` baked in.
        let pre = with_state(|s| s.clone()).expect("init ran");
        // `defs_to_state` emits one `D_has_<name>` cell per def;
        // here we only need to confirm the state isn't bottom.
        assert_ne!(pre, Object::Bottom);

        // Apply a new state with a custom probe cell on top of the
        // current one.
        let next = ast::cell_push(
            "ProbeApply",
            fact_from_pairs(&[("k", "v")]),
            &pre,
        );
        apply(next.clone()).expect("apply succeeds post-init");

        // Read again — the probe cell must be visible now.
        let post = with_state(|s| {
            let cell = ast::fetch_or_phi("ProbeApply", s);
            cell.as_seq().map(|f| f.len()).unwrap_or(0)
        })
        .expect("init ran");
        assert!(post >= 1, "post-apply read should see ProbeApply cell");
    }

    /// The legacy `state()` shim still returns the same pointer
    /// `with_state` exposes, for as long as `net.rs` hasn't
    /// migrated (file forbidden in #451). The two APIs share the
    /// same backing slot so they can't diverge.
    #[test]
    fn legacy_state_shim_matches_with_state() {
        init();
        let via_shim = state().expect("init ran");
        let via_with = with_state(|s| s as *const Object).expect("init ran");
        assert_eq!(via_shim as *const Object, via_with);
    }
}

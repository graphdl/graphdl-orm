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

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use arest::ast::{self, Func, Object};
use spin::Once;

/// Baked kernel state — built once during boot and then read-only
/// for the tenant's lifetime. Behind a `spin::Once` so both the
/// HTTP handler and the REPL can access it after `init()` finishes
/// without needing a Mutex on every dispatch.
static SYSTEM: Once<Object> = Once::new();

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
        ast::defs_to_state(&defs, &Object::phi())
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
    let state = match SYSTEM.get() {
        Some(s) => s,
        None => return serialise(&Object::Bottom),
    };
    let name_obj = Object::atom(name);
    let tuple = Object::seq(alloc::vec![name_obj, state.clone()]);
    let val = ast::apply(&Func::FetchOrPhi, &tuple, state);
    serialise(&val)
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
    let state = SYSTEM.get().expect("system::init() not called");
    let name_obj = Object::atom(name);
    let tuple = Object::seq(alloc::vec![name_obj, state.clone()]);
    let f_obj = ast::apply(&Func::FetchOrPhi, &tuple, state);
    let f = ast::metacompose(&f_obj, state);

    let input = match core::str::from_utf8(body) {
        Ok(s) if !s.is_empty() => Object::atom(s),
        _ => Object::phi(),
    };

    let out = ast::apply(&f, &input, state);
    serialise(&out)
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

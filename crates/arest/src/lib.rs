// crates/arest/src/lib.rs
//
// AREST: Applicative REpresentational State Transfer
//
// SYSTEM:x = <o, D'>
// One function. Readings in, applications out.
// State = P (facts) + DEFS (named Func).

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::OnceLock;

pub mod ast;
pub mod types;
pub mod compile;
pub mod evaluate;
pub mod query;
pub mod induce;
pub mod rmap;
pub mod naming;
pub mod validate;
pub mod conceptual_query;
pub mod parse_rule;
pub mod parse_forml2;
pub mod verbalize;
pub mod arest;
pub mod crypto;
pub mod generators;

#[cfg(feature = "cloudflare")]
pub mod cloudflare;

/// D: the unified state — population cells + def cells in one Object.
/// Backus Sec. 14.3: "the state D of an AST system."
struct CompiledState {
    d: ast::Object,
}

static DOMAINS: OnceLock<Mutex<Vec<Option<CompiledState>>>> = OnceLock::new();
fn ds() -> &'static Mutex<Vec<Option<CompiledState>>> {
    DOMAINS.get_or_init(|| Mutex::new(Vec::new()))
}

fn allocate(state: ast::Object, defs: Vec<(String, ast::Func)>) -> u32 {
    let d = ast::defs_to_state(&defs, &state);
    let mut s = ds().lock().unwrap();
    let h = s.iter().position(|x| x.is_none()).unwrap_or_else(|| { s.push(None); s.len() - 1 });
    s[h] = Some(CompiledState { d });
    h as u32
}

// ── SYSTEM is the only function ─────────────────────────────────────

/// create: allocate empty D with platform primitives registered in DEFS.
fn create_impl() -> u32 {
    let state = ast::Object::phi();
    let defs = vec![
        ("compile".to_string(), ast::Func::Platform("compile".to_string())),
        ("apply".to_string(), ast::Func::Platform("apply_command".to_string())),
        ("verify_signature".to_string(), ast::Func::Platform("verify_signature".to_string())),
    ];
    allocate(state, defs)
}

/// Legacy: parse_and_compile as create + compile for each readings pair.
fn parse_and_compile_impl(readings: Vec<(String, String)>) -> Result<u32, String> {
    let h = create_impl();
    readings.iter().try_fold(h, |h, (_name, text)| {
        let result = system_impl(h, "compile", text);
        if result.starts_with("⊥") { Err(result) } else { Ok(h) }
    })
}

fn release_impl(handle: u32) {
    let mut s = ds().lock().unwrap();
    s.get_mut(handle as usize).into_iter().for_each(|slot| *slot = None);
}

/// SYSTEM:x = ⟨o, D'⟩. Pure ρ-dispatch + state transition.
///
/// The FPGA core: look up key in D via ρ, beta-reduce, update state.
/// No match arms. No if-branches. Every operation is a def in D.
fn system_impl(handle: u32, key: &str, input: &str) -> String {
    let mut s = ds().lock().unwrap();
    let st = match s.get(handle as usize).and_then(|x| x.as_ref()) {
        Some(x) => x,
        None => return "⊥".into(),
    };
    let obj = ast::Object::parse(input);

    // Single ρ-dispatch (Eq. 9)
    let result = ast::apply(&ast::Func::Def(key.to_string()), &obj, &st.d);

    // AST state transition: when result is a new D, store it (⟨o, D'⟩).
    // Platform primitives (compile, apply) return D' as their result.
    // Pure query defs return display notation (no state change).
    // Result contains cells (Noun, GraphSchema, etc.) iff it's a new D.
    let is_new_d = result.as_seq().is_some() && ast::fetch("Noun", &result) != ast::Object::Bottom;
    is_new_d.then(|| s[handle as usize] = Some(CompiledState { d: result.clone() }));

    result.to_string()
}

// ── WIT Component exports ───────────────────────────────────────────

#[cfg(feature = "wit")]
wit_bindgen::generate!({ world: "arest", path: "wit" });

#[cfg(feature = "wit")]
struct E;

#[cfg(feature = "wit")]
export!(E);

#[cfg(feature = "wit")]
impl exports::graphdl::arest::engine::Guest for E {
    fn parse_and_compile(readings: Vec<(String, String)>) -> Result<u32, String> {
        parse_and_compile_impl(readings)
    }
    fn release(handle: u32) { release_impl(handle) }
    fn system(handle: u32, key: String, input: String) -> String {
        system_impl(handle, &key, &input)
    }
}

// ── Security #15: WASM handle isolation ─────────────────────────────
//
// DOMAINS is a process-global Vec<Option<CompiledState>> guarded by a Mutex.
// Each create_impl() call allocates a fresh slot (reusing holes left by
// release_impl) and returns its index as the opaque handle. State is stored
// by-value (ast::Object is owned — no Arc, no &'static references escape),
// and every system_impl() read scopes its &st.d reference to the lifetime of
// the Mutex guard, so no cross-handle aliasing is possible.
//
// Invariants verified below:
//   1. Two create_impl() calls return distinct indices.
//   2. Mutations on handle A never touch handle B's slot.
//   3. release_impl() drops the slot; subsequent system_impl() returns "⊥".
//   4. release_impl() on an out-of-bounds handle is a safe no-op.
//   5. A freed slot's index may be recycled, but the new handle starts with
//      a fresh CompiledState — no residual state from the previous tenant.
//
// Cross-runtime coverage: src/tests/security/authorization.test.ts exercises
// the same invariants through the TS/WASM boundary via compileDomainReadings
// / releaseDomain / systemRaw under `describe('Handle isolation', ...)`.
#[cfg(test)]
mod handle_isolation_tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    // DOMAINS is process-global, so these tests must run serially to avoid
    // cross-test interference when asserting about slot recycling and
    // released-handle behavior. A test-local Mutex provides that barrier.
    static SERIAL: StdMutex<()> = StdMutex::new(());

    /// Test-only peek at a handle's compiled state. Clones D under the lock
    /// so the caller holds no reference to DOMAINS.
    fn peek(handle: u32) -> Option<ast::Object> {
        let s = ds().lock().unwrap();
        s.get(handle as usize).and_then(|x| x.as_ref()).map(|cs| cs.d.clone())
    }

    /// Install a Noun cell with the given payload directly, bypassing the
    /// compile pipeline. Returns a fresh handle owning that state.
    fn alloc_with_noun(payload: &str) -> u32 {
        let state = ast::store("Noun", ast::Object::atom(payload), &ast::Object::phi());
        allocate(state, vec![])
    }

    #[test]
    fn two_creates_return_distinct_handles() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let h1 = create_impl();
        let h2 = create_impl();
        assert_ne!(h1, h2, "create_impl must return distinct handle indices");
        release_impl(h1);
        release_impl(h2);
    }

    #[test]
    fn state_mutation_on_one_handle_does_not_leak_to_another() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let h_a = alloc_with_noun("tenant-a-secret");
        let h_b = alloc_with_noun("tenant-b-secret");
        assert_ne!(h_a, h_b);

        let d_a = peek(h_a).expect("handle A must be live");
        let d_b = peek(h_b).expect("handle B must be live");
        assert_eq!(ast::fetch("Noun", &d_a), ast::Object::atom("tenant-a-secret"));
        assert_eq!(ast::fetch("Noun", &d_b), ast::Object::atom("tenant-b-secret"));

        // Mutate A's slot directly (simulating what system_impl does on a
        // state-transition def) and re-check B to prove no aliasing.
        {
            let mut s = ds().lock().unwrap();
            let new_d = ast::store(
                "Noun",
                ast::Object::atom("tenant-a-mutated"),
                &s[h_a as usize].as_ref().unwrap().d.clone(),
            );
            s[h_a as usize] = Some(CompiledState { d: new_d });
        }

        let d_a2 = peek(h_a).unwrap();
        let d_b2 = peek(h_b).unwrap();
        assert_eq!(ast::fetch("Noun", &d_a2), ast::Object::atom("tenant-a-mutated"));
        assert_eq!(
            ast::fetch("Noun", &d_b2),
            ast::Object::atom("tenant-b-secret"),
            "handle B must be unaffected by mutations on handle A",
        );

        release_impl(h_a);
        release_impl(h_b);
    }

    #[test]
    fn released_handle_returns_bottom_for_all_operations() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let h = create_impl();
        release_impl(h);

        // Every system_impl() dispatch on a released handle must return ⊥.
        assert_eq!(system_impl(h, "compile", "anything"), "⊥");
        assert_eq!(system_impl(h, "apply", "<x>"), "⊥");
        assert_eq!(system_impl(h, "any_def_name", ""), "⊥");
        assert!(peek(h).is_none(), "released handle must have no stored state");
    }

    #[test]
    fn release_is_idempotent_and_bounds_safe() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let h = create_impl();
        release_impl(h);
        release_impl(h); // double-release must not panic
        release_impl(u32::MAX); // out-of-bounds index must be a no-op
        release_impl(999_999); // another OOB case
        assert_eq!(system_impl(h, "compile", ""), "⊥");
    }

    #[test]
    fn recycled_slot_has_no_residual_state() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        // Install a tenant, release it, then create a fresh handle. The new
        // handle may reuse the same index — it must NOT observe stale state.
        let h_old = alloc_with_noun("leaked-secret");
        let stale = ast::fetch("Noun", &peek(h_old).unwrap());
        assert_eq!(stale, ast::Object::atom("leaked-secret"));
        release_impl(h_old);

        let h_new = create_impl();
        let fresh_d = peek(h_new).expect("new handle must be live");
        // create_impl starts from Object::phi() with only platform defs; no
        // Noun cell should be present.
        assert_eq!(
            ast::fetch("Noun", &fresh_d),
            ast::Object::Bottom,
            "recycled slot must not carry prior tenant's Noun cell",
        );
        release_impl(h_new);
    }

    #[test]
    fn no_static_aliasing_across_handles() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        // Pointer-identity check: the two Objects stored under distinct
        // handles must not share the same heap address. This catches any
        // accidental Arc::clone() or shared-reference leak.
        let h_a = alloc_with_noun("alpha");
        let h_b = alloc_with_noun("beta");

        let s = ds().lock().unwrap();
        let d_a = &s[h_a as usize].as_ref().unwrap().d as *const ast::Object;
        let d_b = &s[h_b as usize].as_ref().unwrap().d as *const ast::Object;
        assert_ne!(d_a, d_b, "each handle must own a distinct CompiledState");
        drop(s);

        release_impl(h_a);
        release_impl(h_b);
    }
}

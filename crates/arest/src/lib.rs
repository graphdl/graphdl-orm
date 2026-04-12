// crates/arest/src/lib.rs
//
// AREST: Applicative REpresentational State Transfer
//
// SYSTEM:x = <o, D'>
// One function. Readings in, applications out.
// State = P (facts) + DEFS (named Func).

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
pub mod command;
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

#[allow(dead_code)] // used by tests and the cloudflare feature
fn allocate(state: ast::Object, defs: Vec<(String, ast::Func)>) -> u32 {
    let d = ast::defs_to_state(&defs, &state);
    let mut s = ds().lock().unwrap();
    let h = s.iter().position(|x| x.is_none()).unwrap_or_else(|| { s.push(None); s.len() - 1 });
    s[h] = Some(CompiledState { d });
    h as u32
}

// ── SYSTEM is the only function ─────────────────────────────────────

/// Bundled metamodel readings. Compiled into the binary at build time.
/// Loaded by `create_impl` so every fresh domain starts with the
/// self-describing metamodel available. Use `create_bare_impl` to skip
/// the auto-load when experimenting with a replacement core.
///
/// Load order matters: core defines the base object types (Noun, Graph
/// Schema, Role, Constraint) that every later reading references.
const METAMODEL_READINGS: &[(&str, &str)] = &[
    ("core",          include_str!("../../../readings/core.md")),
    ("state",         include_str!("../../../readings/state.md")),
    ("instances",     include_str!("../../../readings/instances.md")),
    ("outcomes",      include_str!("../../../readings/outcomes.md")),
    ("validation",    include_str!("../../../readings/validation.md")),
    ("evolution",     include_str!("../../../readings/evolution.md")),
    ("organizations", include_str!("../../../readings/organizations.md")),
    ("agents",        include_str!("../../../readings/agents.md")),
    ("ui",            include_str!("../../../readings/ui.md")),
];

/// create_bare: allocate empty D with ONLY the platform primitives
/// registered in DEFS. Use this when testing a new core or rebuilding
/// the metamodel from scratch. Most apps should use `create_impl`.
#[allow(dead_code)] // used by tests and the cloudflare feature
fn create_bare_impl() -> u32 {
    let state = ast::Object::phi();
    let defs = vec![
        ("compile".to_string(), ast::Func::Platform("compile".to_string())),
        ("apply".to_string(), ast::Func::Platform("apply_command".to_string())),
        ("verify_signature".to_string(), ast::Func::Platform("verify_signature".to_string())),
        ("audit".to_string(), ast::Func::Platform("audit".to_string())),
    ];
    allocate(state, defs)
}

/// create: allocate D with platform primitives AND the bundled metamodel
/// readings (core, state, instances, outcomes, validation, evolution,
/// organizations, agents, ui). One call yields a fully self-describing
/// engine ready to ingest user domain readings via `system(h, "compile", ...)`.
///
/// Use `create_bare_impl` to opt out when experimenting with a new core.
/// Cached metamodel state — PARSED cells + platform primitives only.
///
/// We deliberately skip `compile_to_defs_state` at cache build time because
/// `platform_compile` already runs it on every user compile, taking the
/// metamodel cells as context. Pre-compiling would be wasted work and slows
/// down `create_impl` by seconds. The expensive per-def construction (CWA
/// negation, per-constraint validate funcs, query/schema/resolve defs)
/// happens lazily on first user compile.
///
/// What IS in the cache:
///   - Metamodel Noun cell (self-describing types)
///   - Metamodel Fact Type cell
///   - Metamodel Role cell
///   - Metamodel Constraint cell
///   - 3 platform primitive defs (compile, apply, verify_signature)
///
/// Bootstrap mode (#23 guard bypass) wraps the parse fold.
static METAMODEL_STATE: OnceLock<ast::Object> = OnceLock::new();

fn metamodel_state() -> &'static ast::Object {
    METAMODEL_STATE.get_or_init(|| {
        struct BootstrapGuard;
        impl BootstrapGuard {
            fn enter() -> Self {
                parse_forml2::set_bootstrap_mode(true);
                BootstrapGuard
            }
        }
        impl Drop for BootstrapGuard {
            fn drop(&mut self) { parse_forml2::set_bootstrap_mode(false); }
        }
        let _guard = BootstrapGuard::enter();

        // Fold all 9 readings into a single merged state (parser only).
        let merged = METAMODEL_READINGS.iter().fold(ast::Object::phi(), |acc, (name, text)| {
            let parsed = parse_forml2::parse_to_state_from(text, &acc)
                .unwrap_or_else(|e| panic!("metamodel parse failed at readings/{}.md: {}", name, e));
            ast::merge_states(&acc, &parsed)
        });

        // Register ONLY the platform primitives — no constraint/query/derivation
        // compilation here. `platform_compile` does the full compile_to_defs_state
        // on every user compile and will pick up the metamodel cells naturally.
        let defs = vec![
            ("compile".to_string(), ast::Func::Platform("compile".to_string())),
            ("apply".to_string(), ast::Func::Platform("apply_command".to_string())),
            ("verify_signature".to_string(), ast::Func::Platform("verify_signature".to_string())),
            ("audit".to_string(), ast::Func::Platform("audit".to_string())),
        ];
        ast::defs_to_state(&defs, &merged)
    })
}

fn create_impl() -> u32 {
    // Clone the cached metamodel state into a fresh handle. First call
    // builds the cache; subsequent calls are just a handle allocation +
    // Object clone.
    let d = metamodel_state().clone();
    let mut s = ds().lock().unwrap();
    let h = s.iter().position(|x| x.is_none()).unwrap_or_else(|| { s.push(None); s.len() - 1 });
    s[h] = Some(CompiledState { d });
    h as u32
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

    // AST state transition (⟨o, D'⟩) — three result shapes to handle:
    //
    // 1. CommandResult carrier: Object::Map with __state + __result keys,
    //    emitted by encode_command_result for create/update/transition.
    //    Persist __state into the handle; return the __result JSON atom.
    //
    // 2. New D directly: a store (Map or Seq) containing a Noun cell —
    //    emitted by platform_compile. Persist it; return the default
    //    display representation (FFP). Callers that need JSON should use
    //    dedicated defs (debug, list:{noun}, get:{noun}, query:{ft_id}).
    //
    // 3. Anything else: a pure query result. Return display, don't persist.
    if let Some(map) = result.as_map() {
        if map.contains_key("__state") && map.contains_key("__result") {
            let new_state = map.get("__state").cloned().unwrap_or(ast::Object::Bottom);
            let response = map.get("__result").cloned().unwrap_or(ast::Object::Bottom);
            let new_state_looks_valid =
                (new_state.as_map().is_some() || new_state.as_seq().is_some())
                && ast::fetch("Noun", &new_state) != ast::Object::Bottom;
            if new_state_looks_valid {
                s[handle as usize] = Some(CompiledState { d: new_state });
            }
            return response.as_atom().map(|s| s.to_string())
                .unwrap_or_else(|| response.to_string());
        }
    }
    let looks_like_store = result.as_seq().is_some() || result.as_map().is_some();
    let is_new_d = looks_like_store && ast::fetch("Noun", &result) != ast::Object::Bottom;

    if is_new_d {
        // Platform compile returns a full D. Persist it; respond with a
        // tiny JSON summary rather than dumping the store (which can be
        // megabytes at realistic scale). Call `debug` to query the full
        // schema if needed.
        s[handle as usize] = Some(CompiledState { d: result.clone() });
        return r#"{"ok":true,"compiled":true}"#.to_string();
    }

    // Non-D results: serialize as JSON. Atoms that already parse as JSON
    // are passed through; other atoms become JSON strings; seqs → arrays;
    // maps → objects; bottom → null. MCP and HTTP callers can always
    // JSON.parse the response.
    result.to_json_string()
}

// ── WIT Component exports ───────────────────────────────────────────

#[cfg(feature = "wit")]
wit_bindgen::generate!({ world: "arest", path: "wit" });

#[cfg(feature = "wit")]
struct E;

#[cfg(feature = "wit")]
export!(E);

#[cfg(feature = "wit")]
impl exports::arest::engine::engine::Guest for E {
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
        let h1 = create_bare_impl();
        let h2 = create_bare_impl();
        assert_ne!(h1, h2, "create must return distinct handle indices");
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
        let h = create_bare_impl();
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
        let h = create_bare_impl();
        release_impl(h);
        release_impl(h); // double-release must not panic
        release_impl(u32::MAX); // out-of-bounds index must be a no-op
        release_impl(999_999); // another OOB case
        assert_eq!(system_impl(h, "compile", ""), "⊥");
    }

    #[test]
    fn recycled_slot_has_no_residual_state() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        // Install a tenant, release it, then create a fresh bare handle.
        // The new handle may reuse the same index — it must NOT observe
        // stale state from the previous tenant.
        let h_old = alloc_with_noun("leaked-secret");
        let stale = ast::fetch("Noun", &peek(h_old).unwrap());
        assert_eq!(stale, ast::Object::atom("leaked-secret"));
        release_impl(h_old);

        let h_new = create_bare_impl();
        let fresh_d = peek(h_new).expect("new handle must be live");
        // create_bare_impl starts from Object::phi() with only platform
        // defs; no Noun cell should be present.
        assert_eq!(
            ast::fetch("Noun", &fresh_d),
            ast::Object::Bottom,
            "recycled bare slot must not carry prior tenant's Noun cell",
        );
        release_impl(h_new);
    }

    /// create_impl loads the bundled metamodel, so a fresh handle MUST
    /// have a populated Noun cell (from core.md at minimum).
    #[test]
    fn create_impl_loads_metamodel() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let h = create_impl();
        let d = peek(h).expect("handle must be live");
        let nouns = ast::fetch("Noun", &d);
        assert_ne!(nouns, ast::Object::Bottom,
            "create_impl must load the metamodel — Noun cell should be populated");
        // The metamodel defines at least Noun, Fact Type, Role, Constraint
        // as reserved noun names. Verify the cell has multiple entries.
        let count = nouns.as_seq().map(|s| s.len()).unwrap_or(0);
        assert!(count > 5,
            "metamodel should populate at least a handful of noun entries, got {}", count);
        release_impl(h);
    }

    #[test]
    fn create_bare_impl_skips_metamodel() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let h = create_bare_impl();
        let d = peek(h).expect("handle must be live");
        // Bare mode: no Noun cell, no metamodel facts at all — just the
        // three platform primitives.
        assert_eq!(ast::fetch("Noun", &d), ast::Object::Bottom,
            "create_bare_impl must NOT load the metamodel");
        release_impl(h);
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

    /// #108: `audit_log` must be reachable as a system def — return the
    /// audit trail as a JSON array, and each entry for an entity-scoped
    /// apply must carry the entity id so `explain` can filter by it.
    #[test]
    fn audit_log_reachable_via_system_and_carries_entity_id() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let h = create_impl();

        let _ = system_impl(h, "compile", "\
Order(.id) is an entity type.
Order has total.
");
        let create_out = system_impl(h, "create:Order", "<<id, audit-ord-1>, <total, 7>>");
        assert!(!create_out.starts_with('⊥'), "create:Order must succeed, got: {create_out}");

        // Pass "0" as the (unused) input because apply() short-circuits on
        // Object::Bottom — an empty string parses to ⊥. The def is named
        // `audit` (not `audit_log`) to avoid shadowing the `audit_log` data
        // cell that cell_push overwrites on every create.
        let audit_out = system_impl(h, "audit", "0");
        assert!(!audit_out.starts_with('⊥'),
            "system('audit', '0') must not return ⊥; got: {audit_out}");
        assert!(audit_out.starts_with('['),
            "audit must return a JSON array; got: {audit_out}");
        assert!(audit_out.contains("apply:create"),
            "audit must record the apply:create operation; got: {audit_out}");
        assert!(audit_out.contains("audit-ord-1"),
            "audit entries for entity-scoped applies must carry the entity id; got: {audit_out}");

        release_impl(h);
    }

    /// #107: After `create:Order` adds an entity to D via apply, both
    /// `list:Order` and `get:Order` must see it. Currently those defs
    /// are compile-time constants baked from Instance Facts, so they
    /// never observe runtime-created entities.
    ///
    /// Per whitepaper Eq 9: SYSTEM:x = (ρ(↑entity(x):D)):↑op(x). The
    /// read path is a ρ-application that fetches from the live D.
    #[test]
    fn list_and_get_see_runtime_created_entities() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let h = create_impl();

        let readings = "\
Order(.id) is an entity type.
Order has total.
  Each Order has at most one total.
";
        let compile_out = system_impl(h, "compile", readings);
        assert!(!compile_out.starts_with('⊥'),
            "compile must not reject simple Order schema, got: {compile_out}");

        let create_out = system_impl(h, "create:Order", "<<id, ord-1>, <total, 100>>");
        assert!(!create_out.starts_with('⊥'),
            "create:Order must not return ⊥, got: {create_out}");

        let list_out = system_impl(h, "list:Order", "");
        assert!(!list_out.starts_with('⊥'),
            "list:Order must not return ⊥ after an entity has been created");
        assert!(list_out.contains("ord-1"),
            "list:Order must surface the runtime-created entity 'ord-1'; got: {list_out}");

        let get_out = system_impl(h, "get:Order", "ord-1");
        assert!(!get_out.starts_with('⊥'),
            "get:Order must not return ⊥ for an entity that was just created");
        assert!(get_out.contains("ord-1"),
            "get:Order must return a payload containing the entity id; got: {get_out}");

        release_impl(h);
    }
}

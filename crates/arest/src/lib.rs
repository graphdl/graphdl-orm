// crates/arest/src/lib.rs
//
// AREST: Applicative REpresentational State Transfer
//
// SYSTEM:x = <o, D'>
// One function. Readings in, applications out.
// State = P (facts) + DEFS (named Func).

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::OnceLock;
use std::sync::RwLock;

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
pub mod quota;
pub mod scheduler;

#[cfg(feature = "wasm-lower")]
pub mod wasm_lower;

#[cfg(feature = "cloudflare")]
pub mod cloudflare;

/// D: the unified state — population cells + def cells, split into
/// per-cell `Arc<RwLock<Object>>`. Backus Sec. 14.3 state-as-cells,
/// but with each cell independently lockable so disjoint writers
/// don't serialize through a single tenant-wide lock.
///
/// Access patterns:
///   - Reads:  `snapshot_d(&self)` builds a consistent Object::Map
///             view by acquiring every cell's read lock briefly.
///   - Whole-state writes (compile, rollback): `replace_d(&mut self,
///             new_d)` rebuilds the cells map. Requires the outer
///             `RwLock<CompiledState>::write()` guard.
///   - Targeted writes (create/update/transition): `try_commit_diff(
///             &self, snapshot, new_d)` acquires per-cell write
///             locks for only the cells that changed. CAS-checks
///             each against the snapshot; returns an error if any
///             cell changed meanwhile (caller retries) or if new
///             cells must be added (caller escalates to `write()`).
///             Needs only the outer `read()` guard, so two disjoint-
///             cell writers run in parallel.
///
/// `snapshots` holds named captures of `d` taken via `system(h,
/// "snapshot", "")` and restorable via `system(h, "rollback", id)`.
/// Cheap in memory because cells share `Arc` storage — a snapshot
/// is one map insert plus an Arc ref bump per cell.
struct CompiledState {
    cells: std::collections::HashMap<String, Arc<RwLock<ast::Object>>>,
    snapshots: std::collections::HashMap<String, ast::Object>,
}

/// Outcome of a targeted-write attempt via `try_commit_diff`.
enum CommitOutcome {
    /// All cell-level CAS checks passed; the writes have been applied.
    Committed,
    /// One or more cells changed between snapshot and commit. The
    /// caller should re-snapshot, re-run apply(), and retry.
    StaleSnapshot,
    /// The new state introduces cells that don't exist yet, or
    /// removes existing cells. The cells map itself must be mutated,
    /// which requires the outer write guard. Caller should escalate.
    StructuralChange,
}

impl CompiledState {
    fn new(initial_d: ast::Object) -> Self {
        let mut s = Self {
            cells: std::collections::HashMap::new(),
            snapshots: std::collections::HashMap::new(),
        };
        s.replace_d(initial_d);
        s
    }

    /// Assemble an `Object::Map` view of the full state. Each cell's
    /// read lock is held briefly for the clone; readers don't block
    /// each other, but a writer on that cell will block the snapshot
    /// momentarily.
    fn snapshot_d(&self) -> ast::Object {
        let mut map = std::collections::HashMap::with_capacity(self.cells.len());
        for (name, lock) in &self.cells {
            map.insert(name.clone(), lock.read().unwrap().clone());
        }
        ast::Object::Map(map)
    }

    /// Wholesale rebuild the cell map from a new D. Reuses existing
    /// cell locks where possible (so concurrent readers still see a
    /// live `Arc<RwLock>` rather than a freed one), then prunes any
    /// cells absent from the new state.
    fn replace_d(&mut self, new_d: ast::Object) {
        let new_map: std::collections::HashMap<String, ast::Object> = match new_d {
            ast::Object::Map(m) => m,
            ast::Object::Seq(seq) => {
                // CELL-triple representation: <<CELL, name, contents>, …>.
                // Fall through to an empty map if the shape doesn't match.
                let mut m = std::collections::HashMap::new();
                for cell in seq.iter() {
                    if let Some(items) = cell.as_seq() {
                        if items.len() == 3 {
                            if let (Some(_), Some(name)) = (
                                items[0].as_atom(),
                                items[1].as_atom(),
                            ) {
                                m.insert(name.to_string(), items[2].clone());
                            }
                        }
                    }
                }
                m
            }
            ast::Object::Bottom => std::collections::HashMap::new(),
            other => {
                // Unexpected shape — store the whole thing under a
                // sentinel cell so we don't silently drop it.
                let mut m = std::collections::HashMap::new();
                m.insert("__unshaped__".to_string(), other);
                m
            }
        };
        // Reuse existing locks where possible; replace contents under
        // the per-cell write lock.
        let mut next_cells: std::collections::HashMap<String, Arc<RwLock<ast::Object>>> =
            std::collections::HashMap::with_capacity(new_map.len());
        for (name, value) in new_map {
            match self.cells.remove(&name) {
                Some(existing) => {
                    *existing.write().unwrap() = value;
                    next_cells.insert(name, existing);
                }
                None => {
                    next_cells.insert(name, Arc::new(RwLock::new(value)));
                }
            }
        }
        // Any cell still in self.cells was removed by the new state;
        // dropped implicitly.
        self.cells = next_cells;
    }

    /// Targeted commit: write only the cells whose contents differ
    /// between `snapshot` (what apply() saw) and `new_d` (what apply()
    /// returned). Each changed cell is CAS-checked against the
    /// snapshot value before writing to detect stale snapshots.
    ///
    /// Requires only `&self` because the cells-map structure isn't
    /// mutated — only per-cell contents. Callers should therefore
    /// hold `RwLock<CompiledState>::read()`, which lets concurrent
    /// writers to disjoint cells proceed without contending on the
    /// outer lock.
    ///
    /// Returns `Committed` on success, `StaleSnapshot` when another
    /// writer modified one of the target cells between snapshot and
    /// commit (caller should retry), or `StructuralChange` when new
    /// cells must be introduced or existing cells removed (caller
    /// must escalate to `write()` and use `replace_d`).
    fn try_commit_diff(&self, snapshot: &ast::Object, new_d: &ast::Object) -> CommitOutcome {
        let snap_map = match snapshot.as_map() {
            Some(m) => m,
            None => return CommitOutcome::StructuralChange,
        };
        let new_map = match new_d.as_map() {
            Some(m) => m,
            None => return CommitOutcome::StructuralChange,
        };
        // Detect structural change: added or removed cells require
        // the outer write lock to mutate the cells map.
        for key in new_map.keys() {
            if !self.cells.contains_key(key) {
                return CommitOutcome::StructuralChange;
            }
        }
        for key in self.cells.keys() {
            if !new_map.contains_key(key) {
                return CommitOutcome::StructuralChange;
            }
        }
        // Collect changed cells.
        let mut changed: Vec<&String> = new_map
            .iter()
            .filter(|(k, v)| snap_map.get(*k) != Some(*v))
            .map(|(k, _)| k)
            .collect();
        if changed.is_empty() {
            return CommitOutcome::Committed; // no-op
        }
        // Sort for deterministic lock acquisition (deadlock avoidance
        // between concurrent writers with overlapping cell sets).
        changed.sort();
        // Acquire write locks in order.
        let mut guards: Vec<(&String, std::sync::RwLockWriteGuard<'_, ast::Object>)> =
            Vec::with_capacity(changed.len());
        for key in changed {
            let lock = self.cells.get(key).expect("membership was checked above");
            let guard = lock.write().unwrap();
            guards.push((key, guard));
        }
        // CAS: every changed cell's current contents must still match
        // the snapshot; otherwise someone committed under us.
        for (key, guard) in &guards {
            let expected = snap_map.get(*key);
            if Some(&**guard) != expected {
                return CommitOutcome::StaleSnapshot;
            }
        }
        // Apply the writes under the already-held guards.
        for (key, guard) in guards.iter_mut() {
            let new_value = new_map.get(*key).expect("membership was checked above").clone();
            **guard = new_value;
        }
        CommitOutcome::Committed
    }
}

// The per-handle process table:
//
// Outer Mutex protects slot allocation/recycling (Vec mutations).
// Inner RwLock<CompiledState> protects per-tenant state, held only
// for the duration of one operation. Two tenants run concurrently —
// the outer lock is held only for slot lookup, then dropped; the
// inner lock is per-Arc, so different tenants don't contend.
//
// This realises Cell Isolation (Definition 2) at the per-tenant
// granularity. Per-cell concurrency within a tenant is a follow-up
// that needs apply() to acquire cell-level locks just-in-time.
static DOMAINS: OnceLock<Mutex<Vec<Option<Arc<RwLock<CompiledState>>>>>> = OnceLock::new();
fn ds() -> &'static Mutex<Vec<Option<Arc<RwLock<CompiledState>>>>> {
    DOMAINS.get_or_init(|| Mutex::new(Vec::new()))
}

/// Look up a slot's tenant lock by handle. Returns None for invalid
/// handles or freed slots. The outer Vec mutex is held only for the
/// duration of the lookup and Arc clone, then released.
fn tenant_lock(handle: u32) -> Option<Arc<RwLock<CompiledState>>> {
    let s = ds().lock().unwrap();
    s.get(handle as usize).and_then(|x| x.as_ref()).map(Arc::clone)
}

#[allow(dead_code)] // used by tests and the cloudflare feature
fn allocate(state: ast::Object, defs: Vec<(String, ast::Func)>) -> u32 {
    let d = ast::defs_to_state(&defs, &state);
    let mut s = ds().lock().unwrap();
    let h = s.iter().position(|x| x.is_none()).unwrap_or_else(|| { s.push(None); s.len() - 1 });
    s[h] = Some(Arc::new(RwLock::new(CompiledState::new(d))));
    h as u32
}

// ── SYSTEM is the only function ─────────────────────────────────────

/// Bundled metamodel readings. Compiled into the binary at build time.
/// Loaded by `create_impl` so every fresh domain starts with the
/// self-describing metamodel available. Use `create_bare_impl` to skip
/// the auto-load when experimenting with a replacement core.
///
/// Load order matters: core defines the base object types (Noun, Fact
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

        // Compile the metamodel once and bake the full def set into the
        // cached state. With `Object::Seq(Arc<[Object]>)`, cloning this
        // fat state on every `create_impl` is a ref-count bump per cell
        // instead of a deep Object copy — the cost that blocked the
        // previous baked-defs attempt is gone.
        //
        // Fresh handles now start with all metamodel constraint /
        // derivation / per-noun-validate defs already compiled; the
        // first `compile` command on a new handle incurs zero
        // metamodel re-compile cost. User readings still trigger a
        // full recompile when added (future optimization: splitting
        // the compile pipeline so the metamodel pass is a no-op when
        // the cached defs are already present).
        let mut defs = crate::compile::compile_to_defs_state(&merged);
        defs.extend([
            ("compile".to_string(), ast::Func::Platform("compile".to_string())),
            ("apply".to_string(), ast::Func::Platform("apply_command".to_string())),
            ("verify_signature".to_string(), ast::Func::Platform("verify_signature".to_string())),
            ("audit".to_string(), ast::Func::Platform("audit".to_string())),
        ]);
        ast::defs_to_state(&defs, &merged)
    })
}

fn create_impl() -> u32 {
    // Clone the cached metamodel state into a fresh handle. First call
    // builds the cache (parses 9 metamodel readings + runs the full
    // compile pipeline to bake every constraint/derivation/per-noun-
    // validate def into the state); subsequent calls are just a handle
    // allocation + Object clone.
    //
    // The clone is cheap because Object::Seq is Arc<[Object]> — each
    // cell clone is a ref-count bump, not a deep copy. Before the Arc
    // refactor, an earlier attempt at baking defs into this cache was
    // slower net because the ~MB state paid a Vec deep-copy per handle
    // create. With Arc-sharing that tax is gone and the baked-defs
    // approach lands naturally: new handles start with zero metamodel
    // compile cost.
    let d = metamodel_state().clone();
    let mut s = ds().lock().unwrap();
    let h = s.iter().position(|x| x.is_none()).unwrap_or_else(|| { s.push(None); s.len() - 1 });
    s[h] = Some(Arc::new(RwLock::new(CompiledState::new(d))));
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

/// Classify an op as read-only by key prefix. Read-only ops take the
/// per-tenant RwLock in shared (read) mode, so two concurrent `list:X`
/// or `debug` calls on the same handle don't block each other.
///
/// Conservative list — when in doubt, a key falls through to the write
/// path, which is still correct (just serializes). Extending this list
/// is the right way to unlock more per-tenant concurrency.
fn is_read_only_op(key: &str) -> bool {
    matches!(
        key,
        "debug" | "audit" | "verify_signature" | "snapshots"
    )
    || key.starts_with("list:")
    || key.starts_with("get:")
    || key.starts_with("query:")
    || key.starts_with("explain:")
}

/// SYSTEM:x = ⟨o, D'⟩. Pure ρ-dispatch + state transition.
///
/// The FPGA core: look up key in D via ρ, beta-reduce, update state.
/// No match arms. No if-branches. Every operation is a def in D.
///
/// Concurrency:
///   - Outer process-table mutex: held briefly to clone the per-tenant
///     Arc<RwLock<CompiledState>>. Two tenants run concurrently.
///   - Per-tenant RwLock: read-only ops take `read()` (shared);
///     write-path ops take `write()` (exclusive). Licenses Definition 2
///     at the tenant granularity — parallel queries on a handle don't
///     contend with each other, only with writers. Full per-cell
///     concurrency (parallel disjoint writes within one handle) is a
///     follow-up; it needs apply() to acquire cell locks just-in-time.
fn system_impl(handle: u32, key: &str, input: &str) -> String {
    let tenant = match tenant_lock(handle) {
        Some(t) => t,
        None => return "⊥".into(),
    };

    // ── CompiledState-level intercepts ──────────────────────────────
    //
    // `snapshot` and `rollback` mutate the tenant's snapshot map or
    // replace `d`; they need a write lock. `snapshots` only reads the
    // map and can share with concurrent readers.
    //
    //   system(h, "snapshot", "")      → <snap-id>                (fresh id)
    //   system(h, "snapshot", "label") → label                    (caller-named)
    //   system(h, "rollback", "id")    → id on success, ⊥ on miss
    //   system(h, "snapshots", "")     → <id₁, id₂, ...> FFP seq
    if key == "snapshot" {
        let mut st = tenant.write().unwrap();
        let label = if input.is_empty() {
            format!("snap-{}", st.snapshots.len())
        } else {
            input.to_string()
        };
        let snap = st.snapshot_d();
        st.snapshots.insert(label.clone(), snap);
        return label;
    }
    if key == "rollback" {
        let mut st = tenant.write().unwrap();
        return match st.snapshots.get(input).cloned() {
            Some(snap) => {
                st.replace_d(snap);
                input.to_string()
            }
            None => "⊥".into(),
        };
    }
    if key == "snapshots" {
        let st = tenant.read().unwrap();
        let mut ids: Vec<&String> = st.snapshots.keys().collect();
        ids.sort();
        return format!(
            "<{}>",
            ids.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")
        );
    }

    // ── Read-only dispatch path ─────────────────────────────────────
    //
    // Known-read ops (list / get / query / debug / audit / explain /
    // verify_signature) take a shared lock. Result can never be a
    // "new D"; if apply() somehow returns a store-shaped Object for
    // one of these keys we silently don't persist it — that's a bug
    // in the op's definition, not a concurrency issue.
    if is_read_only_op(key) {
        let st = tenant.read().unwrap();
        let obj = ast::Object::parse(input);
        let snapshot = st.snapshot_d();
        let result = ast::apply(&ast::Func::Def(key.to_string()), &obj, &snapshot);
        return result.to_json_string();
    }

    // ── Write dispatch path ─────────────────────────────────────────
    //
    // Two-tier commit:
    //   Tier 1 (shared-lock fast path): acquire tenant.read(), snapshot,
    //     apply, classify the result, and try `try_commit_diff` — this
    //     writes only the cells whose contents actually changed, each
    //     under its own per-cell write lock. Two disjoint-cell writers
    //     run in parallel; same-cell writers serialize on the cell lock.
    //   Tier 2 (exclusive-lock escalation): on Stale/Structural outcome,
    //     drop the read, take tenant.write(), re-snapshot + re-apply +
    //     `replace_d`. Structural = new or removed cells; Stale = a
    //     concurrent writer's CAS check detected that our snapshot is
    //     no longer current.
    //
    // Re-running apply() on the escalated path is idempotent: apply is
    // functional on `&Object`; the cost is the second evaluation, paid
    // only on contention.
    let obj = ast::Object::parse(input);

    // Tier 1: shared-lock fast path.
    {
        let st = tenant.read().unwrap();
        let snapshot = st.snapshot_d();
        let apply_result = ast::apply(&ast::Func::Def(key.to_string()), &obj, &snapshot);
        match classify_writer_result(&apply_result) {
            WriterResult::NoCommit { response } => return response,
            WriterResult::Commit { ref new_d, .. } => {
                match st.try_commit_diff(&snapshot, new_d) {
                    CommitOutcome::Committed => {
                        if let WriterResult::Commit { response, .. } = classify_writer_result(&apply_result) {
                            return response;
                        }
                        unreachable!("classify is deterministic");
                    }
                    CommitOutcome::StaleSnapshot | CommitOutcome::StructuralChange => {
                        // fall through to Tier 2
                    }
                }
            }
        }
    }

    // Tier 2: exclusive-lock escalation.
    let mut st = tenant.write().unwrap();
    let snapshot = st.snapshot_d();
    let apply_result = ast::apply(&ast::Func::Def(key.to_string()), &obj, &snapshot);
    match classify_writer_result(&apply_result) {
        WriterResult::NoCommit { response } => response,
        WriterResult::Commit { new_d, response } => {
            st.replace_d(new_d);
            response
        }
    }
}

/// Outcome of classifying an `ast::apply` result in the write path.
enum WriterResult {
    /// Result is a full new D (possibly extracted from a `__state`
    /// carrier), to be persisted.
    Commit { new_d: ast::Object, response: String },
    /// Result is a query / non-D response; nothing to persist.
    NoCommit { response: String },
}

/// Classify an apply() result according to the three writer-path
/// shapes the system recognises. Pure: no tenant mutation; callers
/// decide whether to commit under Tier-1 or Tier-2 locks.
///
/// Shapes:
///   1. CommandResult carrier `{__state, __result}` — used by
///      create/update/transition. Commit __state if it looks like a
///      valid store; return __result as the response.
///   2. Bare store with a Noun cell — used by platform_compile.
///      Commit the result; return a compact summary.
///   3. Anything else — pure query result; return as JSON.
fn classify_writer_result(result: &ast::Object) -> WriterResult {
    if let Some(map) = result.as_map() {
        if map.contains_key("__state") && map.contains_key("__result") {
            let new_state = map.get("__state").cloned().unwrap_or(ast::Object::Bottom);
            let response_obj = map.get("__result").cloned().unwrap_or(ast::Object::Bottom);
            let response = response_obj.as_atom().map(|s| s.to_string())
                .unwrap_or_else(|| response_obj.to_string());
            let valid = (new_state.as_map().is_some() || new_state.as_seq().is_some())
                && ast::fetch("Noun", &new_state) != ast::Object::Bottom;
            if valid {
                return WriterResult::Commit { new_d: new_state, response };
            }
            return WriterResult::NoCommit { response };
        }
    }
    let looks_like_store = result.as_seq().is_some() || result.as_map().is_some();
    let is_new_d = looks_like_store
        && ast::fetch("Noun", result) != ast::Object::Bottom;
    if is_new_d {
        return WriterResult::Commit {
            new_d: result.clone(),
            response: r#"{"ok":true,"compiled":true}"#.to_string(),
        };
    }
    WriterResult::NoCommit { response: result.to_json_string() }
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
// and every system_impl() read scopes its snapshot to the lifetime of
// the Mutex guard, so no cross-handle aliasing is possible.
//
// Invariants verified below:
//   1. Two create_impl() calls return distinct indices.
//   2. Mutations on handle A never touch handle B's slot.
//   3. An invalid handle (never-allocated, out-of-bounds) returns ⊥ from
//      every system_impl() dispatch and has no stored state. Released
//      handles' slot contents are not asserted directly because slot
//      recycling races with parallel tests — the freshness invariant (5)
//      is the real guarantee.
//   4. release_impl() on any handle — live, recently freed, or out of
//      bounds — is a safe no-op and never panics.
//   5. A freed slot's index may be recycled, but the new handle starts with
//      a fresh CompiledState — no residual state from the previous tenant.
//
// Cross-runtime coverage: src/tests/security/authorization.test.ts exercises
// the same invariants through the TS/WASM boundary via compileDomainReadings
// / releaseDomain / systemRaw under `describe('Handle isolation', ...)`.
#[cfg(test)]
mod handle_isolation_tests {
    use super::*;

    /// Test-only peek at a handle's compiled state. Takes a shared
    /// (read) lock and assembles an Object::Map snapshot from the
    /// per-cell locks; no DOMAINS / tenant references held after
    /// return. Read-only; doesn't block other readers on the same
    /// handle.
    fn peek(handle: u32) -> Option<ast::Object> {
        let tenant = tenant_lock(handle)?;
        let st = tenant.read().unwrap();
        Some(st.snapshot_d())
    }

    /// Install a Noun cell with the given payload directly, bypassing the
    /// compile pipeline. Returns a fresh handle owning that state.
    fn alloc_with_noun(payload: &str) -> u32 {
        let state = ast::store("Noun", ast::Object::atom(payload), &ast::Object::phi());
        allocate(state, vec![])
    }

    #[test]
    fn two_creates_return_distinct_handles() {
        let h1 = create_bare_impl();
        let h2 = create_bare_impl();
        assert_ne!(h1, h2, "create must return distinct handle indices");
        release_impl(h1);
        release_impl(h2);
    }

    #[test]
    fn state_mutation_on_one_handle_does_not_leak_to_another() {
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
            let tenant_a = tenant_lock(h_a).expect("handle A must be live");
            let mut st = tenant_a.write().unwrap();
            let snapshot = st.snapshot_d();
            let new_d = ast::store(
                "Noun",
                ast::Object::atom("tenant-a-mutated"),
                &snapshot,
            );
            st.replace_d(new_d);
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
    fn invalid_handle_returns_bottom_for_all_operations() {
        // u32::MAX is beyond any allocation (Vec<CompiledState> never grows
        // that large), so the slot is guaranteed absent. A released handle's
        // slot may be recycled by a parallel test before we read it, so
        // asserting ⊥ post-release races with the allocator. u32::MAX dodges
        // that entirely while covering the same invariant: any handle not
        // currently owning a live slot returns ⊥ from every system dispatch.
        let h = u32::MAX;
        assert_eq!(system_impl(h, "compile", "anything"), "⊥");
        assert_eq!(system_impl(h, "apply", "<x>"), "⊥");
        assert_eq!(system_impl(h, "any_def_name", ""), "⊥");
        assert!(peek(h).is_none(), "invalid handle must have no stored state");
    }

    #[test]
    fn release_is_idempotent_and_bounds_safe() {
        // The safety property is "release never panics" — on a live slot,
        // a recently-freed slot, or a completely-out-of-bounds index. A
        // slot's post-release content is covered by the invalid_handle
        // test above; asserting it here races with recycling under
        // cargo's default parallel test runner.
        let h = create_bare_impl();
        release_impl(h);
        release_impl(h); // double-release
        release_impl(u32::MAX);
        release_impl(999_999);
    }

    #[test]
    fn recycled_slot_has_no_residual_state() {
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
        // Pointer-identity check: the two tenants stored under distinct
        // handles must not share the same Arc — distinct allocations.
        // The per-tenant inner Mutex is per-Arc; if the Arcs aliased,
        // tenant A's lock would also block tenant B.
        let h_a = alloc_with_noun("alpha");
        let h_b = alloc_with_noun("beta");

        let arc_a = tenant_lock(h_a).expect("h_a must be live");
        let arc_b = tenant_lock(h_b).expect("h_b must be live");
        assert!(!Arc::ptr_eq(&arc_a, &arc_b),
            "each handle must own a distinct tenant Arc<RwLock<CompiledState>>");

        release_impl(h_a);
        release_impl(h_b);
    }

    /// `audit_log` must be reachable as a system def — return the
    /// audit trail as a JSON array, and each entry for an entity-scoped
    /// apply must carry the entity id so `explain` can filter by it.
    #[test]
    fn audit_log_reachable_via_system_and_carries_entity_id() {
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

    /// After `create:Order` adds an entity to D via apply, both
    /// `list:Order` and `get:Order` must see it. Currently those defs
    /// are compile-time constants baked from Instance Facts, so they
    /// never observe runtime-created entities.
    ///
    /// Per whitepaper Eq 9: SYSTEM:x = (ρ(↑entity(x):D)):↑op(x). The
    /// read path is a ρ-application that fetches from the live D.
    #[test]
    fn list_and_get_see_runtime_created_entities() {
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

    /// Profiling invocation — runs the same create/list/get workload as
    /// `list_and_get_see_runtime_created_entities` with the apply-
    /// variant profiler enabled, then dumps the histogram to stderr.
    /// #[ignore]'d by default because profiling adds ~20% overhead and
    /// clutters ordinary test runs. Invoke explicitly:
    ///
    ///   cargo test --lib profile_create_order -- --ignored --nocapture
    ///
    /// Read the dump to decide where each remaining perf cycle goes.
    #[cfg(feature = "profile")]
    #[test]
    #[ignore = "profiling run; invoke with --features profile --ignored --nocapture"]
    fn profile_create_order_dump_histogram() {
        ast::profile_reset();
        ast::profile_enable();

        let h = create_impl();
        let readings = "\
Order(.id) is an entity type.
Order has total.
  Each Order has at most one total.
";
        let _ = system_impl(h, "compile", readings);
        let _ = system_impl(h, "create:Order", "<<id, ord-1>, <total, 100>>");
        let _ = system_impl(h, "list:Order", "");
        let _ = system_impl(h, "get:Order", "ord-1");
        release_impl(h);

        ast::profile_disable();
        ast::profile_dump();
    }

    // ── Snapshot / rollback ─────────────────────────────────────

    #[test]
    fn snapshot_returns_auto_id_when_input_empty() {
        let h = create_bare_impl();
        let id1 = system_impl(h, "snapshot", "");
        let id2 = system_impl(h, "snapshot", "");
        assert_eq!(id1, "snap-0", "first auto id");
        assert_eq!(id2, "snap-1", "second auto id — monotonic counter");
        release_impl(h);
    }

    #[test]
    fn snapshot_accepts_caller_label_verbatim() {
        let h = create_bare_impl();
        assert_eq!(system_impl(h, "snapshot", "before-migrate"), "before-migrate");
        assert_eq!(system_impl(h, "snapshot", "before-migrate"), "before-migrate",
            "same label is idempotent — overwrites the prior snapshot");
        release_impl(h);
    }

    #[test]
    fn snapshots_listing_is_sorted_and_ffp_sequence() {
        let h = create_bare_impl();
        let _ = system_impl(h, "snapshot", "b");
        let _ = system_impl(h, "snapshot", "a");
        let _ = system_impl(h, "snapshot", "c");
        assert_eq!(system_impl(h, "snapshots", ""), "<a, b, c>");
        release_impl(h);
    }

    #[test]
    fn rollback_to_missing_snapshot_returns_bottom() {
        let h = create_bare_impl();
        assert_eq!(system_impl(h, "rollback", "nonexistent"), "⊥");
        release_impl(h);
    }

    #[test]
    fn rollback_restores_state_to_snapshot() {
        // Snapshot a known-good state; mutate it via direct cell write;
        // rollback; confirm the cell is back to its pre-mutation content.
        let h = alloc_with_noun("before");
        let _ = system_impl(h, "snapshot", "v1");
        // Mutate the Noun cell by replacing the whole state.
        {
            let tenant = tenant_lock(h).unwrap();
            let mut st = tenant.write().unwrap();
            st.replace_d(ast::store("Noun", ast::Object::atom("after"), &ast::Object::phi()));
        }
        assert_eq!(
            ast::fetch("Noun", &peek(h).unwrap()),
            ast::Object::atom("after"),
            "mutation landed"
        );
        // Roll back to v1.
        assert_eq!(system_impl(h, "rollback", "v1"), "v1");
        assert_eq!(
            ast::fetch("Noun", &peek(h).unwrap()),
            ast::Object::atom("before"),
            "rollback restored the v1 payload"
        );
        release_impl(h);
    }

    #[test]
    fn rollback_is_repeatable_from_same_snapshot() {
        // One snapshot can be rolled back to many times — the snapshot
        // map is not drained on rollback.
        let h = alloc_with_noun("origin");
        let _ = system_impl(h, "snapshot", "anchor");
        for round in 0..3 {
            {
                let tenant = tenant_lock(h).unwrap();
                let mut st = tenant.write().unwrap();
                st.replace_d(ast::store(
                    "Noun",
                    ast::Object::atom(&format!("mutation-{round}")),
                    &ast::Object::phi(),
                ));
            }
            assert_eq!(system_impl(h, "rollback", "anchor"), "anchor");
            assert_eq!(
                ast::fetch("Noun", &peek(h).unwrap()),
                ast::Object::atom("origin"),
                "round {round} rollback lands"
            );
        }
        release_impl(h);
    }

    #[test]
    fn snapshots_are_per_handle_not_shared() {
        // h1's snapshot must be invisible to h2. Taking snapshots under
        // the same label in different handles must not cross-contaminate.
        let h1 = alloc_with_noun("h1-payload");
        let h2 = alloc_with_noun("h2-payload");
        let _ = system_impl(h1, "snapshot", "shared-label");

        // h2 has no snapshot called "shared-label".
        assert_eq!(system_impl(h2, "rollback", "shared-label"), "⊥");
        assert_eq!(system_impl(h2, "snapshots", ""), "<>");

        // h1 still sees its own snapshot.
        assert_eq!(system_impl(h1, "snapshots", ""), "<shared-label>");
        release_impl(h1);
        release_impl(h2);
    }

    #[test]
    fn snapshot_and_rollback_on_invalid_handle_return_bottom() {
        // Invalid handles must not panic and must yield ⊥.
        assert_eq!(system_impl(u32::MAX, "snapshot", ""), "⊥");
        assert_eq!(system_impl(u32::MAX, "rollback", "whatever"), "⊥");
        assert_eq!(system_impl(u32::MAX, "snapshots", ""), "⊥");
    }

    // ── Per-tenant read/write lock classification ──────────────────

    #[test]
    fn read_only_op_classification_covers_query_verbs() {
        assert!(is_read_only_op("debug"));
        assert!(is_read_only_op("audit"));
        assert!(is_read_only_op("verify_signature"));
        assert!(is_read_only_op("snapshots"));
        assert!(is_read_only_op("list:Order"));
        assert!(is_read_only_op("get:Customer"));
        assert!(is_read_only_op("query:order_has_total"));
        assert!(is_read_only_op("explain:123"));
        // Mutating ops stay on the write path.
        assert!(!is_read_only_op("compile"));
        assert!(!is_read_only_op("create:Order"));
        assert!(!is_read_only_op("update:Order"));
        assert!(!is_read_only_op("transition:Order"));
        assert!(!is_read_only_op("snapshot"));
        assert!(!is_read_only_op("rollback"));
    }

    #[test]
    fn two_concurrent_readers_hold_the_tenant_lock_simultaneously() {
        // The per-tenant RwLock should let two readers hold the shared
        // guard at the same instant. A Barrier(2) forces both threads
        // to be inside the read guard concurrently — under the prior
        // Mutex this would deadlock (wait would block the second
        // reader since the first hasn't released yet).
        use std::sync::Barrier;
        use std::thread;

        let h = alloc_with_noun("shared-payload");
        let barrier = Arc::new(Barrier::new(2));

        let reader = |h: u32, barrier: Arc<Barrier>| move || -> ast::Object {
            let tenant = tenant_lock(h).unwrap();
            let st = tenant.read().unwrap();
            // Both readers reach the barrier while holding their read
            // guards. If the lock doesn't allow sharing, only one will
            // ever get here and the test hangs.
            barrier.wait();
            let d = st.snapshot_d();
            drop(st);
            d
        };

        let t1 = thread::spawn(reader(h, barrier.clone()));
        let t2 = thread::spawn(reader(h, barrier.clone()));
        let (d1, d2) = (t1.join().unwrap(), t2.join().unwrap());
        assert_eq!(d1, d2, "both readers saw the same state");
        release_impl(h);
    }

    // ── Per-cell write locks: parallel disjoint-cell writes ────────

    #[test]
    fn disjoint_cell_writers_run_in_parallel_via_try_commit_diff() {
        // Two threads attempt to write to DIFFERENT cells on the same
        // handle. Under the per-cell-lock design, both should hold
        // tenant.read() simultaneously (via a Barrier synchronization
        // point), then each writes only its target cell through
        // try_commit_diff. No tenant.write() escalation; both commit.
        use std::sync::Barrier;
        use std::thread;

        // Seed the handle with cells Order + Customer alongside the
        // Noun sentinel that `alloc_with_noun` installs. We need the
        // cells to pre-exist so try_commit_diff's structural-change
        // detector passes.
        let h = alloc_with_noun("seed");
        {
            let tenant = tenant_lock(h).unwrap();
            let mut st = tenant.write().unwrap();
            let state = {
                let s = ast::store("Noun", ast::Object::atom("seed"), &ast::Object::phi());
                let s = ast::store("Order", ast::Object::atom("o0"), &s);
                ast::store("Customer", ast::Object::atom("c0"), &s)
            };
            st.replace_d(state);
        }

        let barrier = Arc::new(Barrier::new(2));
        let write = |h: u32, b: Arc<Barrier>, cell: &'static str, val: &'static str| move || -> CommitOutcome {
            let tenant = tenant_lock(h).unwrap();
            let st = tenant.read().unwrap();
            let snapshot = st.snapshot_d();
            let new_d = ast::store(cell, ast::Object::atom(val), &snapshot);
            // Both writers reach the barrier while holding the shared
            // tenant lock. If per-cell commit didn't work, either the
            // snapshot or the commit would deadlock/serialize.
            b.wait();
            st.try_commit_diff(&snapshot, &new_d)
        };

        let t1 = thread::spawn(write(h, barrier.clone(), "Order", "o1"));
        let t2 = thread::spawn(write(h, barrier.clone(), "Customer", "c1"));
        let o1 = t1.join().unwrap();
        let o2 = t2.join().unwrap();
        assert!(matches!(o1, CommitOutcome::Committed),
            "Order writer committed (got {:?})", o1 as u8);
        assert!(matches!(o2, CommitOutcome::Committed),
            "Customer writer committed (got {:?})", o2 as u8);

        let d = peek(h).unwrap();
        assert_eq!(ast::fetch("Order", &d), ast::Object::atom("o1"));
        assert_eq!(ast::fetch("Customer", &d), ast::Object::atom("c1"));
        assert_eq!(ast::fetch("Noun", &d), ast::Object::atom("seed"),
            "untouched cell preserved");
        release_impl(h);
    }

    #[test]
    fn same_cell_cas_rejects_stale_snapshot() {
        // Write contention on the same cell must NOT silently lose an
        // update. Simulate: thread A snapshots at v0 and holds its
        // snapshot while thread B completes a full v0 → v1 write. A
        // then tries to commit v2 based on its stale snapshot.
        // try_commit_diff must return StaleSnapshot so A retries (or
        // escalates) rather than clobbering B's v1.
        let h = alloc_with_noun("v0");

        // A's snapshot, captured before B's write.
        let stale_snapshot = {
            let tenant = tenant_lock(h).unwrap();
            let st = tenant.read().unwrap();
            st.snapshot_d()
        };

        // B commits a full replacement to "v1-other".
        {
            let tenant = tenant_lock(h).unwrap();
            let mut st = tenant.write().unwrap();
            st.replace_d(ast::store(
                "Noun",
                ast::Object::atom("v1-other"),
                &ast::Object::phi(),
            ));
        }

        // A builds a new_d from its stale snapshot and tries to commit.
        let attempted_new_d = ast::store(
            "Noun",
            ast::Object::atom("v2-us"),
            &stale_snapshot,
        );
        let outcome = {
            let tenant = tenant_lock(h).unwrap();
            let st = tenant.read().unwrap();
            st.try_commit_diff(&stale_snapshot, &attempted_new_d)
        };
        assert!(matches!(outcome, CommitOutcome::StaleSnapshot),
            "stale snapshot must be rejected by CAS check");

        // Noun still holds B's write; A's attempt was refused.
        let d = peek(h).unwrap();
        assert_eq!(ast::fetch("Noun", &d), ast::Object::atom("v1-other"));
        release_impl(h);
    }

    #[test]
    fn try_commit_diff_flags_structural_change_for_new_cells() {
        // A commit that introduces a cell name not present in the
        // current state must return StructuralChange — the cells map
        // itself needs mutation, which requires tenant.write().
        let h = alloc_with_noun("seed");
        let tenant = tenant_lock(h).unwrap();
        let st = tenant.read().unwrap();
        let snapshot = st.snapshot_d();
        // Add a NEW cell not in the snapshot.
        let new_d = ast::store("Fresh", ast::Object::atom("unseen"), &snapshot);
        let outcome = st.try_commit_diff(&snapshot, &new_d);
        assert!(matches!(outcome, CommitOutcome::StructuralChange),
            "adding a cell requires the outer write lock");
        drop(st);
        release_impl(h);
    }

    #[test]
    fn concurrent_read_ops_via_system_impl_both_return() {
        // End-to-end: two `debug` calls on the same handle, both on
        // the read-path (is_read_only_op == true), both succeed. No
        // mutation happens, so neither thread's result shadows the
        // other.
        use std::thread;

        let h = create_bare_impl();
        let t1 = thread::spawn(move || system_impl(h, "debug", ""));
        let t2 = thread::spawn(move || system_impl(h, "debug", ""));
        let r1 = t1.join().unwrap();
        let r2 = t2.join().unwrap();
        assert!(!r1.is_empty(), "first reader got a debug projection");
        assert!(!r2.is_empty(), "second reader got a debug projection");
        release_impl(h);
    }
}

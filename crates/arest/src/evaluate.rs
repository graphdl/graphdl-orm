// crates/arest/src/evaluate.rs
//
// Evaluation is beta reduction. That's it.
//
// Constraint verification:  constraints.flat_map(|c| apply(c.func, ctx)) -> [Violation]
// Forward inference:        derivations.flat_map(|d| apply(d.func, pop)) -> [DerivedFact]
// State machine execution:  fold(transition)(initial)(stream) -> final_state
// Synthesis:                collect all knowledge about a noun from the compiled model.

use hashbrown::HashSet;
use crate::types::*;
use crate::ast;
#[allow(unused_imports)]
use alloc::{string::{String, ToString}, vec::Vec, boxed::Box, borrow::ToOwned};

// -- Forward Chaining -------------------------------------------------
//
// Correctness: FORML 2 derivation rules are monotonic (add facts, never
// remove). The population is finite. A monotonic sequence over a finite
// set reaches a fixed point. The loop terminates when no new facts are
// derived.
//
// Safety: the iteration bound prevents pathological rule sets from
// producing unbounded intermediate populations. If the bound is hit,
// the engine stops and returns what it has -- a partial fixed point.
//
// ── Non-termination guard (cli-apply-large-tasksdb-nonterminating) ───
//
// The round cap alone is NOT a wall-clock guard: a chain that fails to
// converge (e.g. the alethic-UC re-fire pathology — a rule re-derives a
// keyed fact whose stored value conflicts, `cell_put_keyed` drops it,
// dedup never recognizes the dropped key, so the rule re-fires every
// round) churns all `max_rounds` rounds, each doing the FULL
// O(rules × population) apply pass. On the ~870-entity tasks.db that is
// ~24 s/round × 100 rounds ≈ 40 minutes — observed: a `create:Task`
// burned 8800+ CPU-s and never returned.
//
// The guard is a WALL-CLOCK deadline checked at each round boundary: a
// chain gets a generous budget (`CHAIN_BUDGET`, default 3 min); once a
// round boundary is crossed past the deadline the LFP is declared non-
// terminating, the loop arms a ⊥-trace naming the rule/cell it was
// churning on (`ast::note_bottom_*`), raises an out-of-band abort flag,
// and returns the partial state. A healthy create converges in a handful
// of rounds (seconds), never approaching the deadline, so the success
// path is byte-for-byte unchanged — the only added cost is one
// `Instant::now()` compare per ROUND (not per apply).
//
// WHY NOT the `apply` reduction counter (`ast::with_fuel`): setting a
// non-`u64::MAX` budget makes `ast::fuel_is_bounded()` true, which forces
// the parallel branches of α / Construction / Filter in `apply` onto the
// SERIAL path (Rayon workers would escape a thread-local bound). Serial
// recursion over the ~870-fact population blows the main-thread stack
// (silent exit 255). The chain must stay parallel; a boundary-checked
// wall-clock deadline bounds the runaway WITHOUT touching `apply`'s
// recursion or its parallel/serial decision.

/// Wall-clock budget a single forward-chain LFP run may spend before it
/// is declared non-terminating and aborted with a traced ⊥.
///
/// Tuning (empirical, debug build, live tasks.db: ~870 Task entities, 69
/// derivation rules — instrument with `AREST_CHAIN_FUEL_TRACE=1`): ONE
/// full-width round over that population is ~24 s; a HEALTHY create
/// converges in a handful of rounds whose active-rule set SHRINKS after
/// the first (well under a minute). The RUNAWAY (alethic-UC re-fire)
/// instead SUSTAINS/GROWS the active set every round (measured: 52 → 65 →
/// … active) and would run all 100 rounds ≈ 40 min.
///
/// 3 minutes sits several × above the largest healthy multi-round create
/// seen, so real apps finish comfortably under it even on a slow box,
/// while the pathological chain trips the guard at ~3 min (vs the 40-min
/// open-ended hang) and returns a traced ⊥ naming the churning rule.
/// Raise it if a legitimate very-large app is ever observed to trip the
/// guard — the traced ⊥ names the rule, making that a one-line diagnosis.
///
/// Only meaningful where `time_shim::Instant` is the real monotonic
/// `std::time::Instant` (native host). On wasm32 / `no_std` the shim is a
/// zero-sentinel with no clock, so the deadline guard compiles to a
/// no-op there (those targets run small populations and never render ⊥).
#[cfg(all(feature = "std-deps", not(target_arch = "wasm32"), not(feature = "no_std")))]
pub(crate) const CHAIN_BUDGET: core::time::Duration = core::time::Duration::from_secs(180);

#[cfg(not(feature = "no_std"))]
thread_local! {
    /// Set by a chain loop when it aborts on the deadline; read-and-
    /// cleared by `take_chain_abort`. Out-of-band so the chain functions
    /// keep their `(Object, Vec<DerivedFact>)` contract unchanged — the
    /// ~80 call sites (incl. tests, induce, rebuild, grammar expansion)
    /// see the partial state exactly as before; only the user-facing
    /// write paths (`create`/`update`/`transition`) consult the flag and
    /// translate it into a ⊥ for the dispatcher to render with the trace.
    static CHAIN_ABORT: core::cell::Cell<bool> = const { core::cell::Cell::new(false) };
    /// Per-thread budget OVERRIDE. `None` ⇒ use `CHAIN_BUDGET`. Tests set
    /// `Some(Duration::ZERO)` via `with_chain_budget` to force a
    /// deterministic abort at the first round boundary without a real
    /// long-running chain; production never sets it.
    static CHAIN_BUDGET_OVERRIDE: core::cell::Cell<Option<core::time::Duration>> =
        const { core::cell::Cell::new(None) };
}

/// Run `f` with the chain non-termination budget overridden to `budget`
/// (restored on return). Used by the regression tests to force the
/// deadline guard at the first round boundary deterministically — a
/// `Duration::ZERO` budget means "every round boundary is already past
/// the deadline". Production code never calls this; the default
/// `CHAIN_BUDGET` applies.
#[cfg(not(feature = "no_std"))]
#[allow(dead_code)] // test-only override (used by the guard regression tests)
pub(crate) fn with_chain_budget<T, F: FnOnce() -> T>(budget: core::time::Duration, f: F) -> T {
    struct Restore(Option<core::time::Duration>);
    impl Drop for Restore {
        fn drop(&mut self) { CHAIN_BUDGET_OVERRIDE.with(|c| c.set(self.0)); }
    }
    let _g = CHAIN_BUDGET_OVERRIDE.with(|c| Restore(c.replace(Some(budget))));
    f()
}

// ── Deadline helpers — two arms ──────────────────────────────────────
// REAL arm (native host with a monotonic clock): compute and compare a
// `std::time::Instant` deadline. NO-OP arm (wasm32 / no_std, where
// `time_shim::Instant` is a clockless zero-sentinel with no `+`/`>=`):
// the deadline never trips, so the guard is inert — correct for those
// targets, which run small populations and never surface ⊥ to a user.

/// The deadline a chain starting now must finish by: `now + budget`,
/// where `budget` is the test override if set, else `CHAIN_BUDGET`.
#[cfg(all(feature = "std-deps", not(target_arch = "wasm32"), not(feature = "no_std")))]
fn chain_deadline() -> crate::time_shim::Instant {
    let budget = CHAIN_BUDGET_OVERRIDE.with(|c| c.get())
        .unwrap_or(CHAIN_BUDGET);
    crate::time_shim::Instant::now() + budget
}
#[cfg(not(all(feature = "std-deps", not(target_arch = "wasm32"), not(feature = "no_std"))))]
fn chain_deadline() -> crate::time_shim::Instant { crate::time_shim::Instant::now() }

/// True once the wall-clock deadline has passed. Checked at round
/// boundaries only — one `Instant::now()` per round, never per apply.
#[cfg(all(feature = "std-deps", not(target_arch = "wasm32"), not(feature = "no_std")))]
#[inline]
fn chain_deadline_exceeded(deadline: crate::time_shim::Instant) -> bool {
    crate::time_shim::Instant::now() >= deadline
}
#[cfg(not(all(feature = "std-deps", not(target_arch = "wasm32"), not(feature = "no_std"))))]
#[inline]
fn chain_deadline_exceeded(_deadline: crate::time_shim::Instant) -> bool { false }

/// Raise the out-of-band "the last chain aborted on the deadline" flag.
/// Paired with `take_chain_abort`. Host-only (the trace surface is
/// host-only); under `no_std` the kernel never renders ⊥ to a user.
#[cfg(not(feature = "no_std"))]
fn note_chain_abort() { CHAIN_ABORT.with(|c| c.set(true)); }
#[cfg(feature = "no_std")]
fn note_chain_abort() {}

/// Read and clear the chain-abort flag. A user-facing write path calls
/// this immediately after running the chain: `true` means the LFP did
/// not converge within `CHAIN_BUDGET` and the partial state must be
/// rejected (the armed ⊥-trace, set at the same abort point, names the
/// offending rule/cell). Auto-clearing keeps the flag from leaking into
/// a later, unrelated chain on the same thread.
#[cfg(not(feature = "no_std"))]
pub(crate) fn take_chain_abort() -> bool { CHAIN_ABORT.with(|c| c.replace(false)) }
#[cfg(feature = "no_std")]
pub(crate) fn take_chain_abort() -> bool { false }

/// Host-only, env-gated (`AREST_CHAIN_FUEL_TRACE`) per-round progress
/// line: round index + active-rule count + elapsed time. Mirrors the
/// `AREST_STAGE12_TRACE` knob (default-off path is a single
/// `std::env::var` miss). Shows the spend RATE so "healthy" (few rounds,
/// converges) is distinguishable from "runaway" (active set never
/// shrinks, wall-clock climbs toward the deadline) — and is the data
/// used to tune `CHAIN_BUDGET`.
#[allow(unused_variables)]
#[inline]
fn chain_round_trace(tag: &str, round: usize, active: usize, started: crate::time_shim::Instant) {
    #[cfg(not(feature = "no_std"))]
    if std::env::var("AREST_CHAIN_FUEL_TRACE").is_ok() {
        crate::diag!("[forward-chain] [{}] round {} entry: {} active, {:?} elapsed",
            tag, round, active, started.elapsed());
    }
}

/// Arm the ⊥-trace + abort flag + a loud diagnostic when a chain loop
/// gives up on the deadline. `culprit` is the rule still firing in the
/// round that overran (its consequent cell, if known, names the cell).
/// The ⊥-trace recording is a no-op unless a caller armed it via
/// `ast::with_bottom_trace` (the CLI dispatcher does); the diag and the
/// abort flag fire unconditionally so the failure is never silent.
#[allow(unused_variables)]
fn abort_chain_nonterminating(round: usize, culprit: Option<(&str, &str)>) {
    let (rule, cell) = culprit.unwrap_or(("<forward-chain>", ""));
    ast::note_bottom_rule(rule);
    if !cell.is_empty() {
        ast::note_bottom_cell(cell, &ast::Object::atom("forward-chain LFP"));
    }
    note_chain_abort();
    crate::diag!(
        "[forward-chain] ABORT: LFP did not converge within its time \
         budget after {} rounds — likely a non-terminating derivation \
         cycle (e.g. alethic-UC re-fire). Churning rule `{}`{}. \
         Returning partial state and a traced ⊥.",
        round, rule,
        if cell.is_empty() { alloc::string::String::new() }
        else { alloc::format!(" over cell `{}`", cell) },
    );
}

/// Forward-chain derivation rules to a fixed point.
///
/// Each derivation def is applied to the current population. New facts
/// are added, and the process repeats until no new facts are derived
/// (fixed point reached) or the iteration bound is hit.
///
/// Iteration bound: 100 iterations maximum. FORML2 derivation rules are
/// monotonic (facts are added, never removed) over a finite domain, so
/// convergence is guaranteed in theory. The 100-iteration bound is a
/// safety net for pathological rule sets that produce very large
/// intermediate populations. If the bound is exceeded, the function
/// returns a partial fixed point -- all facts derived so far, even
/// though additional derivations may be possible. This is safe because
/// each derived fact is individually correct; only completeness is
/// affected.

/// sm-status-scoped-upsert: cells whose forward-chain writes UPSERT (a
/// new value at an existing key OVERWRITES the prior one) instead of
/// being conflict-rejected like every other keyed cell.
///
/// The lone member is `State_Machine_is_currently_in_Status`: a KEYED
/// cell (one-status-per-State-Machine alethic UC, key role
/// "State Machine") that the SM event-fold ADVANCES. The fold is
/// recursive — its from-guard reads the current status to gate, then
/// emits the `to` status. On a plain keyed cell the advance
/// (e.g. `(o2, Placed)` over seeded `(o2, Draft)`) collides at the
/// `o2` key and `cell_put_keyed` drops it, FREEZING status at the
/// seeded value. The from-guard already guarantees only a LEGAL advance
/// is emitted (only a resource currently in `from` gets `to`), so
/// last-write-wins here lands a valid transition; it never applies an
/// illegal one.
///
/// SCOPE: this allowlist is consulted ONLY on the forward-chain
/// (`integrate_round_facts`) keyed path. Every other keyed cell — and
/// every user-facing apply-path write (see `command.rs::push_with_uc_check`,
/// which has its own explicit `overwrite` flag) — keeps the global
/// conflict-reject behavior unchanged.
///
/// The companion SM cells the fold co-emits
/// (`State_Machine_is_instance_of_Noun`, `State_Machine_is_for_Resource`)
/// are intentionally NOT here: their tuples are byte-stable across rounds
/// (`<SM, Noun>` / `<SM, Resource=SM>` never change as Status advances),
/// so a re-emit is `cell_put_keyed`'s idempotent byte-equal no-op, never
/// a conflict — they need no upsert.
pub(crate) const SM_STATUS_UPSERT_CELLS: &[&str] = &["State_Machine_is_currently_in_Status"];

/// Whether forward-chain writes to `cell_name` should upsert (overwrite a
/// prior value at the same key) rather than conflict-reject. True only for
/// the scoped SM-status cell(s) in [`SM_STATUS_UPSERT_CELLS`].
#[inline]
pub(crate) fn cell_is_sm_status_upsert(cell_name: &str) -> bool {
    SM_STATUS_UPSERT_CELLS.contains(&cell_name)
}

/// Read the `_CellKeyRoles` metadata cell (emitted by
/// `compile_to_defs_state`) into a `ft_id → role_names` map. Forward-
/// chain emit paths consult this to route writes for keyed cells
/// through `ast::cell_put_keyed` (Map storage) instead of the legacy
/// Seq append. Cells absent from the map keep the legacy Seq path —
/// behavior-preserving for everything without an alethic UC.
///
/// The cell is stored via `Func::constant(Object::Seq(entries))` so
/// `func_to_object` wraps it as `<atom("'"), seq_of_entries>`. Unwrap
/// the wrapper here so callers see the entries directly. Each entry
/// is a named-tuple fact: `<<ftId, ft_id_atom>, <keyRoles, "Role1,…">>`.
pub(crate) fn read_cell_key_roles(d: &ast::Object) -> hashbrown::HashMap<String, Vec<String>> {
    use hashbrown::HashMap;
    let cell = ast::fetch_or_phi("_CellKeyRoles", d);
    let entries: Vec<ast::Object> = cell.as_seq()
        .and_then(|items| {
            if items.len() == 2 && items[0].as_atom() == Some("'") {
                items[1].as_seq().map(|s| s.to_vec())
            } else {
                Some(items.to_vec())
            }
        })
        .unwrap_or_default();
    let mut out: HashMap<String, Vec<String>> = HashMap::with_capacity(entries.len());
    for fact in entries.iter() {
        let Some(ft_id) = ast::binding(fact, "ftId") else { continue };
        let Some(roles_csv) = ast::binding(fact, "keyRoles") else { continue };
        let names: Vec<String> = roles_csv.split(',')
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect();
        if names.is_empty() { continue; }
        out.insert(ft_id.to_string(), names);
    }
    out
}

/// Integrate one round's `(cell_name → facts)` batch into `state`,
/// routing each cell write through `cell_put_keyed` when `key_roles`
/// names that cell as a Map-backed (alethic-UC-keyed) cell, and through
/// `cell_put_folded` (full-tuple keyed fold — #932 phase-2) otherwise.
/// Both paths produce a Map-backed cell (`eq:cellfold` `D_n`); the Seq
/// append path is retired.
///
/// Factored out of the two forward-chain entry points so they share
/// the routing rule. `key_roles` is built once per forward-chain
/// invocation via [`read_cell_key_roles`]; this function consults it
/// without further parsing.
fn integrate_round_facts(
    state: ast::Object,
    by_cell: hashbrown::HashMap<String, Vec<ast::Object>>,
    key_roles: &hashbrown::HashMap<String, Vec<String>>,
) -> ast::Object {
    let mut current_state = state;
    for (cell_name, facts) in by_cell {
        if let Some(roles) = key_roles.get(&cell_name) {
            // Map-backed cell: each fact is upserted by its named-role
            // key. Multiple facts in the same round at the same key
            // collapse to the last-write — matches the alethic-UC
            // semantics (only one tuple per key is structurally
            // permitted). The first write also migrates any pre-existing
            // Seq contents into the Map (see `cell_put_keyed`).
            let role_refs: Vec<&str> = roles.iter().map(|s| s.as_str()).collect();
            for fact in facts {
                // task-820: KeyConflict means a rule emitted a fact
                // that collides with an existing keyed entry. Log to
                // stderr and skip the emit so a single misbehaving
                // rule doesn't kill the whole compile. The diagnostic
                // is preserved (loud and noisy on stderr) but no
                // longer fatal. Idempotent re-emission is filtered by
                // the `existing_keys + round_keys` dedup upstream;
                // only genuinely-conflicting writes hit this path.
                match ast::cell_put_keyed(&cell_name, &role_refs, fact, &current_state) {
                    Ok(next) => { current_state = next; }
                    Err(conflict) if cell_is_sm_status_upsert(&cell_name) => {
                        // sm-status-scoped-upsert: this is the SM-status
                        // cell, whose recursive from-guarded event-fold
                        // ADVANCES status. A conflict here is a LEGAL
                        // advance (the from-guard already filtered to
                        // resources currently in `from`) landing over the
                        // prior status at the same State-Machine key.
                        // Upsert = last-write-wins: vacate the slot, then
                        // re-put. The second put cannot conflict (slot is
                        // empty). Mirrors command.rs::push_with_uc_check's
                        // `overwrite` branch, but scoped to this cell on the
                        // forward-chain path only.
                        let cleared = drop_keyed_entry(
                            &cell_name, &conflict.key, &role_refs, &current_state);
                        current_state = ast::cell_put_keyed(
                            &cell_name, &role_refs, conflict.incoming_fact, &cleared)
                            .unwrap_or(cleared);
                    }
                    Err(conflict) => {
                        crate::diag!("[forward-chain] UC conflict, dropping fact: {:?}", conflict);
                    }
                }
            }
        } else {
            // #932 phase-2: keyless fact cells fold to a Map keyed by the
            // full tuple (cell_put_folded) — set semantics per eq:cellfold
            // — rather than an un-folded Seq append. Re-derivation of an
            // identical fact is an idempotent no-op; distinct tuples (incl.
            // ring fact types whose duplicate role names a by-name key
            // cannot tell apart) each get their own row.
            for fact in facts {
                current_state = ast::cell_put_folded(&cell_name, fact, &current_state);
            }
        }
    }
    current_state
}

/// sm-status-scoped-upsert helper: remove the Map entry under `key` from
/// cell `name` so a subsequent `cell_put_keyed` of a new value at that key
/// cannot conflict. Map case is a direct `HashMap::remove`; a Seq cell
/// (pre-migration shape) is filtered by extracted key. Module-private twin
/// of `command.rs::drop_keyed_entry`; the forward-chain upsert path needs
/// it without taking a cross-module dependency, and the SM-status cell is
/// Map-backed by the time the event-fold advances it (the first keyed put
/// migrates Seq→Map), so the Map arm is the live path.
fn drop_keyed_entry(
    name: &str,
    key: &str,
    key_role_names: &[&str],
    state: &ast::Object,
) -> ast::Object {
    let existing = ast::fetch_or_phi(name, state);
    match &existing {
        ast::Object::Map(m) => {
            let mut next = (**m).clone();
            next.remove(key);
            ast::store(name, ast::Object::Map(next.into()), state)
        }
        ast::Object::Seq(items) => {
            let kept: Vec<ast::Object> = items.iter()
                .filter(|f| ast::extract_key_from_fact(f, key_role_names)
                    .as_deref() != Some(key))
                .cloned()
                .collect();
            ast::store(name, ast::Object::Seq(kept.into()), state)
        }
        _ => state.clone(),
    }
}

/// task-3-incremental: per-thread instrumentation counting the number
/// of rule activations across all rounds of the most recent
/// `semi_naive_inner` invocation. Test-only — the production
/// chainer never observes it. Tests call
/// [`reset_chain_eval_count`] before exercising the chainer, then
/// [`get_chain_eval_count`] after; the delta is the total rule-active
/// count (Σ active_rules_per_round). On gated runs this should be
/// O(touched_cells × dependent_rules); on un-gated runs it grows to
/// O(rules × rounds-to-fixpoint).
#[cfg(any(test, feature = "test-bins"))]
mod chain_eval_counter {
    use core::cell::Cell;
    std::thread_local! {
        pub static COUNT: Cell<usize> = const { Cell::new(0) };
    }
}

#[cfg(any(test, feature = "test-bins"))]
pub fn reset_chain_eval_count() {
    chain_eval_counter::COUNT.with(|c| c.set(0));
}

#[cfg(any(test, feature = "test-bins"))]
pub fn get_chain_eval_count() -> usize {
    chain_eval_counter::COUNT.with(|c| c.get())
}

#[inline]
#[allow(unused_variables)]
fn record_chain_eval_count(active_len: usize) {
    #[cfg(any(test, feature = "test-bins"))]
    chain_eval_counter::COUNT.with(|c| c.set(c.get() + active_len));
}

/// Forward-chain derivation rules over D to fixed point. Returns (D', derived_facts).
/// D contains both population cells and def cells (Backus Sec. 14.3).
pub fn forward_chain_defs_state(
    derivation_defs: &[(&str, &ast::Func)],
    d: &ast::Object,
) -> (ast::Object, Vec<DerivedFact>) {
    forward_chain_defs_state_bounded(derivation_defs, d, 100)
}

/// Apply all derivation rules once, returning novel facts.
///
/// Dedup against three populations: facts already in `current_state`,
/// facts derived in prior rounds (`all_derived`), and facts emitted
/// earlier in this round. All three use a canonical `FactKey`
/// (fact_type_id + sorted bindings) and `HashSet` lookups — a naive
/// per-candidate linear scan is O(K·N); the hashed form is O(K+N).
fn derive_one_round(
    derivation_defs: &[(&str, &ast::Func)],
    current_state: &ast::Object,
    all_derived: &[DerivedFact],
    d: &ast::Object,
) -> Vec<DerivedFact> {
    let existing_keys = state_keys(current_state);
    derive_one_round_with_keys(
        derivation_defs, current_state, all_derived, d, &existing_keys)
}

/// Like [`derive_one_round`] but takes a pre-built `existing_keys`
/// set — lets the semi-naive chainer maintain it incrementally
/// across rounds instead of rebuilding ~5k-element HashSets every
/// round. On core.md this was ~3ms per round of pure re-hashing.
fn derive_one_round_with_keys(
    derivation_defs: &[(&str, &ast::Func)],
    current_state: &ast::Object,
    all_derived: &[DerivedFact],
    d: &ast::Object,
    existing_keys: &HashSet<FactKey>,
) -> Vec<DerivedFact> {
    // `AREST_STAGE12_TRACE` is a host-side perf knob; under no_std there
    // is no `std::env` to query and no stderr to print to, so the gate
    // becomes a const-false and the optimizer drops the trace branches.
    #[cfg(not(feature = "no_std"))]
    let trace = std::env::var("AREST_STAGE12_TRACE").is_ok();
    #[cfg(feature = "no_std")]
    let trace = false;
    let t_dk = crate::time_shim::Instant::now();
    let derived_keys: HashSet<FactKey> = all_derived.iter().map(fact_key).collect();
    if trace { crate::diag!("    [rnd] derived_keys: {:?}", t_dk.elapsed()); }
    // `encode_state` is a ~1-2ms pure-clone pass on core.md-scale
    // inputs. Skip it when every active Func is a `Native` — the
    // specialized grammar classifiers accept `&Object` directly
    // (raw state or encoded pop) and resolve cells via
    // `fetch_or_phi` on `Object::Map`. Non-Native variants
    // (interpreted FFP) still require the encoded pop shape.
    //
    // H4 (#692): production paths no longer emit Native — the last
    // Native leaf (rmap_func) is now Func::Platform. This gate stays
    // for historical specialized-classifier deployments and tests
    // that hand-build Native nodes; the empty-defs branch (all() over
    // an empty iterator returns true → skip the encode) is the only
    // common-case hit today.
    let all_native = derivation_defs.iter()
        .all(|(_, f)| matches!(f, ast::Func::Native(_)));
    let t_en = crate::time_shim::Instant::now();
    let pop_obj;
    let apply_input: &ast::Object = if all_native {
        current_state
    } else {
        pop_obj = ast::encode_state(current_state);
        &pop_obj
    };
    if trace { crate::diag!("    [rnd] encode_state: {:?} (skipped={})",
        t_en.elapsed(), all_native); }
    let t_ap = crate::time_shim::Instant::now();
    let candidates: Vec<DerivedFact> = derivation_defs.iter()
        .flat_map(|(name, func)| {
            let result = ast::apply(func, apply_input, d);
            let name = name.to_string();
            result.as_seq().into_iter()
                .flat_map(move |items| items.iter().cloned().collect::<Vec<_>>())
                .filter_map(move |item| parse_derived_fact(&item, &name))
                .collect::<Vec<_>>()
        })
        .collect();
    if trace { crate::diag!("    [rnd] apply {} defs: {:?} ({} candidates)",
        derivation_defs.len(), t_ap.elapsed(), candidates.len()); }
    let t_dd = crate::time_shim::Instant::now();
    let mut round_keys: HashSet<FactKey> = HashSet::with_capacity(candidates.len());
    let mut out: Vec<DerivedFact> = Vec::with_capacity(candidates.len());
    for cand in candidates {
        let key = fact_key(&cand);
        if !existing_keys.contains(&key)
            && !derived_keys.contains(&key)
            && round_keys.insert(key)
        {
            out.push(cand);
        }
    }
    if trace { crate::diag!("    [rnd] dedup: {:?} ({} novel)",
        t_dd.elapsed(), out.len()); }
    out
}

/// Semi-naive forward-chain: rules that know which cells they read
/// (via the third tuple element) get skipped in any round whose prior
/// round didn't touch any of those cells. Rules without antecedent
/// metadata (`None`) run every round, matching the classical naïve
/// behavior for that rule.
///
/// For the Stage-2 grammar, round 1 writes only
/// `Statement_has_Classification`; with all 69 classification rules
/// tagged, only the one rule that actually reads that cell survives
/// the round-2 filter. Everything else is a ~zero-cost skip.
pub fn forward_chain_defs_state_semi_naive(
    derivation_defs: &[(&str, &ast::Func, Option<&[String]>)],
    d: &ast::Object,
    max_rounds: usize,
) -> (ast::Object, Vec<DerivedFact>) {
    forward_chain_defs_state_semi_naive_with_base_keys(
        derivation_defs, d, max_rounds, None)
}

/// Like [`forward_chain_defs_state_semi_naive`] but accepts a
/// pre-computed `base_keys` set so callers that have already hashed
/// part of `d` (e.g. the cached grammar state in
/// `parse_forml2_stage2::cached_grammar`) can skip the re-hash
/// during the initial state_keys pass. On core.md-scale inputs this
/// saves the ~3-4ms that `state_keys(merged)` would otherwise cost
/// at the start of every `classify_statements` call.
pub fn forward_chain_defs_state_semi_naive_with_base_keys(
    derivation_defs: &[(&str, &ast::Func, Option<&[String]>)],
    d: &ast::Object,
    max_rounds: usize,
    base_keys: Option<HashSet<FactKey>>,
) -> (ast::Object, Vec<DerivedFact>) {
    semi_naive_inner(derivation_defs, None, d, max_rounds, base_keys, None)
}

/// task-3-incremental: forward-chain with an explicit round-1
/// `dirty_cells` seed. The classical [`forward_chain_defs_state_semi_naive`]
/// starts every run with `dirty_cells = None`, which forces round 1 to
/// evaluate every rule — exactly the cost that dominates the apply
/// path on a 660-task corpus (~125s wall to apply a single Task
/// Priority edit; ~75 task-relevant rules × 660 facts × N rounds).
///
/// `seed` should be the set of cell names that the caller's mutation
/// just touched (e.g. for `apply update Task Priority`, this is
/// `{"Task_has_Task_Priority"}`). Round 1 only runs rules whose
/// declared antecedent cells intersect `seed`. Rules with `None`
/// antecedent metadata still run conservatively in every round (no
/// gating possible). Subsequent rounds use the normal next-dirty
/// propagation — emit cells from this round feed the next round's
/// filter.
///
/// Passing an empty seed produces a no-op chain (round 1 finds no
/// active rules and breaks immediately). Pass `forward_chain_defs_state_semi_naive`
/// when you want the round-1-everywhere semantics.
pub fn forward_chain_defs_state_seeded(
    derivation_defs: &[(&str, &ast::Func, Option<&[String]>)],
    seed: HashSet<String>,
    d: &ast::Object,
    max_rounds: usize,
) -> (ast::Object, Vec<DerivedFact>) {
    semi_naive_inner(derivation_defs, Some(seed), d, max_rounds, None, None)
}

/// Same as [`forward_chain_defs_state_seeded`] but also reports every
/// rule id the per-round gate selected as `active`. Used by the
/// drop-and-rederive bridge-preservation guard in command::update_via_defs:
/// the caller pre-drops derived consequents to phi() and runs this
/// chain; cells whose producing rule was NEVER activated are restored
/// from the pre-drop snapshot (the rule's antecedents didn't change on
/// this apply, so its consequent shouldn't be clobbered). Cells whose
/// rule WAS activated stay at whatever the chain wrote (including
/// empty -- the legitimate stale-clear case from
/// `update_clears_stale_derived_consequents_before_forward_chain`).
pub fn forward_chain_defs_state_seeded_tracked(
    derivation_defs: &[(&str, &ast::Func, Option<&[String]>)],
    seed: HashSet<String>,
    d: &ast::Object,
    max_rounds: usize,
    activated: &mut HashSet<String>,
) -> (ast::Object, Vec<DerivedFact>) {
    semi_naive_inner(derivation_defs, Some(seed), d, max_rounds, None, Some(activated))
}

/// Shared body of the semi-naive variants. `initial_dirty == None`
/// matches the classical "round 1 runs everything" behavior; `Some(set)`
/// seeds the round-1 filter from `set` (used by
/// [`forward_chain_defs_state_seeded`] for incremental apply).
///
/// `activated_rules`, when `Some`, accumulates the id of every rule
/// that the per-round gate selected as `active` -- regardless of
/// whether the rule actually emitted facts. Caller uses the set to
/// distinguish "rule was activated but emitted nothing" (correctly
/// cleared consequent, e.g. a stale-clear) from "rule never activated"
/// (drop-and-rederive should NOT clobber the pre-drop snapshot; e.g.
/// the bridge case where antecedents didn't change). Optional so the
/// non-seeded full-chain caller pays no extra bookkeeping.
fn semi_naive_inner(
    derivation_defs: &[(&str, &ast::Func, Option<&[String]>)],
    initial_dirty: Option<HashSet<String>>,
    d: &ast::Object,
    max_rounds: usize,
    base_keys: Option<HashSet<FactKey>>,
    mut activated_rules: Option<&mut HashSet<String>>,
) -> (ast::Object, Vec<DerivedFact>) {
    use hashbrown::HashMap;
    // Same gate as `derive_one_round_with_keys`: trace knob is host-only.
    #[cfg(not(feature = "no_std"))]
    let trace = std::env::var("AREST_STAGE12_TRACE").is_ok();
    #[cfg(feature = "no_std")]
    let trace = false;
    let mut current_state = d.clone();
    let mut all_derived: Vec<DerivedFact> = Vec::new();
    // task-744 phase 4: per-FT key-roles for routing Map-backed cell
    // writes through `cell_put_keyed`. Same metadata-cell pattern as
    // `forward_chain_defs_state_bounded` — read once, consult per cell.
    let key_roles = read_cell_key_roles(d);
    // Base set of fact keys in `d`. Built once here and updated
    // incrementally as rounds emit new facts — on core.md this cut
    // ~3ms per round of re-hashing the unchanged grammar portion of
    // the state.
    let t_ek = crate::time_shim::Instant::now();
    let mut existing_keys = base_keys.unwrap_or_else(|| state_keys(&current_state));
    if trace { crate::diag!("    [sn] initial state_keys: {:?} ({} keys)",
        t_ek.elapsed(), existing_keys.len()); }
    // `dirty_cells == None` means "run everything" (initial round or
    // caller wants no filtering); `Some(set)` restricts to rules that
    // read at least one of those cells. Callers that want incremental
    // gating from round 1 pass `Some(seed)` here via the
    // `forward_chain_defs_state_seeded` entry point.
    let mut dirty_cells: Option<HashSet<String>> = initial_dirty;
    // Non-termination guard: a wall-clock deadline checked at each round
    // boundary (see the guard note above `forward_chain_defs_state`). A
    // chain that fails to converge stops at the next round boundary past
    // the deadline instead of grinding through all `max_rounds`. The
    // check is one `Instant::now()` per ROUND — zero per-apply cost, and
    // it leaves `apply`'s parallel α/Construction path intact (the fuel
    // counter could not: a bounded fuel budget forces serial recursion,
    // which overflows the stack on a large population).
    let deadline = chain_deadline();
    let started = crate::time_shim::Instant::now();
    for round in 0..max_rounds {
        // Deadline checked BEFORE another full O(rules × population) pass:
        // once it is past, the LFP is declared non-terminating, the
        // partial state is returned, and a traced ⊥ is armed naming the
        // rule still firing.
        if chain_deadline_exceeded(deadline) {
            let culprit = derivation_defs.iter()
                .find(|(_, _, cells)| match (&dirty_cells, cells) {
                    (None, _) | (Some(_), None) => true,
                    (Some(dirty), Some(reads)) => reads.iter().any(|c| dirty.contains(c)),
                })
                .map(|(n, _, _)| *n);
            abort_chain_nonterminating(round, culprit.map(|n| (n, "")));
            break;
        }
        let active: Vec<(&str, &ast::Func)> = derivation_defs.iter()
            .filter(|(_, _, cells)| match (&dirty_cells, cells) {
                (None, _) => true,                       // first round or filtering off
                (Some(_), None) => true,                 // unknown reads → run it
                (Some(dirty), Some(reads)) =>
                    reads.iter().any(|c| dirty.contains(c)),
            })
            .map(|(n, f, _)| (*n, *f))
            .collect();
        if trace {
            crate::diag!("    [sn] round {}: active {}/{} defs",
                round, active.len(), derivation_defs.len());
        }
        record_chain_eval_count(active.len());
        chain_round_trace("sn", round, active.len(), started);
        if let Some(ref mut acc) = activated_rules {
            for (name, _) in &active {
                acc.insert((*name).to_string());
            }
        }
        if active.is_empty() { break; }
        let new_facts = derive_one_round_with_keys(
            active.as_slice(), &current_state, &all_derived, d, &existing_keys);
        if new_facts.is_empty() { break; }

        let mut by_cell: HashMap<String, Vec<ast::Object>> =
            HashMap::with_capacity(new_facts.len().min(active.len()));
        for fact in &new_facts {
            let pairs: Vec<(&str, &str)> = fact.bindings.iter()
                .map(|(k, v)| (k.as_str(), v.as_str())).collect();
            by_cell.entry(fact.fact_type_id.clone()).or_default()
                .push(ast::fact_from_pairs(&pairs));
            // Keep `existing_keys` in sync so the next round's filter
            // doesn't have to re-walk the whole state.
            existing_keys.insert(fact_key(fact));
        }
        let next_dirty: HashSet<String> = by_cell.keys().cloned().collect();
        current_state = integrate_round_facts(current_state, by_cell, &key_roles);
        all_derived.extend(new_facts);
        dirty_cells = Some(next_dirty);
    }
    (current_state, all_derived)
}

/// task-3 phase 2 / DB-task-929: decode the `derivation_reads:<rule_id>`
/// sidecar emitted by `compile_to_defs_state`. Returns the positive FT
/// cells the rule reads (each `AntecedentSource::FactType(cell)` from
/// the rule), or `None` when the sidecar is absent — in which case the
/// caller treats the rule as "unknown reads, run unconditionally".
///
/// The sidecar lives next to `derivation_meta:<id>` and shares its
/// `Func::constant(payload)` wrapping convention (`<atom("'"), payload>`).
/// Payload is a flat Seq of cell-name atoms.
pub fn read_derivation_reads(d: &ast::Object, rule_id: &str) -> Option<Vec<String>> {
    let cell_name = alloc::format!("derivation_reads:{}", rule_id);
    let cell = ast::fetch_or_phi(&cell_name, d);
    let items = cell.as_seq()?;
    let payload = if items.len() == 2 && items[0].as_atom() == Some("'") {
        &items[1]
    } else { return None; };
    let reads: Vec<String> = payload.as_seq()?
        .iter()
        .filter_map(|c| c.as_atom().map(|s| s.to_string()))
        .collect();
    Some(reads)
}

/// Like [`forward_chain_defs_state`] but capped at `max_rounds` rule
/// applications. Callers that know their rule set is stratified (no
/// rule's antecedent reads another rule's consequent cell) can pass
/// `max_rounds = 1` to skip the empty confirmation round the naive
/// fixpoint does last — the round where `derive_one_round` re-applies
/// every rule against the round-1 output only to dedup it all away.
///
/// Unbounded behavior is preserved through the default 100-round cap
/// in [`forward_chain_defs_state`].
pub fn forward_chain_defs_state_bounded(
    derivation_defs: &[(&str, &ast::Func)],
    d: &ast::Object,
    max_rounds: usize,
) -> (ast::Object, Vec<DerivedFact>) {
    // Fixed-point iteration, bounded by `max_rounds`.
    //
    // A `core::iter::successors(…).take(N).last()` form reads cleaner
    // but is an off-by-one footgun: `successors` eagerly pre-computes
    // the NEXT value on every `next()` call, so a bound of N fires
    // `derive_one_round` N+1 times. For core.md with stratified
    // grammar rules, that extra call was ~7s of pure waste against
    // already-saturated state. The manual loop runs exactly
    // `max_rounds` rounds or fewer (early-exits the first time a
    // round produces nothing novel).
    //
    // Per-round fact integration batches by cell: a naive
    // `fold(state.clone(), cell_push)` is O(n²) because each
    // `cell_push` re-clones the cell's full Vec. Grouping the round's
    // new facts by cell and appending once per cell makes it O(n).
    let mut current_state = d.clone();
    let mut all_derived: Vec<DerivedFact> = Vec::new();
    // task-744 phase 4: per-FT key-roles for routing writes to
    // `cell_put_keyed` (Map storage) when an alethic UC exists. Read
    // once from the metadata cell `_CellKeyRoles` (constant after
    // `compile_to_defs_state`); fact-type cells absent from the map
    // keep the legacy Seq-append path.
    let key_roles = read_cell_key_roles(d);
    // Cell written by the most recent round — names the consequent in the
    // traced ⊥ if the chain has to be aborted (the naive chainer has no
    // per-rule antecedent metadata, so the last-written cell is the best
    // available culprit). The rule slot falls back to the first def.
    let mut last_round_cell: Option<String> = None;
    // Wall-clock non-termination guard — see the note above
    // `forward_chain_defs_state`. Same rationale and success-path cost
    // (one `Instant::now()` per round) as `semi_naive_inner`; leaves the
    // parallel apply path intact.
    let deadline = chain_deadline();
    let started = crate::time_shim::Instant::now();
    for round in 0..max_rounds {
        if chain_deadline_exceeded(deadline) {
            let rule = derivation_defs.first().map(|(n, _)| *n).unwrap_or("<forward-chain>");
            abort_chain_nonterminating(
                round, Some((rule, last_round_cell.as_deref().unwrap_or(""))));
            break;
        }
        chain_round_trace("naive", round, derivation_defs.len(), started);
        let new_facts = derive_one_round(derivation_defs, &current_state, &all_derived, d);
        if new_facts.is_empty() { break; }
        use hashbrown::HashMap;
        let mut by_cell: HashMap<String, Vec<ast::Object>> =
            HashMap::with_capacity(new_facts.len().min(derivation_defs.len()));
        for fact in &new_facts {
            let pairs: Vec<(&str, &str)> = fact.bindings.iter()
                .map(|(k, v)| (k.as_str(), v.as_str())).collect();
            by_cell.entry(fact.fact_type_id.clone()).or_default()
                .push(ast::fact_from_pairs(&pairs));
        }
        last_round_cell = by_cell.keys().next().cloned();
        current_state = integrate_round_facts(current_state, by_cell, &key_roles);
        all_derived.extend(new_facts);
    }
    (current_state, all_derived)
}

/// Parse a derivation result Object into a DerivedFact.
fn parse_derived_fact(item: &ast::Object, derived_by: &str) -> Option<DerivedFact> {
    let fact_items = item.as_seq().filter(|f| f.len() >= 3)?;
    let ft_id = fact_items[0].as_atom()?.to_string();
    let reading = fact_items[1].as_atom()?.to_string();
    let bindings: Vec<(String, String)> = fact_items[2].as_seq()
        .unwrap_or(&[])
        .iter()
        .filter_map(|b| {
            let pair = b.as_seq()?;
            if pair.len() == 2 {
                Some((pair[0].as_atom()?.to_string(), pair[1].as_atom()?.to_string()))
            } else { None }
        })
        .collect();
    Some(DerivedFact {
        fact_type_id: ft_id, reading, bindings,
        derived_by: derived_by.to_string(),
        confidence: Confidence::Definitive,
    })
}

/// Canonical key for deduplicating facts across rounds and against
/// `current_state`. A 64-bit FNV-1a hash of `fact_type_id` + the
/// multiset of bindings (role atoms sorted for order-independence).
/// Collision probability at the scales we see (<10^4 facts) is ~10^-12,
/// so `HashSet<FactKey>` is effectively exact without the String
/// allocation cost a `(String, Vec<_>)` key would pay per insertion.
pub(crate) type FactKey = u64;

const FNV_OFFSET: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;

#[inline]
fn fnv_mix(mut h: u64, bytes: &[u8]) -> u64 {
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(FNV_PRIME);
    }
    h
}

fn fact_key(f: &DerivedFact) -> FactKey {
    let mut refs: Vec<&(String, String)> = f.bindings.iter().collect();
    refs.sort();
    let mut h = fnv_mix(FNV_OFFSET, f.fact_type_id.as_bytes());
    for (k, v) in refs {
        h = fnv_mix(h, b"|");
        h = fnv_mix(h, k.as_bytes());
        h = fnv_mix(h, b"=");
        h = fnv_mix(h, v.as_bytes());
    }
    h
}

/// Build a set of fact keys for every fact currently in `state`. One
/// O(N) pass replaces the K × O(N) linear scans the filter would
/// otherwise make via `state_contains_fact`. Borrows &str out of the
/// population — no String allocation per key.
///
/// task-927 follow-up: a non-atom binding value (e.g. `Seq([])` which
/// is `Object::phi()`) is normalized to the canonical "φ" string for
/// hashing — matching the way chain rules emit vacuous bindings as
/// `Atom("φ")`. Without this, existing facts with `<Resource, Seq([])>`
/// produce keys that omit Resource entirely while the matching
/// candidate `<Resource, Atom("φ")>` includes "φ", so dedup misses,
/// cell_put_keyed detects a UC collision, drops, and the chain re-fires
/// the same rule next round — infinite UC-conflict loop until the LFP
/// cap. Mirrors Object::phi()'s Display rendering at ast.rs:521.
pub(crate) fn state_keys(state: &ast::Object) -> HashSet<FactKey> {
    let mut set: HashSet<FactKey> = HashSet::new();
    for (cell_name, cell_contents) in ast::cells_iter(state) {
        // task-960: skip compiled-def / codegen cells. The post-compile
        // def-state bundles population fact cells together with thousands
        // of compiled defs (`derivation:*`, `validate:*`, `schema:*`,
        // `get:*`, `create:*`, codegen targets, …) whose Func bodies are
        // encoded as Object trees. `cell_facts_iter` can't tell a func
        // node from a fact tuple, so without this guard state_keys mis-
        // reads a def's encoded structure as facts and the vacuous-binding
        // cartesian product below explodes (one `validate:` def fanned out
        // to 1024+ combos × thousands of def cells), allocating until OOM
        // on the full metamodel. Def cells carry no population facts, so
        // dedup never needs their keys. Discriminator: a `<prefix>:` name
        // not starting with `_`. Derived-fact cells dedup DOES need are
        // kept — colon-free FT ids (`Task_has_…`, `_transitive_…`) or
        // `_`-prefixed (`_cwa_negation:*`, `_sm_event_fold:*`), all retained
        // by the `!starts_with('_')` test. (The earlier plain
        // `contains(':')` filter wrongly dropped those `_`-prefixed derived
        // facts and broke dedup — hence the `_` carve-out.)
        if cell_name.contains(':') && !cell_name.starts_with('_') {
            continue;
        }
        let facts: alloc::vec::Vec<&ast::Object> =
            ast::cell_facts_iter(cell_contents).collect();
        for f in facts {
            let Some(pairs) = f.as_seq() else { continue };
            // Collect each binding pair with a list of canonical-key
            // variants. Vacuous Resource role values arrive in three
            // shapes in the wild: `Atom("")`, `Atom("φ")`, and
            // `Seq([])` (Object::phi). The chain emits whichever shape
            // its dispatch path uses (SM init produces both depending
            // on the role); persisted state may carry the third.
            // Emitting BOTH variant keys for the vacuous case ensures
            // dedup hits whichever shape the candidate's fact_key
            // hashes to. See state_keys_collides_seq_phi_with_atom_*
            // tests.
            let kv: alloc::vec::Vec<(&str, alloc::vec::Vec<&str>)> = pairs.iter().filter_map(|pair| {
                let items = pair.as_seq()?;
                let key = items.get(0)?.as_atom()?;
                let variants: alloc::vec::Vec<&str> = match items.get(1) {
                    Some(v) => match v.as_atom() {
                        Some("φ") | Some("") => alloc::vec![ "", "φ" ],
                        Some(a) => alloc::vec![a],
                        None => match v {
                            ast::Object::Seq(s) if s.is_empty() => alloc::vec![ "", "φ" ],
                            _ => alloc::vec![ "" ],
                        },
                    },
                    None => alloc::vec![ "" ],
                };
                Some((key, variants))
            }).collect();

            // Cartesian-product variant keys; each combination → one
            // FactKey. For facts with no vacuous bindings, this
            // degenerates to one key (the common case). For the
            // typically rare vacuous-binding case we emit 2^N keys
            // (N = number of vacuous roles, usually 1).
            let mut combos: alloc::vec::Vec<alloc::vec::Vec<(&str, &str)>> = alloc::vec![alloc::vec![]];
            for (k, variants) in &kv {
                // task-960 guard: this cartesian product is 2^N in the
                // number of vacuous-valued roles. A real population fact has
                // at most a handful (the note above: N is usually 1), so
                // `combos` stays tiny. But a def cell that slips past the
                // `<prefix>:`-skip above by having NO colon — the aggregate
                // `validate` cell, whose encoded body read as a single
                // pseudo-fact with kv=828 — would fan out to 2^N and exhaust
                // memory (the #960 OOM). Cap the expansion: no real fact
                // reaches 4096 variant keys, so this only ever bounds a
                // degenerate non-fact, whose partial keys are harmless (they
                // are FNV-namespaced by cell_name and cannot collide with a
                // real fact's key, so dedup of real facts is unaffected).
                if combos.len() > 4096 {
                    break;
                }
                let mut next = alloc::vec::Vec::with_capacity(combos.len() * variants.len());
                for combo in &combos {
                    for v in variants {
                        let mut extended = combo.clone();
                        extended.push((*k, *v));
                        next.push(extended);
                    }
                }
                combos = next;
            }
            for combo in combos {
                let mut sorted = combo;
                sorted.sort();
                let mut h = fnv_mix(FNV_OFFSET, cell_name.as_bytes());
                for (k, v) in &sorted {
                    h = fnv_mix(h, b"|");
                    h = fnv_mix(h, k.as_bytes());
                    h = fnv_mix(h, b"=");
                    h = fnv_mix(h, v.as_bytes());
                }
                set.insert(h);
            }
        }
    }
    set
}

// -- Proof Engine (Backward Chaining) ---------------------------------
// Given a goal fact, work backward through derivation rules to build a proof tree.
// Each step either finds the fact in the population (axiom), derives it via a rule
// (recursively proving antecedents), or concludes based on world assumption.

/// Attempt to prove a goal fact.
///
/// `goal` is a string like "Academic has Rank 'P'" -- a reading with optional values.
/// The engine searches the population for a matching fact, then tries derivation
/// Prove from Object state directly. No Domain reconstruction.
pub fn prove_from_state(state: &ast::Object, goal: &str, world_assumption: &WorldAssumption) -> ProofResult {
    let schemas = ast::fetch_cell_seq("FactType", state);
    let rules = ast::fetch_cell_seq("DerivationRule", state);
    let proof = prove_goal_state_pop(state, goal, &HashSet::new(), &schemas, &rules);
    let status = match &proof {
        Some(_) => ProofStatus::Proven,
        None => match world_assumption {
            WorldAssumption::Closed => ProofStatus::Disproven,
            WorldAssumption::Open => ProofStatus::Unknown,
        },
    };
    ProofResult { goal: goal.to_string(), status, proof, world_assumption: world_assumption.clone() }
}

fn prove_goal_state_pop(
    state: &ast::Object, goal: &str, visited: &HashSet<String>,
    schemas: &ast::Object, rules: &ast::Object,
) -> Option<ProofStep> {
    (!visited.contains(goal)).then_some(())?;
    let visited = &{ let mut v = visited.clone(); v.insert(goal.to_string()); v };

    let schema_reading = |ft_id: &str| -> Option<String> {
        schemas.as_seq()?.iter()
            .find(|s| ast::binding(s, "id") == Some(ft_id))
            .and_then(|s| ast::binding(s, "reading").map(|r| r.to_string()))
    };

    // Axiom search first (Step 1), else derivation search (Step 2).
    // `or_else` is Backus cond lifted into Option: axiom ? axiom : derive().
    ast::cells_iter(state).into_iter()
        .filter_map(|(ft_id, contents)| {
            let reading = schema_reading(ft_id)?;
            ast::cell_facts_iter(contents)
                .map(|fact| {
                    let bindings = extract_bindings(fact);
                    format_fact(&reading, &bindings)
                })
                .find(|fact_text| fact_text_matches(goal, fact_text, &reading))
                .map(|fact_text| ProofStep { fact: fact_text, justification: Justification::Axiom, children: vec![] })
        })
        .next()
        .or_else(|| rules.as_seq().and_then(|rule_list| {
        rule_list.iter().find_map(|rule| {
            let cons_ft_id = ast::binding(rule, "consequentFactTypeId")?.to_string();
            let cons_reading = schema_reading(&cons_ft_id)?;
            let goal_prefix = goal.split(' ').next().unwrap_or("");
            (goal.contains(&cons_reading) || cons_reading.contains(goal_prefix)).then_some(())?;

            let ant_ids_str = ast::binding(rule, "antecedentFactTypeIds")?.to_string();
            let child_proofs: Option<Vec<ProofStep>> = ant_ids_str.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .map(|ant_id| {
                    let ant_reading = schema_reading(&ant_id)?;
                    prove_goal_state_pop(state, &ant_reading, visited, schemas, rules)
                })
                .collect();

            let children = child_proofs.filter(|c| !c.is_empty())?;
            Some(ProofStep {
                fact: goal.to_string(),
                justification: Justification::Derived {
                    rule_id: ast::binding(rule, "id").unwrap_or("").to_string(),
                    rule_text: ast::binding(rule, "text").unwrap_or("").to_string(),
                },
                children,
            })
        })
    }))
}

/// Extract bindings from a fact Object as (key, value) pairs.
fn extract_bindings(fact: &ast::Object) -> Vec<(String, String)> {
    fact.as_seq().map(|pairs| {
        pairs.iter().filter_map(|pair| {
            let items = pair.as_seq()?;
            Some((items.get(0)?.as_atom()?.to_string(), items.get(1)?.as_atom()?.to_string()))
        }).collect()
    }).unwrap_or_default()
}

/// Format a fact from its reading template and bindings
#[allow(dead_code)] // called by prove_goal()
fn format_fact(reading: &str, bindings: &[(String, String)]) -> String {
    bindings.iter().fold(reading.to_string(), |result, (noun, value)| {
        result.find(noun.as_str())
            .map(|pos| format!("{}{} '{}'{}",  &result[..pos], noun, value, &result[pos + noun.len()..]))
            .unwrap_or(result)
    })
}

/// Check if a goal string matches a formatted fact
#[allow(dead_code)] // called by prove_goal()
fn fact_text_matches(goal: &str, fact_text: &str, reading: &str) -> bool {
    let goal_lower = goal.to_lowercase();
    let fact_lower = fact_text.to_lowercase();
    let reading_lower = reading.to_lowercase();
    goal == fact_text || goal == reading
        || goal_lower == fact_lower || goal_lower == reading_lower
        || fact_lower.contains(&goal_lower)
        || goal_lower.contains(&reading_lower)
}

// -- Synthesis --------------------------------------------------------

/// Synthesize from Object state directly.
pub fn synthesize_from_state(state: &ast::Object, noun_name: &str, depth: usize) -> SynthesisResult {
    let b = |f: &ast::Object, key: &str| -> String {
        ast::binding(f, key).unwrap_or("").to_string()
    };

    let wa = WorldAssumption::Closed;

    // 1. Find schemas where this noun plays a role (via Role facts)
    let role_cell = ast::fetch_cell_seq("Role", state);
    let role_facts = role_cell.as_seq().unwrap_or(&[]);
    let schema_ids_for_noun: Vec<(String, usize)> = role_facts.iter()
        .filter(|r| b(r, "nounName") == noun_name)
        .map(|r| (b(r, "factType"), b(r, "position").parse().unwrap_or(0)))
        .collect();

    let schema_cell = ast::fetch_cell_seq("FactType", state);
    let schema_facts = schema_cell.as_seq().unwrap_or(&[]);
    let participates_in: Vec<FactTypeSummary> = schema_ids_for_noun.iter()
        .filter_map(|(sid, role_idx)| {
            let reading = schema_facts.iter()
                .find(|s| b(s, "id") == *sid)
                .map(|s| b(s, "reading"))?;
            Some(FactTypeSummary { id: sid.clone(), reading, role_index: *role_idx })
        })
        .collect();

    // 2. Constraints spanning those fact types
    // Block-scoped ft_ids so its borrow on participates_in ends
    // before the move into SynthesisResult at end of function.
    let applicable_constraints: Vec<ConstraintSummary> = {
        let ft_ids: HashSet<&str> = participates_in.iter().map(|f| f.id.as_str()).collect();
        let constraint_cell = ast::fetch_cell_seq("Constraint", state);
        let constraint_facts = constraint_cell.as_seq().unwrap_or(&[]);
        let mut seen = HashSet::new();
        constraint_facts.iter()
            .filter(|c| {
                // Scan spans contiguously (0,1,2,…) until the first gap so
                // n-ary role-SEQUENCE subset constraints (>4 spans) are not
                // truncated — the superset-side FT can sit past index 3.
                let mut i = 0usize;
                loop {
                    let ft_key = format!("span{}_factTypeId", i);
                    let ft_id = b(c, &ft_key);
                    if ft_id.is_empty() { break false; }
                    if ft_ids.contains(ft_id.as_str()) { break true; }
                    i += 1;
                }
            })
            .filter(|c| seen.insert(b(c, "id")))
            .map(|c| ConstraintSummary {
                id: b(c, "id"), text: b(c, "text"), kind: b(c, "kind"),
                modality: b(c, "modality"), deontic_operator: {
                    let op = b(c, "deonticOperator");
                    if op.is_empty() { None } else { Some(op) }
                },
            })
            .collect()
    };

    // 3. State machines (from InstanceFact: "State Machine Definition 'X' is for Noun 'noun'")
    let inst_cell = ast::fetch_cell_seq("InstanceFact", state);
    let inst_facts = inst_cell.as_seq().unwrap_or(&[]);
    let state_machines: Vec<StateMachineSummary> = inst_facts.iter()
        .filter(|f| b(f, "subjectNoun") == "State Machine Definition" && b(f, "objectNoun") == "Noun" && b(f, "objectValue") == noun_name)
        .map(|f| {
            let sm_name = b(f, "subjectValue");
            let statuses: Vec<String> = inst_facts.iter()
                .filter(|s| b(s, "subjectNoun") == "Status" && b(s, "objectNoun") == "State Machine Definition" && b(s, "objectValue") == sm_name)
                .map(|s| b(s, "subjectValue"))
                .collect();
            let initial = inst_facts.iter()
                .find(|s| b(s, "subjectNoun") == "Status" && b(s, "fieldName") == "is initial in" && b(s, "objectValue") == sm_name)
                .map(|s| b(s, "subjectValue"))
                .unwrap_or_else(|| statuses.first().cloned().unwrap_or_default());
            let valid_transitions: Vec<String> = inst_facts.iter()
                .filter(|t| b(t, "subjectNoun") == "Transition" && b(t, "objectNoun") == "Event Type")
                .filter(|t| {
                    let trans_name = b(t, "subjectValue");
                    inst_facts.iter().any(|tf| b(tf, "subjectNoun") == "Transition" && b(tf, "subjectValue") == trans_name && b(tf, "objectNoun") == "Status" && b(tf, "objectValue") == initial && b(tf, "fieldName").contains("from"))
                })
                .map(|t| b(t, "objectValue"))
                .collect();
            StateMachineSummary { noun_name: sm_name, statuses, current_status: Some(initial), valid_transitions }
        })
        .collect();

    // 4. Related nouns
    let mut seen_related = HashSet::new();
    let related_nouns: Vec<RelatedNoun> = if depth > 0 {
        participates_in.iter()
            .flat_map(|fts| {
                role_facts.iter()
                    .filter(|r| b(r, "factType") == fts.id && b(r, "nounName") != noun_name)
                    .filter(|r| seen_related.insert(b(r, "nounName")))
                    .map(|r| RelatedNoun {
                        name: b(r, "nounName"),
                        via_fact_type: fts.id.clone(),
                        via_reading: fts.reading.clone(),
                        world_assumption: WorldAssumption::Closed,
                    })
                    .collect::<Vec<_>>()
            })
            .collect()
    } else { Vec::new() };

    SynthesisResult {
        noun_name: noun_name.to_string(), world_assumption: wa,
        participates_in, applicable_constraints, state_machines,
        derived_facts: Vec::new(), related_nouns,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hashbrown::HashMap;
    use crate::types::{
        ConstraintDef, DerivationRuleDef, FactTypeDef, GeneralInstanceFact, NounDef,
        RoleDef, SpanDef, TransitionDef, WorldAssumption,
    };

    // task-927 follow-up pin: when a stored fact carries a non-atom
    // (phi) binding value, e.g. `<Resource, Seq([])>`, AND the chain
    // emits the same conceptual fact as `<Resource, Atom("φ")>`, the
    // two MUST hash to the same FactKey. Without this, the chain
    // detects a key collision via cell_put_keyed every round but
    // upstream dedup misses, so the same rule re-fires the same
    // candidate every round until the LFP cap — burning 100 rounds ×
    // every rule on no-progress work. Live apps/tasks recompile took
    // 3+ minutes on this exact shape pre-fix.
    #[test]
    fn state_keys_collides_seq_phi_with_atom_phi() {
        // Build a state with a Seq([])-form Resource binding.
        let seq_phi_fact = ast::Object::seq(vec![
            ast::Object::seq(vec![ast::Object::atom("Resource"), ast::Object::phi()]),
            ast::Object::seq(vec![ast::Object::atom("Status"), ast::Object::atom("Proposed")]),
        ]);
        let state = ast::store(
            "Resource_is_currently_in_Status",
            ast::Object::seq(vec![seq_phi_fact]),
            &ast::Object::phi(),
        );

        // Build the candidate fact_key as the chain would: Atom("φ")
        // for the vacuous Resource binding.
        let candidate = DerivedFact {
            fact_type_id: "Resource_is_currently_in_Status".to_string(),
            reading: String::new(),
            bindings: vec![
                ("Resource".to_string(), "φ".to_string()),
                ("Status".to_string(), "Proposed".to_string()),
            ],
            derived_by: "test".to_string(),
            confidence: Confidence::Definitive,
        };

        let existing = state_keys(&state);
        let cand_key = fact_key(&candidate);

        assert!(existing.contains(&cand_key),
            "state_keys must hash <Resource, Seq([])> + <Status, Proposed> \
             identically to fact_key's <Resource, \"φ\"> + <Status, Proposed>. \
             Without this collision, the chain loops on the same UC-conflict \
             every round until the LFP cap. existing keys: {:?}; cand_key: {}",
             existing, cand_key);
    }

    // task-960 regression guard. state_keys must NOT explode on a def cell
    // whose encoded body, mis-read as a single "fact", carries many
    // vacuous-valued pseudo-bindings — the post-compile def-state's
    // aggregate `validate` cell (no colon, so it slips past the
    // `<prefix>:` def-skip) read as one pseudo-fact with kv=828, and the
    // vacuous-variant cartesian product fanned out to 2^N, OOMing the
    // full-metamodel createEntity. It must ALSO still key `_`-prefixed
    // derived-fact cells (`_cwa_negation:*`) that dedup needs — those are
    // deliberately carved out of the def-skip. Without the combos cap this
    // test allocates ~2^30 combos and OOMs the process.
    #[test]
    fn state_keys_caps_vacuous_explosion_and_keeps_underscore_derived() {
        // A `validate`-like def cell: one pseudo-fact with 30 vacuous bindings.
        let pseudo_fact = ast::Object::seq(
            (0..30).map(|i| ast::Object::seq(vec![
                ast::Object::atom(&format!("r{}", i)),
                ast::Object::phi(),
            ])).collect()
        );
        // A `_cwa_negation:` derived-fact cell dedup must track.
        let cwa_fact = ast::Object::seq(vec![
            ast::Object::seq(vec![ast::Object::atom("Task"), ast::Object::atom("t1")]),
        ]);
        let base = ast::Object::phi();
        let s1 = ast::store("_cwa_negation:Task", ast::Object::seq(vec![cwa_fact]), &base);
        let state = ast::store("validate", ast::Object::seq(vec![pseudo_fact]), &s1);

        // Completes (the cap prevents the 2^30 blow-up); a hang/OOM here is
        // the #960 regression.
        let keys = state_keys(&state);

        // The `_`-prefixed derived fact is kept (carve-out) and keyed, so
        // dedup recognises it as already-present.
        let cwa_candidate = DerivedFact {
            fact_type_id: "_cwa_negation:Task".to_string(),
            reading: String::new(),
            bindings: vec![("Task".to_string(), "t1".to_string())],
            derived_by: "test".to_string(),
            confidence: Confidence::Definitive,
        };
        assert!(keys.contains(&fact_key(&cwa_candidate)),
            "`_cwa_negation:` derived-fact key must be retained for dedup \
             (the `_`-leading carve-out from the def-cell skip). keys={:?}", keys);
    }

    // task-927 follow-up pin: same as above but with the empty-atom
    // shape `Atom("")` instead of `Atom("φ")`. The SM init / for-
    // Resource backfill code emits both shapes depending on the
    // dispatch path, and the chain must dedup against the stored
    // Seq([]) shape regardless.
    #[test]
    fn state_keys_collides_seq_phi_with_atom_empty() {
        let seq_phi_fact = ast::Object::seq(vec![
            ast::Object::seq(vec![ast::Object::atom("Resource"), ast::Object::phi()]),
            ast::Object::seq(vec![ast::Object::atom("Status"), ast::Object::atom("in_progress")]),
        ]);
        let state = ast::store(
            "Resource_is_currently_in_Status",
            ast::Object::seq(vec![seq_phi_fact]),
            &ast::Object::phi(),
        );

        let candidate = DerivedFact {
            fact_type_id: "Resource_is_currently_in_Status".to_string(),
            reading: String::new(),
            bindings: vec![
                ("Resource".to_string(), "".to_string()),
                ("Status".to_string(), "in_progress".to_string()),
            ],
            derived_by: "test".to_string(),
            confidence: Confidence::Definitive,
        };

        let existing = state_keys(&state);
        let cand_key = fact_key(&candidate);

        assert!(existing.contains(&cand_key),
            "state_keys must hash <Resource, Seq([])> identically to \
             fact_key's <Resource, \"\">. existing: {:?}; cand_key: {}",
            existing, cand_key);
    }

    /// task-3-incremental: round-1 rule gating must respect a caller-
    /// supplied `seed` of dirty cells. The classical semi-naive
    /// chainer treats `dirty_cells == None` as "run everything in
    /// round 1", which on the live tasks.db apply path (~75 task-
    /// relevant rules × 660 facts × N rounds) burns the full LFP
    /// budget on every single-field apply. `forward_chain_defs_state_seeded`
    /// is the entry point the apply path will use to seed round 1
    /// with exactly the cells the apply payload just wrote.
    ///
    /// Shape: each rule declares its antecedent reads. Seed with one
    /// cell. Count the active rules surfaced via
    /// `record_chain_eval_count` across all rounds. Rules whose reads
    /// don't intersect the seed (and don't get fed in later rounds)
    /// must be skipped.
    #[test]
    fn seeded_chain_gates_rules_by_dirty_cells_in_round_one() {
        // Empty population so the chain ends after one round (no
        // rule has anything to derive). The activation count is the
        // round-1 gate measurement, not the derivation result.
        let state = ast::Object::phi();
        // Each "rule" is the identity-on-phi: applying it yields
        // nothing, so the chain breaks after round 1 (`new_facts
        // .is_empty()`). The point of the test is the active-rule
        // count surfaced before derive_one_round_with_keys runs.
        let f_noop = ast::Func::constant(ast::Object::phi());

        // Build 10 single-read rules + one without metadata.
        // Names "rule_00".."rule_09" each read a distinct cell;
        // "rule_unknown" carries `None` antecedent reads (the
        // chainer must keep running it conservatively).
        let cells: Vec<Vec<String>> = (0..10)
            .map(|i| alloc::vec![alloc::format!("cell_{:02}", i)])
            .collect();
        let names: Vec<String> = (0..10)
            .map(|i| alloc::format!("rule_{:02}", i)).collect();
        let mut defs: Vec<(&str, &ast::Func, Option<&[String]>)> =
            names.iter().zip(cells.iter())
                .map(|(n, reads)| (n.as_str(), &f_noop, Some(reads.as_slice())))
                .collect();
        defs.push(("rule_unknown", &f_noop, None));

        // 1) Un-seeded run: classical semi-naive with `None` initial
        //    dirty. Round 1 runs every rule (10 gated + 1 unknown
        //    = 11). Chain breaks after round 1 since `f_noop`
        //    produces nothing.
        reset_chain_eval_count();
        let _ = forward_chain_defs_state_semi_naive(&defs, &state, 100);
        assert_eq!(
            get_chain_eval_count(), 11,
            "un-seeded semi-naive must run every rule in round 1 — \
             that's the cost the seeded variant is designed to skip",
        );

        // 2) Seeded run with {"cell_03"}: round 1 runs only
        //    rule_03 (intersects seed) + rule_unknown (conservative
        //    None metadata). All 9 other gated rules skipped.
        reset_chain_eval_count();
        let mut seed: HashSet<String> = HashSet::new();
        seed.insert("cell_03".to_string());
        let _ = forward_chain_defs_state_seeded(&defs, seed, &state, 100);
        assert_eq!(
            get_chain_eval_count(), 2,
            "seeded with {{cell_03}}, expected round 1 to activate \
             rule_03 + rule_unknown only (2 rules). Counter shows {}.",
            get_chain_eval_count(),
        );

        // 3) Seeded with an empty set: nothing dirty, no rule
        //    intersects. Only the conservative `None`-metadata rule
        //    fires (1 activation). This is the explicit no-op
        //    boundary case — useful for apply payloads that touch
        //    cells none of the gated rules read.
        reset_chain_eval_count();
        let _ = forward_chain_defs_state_seeded(
            &defs, HashSet::new(), &state, 100);
        assert_eq!(
            get_chain_eval_count(), 1,
            "empty seed must skip every gated rule; the conservative \
             unknown-reads rule still fires (count=1). Got {}.",
            get_chain_eval_count(),
        );
    }

    // ── Forward-chain non-termination guard (cli-apply-large-tasksdb-
    //    nonterminating) ──────────────────────────────────────────────
    //
    // Regression for the ~10 MB / ~870-entity tasks.db `create:Task` that
    // ran 40+ min and never returned: a derivation rule re-fired every
    // round (alethic-UC re-fire — `cell_put_keyed` drops a conflicting
    // re-derivation whose key the dedup never recognizes), so the LFP
    // churned all 100 rounds, each doing the full O(rules × population)
    // pass. The guard puts a WALL-CLOCK deadline on the whole chain,
    // checked at each round boundary; once it is past, the loop aborts
    // with a traced ⊥ naming the churning rule — turning the hang into a
    // fast, legible failure WITHOUT touching `apply`'s parallel recursion
    // (a fuel/reduction bound would force serial α and overflow the
    // stack). These tests pin: (1) a chain whose deadline has passed
    // aborts with the ⊥-trace + abort flag set, on BOTH the naive and
    // semi-naive loops; (2) a normal chain under the full budget converges
    // with the guard completely dormant (no abort, identical derived
    // facts) — the success path is untouched.
    //
    // The deadline is forced via `with_chain_budget(Duration::ZERO, …)`:
    // a zero budget means "every round boundary is already past the
    // deadline", reproducing "exceeded the bound" deterministically and
    // instantly — no real long-running chain in a unit test.

    /// A real, two-round-converging derivation rule + a population it
    /// fires over. `big_city ⊣ city_has_population` (no filter): every
    /// City row yields a Big City row, then round 2 dedups and the chain
    /// settles. Reused by the guard tests.
    fn chain_guard_fixture() -> (Vec<(String, ast::Func)>, ast::Object) {
        let cells = city_population_cells(None);
        let (_meta, defs, _def_map) = compile_cells(cells);
        let mut pop = ast::Object::phi();
        for i in 0..24 {
            let city = alloc::format!("City{:02}", i);
            pop = ast::cell_push(
                "city_has_population",
                ast::fact_from_pairs(&[("City", city.as_str()), ("Population", "1500000")]),
                &pop,
            );
        }
        (defs, pop)
    }

    #[test]
    fn naive_chain_aborts_with_traced_bottom_when_deadline_exceeded() {
        let (defs, pop) = chain_guard_fixture();
        let dd = derivation_defs_from(&defs);
        assert!(!dd.is_empty(), "fixture must compile at least one derivation rule");

        // Run the NAIVE loop (forward_chain_defs_state → _bounded) with a
        // ZERO time budget — the first round boundary is already past the
        // deadline — and ⊥-tracing armed. The chain must abort, not spin.
        let _ = take_chain_abort(); // clear any stray flag first.
        let ((_state, _derived), trace) = ast::with_bottom_trace(|| {
            with_chain_budget(core::time::Duration::ZERO,
                || forward_chain_defs_state(&dd, &pop))
        });

        assert!(take_chain_abort(),
            "naive chain past its deadline must raise the abort flag");
        let trace = trace.expect(
            "an armed ⊥-trace must be populated when the chain aborts");
        let rule = trace.rule.as_deref().unwrap_or("");
        assert!(rule.starts_with("derivation:"),
            "the ⊥-trace must name the derivation rule the chain was churning on; \
             got rule={:?} (full trace: {:?})", rule, trace);
        // `describe()` must render a non-empty ⊥-origin string for the user.
        assert!(trace.describe().map_or(false, |s| s.starts_with("⊥ origin:")),
            "trace.describe() must produce a ⊥-origin line; got {:?}", trace.describe());
    }

    #[test]
    fn semi_naive_chain_aborts_with_traced_bottom_when_deadline_exceeded() {
        let (defs, pop) = chain_guard_fixture();
        // Seeded entry point → semi_naive_inner (the path `create:Task`
        // takes). Seed the antecedent cell so round 1 activates the rule.
        let packed: Vec<(&str, &ast::Func, Option<&[String]>)> = defs.iter()
            .filter(|(n, _)| n.starts_with("derivation:"))
            .map(|(n, f)| (n.as_str(), f, None))
            .collect();
        assert!(!packed.is_empty(), "fixture must compile a derivation rule");
        let mut seed = HashSet::new();
        seed.insert("city_has_population".to_string());

        let _ = take_chain_abort();
        let ((_state, _derived), trace) = ast::with_bottom_trace(|| {
            with_chain_budget(core::time::Duration::ZERO, || {
                forward_chain_defs_state_seeded(&packed, seed.clone(), &pop, 100)
            })
        });

        assert!(take_chain_abort(),
            "semi-naive chain past its deadline must raise the abort flag");
        let trace = trace.expect("armed ⊥-trace must be set on semi-naive abort");
        assert!(trace.rule.as_deref().unwrap_or("").starts_with("derivation:"),
            "semi-naive ⊥-trace must name the churning derivation rule; got {:?}", trace);
    }

    #[test]
    fn healthy_chain_under_full_budget_does_not_trip_guard() {
        let (defs, pop) = chain_guard_fixture();
        let dd = derivation_defs_from(&defs);

        // No `with_chain_budget`: the chain runs under the full 3-minute
        // CHAIN_BUDGET. A 24-City population converges in two rounds in
        // microseconds — the guard must stay completely dormant.
        // (We assert on the ABORT FLAG, not the ⊥-trace: `apply` may arm
        // the trace for an intermediate ⊥ that the chain legitimately
        // absorbs — the trace is contractually meaningful only when the
        // TOP-level result is ⊥, which a successful chain's is not. The
        // abort flag is the definitive "did the guard fire" signal.)
        let _ = take_chain_abort();
        let (_state, derived) = forward_chain_defs_state(&dd, &pop);

        assert!(!take_chain_abort(),
            "a normal, converging chain must NOT raise the abort flag — \
             the guard is a non-termination safety net, not a normal-path gate");
        // And the rule actually fired: every City became a Big City.
        let big = derived.iter().filter(|d| d.fact_type_id == "big_city").count();
        assert_eq!(big, 24,
            "healthy chain must derive one Big City per City (24); got {}", big);
    }

    #[test]
    fn chain_abort_flag_auto_clears_between_runs() {
        // The abort flag must not leak from an aborted chain into a later,
        // unrelated one on the same thread: `take_chain_abort` read-and-
        // clears, and a fresh healthy chain never re-sets it.
        let (defs, pop) = chain_guard_fixture();
        let dd = derivation_defs_from(&defs);

        let _ = take_chain_abort();
        let _ = with_chain_budget(core::time::Duration::ZERO,
            || forward_chain_defs_state(&dd, &pop));
        assert!(take_chain_abort(), "first (deadline-exceeded) run must set the flag");
        // Second read with no chain in between: already cleared.
        assert!(!take_chain_abort(), "take_chain_abort must clear the flag");
        // A healthy run afterwards leaves it clear.
        let _ = forward_chain_defs_state(&dd, &pop);
        assert!(!take_chain_abort(),
            "a healthy run after an aborted one must NOT carry the stale flag");
    }

    // Test-only SM fixture. The production SM compile path is fully
    // cell-driven (#763) — there's no public typed `StateMachineDef`
    // in flight any more. This struct exists solely so the existing
    // test corpora that build SMs by literal can stay readable; the
    // `with_state_machine` helper below fans these fields out to the
    // normalized SM cells the compile path actually consumes.
    #[derive(Debug, Clone, Default)]
    struct StateMachineDef {
        noun_name: String,
        statuses: Vec<String>,
        transitions: Vec<TransitionDef>,
        initial: String,
    }

    fn empty_state() -> ast::Object {
        ast::Object::phi()
    }

    fn make_noun(object_type: &str) -> NounDef {
        NounDef {
            object_type: object_type.to_string(),
            world_assumption: WorldAssumption::default(),
        }
    }

    /// Build Object state with facts from pairs.
    fn state_with_facts(ft_id: &str, pairs_list: &[&[(&str, &str)]]) -> ast::Object {
        pairs_list.iter().fold(ast::Object::phi(), |acc, pairs|
            ast::cell_push(ft_id, ast::fact_from_pairs(pairs), &acc))
    }

    // ── Cell-push test builders (no Domain IR) ──────────────────────────
    //
    // Tests build Object cells directly via these helpers. All helpers take
    // and return Object — facts all the way down. The `S` alias is a
    // convenience for the working map; terminate with `build(cells)`.

    type S = HashMap<String, Vec<ast::Object>>;

    fn empty_cells() -> S { HashMap::new() }

    fn build(cells: S) -> ast::Object {
        ast::Object::Map(cells.into_iter()
            .map(|(k, v)| (k, ast::Object::Seq(v.into())))
            .collect::<hashbrown::HashMap<_, _>>().into())
    }

    fn with_noun(mut cells: S, name: &str, def: &NounDef) -> S {
        let wa = match def.world_assumption {
            WorldAssumption::Closed => "closed", WorldAssumption::Open => "open",
        };
        let ref_scheme = (def.object_type == "entity").then(|| "id");
        let mut pairs: Vec<(&str, &str)> = vec![
            ("name", name), ("objectType", def.object_type.as_str()),
            ("worldAssumption", wa),
        ];
        if let Some(rs) = ref_scheme { pairs.push(("referenceScheme", rs)); }
        cells.entry("Noun".into()).or_default().push(ast::fact_from_pairs(&pairs));
        cells
    }

    fn with_ft(mut cells: S, id: &str, ft: &FactTypeDef) -> S {
        let arity = ft.roles.len().to_string();
        cells.entry("FactType".into()).or_default().push(ast::fact_from_pairs(&[
            ("id", id), ("reading", ft.reading.as_str()), ("arity", arity.as_str()),
        ]));
        for role in &ft.roles {
            let pos = role.role_index.to_string();
            cells.entry("Role".into()).or_default().push(ast::fact_from_pairs(&[
                ("factType", id), ("nounName", role.noun_name.as_str()), ("position", pos.as_str()),
            ]));
        }
        cells
    }

    fn with_constraint(mut cells: S, c: &ConstraintDef) -> S {
        cells.entry("Constraint".into()).or_default()
            .push(crate::parse_forml2::constraint_to_fact_test(c));
        cells
    }

    fn with_derivation(mut cells: S, r: &DerivationRuleDef) -> S {
        let json = serde_json::to_string(r).unwrap_or_default();
        let consequent_encoded = r.consequent_cell.encode();
        cells.entry("DerivationRule".into()).or_default().push(ast::fact_from_pairs(&[
            ("id", r.id.as_str()), ("text", r.text.as_str()),
            ("consequentFactTypeId", consequent_encoded.as_str()),
            ("json", json.as_str()),
        ]));
        cells
    }

    /// ss-autofill-retire-2 — the SS (Subset) Constraint auto-fill metamodel
    /// rule as a `DerivationRuleDef` in its post-resolution shape: sole
    /// antecedent `FactType("SubsetConstraint")`, empty consequent. The
    /// procedural `compile_ss_autofill_metamodel` synthesiser is RETIRED;
    /// `compile_explicit_derivation` detects this antecedent shape as the
    /// reading-lift and drives the per-SS-Constraint copy-fanout off
    /// `CellIndex::ss_autofill_pairs`. These cell-built fixtures bypass the
    /// `parse_to_state_via_stage12_impl` injection (and `subset_autofill` has
    /// no FORML surface syntax), so they inject the rule explicitly — the
    /// manual-state analog of the parse-path injection. Building the struct
    /// directly (not from text) means `cell_index_from_state`'s lossless JSON
    /// path keeps the `FactType("SubsetConstraint")` antecedent as-is.
    fn ss_autofill_rule() -> DerivationRuleDef {
        DerivationRuleDef {
            id: "rule_c210dd625f8eeaf3".to_string(),
            text: "Fact Type has auto-filled Fact iff some Subset Constraint \
                   has antecedent Fact Type Ant and that Subset Constraint has \
                   consequent Fact Type Cons and that Subset Constraint has \
                   autofill 'true' and that Fact is instance of Ant and that \
                   Fact Type is Cons".to_string(),
            antecedent_sources: vec![AntecedentSource::FactType("SubsetConstraint".to_string())],
            consequent_cell: ConsequentCellSource::Literal(String::new()),
            consequent_instance_role: String::new(),
            kind: DerivationKind::ModusPonens,
            join_on: vec![], match_on: vec![], consequent_bindings: vec![],
            antecedent_filters: vec![], consequent_computed_bindings: vec![],
            consequent_aggregates: vec![], consequent_universals: vec![], unresolved_clauses: vec![],
            antecedent_role_literals: vec![], antecedent_role_comparisons: vec![],
            consequent_role_literals: vec![], materialization: MaterializationPolicy::Stored,
            ring_join: None, skolem_head_roles: vec![], antecedent_cardinalities: vec![],
        }
    }

    /// Push SM cells (cell-driven path only — #763 retired the JSON-blob
    /// `StateMachine` cell and the typed-struct fallback).
    ///
    /// Cell shapes match what the parser's instance-fact fanout would
    /// emit for the equivalent FORML 2 statements declared in
    /// `readings/core/state.md`:
    ///
    ///   `State Machine Definition 'name' is for Noun 'sm.noun_name'`
    ///   `Status 'sm.initial' is initial in State Machine Definition 'name'`
    ///       — only when initial is explicitly declared
    ///   `Status 'S' is defined in State Machine Definition 'name'`
    ///       — for every S in sm.statuses
    ///   `Transition 'T_i' is defined in State Machine Definition 'name'`
    ///   `Transition 'T_i' is from Status 'from'`
    ///   `Transition 'T_i' is to Status 'to'`
    ///   `Transition 'T_i' is triggered by Fact Type 'event'`
    ///
    /// Pass 4 (graph-derived initial, #760) is also pre-computed here:
    /// the source-never-target Statuses are written to
    /// `Status_is_rooted_in_State_Machine_Definition`, mirroring what
    /// the readings/core/state.md derivation rule would produce after
    /// forward-chaining over the Transition cells. The consumer-side
    /// uniqueness gate in `compile_state_machine_from_cells` enforces
    /// |rooted| == 1 ⇒ promote-to-initial; ambiguity leaves initial
    /// empty so the runtime fails visibly at first SM call.
    fn with_state_machine(mut cells: S, name: &str, sm: &StateMachineDef) -> S {
        // SM-for-Noun: binds the SM definition to the noun it governs.
        cells.entry("State_Machine_Definition_is_for_Noun".into()).or_default().push(
            ast::fact_from_pairs(&[
                ("State Machine Definition", name),
                ("Noun", sm.noun_name.as_str()),
            ]));

        // Explicit initial declaration (Pass 2). Skip when empty so the
        // cell-driven path falls through to the rooted-cell Pass 4 fold.
        if !sm.initial.is_empty() {
            cells.entry("Status_is_initial_in_State_Machine_Definition".into()).or_default()
                .push(ast::fact_from_pairs(&[
                    ("Status", sm.initial.as_str()),
                    ("State Machine Definition", name),
                ]));
        }

        // Status set (Pass 1/2/2b). Push every declared status so the
        // cell-driven path sees the same status surface the typed path
        // builds via Passes 2 + 2b + 3 backfill.
        for s in &sm.statuses {
            cells.entry("Status_is_defined_in_State_Machine_Definition".into()).or_default()
                .push(ast::fact_from_pairs(&[
                    ("Status", s.as_str()),
                    ("State Machine Definition", name),
                ]));
        }

        // Transitions (Pass 3). Each Transition needs a stable synthetic
        // name; we use the event field — events are unique per SM by
        // construction in this test surface, and the cell-driven path
        // joins on Transition name across the four `Transition_is_*`
        // cells so the name only needs to be internally consistent.
        for t in &sm.transitions {
            let t_name = t.event.as_str();
            cells.entry("Transition_is_defined_in_State_Machine_Definition".into()).or_default()
                .push(ast::fact_from_pairs(&[
                    ("Transition", t_name),
                    ("State Machine Definition", name),
                ]));
            cells.entry("Transition_is_from_Status".into()).or_default()
                .push(ast::fact_from_pairs(&[
                    ("Transition", t_name),
                    ("Status", t.from.as_str()),
                ]));
            cells.entry("Transition_is_to_Status".into()).or_default()
                .push(ast::fact_from_pairs(&[
                    ("Transition", t_name),
                    ("Status", t.to.as_str()),
                ]));
            cells.entry("Transition_is_triggered_by_Fact_Type".into()).or_default()
                .push(ast::fact_from_pairs(&[
                    ("Transition", t_name),
                    ("Fact Type", t.event.as_str()),
                ]));
        }

        // Pass 4 rooted-cell computation. Mirrors the source-never-
        // target topology fold the parser-side derivation rule in
        // readings/core/state.md produces. Only emits when no explicit
        // initial exists — when an explicit initial is present it
        // always wins and the rooted cell is unused.
        if sm.initial.is_empty() {
            // Source set: from-Statuses across all transitions.
            // Target set: to-Statuses across all transitions.
            // Rooted := Source \ Target.
            let sources: Vec<&str> = sm.transitions.iter()
                .map(|t| t.from.as_str()).collect();
            let targets: Vec<&str> = sm.transitions.iter()
                .map(|t| t.to.as_str()).collect();
            // Stable, dedup'd rooted set. Cardinality > 1 means the
            // consumer's uniqueness gate refuses to infer an initial,
            // which matches the typed path's "ambiguous" branch.
            let mut seen: Vec<&str> = Vec::new();
            for s in &sources {
                if !targets.contains(s) && !seen.contains(s) {
                    seen.push(*s);
                    cells.entry("Status_is_rooted_in_State_Machine_Definition".into())
                        .or_default()
                        .push(ast::fact_from_pairs(&[
                            ("Status", s),
                            ("State Machine Definition", name),
                        ]));
                }
            }
        }

        cells
    }

    #[allow(dead_code)]
    fn with_instance_fact(mut cells: S, f: &GeneralInstanceFact) -> S {
        cells.entry("InstanceFact".into()).or_default().push(ast::fact_from_pairs(&[
            ("subjectNoun", f.subject_noun.as_str()),
            ("subjectValue", f.subject_value.as_str()),
            ("fieldName", f.field_name.as_str()),
            ("objectNoun", f.object_noun.as_str()),
            ("objectValue", f.object_value.as_str()),
        ]));
        let object = if f.object_noun.is_empty() { f.field_name.as_str() } else { f.object_noun.as_str() };
        cells.entry(f.field_name.clone()).or_default().push(ast::fact_from_pairs(&[
            (f.subject_noun.as_str(), f.subject_value.as_str()),
            (object, f.object_value.as_str()),
        ]));
        cells
    }

    fn with_enum_values(mut cells: S, name: &str, obj_type: &str, values: &[String]) -> S {
        let wa = "closed";
        let ref_scheme = (obj_type == "entity").then(|| "id");
        let joined = values.join(",");
        let mut pairs: Vec<(&str, &str)> = vec![
            ("name", name), ("objectType", obj_type),
            ("worldAssumption", wa),
            ("enumValues", joined.as_str()),
        ];
        if let Some(rs) = ref_scheme { pairs.push(("referenceScheme", rs)); }
        cells.entry("Noun".into()).or_default().push(ast::fact_from_pairs(&pairs));
        cells
    }

    /// Compile a cell map into (state, defs, def_map). Mirrors the old
    /// ir_to_defs API but takes cell-push-built state, not a typed Domain.
    fn compile_cells(cells: S) -> (ast::Object, Vec<(String, ast::Func)>, ast::Object) {
        let state = build(cells);
        let (defs, def_map) = state_to_defs(&state);
        (state, defs, def_map)
    }

    /// Compile the Object state into defs + def_map.
    fn state_to_defs(state: &ast::Object) -> (Vec<(String, ast::Func)>, ast::Object) {
        let model = crate::compile::compile(state);
        let defs: Vec<(String, ast::Func)> = model.constraints.iter()
            .map(|c| (format!("constraint:{}", c.id), c.func.clone()))
            .chain(model.state_machines.iter().flat_map(|sm| [
                (format!("machine:{}", sm.noun_name), sm.func.clone()),
                (format!("machine:{}:initial", sm.noun_name), ast::Func::constant(ast::Object::atom(&sm.initial))),
            ]))
            .chain(model.derivations.iter().map(|d| (format!("derivation:{}", d.id), d.func.clone())))
            .chain(model.schemas.iter().map(|(id, schema)| (format!("schema:{}", id), schema.construction.clone())))
            .collect();
        let def_map = ast::defs_to_state(&defs, state);
        (defs, def_map)
    }

    /// Evaluate constraints via defs.
    fn eval_constraints_defs(
        defs: &[(String, ast::Func)],
        def_map: &ast::Object,
        text: &str,
        sender: Option<&str>,
        state: &ast::Object,
    ) -> Vec<Violation> {
        let ctx_obj = ast::encode_eval_context_state(text, sender, state);
        defs.iter()
            .filter(|(n, _)| n.starts_with("constraint:"))
            .flat_map(|(name, func)| {
                let result = ast::apply(func, &ctx_obj, def_map);
                let is_deontic = name.contains("obligatory") || name.contains("forbidden");
                ast::decode_violations(&result).into_iter().map(move |mut v| {
                    v.alethic = !is_deontic;
                    v
                })
            })
            .collect()
    }

    /// Run a state machine from defs (replaces run_machine_ast).
    fn run_machine_defs(
        defs: &[(String, ast::Func)],
        def_map: &ast::Object,
        noun_name: &str,
        events: &[&str],
    ) -> String {
        let machine_key = format!("machine:{}", noun_name);
        let initial_key = format!("machine:{}:initial", noun_name);
        let func = defs.iter().find(|(n, _)| *n == machine_key).map(|(_, f)| f);
        let initial = defs.iter().find(|(n, _)| *n == initial_key)
            .and_then(|(_, f)| {
                let r = ast::apply(f, &ast::Object::phi(), def_map);
                r.as_atom().map(|s| s.to_string())
            })
            .unwrap_or_default();

        let func = match func {
            Some(f) => f,
            None => return initial,
        };

        events.into_iter().fold(initial, |state, event| {
            let input = ast::Object::seq(vec![
                ast::Object::atom(&state),
                ast::Object::atom(event),
            ]);
            let result = ast::apply(func, &input, def_map);
            result.as_atom().map(|s| s.to_string()).unwrap_or(state)
        })
    }

    /// Extract derivation defs from the full defs list.
    fn derivation_defs_from<'a>(defs: &'a [(String, ast::Func)]) -> Vec<(&'a str, &'a ast::Func)> {
        defs.iter()
            .filter(|(n, _)| n.starts_with("derivation:"))
            .map(|(n, f)| (n.as_str(), f))
            .collect()
    }

    // -- DEFS evaluation path tests ------------------------------------

    /// Post-task-820: alethic UC violations on keyed cells surface
    /// via `ast::cell_put_keyed` returning `Err(KeyConflict)`, not
    /// via the constraint evaluator. The Func is a no-op φ for these
    /// UCs (storage IS the constraint). This test migrated from the
    /// legacy "constraint detects duplicate" assertion to the new
    /// "storage layer rejects the write" contract.
    #[test]
    fn test_uniqueness_violation_surfaces_via_cell_put_keyed_err() {
        let f1 = ast::fact_from_pairs(&[("Person", "Alice"), ("Name", "A")]);
        let f2 = ast::fact_from_pairs(&[("Person", "Alice"), ("Name", "B")]);
        let state = ast::Object::phi();
        let state = ast::cell_put_keyed("ft1", &["Person"], f1, &state)
            .expect("first put must succeed");
        let conflict = ast::cell_put_keyed("ft1", &["Person"], f2, &state)
            .expect_err("duplicate Person 'Alice' must be a KeyConflict");
        assert_eq!(conflict.name, "ft1");
        assert_eq!(conflict.key, "Alice");
    }

    #[test]
    fn test_evaluate_via_ast_no_violations() {
        let mut cells = empty_cells();
        cells = with_ft(cells, "ft1", &FactTypeDef {
            schema_id: String::new(),
            reading: "Person has Name".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Person".to_string(), role_index: 0 },
                RoleDef { noun_name: "Name".to_string(), role_index: 1 },
            ],
        });
        cells = with_constraint(cells, &ConstraintDef {
            id: "uc1".to_string(),
            kind: "UC".to_string(),
            modality: "Alethic".to_string(),
            text: "Each Person has at most one Name".to_string(),
            spans: vec![crate::types::SpanDef {
                fact_type_id: "ft1".to_string(),
                role_index: 0,
                subset_autofill: None,
            }],
            ..Default::default()
        });

        let (_meta_state, defs, def_map) = compile_cells(cells);

        let state = state_with_facts("ft1", &[
            &[("Person", "Alice"), ("Name", "A")],
        ]);

        let violations = eval_constraints_defs(&defs, &def_map, "", None, &state);
        assert_eq!(violations.len(), 0);
    }

    #[test]
    fn test_run_machine_via_ast() {
        // Domain Change state machine: Proposed -> Under Review -> Approved -> Applied
        let mut cells = empty_cells();
        cells = with_state_machine(cells, "DomainChange", &StateMachineDef {
            noun_name: "DomainChange".to_string(),
            statuses: vec![
                "Proposed".to_string(),
                "Under Review".to_string(),
                "Approved".to_string(),
                "Applied".to_string(),
                "Rejected".to_string(),
            ],
            transitions: vec![
                TransitionDef { from: "Proposed".to_string(), to: "Under Review".to_string(), event: "review-requested".to_string(), guard: None },
                TransitionDef { from: "Under Review".to_string(), to: "Approved".to_string(), event: "approved".to_string(), guard: None },
                TransitionDef { from: "Under Review".to_string(), to: "Rejected".to_string(), event: "rejected".to_string(), guard: None },
                TransitionDef { from: "Approved".to_string(), to: "Applied".to_string(), event: "applied".to_string(), guard: None },
            ],
            initial: String::new(),
        });

        let (_meta_state, defs, def_map) = compile_cells(cells);

        // Happy path: Proposed -> Under Review -> Approved -> Applied
        let final_state = run_machine_defs(&defs, &def_map, "DomainChange", &["review-requested", "approved", "applied"]);
        assert_eq!(final_state, "Applied");

        // Rejection path: Proposed -> Under Review -> Rejected
        let final_state = run_machine_defs(&defs, &def_map, "DomainChange", &["review-requested", "rejected"]);
        assert_eq!(final_state, "Rejected");

        // Invalid event: stays in current state
        let final_state = run_machine_defs(&defs, &def_map, "DomainChange", &["applied"]);
        assert_eq!(final_state, "Proposed"); // "applied" not valid from Proposed

        // Partial: just review
        let final_state = run_machine_defs(&defs, &def_map, "DomainChange", &["review-requested"]);
        assert_eq!(final_state, "Under Review");
    }

    #[test]
    fn test_initial_status_from_graph_topology() {
        // No explicit `Status is initial in SM` fact. Graph topology has a
        // unique source-never-target ("Pending" is never a transition
        // target), so compile derives initial from the transition facts.
        let mut cells = empty_cells();
        cells = with_state_machine(cells, "SM", &StateMachineDef {
            noun_name: "Order".to_string(),
            statuses: vec!["Pending".to_string(), "Shipped".to_string(), "Delivered".to_string()],
            transitions: vec![
                TransitionDef { from: "Pending".to_string(), to: "Shipped".to_string(), event: "ship".to_string(), guard: None },
                TransitionDef { from: "Shipped".to_string(), to: "Delivered".to_string(), event: "deliver".to_string(), guard: None },
            ],
            initial: String::new(),
        });
        let (_meta_state, defs, def_map) = compile_cells(cells);
        let initial_key = "machine:Order:initial";
        let initial = defs.iter().find(|(n, _)| n == initial_key)
            .and_then(|(_, f)| {
                let r = ast::apply(f, &ast::Object::phi(), &def_map);
                r.as_atom().map(|s| s.to_string())
            })
            .unwrap_or_default();
        assert_eq!(initial, "Pending", "graph topology: Pending is source-never-target");
    }

    #[test]
    fn test_initial_status_from_explicit_declaration() {
        // Explicit `initial: "Shipped"` on the SM def (mirrors
        // `Status 'Shipped' is initial in SM 'Order'.` instance fact).
        // Even though graph topology would suggest "Pending" (source-
        // never-target), the explicit declaration wins.
        let mut cells = empty_cells();
        cells = with_state_machine(cells, "SM", &StateMachineDef {
            noun_name: "Order".to_string(),
            statuses: vec!["Pending".to_string(), "Shipped".to_string(), "Delivered".to_string()],
            transitions: vec![
                TransitionDef { from: "Pending".to_string(), to: "Shipped".to_string(), event: "ship".to_string(), guard: None },
                TransitionDef { from: "Shipped".to_string(), to: "Delivered".to_string(), event: "deliver".to_string(), guard: None },
            ],
            initial: "Shipped".to_string(),
        });
        let (_meta_state, defs, def_map) = compile_cells(cells);
        let initial = defs.iter().find(|(n, _)| n == "machine:Order:initial")
            .and_then(|(_, f)| ast::apply(f, &ast::Object::phi(), &def_map).as_atom().map(|s| s.to_string()))
            .unwrap_or_default();
        assert_eq!(initial, "Shipped", "explicit declaration overrides graph topology");
    }

    /// task-6 / #6 regression: SM-init must emit the INITIAL status for a
    /// "bare" instance — one that exists (plays the SM noun's role in some
    /// cell) but has NO transition events and no prior SM row. In the live
    /// apps/tasks DB, 0 of 661 entities carried the initial 'pending'
    /// status: this emit never fired, so only event-fold produced statuses
    /// and un-transitioned tasks (922-929) ended up with no state machine.
    #[test]
    fn sm_init_emits_initial_status_for_bare_instance() {
        let mut cells = empty_cells();
        cells = with_noun(cells, "Order", &make_noun("entity"));
        cells = with_noun(cells, "Name", &make_noun("value"));
        cells = with_ft(cells, "Order_has_Name", &FactTypeDef {
            schema_id: String::new(),
            reading: "Order has Name".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Order".to_string(), role_index: 0 },
                RoleDef { noun_name: "Name".to_string(), role_index: 1 },
            ],
        });
        cells = with_state_machine(cells, "OrderSM", &StateMachineDef {
            noun_name: "Order".to_string(),
            statuses: vec!["Draft".to_string(), "Placed".to_string(), "Shipped".to_string()],
            transitions: vec![
                TransitionDef { from: "Draft".to_string(), to: "Placed".to_string(), event: "Order_was_placed".to_string(), guard: None },
                TransitionDef { from: "Placed".to_string(), to: "Shipped".to_string(), event: "Order_was_shipped".to_string(), guard: None },
            ],
            initial: "Draft".to_string(),
        });
        // A bare Order instance: exists via Order_has_Name, no events.
        cells.entry("Order_has_Name".to_string()).or_default().push(
            ast::fact_from_pairs(&[("Order", "o1"), ("Name", "Widget")]));

        let (state, defs, _def_map) = compile_cells(cells);
        let dd = derivation_defs_from(&defs);
        let (_new_state, derived) = forward_chain_defs_state(&dd, &state);

        let got: Vec<_> = derived.iter()
            .filter(|d| d.fact_type_id == "State_Machine_is_currently_in_Status")
            .collect();
        let has_o1_draft = got.iter().any(|d| {
            let sm = d.bindings.iter().find(|(k, _)| k == "State Machine").map(|(_, v)| v.as_str());
            let st = d.bindings.iter().find(|(k, _)| k == "Status").map(|(_, v)| v.as_str());
            sm == Some("o1") && st == Some("Draft")
        });
        assert!(has_o1_draft,
            "SM-init must emit initial status 'Draft' for bare instance o1 \
             (no events). Got currently_in_Status emits: {:?}", got);
    }

    /// task-6 / #6 scale repro: a bare instance alongside an
    /// event-bearing one. In round 1 of recompile, for_Resource is empty
    /// (just dropped) but event cells are present, so is_new must skip the
    /// event-bearing instance (event-fold owns it) while still emitting the
    /// initial status for the bare one.
    #[test]
    fn sm_init_emits_initial_for_bare_instance_when_another_has_event() {
        let mut cells = empty_cells();
        cells = with_noun(cells, "Order", &make_noun("entity"));
        cells = with_noun(cells, "Name", &make_noun("value"));
        cells = with_ft(cells, "Order_has_Name", &FactTypeDef {
            schema_id: String::new(), reading: "Order has Name".to_string(), readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Order".to_string(), role_index: 0 },
                RoleDef { noun_name: "Name".to_string(), role_index: 1 },
            ],
        });
        cells = with_ft(cells, "Order_was_placed", &FactTypeDef {
            schema_id: String::new(), reading: "Order was placed".to_string(), readings: vec![],
            roles: vec![RoleDef { noun_name: "Order".to_string(), role_index: 0 }],
        });
        cells = with_state_machine(cells, "OrderSM", &StateMachineDef {
            noun_name: "Order".to_string(),
            statuses: vec!["Draft".to_string(), "Placed".to_string(), "Shipped".to_string()],
            transitions: vec![
                TransitionDef { from: "Draft".to_string(), to: "Placed".to_string(), event: "Order_was_placed".to_string(), guard: None },
                TransitionDef { from: "Placed".to_string(), to: "Shipped".to_string(), event: "Order_was_shipped".to_string(), guard: None },
            ],
            initial: "Draft".to_string(),
        });
        // o1 bare; o2 has a placed event (event-fold owns it).
        cells.entry("Order_has_Name".to_string()).or_default().push(
            ast::fact_from_pairs(&[("Order", "o1"), ("Name", "Widget")]));
        cells.entry("Order_has_Name".to_string()).or_default().push(
            ast::fact_from_pairs(&[("Order", "o2"), ("Name", "Gadget")]));
        cells.entry("Order_was_placed".to_string()).or_default().push(
            ast::fact_from_pairs(&[("Order", "o2")]));

        let (state, defs, _def_map) = compile_cells(cells);
        let dd = derivation_defs_from(&defs);
        let (_new_state, derived) = forward_chain_defs_state(&dd, &state);

        let status_of = |id: &str| -> Vec<String> {
            derived.iter()
                .filter(|d| d.fact_type_id == "State_Machine_is_currently_in_Status")
                .filter(|d| d.bindings.iter().any(|(k, v)| k == "State Machine" && v == id))
                .filter_map(|d| d.bindings.iter().find(|(k, _)| k == "Status").map(|(_, v)| v.clone()))
                .collect()
        };
        assert!(status_of("o1").iter().any(|s| s == "Draft"),
            "bare o1 must get initial 'Draft'; got {:?}", status_of("o1"));
    }

    /// REPRO (task sm-fold-as-predicate): the COMPILED event-fold must respect
    /// each transition's `from` status, mirroring the abstract run_machine fold
    /// (test_valid_transitions_from_status). o1 carries ONLY `Order_was_shipped`
    /// (from=Placed) but was never placed, so it is in Draft and shipped must
    /// NOT fire. The unguarded fold (compile.rs:7719 discards `from`) wrongly
    /// emits Shipped.
    ///
    /// task sm-fold-as-predicate: now GREEN — `compile_sm_event_fold` is
    /// from-guarded (semi-join against `State_Machine_is_currently_in_Status`),
    /// so a transition fires only from its declared `from`.
    #[test]
    fn event_fold_respects_from_status() {
        let mut cells = empty_cells();
        cells = with_noun(cells, "Order", &make_noun("entity"));
        cells = with_noun(cells, "Name", &make_noun("value"));
        cells = with_ft(cells, "Order_has_Name", &FactTypeDef {
            schema_id: String::new(), reading: "Order has Name".to_string(), readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Order".to_string(), role_index: 0 },
                RoleDef { noun_name: "Name".to_string(), role_index: 1 },
            ],
        });
        cells = with_ft(cells, "Order_was_placed", &FactTypeDef {
            schema_id: String::new(), reading: "Order was placed".to_string(), readings: vec![],
            roles: vec![RoleDef { noun_name: "Order".to_string(), role_index: 0 }],
        });
        cells = with_ft(cells, "Order_was_shipped", &FactTypeDef {
            schema_id: String::new(), reading: "Order was shipped".to_string(), readings: vec![],
            roles: vec![RoleDef { noun_name: "Order".to_string(), role_index: 0 }],
        });
        cells = with_state_machine(cells, "OrderSM", &StateMachineDef {
            noun_name: "Order".to_string(),
            statuses: vec!["Draft".to_string(), "Placed".to_string(), "Shipped".to_string()],
            transitions: vec![
                TransitionDef { from: "Draft".to_string(), to: "Placed".to_string(), event: "Order_was_placed".to_string(), guard: None },
                TransitionDef { from: "Placed".to_string(), to: "Shipped".to_string(), event: "Order_was_shipped".to_string(), guard: None },
            ],
            initial: "Draft".to_string(),
        });
        cells.entry("Order_has_Name".to_string()).or_default().push(
            ast::fact_from_pairs(&[("Order", "o1"), ("Name", "Widget")]));
        cells.entry("Order_was_shipped".to_string()).or_default().push(
            ast::fact_from_pairs(&[("Order", "o1")]));

        let (state, defs, _def_map) = compile_cells(cells);
        let dd = derivation_defs_from(&defs);
        let (_new_state, derived) = forward_chain_defs_state(&dd, &state);

        let statuses: Vec<String> = derived.iter()
            .filter(|d| d.fact_type_id == "State_Machine_is_currently_in_Status")
            .filter(|d| d.bindings.iter().any(|(k, v)| k == "State Machine" && v == "o1"))
            .filter_map(|d| d.bindings.iter().find(|(k, _)| k == "Status").map(|(_, v)| v.clone()))
            .collect();
        assert!(!statuses.iter().any(|s| s == "Shipped"),
            "event-fold must respect `from`: o1 was never Placed, so a shipped event (from=Placed) must not fire from Draft. Got: {:?}", statuses);
        assert!(statuses.iter().any(|s| s == "Draft"),
            "o1 must remain in initial Draft. Got: {:?}", statuses);
    }

    /// task sm-fold-as-predicate (multi-step): the from-guarded fold must
    /// CHAIN across forward-chain rounds. o2 carries BOTH `Order_was_placed`
    /// (Draft→Placed) and `Order_was_shipped` (Placed→Shipped). The fixpoint
    /// must walk it Draft → Placed → Shipped: `shipped` is from-guarded on
    /// Placed, so it can only fire AFTER `placed` has advanced o2 to Placed
    /// in an earlier round. Reaching `Shipped` therefore proves the guarded
    /// chain composes step-by-step. This is the compiled mirror of
    /// run_machine's `["placed","shipped"]` fold landing on Shipped.
    ///
    /// CONVERGENCE NOTE (the cell shape matters — see report). The event-fold
    /// is a RECURSIVE derivation: its from-guard reads its own consequent
    /// (`State_Machine_is_currently_in_Status`). Run against the natural
    /// derivation cell (full-tuple set fold, `cell_put_folded`), every
    /// advance is a NEW distinct tuple, so the cell ACCUMULATES the trail
    /// {Draft, Placed, Shipped} and the chain reaches a stable fixpoint with
    /// the terminal status present (current status = the maximal/last one).
    /// We assert reachability of the terminal `Shipped` AND that no status
    /// outside the declared path appears.
    ///
    /// This variant deliberately DOESN'T key the cell, so the trail
    /// ACCUMULATES (every advance is a distinct tuple). The KEYED variant —
    /// where the cell carries the production one-status-per-SM UC and the
    /// scoped upsert (sm-status-scoped-upsert) lets each advance OVERWRITE
    /// the prior status so the cell holds exactly the CURRENT status — is
    /// `event_fold_chains_multi_step_to_terminal_keyed` below.
    #[test]
    fn event_fold_chains_multi_step_to_terminal() {
        let mut cells = empty_cells();
        cells = with_noun(cells, "Order", &make_noun("entity"));
        cells = with_noun(cells, "Name", &make_noun("value"));
        cells = with_ft(cells, "Order_has_Name", &FactTypeDef {
            schema_id: String::new(), reading: "Order has Name".to_string(), readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Order".to_string(), role_index: 0 },
                RoleDef { noun_name: "Name".to_string(), role_index: 1 },
            ],
        });
        cells = with_ft(cells, "Order_was_placed", &FactTypeDef {
            schema_id: String::new(), reading: "Order was placed".to_string(), readings: vec![],
            roles: vec![RoleDef { noun_name: "Order".to_string(), role_index: 0 }],
        });
        cells = with_ft(cells, "Order_was_shipped", &FactTypeDef {
            schema_id: String::new(), reading: "Order was shipped".to_string(), readings: vec![],
            roles: vec![RoleDef { noun_name: "Order".to_string(), role_index: 0 }],
        });
        cells = with_state_machine(cells, "OrderSM", &StateMachineDef {
            noun_name: "Order".to_string(),
            statuses: vec!["Draft".to_string(), "Placed".to_string(), "Shipped".to_string()],
            transitions: vec![
                TransitionDef { from: "Draft".to_string(), to: "Placed".to_string(), event: "Order_was_placed".to_string(), guard: None },
                TransitionDef { from: "Placed".to_string(), to: "Shipped".to_string(), event: "Order_was_shipped".to_string(), guard: None },
            ],
            initial: "Draft".to_string(),
        });
        // o2 has BOTH events: it must advance Draft → Placed → Shipped.
        cells.entry("Order_has_Name".to_string()).or_default().push(
            ast::fact_from_pairs(&[("Order", "o2"), ("Name", "Gadget")]));
        cells.entry("Order_was_placed".to_string()).or_default().push(
            ast::fact_from_pairs(&[("Order", "o2")]));
        cells.entry("Order_was_shipped".to_string()).or_default().push(
            ast::fact_from_pairs(&[("Order", "o2")]));

        let (state, defs, _def_map) = compile_cells(cells);
        let dd = derivation_defs_from(&defs);
        // forward_chain_defs_state is bounded (100 rounds) and early-exits at
        // the fixpoint; if the guarded chain failed to converge it would burn
        // all rounds / oscillate. It returns here, so the chain DID settle.
        let (new_state, _derived) = forward_chain_defs_state(&dd, &state);

        let statuses: Vec<String> = ast::fetch_cell_seq("State_Machine_is_currently_in_Status", &new_state)
            .as_seq()
            .map(|facts| facts.iter()
                .filter(|f| ast::binding(f, "State Machine") == Some("o2"))
                .filter_map(|f| ast::binding(f, "Status").map(|s| s.to_string()))
                .collect())
            .unwrap_or_default();
        // The terminal status is reached: the Placed→Shipped step fired,
        // which is only possible if Draft→Placed advanced o2 in an earlier
        // round (shipped is from-guarded on Placed). That IS the multi-step
        // chain.
        assert!(statuses.iter().any(|s| s == "Shipped"),
            "o2 (placed+shipped) must CHAIN to terminal 'Shipped' (Draft→Placed→Shipped); got {:?}", statuses);
        // And nothing off the declared status set ever appears.
        assert!(statuses.iter().all(|s| s == "Draft" || s == "Placed" || s == "Shipped"),
            "only declared statuses may appear; got {:?}", statuses);
    }

    // ── sm-status-scoped-upsert: shared KEYED-status-cell fixture ────────
    //
    // Declares `State_Machine_is_currently_in_Status` with explicit roles
    // (State Machine @0, Status @1) and a one-status-per-SM alethic UC on
    // role 0, so `_CellKeyRoles` registers the cell and the forward-chain
    // routes its writes through `cell_put_keyed`. This is the PRODUCTION
    // shape — the cell the scoped upsert targets. Runs the init+event-fold
    // derivation defs to fixpoint via the production defs path and returns
    // the final Status row(s) for `entity_id`.
    //
    // Used by the keyed event-fold tests (#1–#4) so each exercises the real
    // keyed/upsert routing, not the un-keyed accumulate-the-trail path.
    fn run_keyed_sm_to_fixpoint(cells: S, entity_id: &str) -> Vec<String> {
        let mut cells = cells;
        cells = with_ft(cells, "State_Machine_is_currently_in_Status", &FactTypeDef {
            schema_id: String::new(),
            reading: "State Machine is currently in Status".to_string(), readings: vec![],
            roles: vec![
                RoleDef { noun_name: "State Machine".to_string(), role_index: 0 },
                RoleDef { noun_name: "Status".to_string(), role_index: 1 },
            ],
        });
        cells = with_constraint(cells, &ConstraintDef {
            id: "uc_one_status_per_sm".to_string(),
            kind: "UC".to_string(),
            modality: "Alethic".to_string(),
            text: "Each State Machine is currently in at most one Status".to_string(),
            spans: vec![SpanDef {
                fact_type_id: "State_Machine_is_currently_in_Status".to_string(),
                role_index: 0, subset_autofill: None,
            }],
            ..Default::default()
        });
        let state = build(cells);
        let defs = crate::compile::compile_to_defs_state(&state);
        let d = ast::defs_to_state(&defs, &state);

        // Guard the fixture: the cell MUST be keyed, else the test would
        // silently fall back to the un-keyed accumulate path and prove
        // nothing about upsert.
        let kr = read_cell_key_roles(&d);
        assert_eq!(
            kr.get("State_Machine_is_currently_in_Status").map(|v| v.as_slice()),
            Some(&["State Machine".to_string()][..]),
            "fixture must register State_Machine_is_currently_in_Status as a \
             keyed cell (one-status-per-SM alethic UC); got {:?}",
            kr.get("State_Machine_is_currently_in_Status"));

        let dd: Vec<(&str, &ast::Func)> = defs.iter()
            .filter(|(n, _)| n.starts_with("derivation:"))
            .map(|(n, f)| (n.as_str(), f))
            .collect();
        let (new_d, _derived) = forward_chain_defs_state(&dd, &d);
        let cell = ast::fetch_or_phi("State_Machine_is_currently_in_Status", &new_d);
        ast::cell_facts_iter(&cell)
            .filter(|f| ast::binding(f, "State Machine") == Some(entity_id))
            .filter_map(|f| ast::binding(f, "Status").map(|s| s.to_string()))
            .collect()
    }

    // Build the Order SM (Draft(initial)→Placed→Shipped) cells with the
    // three event FTs declared, shared by the keyed Order tests.
    fn order_sm_cells() -> S {
        let mut cells = empty_cells();
        cells = with_noun(cells, "Order", &make_noun("entity"));
        cells = with_noun(cells, "Name", &make_noun("value"));
        cells = with_ft(cells, "Order_has_Name", &FactTypeDef {
            schema_id: String::new(), reading: "Order has Name".to_string(), readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Order".to_string(), role_index: 0 },
                RoleDef { noun_name: "Name".to_string(), role_index: 1 },
            ],
        });
        cells = with_ft(cells, "Order_was_placed", &FactTypeDef {
            schema_id: String::new(), reading: "Order was placed".to_string(), readings: vec![],
            roles: vec![RoleDef { noun_name: "Order".to_string(), role_index: 0 }],
        });
        cells = with_ft(cells, "Order_was_shipped", &FactTypeDef {
            schema_id: String::new(), reading: "Order was shipped".to_string(), readings: vec![],
            roles: vec![RoleDef { noun_name: "Order".to_string(), role_index: 0 }],
        });
        cells = with_state_machine(cells, "OrderSM", &StateMachineDef {
            noun_name: "Order".to_string(),
            statuses: vec!["Draft".to_string(), "Placed".to_string(), "Shipped".to_string()],
            transitions: vec![
                TransitionDef { from: "Draft".to_string(), to: "Placed".to_string(), event: "Order_was_placed".to_string(), guard: None },
                TransitionDef { from: "Placed".to_string(), to: "Shipped".to_string(), event: "Order_was_shipped".to_string(), guard: None },
            ],
            initial: "Draft".to_string(),
        });
        cells
    }

    /// TDD #1 (multi-step replay, KEYED): o2 carries BOTH `Order_was_placed`
    /// and `Order_was_shipped` on the production KEYED status cell. The
    /// from-guarded fold advances Draft→Placed→Shipped across rounds, and
    /// the scoped upsert OVERWRITES the prior status each step, so the cell
    /// holds EXACTLY the current status. Unlike the un-keyed variant (which
    /// accumulates the whole trail), the keyed cell must end with a SINGLE
    /// row == `Shipped`.
    #[test]
    fn event_fold_chains_multi_step_to_terminal_keyed() {
        let mut cells = order_sm_cells();
        cells.entry("Order_has_Name".to_string()).or_default().push(
            ast::fact_from_pairs(&[("Order", "o2"), ("Name", "Gadget")]));
        cells.entry("Order_was_placed".to_string()).or_default().push(
            ast::fact_from_pairs(&[("Order", "o2")]));
        cells.entry("Order_was_shipped".to_string()).or_default().push(
            ast::fact_from_pairs(&[("Order", "o2")]));

        let statuses = run_keyed_sm_to_fixpoint(cells, "o2");
        // Upsert ⇒ last-write-wins ⇒ exactly one current status, == Shipped.
        assert_eq!(statuses, vec!["Shipped".to_string()],
            "KEYED multi-step: o2 (placed+shipped) must upsert-advance \
             Draft→Placed→Shipped and hold exactly ['Shipped']; got {:?}", statuses);
    }

    // Build the Task SM (pending(initial)→in_progress→blocked) with the
    // started/blocked/unblocked transitions, shared by the auto-unblock
    // tests. Transitions:
    //   start   : pending     → in_progress  trigger Task_is_started
    //   block   : in_progress → blocked      trigger Task_is_blocked
    //   unblock : blocked     → in_progress  trigger Task_is_unblocked
    fn task_block_sm_cells() -> S {
        let mut cells = empty_cells();
        cells = with_noun(cells, "Task", &make_noun("entity"));
        cells = with_noun(cells, "Name", &make_noun("value"));
        cells = with_ft(cells, "Task_has_Name", &FactTypeDef {
            schema_id: String::new(), reading: "Task has Name".to_string(), readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Task".to_string(), role_index: 0 },
                RoleDef { noun_name: "Name".to_string(), role_index: 1 },
            ],
        });
        for ev in ["Task_is_started", "Task_is_blocked", "Task_is_unblocked"] {
            cells = with_ft(cells, ev, &FactTypeDef {
                schema_id: String::new(),
                reading: ev.replace('_', " "), readings: vec![],
                roles: vec![RoleDef { noun_name: "Task".to_string(), role_index: 0 }],
            });
        }
        cells = with_state_machine(cells, "TaskSM", &StateMachineDef {
            noun_name: "Task".to_string(),
            statuses: vec!["pending".to_string(), "in_progress".to_string(), "blocked".to_string()],
            transitions: vec![
                TransitionDef { from: "pending".to_string(), to: "in_progress".to_string(), event: "Task_is_started".to_string(), guard: None },
                TransitionDef { from: "in_progress".to_string(), to: "blocked".to_string(), event: "Task_is_blocked".to_string(), guard: None },
                TransitionDef { from: "blocked".to_string(), to: "in_progress".to_string(), event: "Task_is_unblocked".to_string(), guard: None },
            ],
            initial: "pending".to_string(),
        });
        cells
    }

    /// TDD #2 (live auto-unblock, KEYED) — the KEY assertion of the task.
    ///
    /// REALISTIC case: t1 carries `Task_is_started` + `Task_is_unblocked`,
    /// with `Task_is_blocked` ABSENT (block/unblock are mutually-exclusive
    /// triggers in a live board — the blocker was resolved, so the block
    /// event no longer holds). By the SEQUENCE the entity reached `blocked`
    /// and the unblock must return it to `in_progress`.
    ///
    /// On the KEYED cell with the scoped upsert the from-guarded fold walks:
    ///   round: pending --started--> in_progress  (started, from=pending)
    ///          in_progress --unblocked--> ??? — unblock is from=blocked,
    ///          and the cell is at in_progress, so unblock does NOT fire.
    /// With no `Task_is_blocked`, the entity is never driven to `blocked` in
    /// this trace, so it SETTLES at `in_progress`. That satisfies the task's
    /// core requirement: an entity that has been unblocked (and is not
    /// currently blocked) is at `in_progress`, NOT stuck at `blocked`.
    #[test]
    fn event_fold_auto_unblock_settles_in_progress_keyed() {
        let mut cells = task_block_sm_cells();
        cells.entry("Task_has_Name".to_string()).or_default().push(
            ast::fact_from_pairs(&[("Task", "t1"), ("Name", "Unblocked")]));
        cells.entry("Task_is_started".to_string()).or_default().push(
            ast::fact_from_pairs(&[("Task", "t1")]));
        cells.entry("Task_is_unblocked".to_string()).or_default().push(
            ast::fact_from_pairs(&[("Task", "t1")]));
        // Task_is_blocked intentionally ABSENT (mutually-exclusive trigger).

        let statuses = run_keyed_sm_to_fixpoint(cells, "t1");
        assert_eq!(statuses, vec!["in_progress".to_string()],
            "KEYED auto-unblock: t1 (started + unblocked, NOT blocked) must \
             settle at exactly ['in_progress'] — never stuck at 'blocked'; \
             got {:?}", statuses);
    }

    /// TDD #2 (worst case, KEYED): t1 carries `Task_is_started` +
    /// `Task_is_blocked` + `Task_is_unblocked` ALL present simultaneously
    /// (the degenerate trace the task asks us to REPORT on — block and
    /// unblock both holding). On the keyed cell with the scoped upsert this
    /// is a from-guarded oscillation candidate: at in_progress the block
    /// fires (→blocked); at blocked the unblock fires (→in_progress). The
    /// recorded converged value is asserted here so the behavior is pinned;
    /// see the report for the analysis (block and unblock are mutually
    /// exclusive in a real board, so this all-present trace is unphysical —
    /// the realistic case is `event_fold_auto_unblock_settles_in_progress_keyed`).
    #[test]
    fn event_fold_auto_unblock_all_events_present_keyed() {
        let mut cells = task_block_sm_cells();
        cells.entry("Task_has_Name".to_string()).or_default().push(
            ast::fact_from_pairs(&[("Task", "t1"), ("Name", "WorstCase")]));
        for ev in ["Task_is_started", "Task_is_blocked", "Task_is_unblocked"] {
            cells.entry(ev.to_string()).or_default().push(
                ast::fact_from_pairs(&[("Task", "t1")]));
        }

        let statuses = run_keyed_sm_to_fixpoint(cells, "t1");
        // Pin whatever the fixpoint converges to (single keyed row). Both
        // in_progress and blocked are reachable; the chain settles
        // deterministically on the last applicable advance per round. The
        // value is reported, not prescribed — the trace is unphysical.
        assert_eq!(statuses.len(), 1,
            "KEYED all-events: keyed cell must hold exactly one current \
             status; got {:?}", statuses);
        assert!(statuses[0] == "in_progress" || statuses[0] == "blocked",
            "KEYED all-events: must converge to in_progress or blocked \
             (both are reachable in the all-present trace); got {:?}", statuses);
        eprintln!("[REPORT] all-events-present (started+blocked+unblocked) \
                   converges to: {:?}", statuses);
    }

    /// TDD #3 (unstarted-blocked stays pending, KEYED): t1 carries ONLY
    /// `Task_is_blocked` (no `Task_is_started`). block is from=in_progress;
    /// the from-guard blocks it because t1 is at the seeded `pending`, never
    /// in_progress. The entity stays `pending`. The scoped upsert must NOT
    /// change this — no advance is emitted, so there is nothing to upsert.
    #[test]
    fn event_fold_unstarted_blocked_stays_pending_keyed() {
        let mut cells = task_block_sm_cells();
        cells.entry("Task_has_Name".to_string()).or_default().push(
            ast::fact_from_pairs(&[("Task", "t1"), ("Name", "NeverStarted")]));
        cells.entry("Task_is_blocked".to_string()).or_default().push(
            ast::fact_from_pairs(&[("Task", "t1")]));

        let statuses = run_keyed_sm_to_fixpoint(cells, "t1");
        assert_eq!(statuses, vec!["pending".to_string()],
            "KEYED unstarted-blocked: t1 (only Task_is_blocked, no start) must \
             stay at ['pending'] — block (from=in_progress) is from-guarded; \
             got {:?}", statuses);
    }

    /// TDD #4 (migration with FULL chain, KEYED): t1 carries
    /// `Task_is_started` + `Task_is_finished` (the complete event record).
    /// On the keyed cell the from-guarded fold replays pending→in_progress
    /// (started) then in_progress→completed (finished), upserting each step,
    /// so the cell ends at exactly ['completed']. This is the fix for the
    /// #900 destination-only freeze: record the FULL chain and replay it.
    #[test]
    fn event_fold_migration_full_chain_keyed() {
        let mut cells = empty_cells();
        cells = with_noun(cells, "Task", &make_noun("entity"));
        cells = with_noun(cells, "Name", &make_noun("value"));
        cells = with_ft(cells, "Task_has_Name", &FactTypeDef {
            schema_id: String::new(), reading: "Task has Name".to_string(), readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Task".to_string(), role_index: 0 },
                RoleDef { noun_name: "Name".to_string(), role_index: 1 },
            ],
        });
        for ev in ["Task_is_started", "Task_is_finished"] {
            cells = with_ft(cells, ev, &FactTypeDef {
                schema_id: String::new(), reading: ev.replace('_', " "), readings: vec![],
                roles: vec![RoleDef { noun_name: "Task".to_string(), role_index: 0 }],
            });
        }
        cells = with_state_machine(cells, "TaskSM", &StateMachineDef {
            noun_name: "Task".to_string(),
            statuses: vec!["pending".to_string(), "in_progress".to_string(), "completed".to_string()],
            transitions: vec![
                TransitionDef { from: "pending".to_string(), to: "in_progress".to_string(), event: "Task_is_started".to_string(), guard: None },
                TransitionDef { from: "in_progress".to_string(), to: "completed".to_string(), event: "Task_is_finished".to_string(), guard: None },
            ],
            initial: "pending".to_string(),
        });
        cells.entry("Task_has_Name".to_string()).or_default().push(
            ast::fact_from_pairs(&[("Task", "t1"), ("Name", "FullChain")]));
        cells.entry("Task_is_started".to_string()).or_default().push(
            ast::fact_from_pairs(&[("Task", "t1")]));
        cells.entry("Task_is_finished".to_string()).or_default().push(
            ast::fact_from_pairs(&[("Task", "t1")]));

        let statuses = run_keyed_sm_to_fixpoint(cells, "t1");
        assert_eq!(statuses, vec!["completed".to_string()],
            "KEYED full-chain: t1 (started + finished) must upsert-replay \
             pending→in_progress→completed and hold exactly ['completed']; \
             got {:?}", statuses);
    }

    /// TDD #5 (global UC unchanged at the forward-chain layer): a NON-SM
    /// keyed cell must STILL conflict-reject a genuinely-conflicting second
    /// value derived in the forward chain. The scoped upsert is allowlisted
    /// to `State_Machine_is_currently_in_Status` ONLY; every other keyed
    /// cell keeps drop-the-conflicting-write.
    ///
    /// `cell_put_keyed`'s Err path is already unit-tested directly
    /// (`test_uniqueness_violation_surfaces_via_cell_put_keyed_err`); this
    /// test pins the ROUTING decision in `integrate_round_facts` — that a
    /// non-allowlisted keyed cell takes the drop branch, not the upsert
    /// branch. We drive `integrate_round_facts` with a pre-seeded keyed cell
    /// (`Person_has_Name`, keyed on "Person") and a round batch carrying a
    /// CONFLICTING value at the same key; the seeded value must survive and
    /// the conflicting write must be dropped (not overwrite).
    #[test]
    fn forward_chain_non_sm_keyed_cell_still_conflict_rejects() {
        use hashbrown::HashMap as HbMap;
        // Seed the keyed cell with (alice → "A") via cell_put_keyed (Map).
        let seed = ast::fact_from_pairs(&[("Person", "alice"), ("Name", "A")]);
        let state = ast::cell_put_keyed("Person_has_Name", &["Person"], seed,
            &ast::Object::phi()).expect("seed put must succeed");

        // Round batch: a CONFLICTING (alice → "B") at the same key.
        let conflicting = ast::fact_from_pairs(&[("Person", "alice"), ("Name", "B")]);
        let mut by_cell: HbMap<String, Vec<ast::Object>> = HbMap::new();
        by_cell.insert("Person_has_Name".to_string(), vec![conflicting]);

        // key_roles marks Person_has_Name as keyed on "Person" — but it is
        // NOT in SM_STATUS_UPSERT_CELLS, so the conflict must DROP.
        let mut key_roles: HbMap<String, Vec<String>> = HbMap::new();
        key_roles.insert("Person_has_Name".to_string(), vec!["Person".to_string()]);

        // Sanity: confirm the allowlist genuinely excludes this cell (and
        // includes the SM cell), so the test is exercising the scoping.
        assert!(!cell_is_sm_status_upsert("Person_has_Name"),
            "Person_has_Name must NOT be in the upsert allowlist");
        assert!(cell_is_sm_status_upsert("State_Machine_is_currently_in_Status"),
            "the SM-status cell MUST be in the upsert allowlist");

        let next = integrate_round_facts(state, by_cell, &key_roles);

        let cell = ast::fetch_or_phi("Person_has_Name", &next);
        let names: Vec<String> = ast::cell_facts_iter(&cell)
            .filter(|f| ast::binding(f, "Person") == Some("alice"))
            .filter_map(|f| ast::binding(f, "Name").map(|s| s.to_string()))
            .collect();
        assert_eq!(names, vec!["A".to_string()],
            "non-SM keyed cell must REJECT the conflicting (alice→B) write and \
             keep the original (alice→A) — global UC enforcement unchanged; \
             got {:?}", names);
    }

    /// CONTRACT (bf5db0de from-guard + sm-status-scoped-upsert):
    /// a DESTINATION-ONLY migration record correctly stays at `pending`.
    ///
    /// A #900-shape migration that records ONLY a *destination* event — a
    /// "completed" Task carrying ONLY `Task_is_finished` (from=in_progress)
    /// with NO preceding `Task_is_started` (pending→in_progress) — is an
    /// INCOMPLETE record: the in_progress predecessor was never written.
    /// The from-GUARDED fold (bf5db0de) only fires a transition for a
    /// resource already in that transition's `from`; the entity is seeded
    /// `pending` (initial) by sm-init and has no `started` event, so
    /// `finished` (from=in_progress) is never applicable and it settles at
    /// `pending`. That is the HONEST answer for a record with no start —
    /// the intended behavior, not a bug. (The fix for the broader #900
    /// freeze is to record the FULL event chain + replay it via the
    /// scoped upsert — see `event_fold_migration_full_chain_keyed`, which
    /// covers `Task_is_started`+`Task_is_finished` → `completed`.)
    ///
    /// The scoped upsert does NOT change this case: the from-guard emits NO
    /// advance for `t1` (no in_progress predecessor), so the SM-status cell
    /// only ever holds the seeded `pending`; there is no conflicting write
    /// for the upsert to last-write. The result is identical on an un-keyed
    /// and a keyed cell.
    ///
    /// This test builds the Task SM (pending(initial)→in_progress→completed)
    /// and entity `t1` carrying ONLY `Task_is_finished`, runs it to fixpoint
    /// on BOTH an un-keyed and a keyed status cell, and asserts t1 stays at
    /// `pending` (never reaches `completed`) in both.
    #[test]
    fn event_fold_destination_only_record_stays_pending_keyed_and_unkeyed() {
        // ── Shared fixture builder ──────────────────────────────────────
        // Task SM: pending(initial) → in_progress → completed.
        //   start : pending     → in_progress  trigger Task_is_started
        //   finish: in_progress → completed    trigger Task_is_finished
        // Entity t1 carries ONLY Task_is_finished (the migration's single
        // destination event) — NO Task_is_started.
        let build_cells = || -> S {
            let mut cells = empty_cells();
            cells = with_noun(cells, "Task", &make_noun("entity"));
            cells = with_noun(cells, "Name", &make_noun("value"));
            cells = with_ft(cells, "Task_has_Name", &FactTypeDef {
                schema_id: String::new(), reading: "Task has Name".to_string(), readings: vec![],
                roles: vec![
                    RoleDef { noun_name: "Task".to_string(), role_index: 0 },
                    RoleDef { noun_name: "Name".to_string(), role_index: 1 },
                ],
            });
            cells = with_ft(cells, "Task_is_started", &FactTypeDef {
                schema_id: String::new(), reading: "Task is started".to_string(), readings: vec![],
                roles: vec![RoleDef { noun_name: "Task".to_string(), role_index: 0 }],
            });
            cells = with_ft(cells, "Task_is_finished", &FactTypeDef {
                schema_id: String::new(), reading: "Task is finished".to_string(), readings: vec![],
                roles: vec![RoleDef { noun_name: "Task".to_string(), role_index: 0 }],
            });
            cells = with_state_machine(cells, "TaskSM", &StateMachineDef {
                noun_name: "Task".to_string(),
                statuses: vec!["pending".to_string(), "in_progress".to_string(), "completed".to_string()],
                transitions: vec![
                    TransitionDef { from: "pending".to_string(), to: "in_progress".to_string(), event: "Task_is_started".to_string(), guard: None },
                    TransitionDef { from: "in_progress".to_string(), to: "completed".to_string(), event: "Task_is_finished".to_string(), guard: None },
                ],
                initial: "pending".to_string(),
            });
            // t1 exists, and carries ONLY the destination event Task_is_finished.
            cells.entry("Task_has_Name".to_string()).or_default().push(
                ast::fact_from_pairs(&[("Task", "t1"), ("Name", "Migrated")]));
            cells.entry("Task_is_finished".to_string()).or_default().push(
                ast::fact_from_pairs(&[("Task", "t1")]));
            cells
        };

        // Collect every Status the fold/init ended up associating with t1
        // in the final State_Machine_is_currently_in_Status cell, regardless
        // of Seq- or Map-backed storage shape.
        let t1_statuses = |new_state: &ast::Object| -> Vec<String> {
            let cell = ast::fetch_or_phi("State_Machine_is_currently_in_Status", new_state);
            ast::cell_facts_iter(&cell)
                .filter(|f| ast::binding(f, "State Machine") == Some("t1"))
                .filter_map(|f| ast::binding(f, "Status").map(|s| s.to_string()))
                .collect()
        };

        // ── (A) UN-KEYED status cell (compile_cells → bare compile, no
        //        _CellKeyRoles ⇒ folded/Seq storage) ─────────────────────
        let unkeyed_statuses = {
            let cells = build_cells();
            let (state, defs, _def_map) = compile_cells(cells);
            let dd = derivation_defs_from(&defs);
            let (new_state, _derived) = forward_chain_defs_state(&dd, &state);
            t1_statuses(&new_state)
        };
        // The from-guard alone strands t1: it is seeded `pending`, has no
        // `started`, so `finished` (from=in_progress) never applies. The
        // fold cannot synthesize the missing in_progress predecessor.
        assert!(!unkeyed_statuses.iter().any(|s| s == "completed"),
            "UN-KEYED: from-guard must prevent t1 (only Task_is_finished, no \
             Task_is_started) from reaching 'completed' — it has no in_progress \
             predecessor for finish (from=in_progress) to fire from. Got: {:?}",
            unkeyed_statuses);
        assert!(unkeyed_statuses.iter().any(|s| s == "pending"),
            "UN-KEYED: t1 must stay at the seeded initial 'pending' (honest \
             answer for a record with no start). Got: {:?}", unkeyed_statuses);

        // ── (B) KEYED status cell (production shape: one-status-per-SM
        //        alethic UC on State_Machine_is_currently_in_Status →
        //        _CellKeyRoles registers it → cell_put_keyed routing) ─────
        let (keyed_statuses, key_roles_registered) = {
            let mut cells = build_cells();
            // Declare the status FT explicitly so the schema carries known
            // role names (State Machine @0, Status @1) — this is the same
            // tuple shape the synthetic event-fold/init emit.
            cells = with_ft(cells, "State_Machine_is_currently_in_Status", &FactTypeDef {
                schema_id: String::new(),
                reading: "State Machine is currently in Status".to_string(), readings: vec![],
                roles: vec![
                    RoleDef { noun_name: "State Machine".to_string(), role_index: 0 },
                    RoleDef { noun_name: "Status".to_string(), role_index: 1 },
                ],
            });
            // Alethic UC on role 0 (State Machine): at most one Status per
            // State Machine. resolve_key_roles_for_ft → Some([0]) ⇒
            // _CellKeyRoles registers the cell ⇒ integrate_round_facts
            // routes writes through cell_put_keyed. For THIS entity the
            // from-guard emits no advance (no in_progress predecessor), so
            // the scoped upsert never fires — the cell holds only seeded
            // `pending`.
            cells = with_constraint(cells, &ConstraintDef {
                id: "uc_one_status_per_sm".to_string(),
                kind: "UC".to_string(),
                modality: "Alethic".to_string(),
                text: "Each State Machine is currently in at most one Status".to_string(),
                spans: vec![SpanDef {
                    fact_type_id: "State_Machine_is_currently_in_Status".to_string(),
                    role_index: 0, subset_autofill: None,
                }],
                ..Default::default()
            });
            let state = build(cells);
            // Production defs path so _CellKeyRoles lands in the overlay.
            let defs = crate::compile::compile_to_defs_state(&state);
            let d = ast::defs_to_state(&defs, &state);

            // Confirm the cell is actually keyed (else the test proves nothing).
            let kr = read_cell_key_roles(&d);
            let registered = kr.get("State_Machine_is_currently_in_Status").cloned();

            // Run only the derivation defs (init + event-fold) to fixpoint;
            // forward_chain_defs_state reads _CellKeyRoles from `d` and routes
            // the keyed cell through cell_put_keyed.
            let dd: Vec<(&str, &ast::Func)> = defs.iter()
                .filter(|(n, _)| n.starts_with("derivation:"))
                .map(|(n, f)| (n.as_str(), f))
                .collect();
            let (new_d, _derived) = forward_chain_defs_state(&dd, &d);
            (t1_statuses(&new_d), registered)
        };

        // Sanity: the cell must really be keyed on "State Machine".
        assert_eq!(key_roles_registered.as_deref(), Some(&["State Machine".to_string()][..]),
            "fixture must register State_Machine_is_currently_in_Status as a \
             keyed cell (one-status-per-SM alethic UC); got {:?}", key_roles_registered);
        // KEYED: t1 still never reaches 'completed' — the from-guard emits
        // no advance (no in_progress predecessor), so even with the scoped
        // upsert the cell only ever holds the seeded `pending`.
        assert!(!keyed_statuses.iter().any(|s| s == "completed"),
            "KEYED: t1 must not reach 'completed' under the from-guard; got {:?}",
            keyed_statuses);
        assert!(keyed_statuses.iter().any(|s| s == "pending"),
            "KEYED: t1 must stay at seeded 'pending'; got {:?}", keyed_statuses);

        // Diagnostic (visible with --nocapture): show t1's final statuses.
        eprintln!("[VERIFY] migration-shape t1 final statuses — un-keyed: {:?}, keyed: {:?}",
            unkeyed_statuses, keyed_statuses);

        // Both paths agree: a destination-only record honestly settles at
        // the initial status. The scoped upsert is irrelevant here because
        // the from-guard never emits an advance to upsert.
        assert_eq!(
            unkeyed_statuses.iter().any(|s| s == "completed"),
            keyed_statuses.iter().any(|s| s == "completed"),
            "both keyed and un-keyed must agree t1 never reaches 'completed' \
             (un-keyed {:?} vs keyed {:?})", unkeyed_statuses, keyed_statuses);
    }

    /// task-6 / #6 ROOT-CAUSE repro: a transition trigger-FT cell that
    /// contains a fact with a φ (phi) value in the SM-noun role — e.g.
    /// `<<Task, φ>>` in Task_is_started (the live tasks.db has these).
    /// SM-init's get_existing extracts that φ into the existing-set;
    /// SetFromSeq / FetchOrPhi over a φ key makes the whole init func
    /// evaluate to Bottom, silently emitting NO initial statuses.
    #[test]
    fn sm_init_survives_phi_value_in_event_cell() {
        let mut cells = empty_cells();
        cells = with_noun(cells, "Order", &make_noun("entity"));
        cells = with_noun(cells, "Name", &make_noun("value"));
        cells = with_ft(cells, "Order_has_Name", &FactTypeDef {
            schema_id: String::new(), reading: "Order has Name".to_string(), readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Order".to_string(), role_index: 0 },
                RoleDef { noun_name: "Name".to_string(), role_index: 1 },
            ],
        });
        cells = with_ft(cells, "Order_was_placed", &FactTypeDef {
            schema_id: String::new(), reading: "Order was placed".to_string(), readings: vec![],
            roles: vec![RoleDef { noun_name: "Order".to_string(), role_index: 0 }],
        });
        cells = with_state_machine(cells, "OrderSM", &StateMachineDef {
            noun_name: "Order".to_string(),
            statuses: vec!["Draft".to_string(), "Placed".to_string(), "Shipped".to_string()],
            transitions: vec![
                TransitionDef { from: "Draft".to_string(), to: "Placed".to_string(), event: "Order_was_placed".to_string(), guard: None },
                TransitionDef { from: "Placed".to_string(), to: "Shipped".to_string(), event: "Order_was_shipped".to_string(), guard: None },
            ],
            initial: "Draft".to_string(),
        });
        // Bare instance o1.
        cells.entry("Order_has_Name".to_string()).or_default().push(
            ast::fact_from_pairs(&[("Order", "o1"), ("Name", "Widget")]));
        // A φ-valued event fact: <<Order, φ>> in the trigger cell, as the
        // live tasks.db carries (`<<Task, ?>, <Task_is_started, ?>>`).
        cells.entry("Order_was_placed".to_string()).or_default().push(
            ast::Object::seq(vec![
                ast::Object::seq(vec![ast::Object::atom("Order"), ast::Object::phi()]),
            ]));

        let (state, defs, _def_map) = compile_cells(cells);
        let dd = derivation_defs_from(&defs);
        let (_new_state, derived) = forward_chain_defs_state(&dd, &state);

        let got_draft = derived.iter()
            .filter(|d| d.fact_type_id == "State_Machine_is_currently_in_Status")
            .any(|d| {
                let sm = d.bindings.iter().find(|(k, _)| k == "State Machine").map(|(_, v)| v.as_str());
                let st = d.bindings.iter().find(|(k, _)| k == "Status").map(|(_, v)| v.as_str());
                sm == Some("o1") && st == Some("Draft")
            });
        assert!(got_draft,
            "bare o1 must get initial 'Draft' even when an event cell has a \
             φ-valued fact; got: {:?}",
            derived.iter().filter(|d| d.fact_type_id == "State_Machine_is_currently_in_Status").collect::<Vec<_>>());
    }

    /// phi-keyed-task-started-orphan-gc: the sibling of the test above for the
    /// *token* φ encoding. `canon_phi` documents three φ forms in a subject
    /// slot — `phi()` (empty Seq, post-SQLite-round-trip), `Atom("φ")` (the
    /// fan-out's literal token), `Atom("")` (the apply/SM blank). The fold
    /// filters caught phi() (NullTest) and "" (Eq) but NOT the `Atom("φ")`
    /// token, so a `<<Order, atom("φ")>>` event leaked the gate and event-fold
    /// minted `State_Machine_is_currently_in_Status [φ, Placed]` — the phantom
    /// that surfaced as a null/φ in_progress task on the live tasks.db board
    /// (and is unretractable via the data API). A real instance must still get
    /// its initial; only the φ token is dropped.
    #[test]
    fn sm_fold_drops_phi_token_subject_event() {
        let mut cells = empty_cells();
        cells = with_noun(cells, "Order", &make_noun("entity"));
        cells = with_noun(cells, "Name", &make_noun("value"));
        cells = with_ft(cells, "Order_has_Name", &FactTypeDef {
            schema_id: String::new(), reading: "Order has Name".to_string(), readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Order".to_string(), role_index: 0 },
                RoleDef { noun_name: "Name".to_string(), role_index: 1 },
            ],
        });
        cells = with_ft(cells, "Order_was_placed", &FactTypeDef {
            schema_id: String::new(), reading: "Order was placed".to_string(), readings: vec![],
            roles: vec![RoleDef { noun_name: "Order".to_string(), role_index: 0 }],
        });
        cells = with_state_machine(cells, "OrderSM", &StateMachineDef {
            noun_name: "Order".to_string(),
            statuses: vec!["Draft".to_string(), "Placed".to_string(), "Shipped".to_string()],
            transitions: vec![
                TransitionDef { from: "Draft".to_string(), to: "Placed".to_string(), event: "Order_was_placed".to_string(), guard: None },
                TransitionDef { from: "Placed".to_string(), to: "Shipped".to_string(), event: "Order_was_shipped".to_string(), guard: None },
            ],
            initial: "Draft".to_string(),
        });
        // Real instance o1.
        cells.entry("Order_has_Name".to_string()).or_default().push(
            ast::fact_from_pairs(&[("Order", "o1"), ("Name", "Widget")]));
        // A φ-TOKEN event fact: <<Order, atom("φ")>> — the literal token, NOT
        // the empty-Seq phi() of the sibling test. This is the fan-out write
        // form before a SQLite round-trip canonicalizes it to phi().
        cells.entry("Order_was_placed".to_string()).or_default().push(
            ast::Object::seq(vec![
                ast::Object::seq(vec![ast::Object::atom("Order"), ast::Object::atom("φ")]),
            ]));

        let (state, defs, _def_map) = compile_cells(cells);
        let dd = derivation_defs_from(&defs);
        let (_new_state, derived) = forward_chain_defs_state(&dd, &state);

        // The φ token must NOT mint ANY SM status keyed on "φ" (or "").
        let phi_status: Vec<_> = derived.iter()
            .filter(|d| d.fact_type_id == "State_Machine_is_currently_in_Status")
            .filter(|d| d.bindings.iter().any(|(k, v)|
                k == "State Machine" && (v == "φ" || v.is_empty())))
            .collect();
        assert!(phi_status.is_empty(),
            "φ-token event subject must not mint a φ-keyed SM status; got: {:?}",
            phi_status);

        // …and the real instance o1 still gets its initial 'Draft' (the token
        // is filtered out, the fold does not collapse to Bottom).
        let got_draft = derived.iter()
            .filter(|d| d.fact_type_id == "State_Machine_is_currently_in_Status")
            .any(|d| {
                let sm = d.bindings.iter().find(|(k, _)| k == "State Machine").map(|(_, v)| v.as_str());
                let st = d.bindings.iter().find(|(k, _)| k == "Status").map(|(_, v)| v.as_str());
                sm == Some("o1") && st == Some("Draft")
            });
        assert!(got_draft,
            "real instance o1 must still get initial 'Draft'; got: {:?}",
            derived.iter().filter(|d| d.fact_type_id == "State_Machine_is_currently_in_Status").collect::<Vec<_>>());
    }

    #[test]
    fn test_initial_status_empty_when_cyclic() {
        // Fully cyclic machine: every status is both source and target.
        // No explicit declaration. Graph topology yields no
        // source-never-target. Per §5.1, the fold needs s_0; when one
        // cannot be derived, compile emits an empty initial and the
        // runtime fails explicitly at first SM call.
        let mut cells = empty_cells();
        cells = with_state_machine(cells, "SM", &StateMachineDef {
            noun_name: "Cycle".to_string(),
            statuses: vec!["A".to_string(), "B".to_string()],
            transitions: vec![
                TransitionDef { from: "A".to_string(), to: "B".to_string(), event: "forward".to_string(), guard: None },
                TransitionDef { from: "B".to_string(), to: "A".to_string(), event: "back".to_string(), guard: None },
            ],
            initial: String::new(),
        });
        let (_meta_state, defs, def_map) = compile_cells(cells);
        let initial = defs.iter().find(|(n, _)| n == "machine:Cycle:initial")
            .and_then(|(_, f)| ast::apply(f, &ast::Object::phi(), &def_map).as_atom().map(|s| s.to_string()))
            .unwrap_or_default();
        assert!(initial.is_empty(), "cyclic machine with no explicit initial -> empty (no insertion-order fallback)");
    }

    #[test]
    fn test_noun_without_state_machine() {
        let cells = empty_cells(); // no state machines
        let (_meta_pop, defs, _def_map) = compile_cells(cells);
        let has_machine = defs.iter().any(|(n, _)| n.starts_with("machine:Customer"));
        assert!(!has_machine);
    }

    #[test]
    fn test_valid_transitions_from_status() {
        let mut cells = empty_cells();
        cells = with_state_machine(cells, "SM", &StateMachineDef {
            noun_name: "SupportRequest".to_string(),
            statuses: vec!["Triaging".to_string(), "Investigating".to_string(), "Resolved".to_string()],
            transitions: vec![
                TransitionDef { from: "Triaging".to_string(), to: "Investigating".to_string(), event: "investigate".to_string(), guard: None },
                TransitionDef { from: "Triaging".to_string(), to: "Resolved".to_string(), event: "quick-resolve".to_string(), guard: None },
                TransitionDef { from: "Investigating".to_string(), to: "Resolved".to_string(), event: "resolve".to_string(), guard: None },
            ],
            initial: String::new(),
        });
        let (_meta_state, defs, def_map) = compile_cells(cells);

        // From Triaging: two valid transitions
        let after_investigate = run_machine_defs(&defs, &def_map, "SupportRequest", &["investigate"]);
        assert_eq!(after_investigate, "Investigating");
        let after_quick_resolve = run_machine_defs(&defs, &def_map, "SupportRequest", &["quick-resolve"]);
        assert_eq!(after_quick_resolve, "Resolved");

        // From Investigating: one valid transition
        let after_resolve = run_machine_defs(&defs, &def_map, "SupportRequest", &["investigate", "resolve"]);
        assert_eq!(after_resolve, "Resolved");

        // From Resolved: no transitions (terminal) - invalid event stays put
        let after_terminal = run_machine_defs(&defs, &def_map, "SupportRequest", &["investigate", "resolve", "investigate"]);
        assert_eq!(after_terminal, "Resolved");
    }

    #[test]
    fn test_run_machine_support_request_lifecycle() {
        let mut cells = empty_cells();
        cells = with_state_machine(cells, "SM", &StateMachineDef {
            noun_name: "SupportRequest".to_string(),
            statuses: vec!["Triaging".to_string(), "Investigating".to_string(), "WaitingOnCustomer".to_string(), "Resolved".to_string()],
            transitions: vec![
                TransitionDef { from: "Triaging".to_string(), to: "Investigating".to_string(), event: "investigate".to_string(), guard: None },
                TransitionDef { from: "Investigating".to_string(), to: "WaitingOnCustomer".to_string(), event: "request-info".to_string(), guard: None },
                TransitionDef { from: "WaitingOnCustomer".to_string(), to: "Investigating".to_string(), event: "customer-replied".to_string(), guard: None },
                TransitionDef { from: "Investigating".to_string(), to: "Resolved".to_string(), event: "resolve".to_string(), guard: None },
            ],
            initial: String::new(),
        });
        let (_meta_state, defs, def_map) = compile_cells(cells);

        // Full lifecycle with back-and-forth
        let final_state = run_machine_defs(&defs, &def_map, "SupportRequest", &[
            "investigate",
            "request-info",
            "customer-replied",
            "resolve",
        ]);
        assert_eq!(final_state, "Resolved");

        // Invalid event mid-flow stays in current state
        let final_state = run_machine_defs(&defs, &def_map, "SupportRequest", &["investigate", "resolve", "investigate"]);
        assert_eq!(final_state, "Resolved"); // already resolved, "investigate" has no effect
    }

    #[test]
    fn test_deontic_forbidden_text_via_ast() {
        let mut cells = empty_cells();
        cells = with_noun(cells, "Markdown Syntax", &make_noun("value"));
        cells = with_enum_values(cells, "Markdown Syntax", "value", &vec!["#".to_string(), "##".to_string(), "**".to_string()]);
        cells = with_ft(cells, "ft1", &FactTypeDef {
            schema_id: String::new(),
            reading: "Response contains Markdown Syntax".to_string(),
            readings: vec![],
            roles: vec![RoleDef { noun_name: "Markdown Syntax".to_string(), role_index: 0 }],
        });
        cells = with_constraint(cells, &ConstraintDef {
            id: "dc1".to_string(),
            kind: "FC".to_string(),
            modality: "Deontic".to_string(),
            deontic_operator: Some("forbidden".to_string()),
            text: "It is forbidden that a Response contains Markdown Syntax.".to_string(),
            spans: vec![SpanDef { fact_type_id: "ft1".to_string(), role_index: 0, subset_autofill: None }],
            ..Default::default()
        });
        let (_meta_state, defs, def_map) = compile_cells(cells);

        // Text with markdown -> violations
        let violations = eval_constraints_defs(&defs, &def_map, "## Heading here", None, &empty_state());
        assert!(violations.len() > 0, "should detect forbidden markdown");

        // Clean text -> no violations
        let clean_violations = eval_constraints_defs(&defs, &def_map, "No special formatting here.", None, &empty_state());
        assert_eq!(clean_violations.len(), 0);
    }

    #[test]
    fn test_deontic_permitted_never_violates_via_ast() {
        let mut cells = empty_cells();
        cells = with_constraint(cells, &ConstraintDef {
            id: "pc1".to_string(),
            kind: "FC".to_string(),
            modality: "Deontic".to_string(),
            deontic_operator: Some("permitted".to_string()),
            text: "It is permitted that something happens.".to_string(),
            spans: vec![],
            ..Default::default()
        });
        let (_meta_state, defs, def_map) = compile_cells(cells);
        let violations = eval_constraints_defs(&defs, &def_map, "anything", None, &empty_state());
        assert_eq!(violations.len(), 0);
    }

    #[test]
    fn test_no_constraints_no_violations_via_ast() {
        let (_meta_pop, defs, def_map) = compile_cells(empty_cells());
        let violations = eval_constraints_defs(&defs, &def_map, "", None, &empty_state());
        assert_eq!(violations.len(), 0);
    }

    #[test]
    fn test_fact_creation_triggers_state_transition() {
        let mut cells = empty_cells();
        cells = with_noun(cells, "Customer", &make_noun("entity"));
        cells = with_noun(cells, "SupportRequest", &make_noun("entity"));

        cells = with_ft(cells, "ft_submit", &FactTypeDef {
            schema_id: String::new(),
            reading: "Customer submits SupportRequest".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Customer".to_string(), role_index: 0 },
                RoleDef { noun_name: "SupportRequest".to_string(), role_index: 1 },
            ],
        });

        cells = with_state_machine(cells, "SupportRequest", &StateMachineDef {
            noun_name: "SupportRequest".to_string(),
            statuses: vec!["Triaging".to_string(), "Investigating".to_string(), "Resolved".to_string()],
            transitions: vec![
                TransitionDef { from: "Triaging".to_string(), to: "Investigating".to_string(), event: "investigate".to_string(), guard: None },
                TransitionDef { from: "Investigating".to_string(), to: "Resolved".to_string(), event: "resolve".to_string(), guard: None },
            ],
            initial: String::new(),
        });

        let (_meta_state, defs, def_map) = compile_cells(cells);

        // The state machine starts at "Triaging"
        let initial_key = "machine:SupportRequest:initial";
        let initial = defs.iter().find(|(n, _)| n == initial_key)
            .and_then(|(_, f)| {
                let r = ast::apply(f, &ast::Object::phi(), &def_map);
                r.as_atom().map(|s| s.to_string())
            })
            .unwrap_or_default();
        assert_eq!(initial, "Triaging");

        // Verify the state machine can transition
        let after_investigate = run_machine_defs(&defs, &def_map, "SupportRequest", &["investigate"]);
        assert_eq!(after_investigate, "Investigating");

        // Verify schema was compiled
        let has_schema = defs.iter().any(|(n, _)| n == "schema:ft_submit");
        assert!(has_schema, "Schema compiled for submit fact type");
    }

    #[test]
    fn test_fact_event_mapping_compiled() {
        let mut cells = empty_cells();
        cells = with_noun(cells, "Customer", &make_noun("entity"));
        cells = with_noun(cells, "SupportRequest", &make_noun("entity"));

        cells = with_ft(cells, "ft_submit", &FactTypeDef {
            schema_id: String::new(),
            reading: "Customer submits SupportRequest".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Customer".to_string(), role_index: 0 },
                RoleDef { noun_name: "SupportRequest".to_string(), role_index: 1 },
            ],
        });

        cells = with_state_machine(cells, "SM", &StateMachineDef {
            noun_name: "SupportRequest".to_string(),
            statuses: vec!["Triaging".to_string(), "Investigating".to_string()],
            transitions: vec![
                TransitionDef { from: "Triaging".to_string(), to: "Investigating".to_string(), event: "submit".to_string(), guard: None },
            ],
            initial: String::new(),
        });

        let (_meta_state, defs, def_map) = compile_cells(cells);

        // Verify the state machine transitions on "submit"
        let final_state = run_machine_defs(&defs, &def_map, "SupportRequest", &["submit"]);
        assert_eq!(final_state, "Investigating");
    }

    #[test]
    fn test_guarded_transition_blocks_on_violation() {
        let mut cells = empty_cells();
        cells = with_noun(cells, "SupportRequest", &make_noun("entity"));
        cells = with_noun(cells, "Prohibited", &make_noun("value"));
        cells = with_enum_values(cells, "Prohibited", "value", &vec!["internal-details".to_string()]);

        cells = with_ft(cells, "ft_resp", &FactTypeDef {
            schema_id: String::new(),
            reading: "Response contains Prohibited".to_string(),
            readings: vec![],
            roles: vec![RoleDef { noun_name: "Prohibited".to_string(), role_index: 0 }],
        });

        cells = with_constraint(cells, &ConstraintDef {
            id: "guard1".to_string(),
            kind: "FC".to_string(),
            modality: "Deontic".to_string(),
            deontic_operator: Some("forbidden".to_string()),
            text: "It is forbidden that a Response contains internal-details".to_string(),
            spans: vec![SpanDef { fact_type_id: "ft_resp".to_string(), role_index: 0, subset_autofill: None }],
            ..Default::default()
        });

        cells = with_state_machine(cells, "SM", &StateMachineDef {
            noun_name: "SupportRequest".to_string(),
            statuses: vec!["Investigating".to_string(), "Resolved".to_string()],
            transitions: vec![
                TransitionDef {
                    from: "Investigating".to_string(), to: "Resolved".to_string(),
                    event: "resolve".to_string(),
                    guard: Some(GuardDef {
                        fact_type_id: "ft_resp".to_string(),
                        constraint_ids: vec!["guard1".to_string()],
                    }),
                },
            ],
            initial: String::new(),
        });

        let (_meta_state, defs, def_map) = compile_cells(cells);

        // Response with forbidden content -> constraint detects violation
        let pop = empty_state();
        let violations = eval_constraints_defs(&defs, &def_map, "Here are the internal-details of the system", None, &pop);
        assert!(!violations.is_empty(), "Guard constraint should produce violations");

        // Clean response -> no constraint violations
        let clean_violations = eval_constraints_defs(&defs, &def_map, "Your issue has been resolved. Thank you.", None, &pop);
        assert!(clean_violations.is_empty(), "No guard violations for clean response");

        // The machine processes the event:
        let state = run_machine_defs(&defs, &def_map, "SupportRequest", &["resolve"]);
        assert_eq!(state, "Resolved",
            "run_machine_defs fires the transition; guard enforcement is the caller's responsibility");
    }

    #[test]
    fn test_fact_driven_event_resolution() {
        let mut cells = empty_cells();
        cells = with_noun(cells, "Customer", &make_noun("entity"));
        cells = with_noun(cells, "SupportRequest", &make_noun("entity"));
        cells = with_noun(cells, "Agent", &make_noun("entity"));

        cells = with_ft(cells, "ft_submit", &FactTypeDef {
            schema_id: String::new(),
            reading: "Customer submits SupportRequest".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Customer".to_string(), role_index: 0 },
                RoleDef { noun_name: "SupportRequest".to_string(), role_index: 1 },
            ],
        });
        cells = with_ft(cells, "ft_resolve", &FactTypeDef {
            schema_id: String::new(),
            reading: "Agent resolves SupportRequest".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Agent".to_string(), role_index: 0 },
                RoleDef { noun_name: "SupportRequest".to_string(), role_index: 1 },
            ],
        });

        cells = with_state_machine(cells, "SupportRequest", &StateMachineDef {
            noun_name: "SupportRequest".to_string(),
            statuses: vec!["Triaging".to_string(), "Investigating".to_string(), "Resolved".to_string()],
            transitions: vec![
                TransitionDef { from: "Triaging".to_string(), to: "Investigating".to_string(), event: "investigate".to_string(), guard: None },
                TransitionDef { from: "Investigating".to_string(), to: "Resolved".to_string(), event: "resolve".to_string(), guard: None },
            ],
            initial: String::new(),
        });

        let (_meta_state, defs, def_map) = compile_cells(cells);

        // Both schemas should compile
        let has_submit = defs.iter().any(|(n, _)| n == "schema:ft_submit");
        let has_resolve = defs.iter().any(|(n, _)| n == "schema:ft_resolve");
        assert!(has_submit);
        assert!(has_resolve);

        // Full lifecycle through events
        let state = run_machine_defs(&defs, &def_map, "SupportRequest", &["investigate", "resolve"]);
        assert_eq!(state, "Resolved");
    }

    #[test]
    fn test_subset_constraint_without_autofill_produces_violation() {
        let mut cells = empty_cells();
        cells = with_noun(cells, "Person", &make_noun("entity"));
        cells = with_ft(cells, "ft1", &FactTypeDef {
            schema_id: String::new(),
            reading: "Person hasLicense".to_string(),
            readings: vec![],
            roles: vec![RoleDef { noun_name: "Person".to_string(), role_index: 0 }],
        });
        cells = with_ft(cells, "ft2", &FactTypeDef {
            schema_id: String::new(),
            reading: "Person hasInsurance".to_string(),
            readings: vec![],
            roles: vec![RoleDef { noun_name: "Person".to_string(), role_index: 0 }],
        });
        // SS constraint WITHOUT autofill -- just validates, doesn't derive
        cells = with_constraint(cells, &ConstraintDef {
            id: "ss_no_auto".to_string(),
            kind: "SS".to_string(),
            modality: "Alethic".to_string(),
            text: "If some Person hasLicense then that Person hasInsurance".to_string(),
            spans: vec![
                SpanDef { fact_type_id: "ft1".to_string(), role_index: 0, subset_autofill: None },
                SpanDef { fact_type_id: "ft2".to_string(), role_index: 0, subset_autofill: None },
            ],
            ..Default::default()
        });

        let (_meta_pop, defs, _def_map) = compile_cells(cells);

        // Behaviour-only assertion (#287 gap #10): an SS constraint
        // without `subset_autofill` must not derive any positive fact
        // into the consequent cell. The derivation id / cell-name
        // format is implementation detail — assert on derived facts.
        let state = state_with_facts("ft1", &[&[("Person", "p1")]]);
        let dd = derivation_defs_from(&defs);
        let (_new_state, derived) = forward_chain_defs_state(&dd, &state);
        let mp_derived: Vec<_> = derived.iter().filter(|d| d.fact_type_id == "ft2").collect();
        // CWA negation may derive "NOT Person hasInsurance" — that's expected.
        // But no POSITIVE autofill derivation should exist.
        let positive_mp = mp_derived.iter().filter(|d| !d.reading.contains("NOT")).count();
        assert_eq!(positive_mp, 0, "No autofill -> no positive derived insurance facts");
    }

    #[test]
    fn test_forward_chain_ast_subtype_inheritance() {
        // Teacher is subtype of Academic. Academic has Rank.
        // Forward chaining should terminate without panicking.
        // task 983: built via parse path so the subtype-inheritance
        // derivation rule is injected by parse_to_state_via_stage12.
        let src = "\
Academic(.id) is an entity type.\n\
Teacher(.id) is an entity type.\n\
  Teacher is a subtype of Academic.\n\
Rank is a value type.\n\
Academic has Rank.\n\
";
        let schema_state = crate::parse_forml2_stage2::parse_to_state_via_stage12(src)
            .expect("parse must succeed");
        let (defs, _def_map) = state_to_defs(&schema_state);

        // Behaviour assertion (#287 gap #10): at least one derivation
        // rule exists — the kind tag survives the parse path.
        let dd = derivation_defs_from(&defs);
        assert!(!dd.is_empty(),
            "Expected at least one derivation for Teacher-is-subtype-of-Academic schema");

        // Teacher T1 has Rank P — forward chain doesn't panic.
        let pop = ast::cell_push(
            "Academic_has_Rank",
            ast::fact_from_pairs(&[("Academic", "T1"), ("Rank", "P")]),
            &schema_state,
        );
        let (_new_state, _derived) = forward_chain_defs_state(&dd, &pop);
    }

    #[test]
    fn test_forward_chain_ast_modus_ponens() {
        let mut cells = empty_cells();
        cells = with_noun(cells, "Academic", &make_noun("entity"));
        cells = with_noun(cells, "Department", &make_noun("entity"));

        cells = with_ft(cells, "ft_heads", &FactTypeDef {
            schema_id: String::new(),
            reading: "Academic heads Department".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Academic".to_string(), role_index: 0 },
                RoleDef { noun_name: "Department".to_string(), role_index: 1 },
            ],
        });
        cells = with_ft(cells, "ft_works", &FactTypeDef {
            schema_id: String::new(),
            reading: "Academic works for Department".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Academic".to_string(), role_index: 0 },
                RoleDef { noun_name: "Department".to_string(), role_index: 1 },
            ],
        });

        // Subset constraint with autofill: heads -> automatically derive works for
        cells = with_constraint(cells, &ConstraintDef {
            id: "ss1".to_string(),
            kind: "SS".to_string(),
            modality: "Alethic".to_string(),
            text: "If some Academic heads some Department then that Academic works for that Department".to_string(),
            spans: vec![
                SpanDef { fact_type_id: "ft_heads".to_string(), role_index: 0, subset_autofill: Some(true) },
                SpanDef { fact_type_id: "ft_works".to_string(), role_index: 0, subset_autofill: None },
            ],
            entity: None,
            set_comparison_argument_length: None,
            clauses: None,
            min_occurrence: None,
            max_occurrence: None,
            deontic_operator: None,
            predicate: None,
        });
        // ss-autofill-retire-2: synthesiser retired — inject the reading-lift rule.
        cells = with_derivation(cells, &ss_autofill_rule());

        let (_meta_pop, defs, _def_map) = compile_cells(cells);

        // Academic A1 heads Department D1
        let state = state_with_facts("ft_heads", &[&[("Academic", "A1"), ("Department", "D1")]]);

        let dd = derivation_defs_from(&defs);
        let (_new_state, ast_derived) = forward_chain_defs_state(&dd, &state);
        // Modus ponens should derive the full tuple: (A1, D1) in ft_works
        let works_for = ast_derived.iter().any(|d|
            d.fact_type_id == "ft_works" &&
            d.bindings.iter().any(|(n, v)| n == "Academic" && v == "A1") &&
            d.bindings.iter().any(|(n, v)| n == "Department" && v == "D1")
        );
        assert!(works_for, "Expected full tuple derivation: A1 works for D1");
    }

    #[test]
    fn test_forward_chain_ast_no_rules_no_derivations() {
        let cells = empty_cells();
        let (_meta_state, defs, _def_map) = compile_cells(cells);
        let dd = derivation_defs_from(&defs);
        let (_new_state, derived) = forward_chain_defs_state(&dd, &empty_state());
        assert_eq!(derived.len(), 0);
    }

    // -- Constraint evaluation tests -----------------------------------

    #[test]
    fn test_no_constraints_no_violations() {
        let cells = empty_cells();
        let (_meta_state, defs, def_map) = compile_cells(cells);
        let result = eval_constraints_defs(&defs, &def_map, "", None, &empty_state());
        assert!(result.is_empty());
    }

    /// Post-task-820: see comment on the sibling
    /// `test_uniqueness_violation_surfaces_via_cell_put_keyed_err` —
    /// the constraint evaluator no longer detects UC violations on
    /// keyed cells; the storage primitive does.
    #[test]
    fn test_uniqueness_violation_customer_via_cell_put_keyed_err() {
        let f1 = ast::fact_from_pairs(&[("Customer", "c1"), ("Name", "Alice")]);
        let f2 = ast::fact_from_pairs(&[("Customer", "c1"), ("Name", "Bob")]);
        let state = ast::Object::phi();
        let state = ast::cell_put_keyed("ft1", &["Customer"], f1, &state)
            .expect("first put must succeed");
        let conflict = ast::cell_put_keyed("ft1", &["Customer"], f2, &state)
            .expect_err("duplicate Customer 'c1' must be a KeyConflict");
        assert_eq!(conflict.name, "ft1");
        assert_eq!(conflict.key, "c1");
    }

    #[test]
    fn test_ring_irreflexive_violation() {
        let mut cells = empty_cells();
        cells = with_ft(cells, "ft1", &FactTypeDef {
            schema_id: String::new(),
            reading: "Person manages Person".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Person".to_string(), role_index: 0 },
                RoleDef { noun_name: "Person".to_string(), role_index: 1 },
            ],
        });
        cells = with_constraint(cells, &ConstraintDef {
            id: "c1".to_string(),
            kind: "IR".to_string(),
            modality: "Alethic".to_string(),
            deontic_operator: None,
            text: "No Person manages itself".to_string(),
            spans: vec![SpanDef { fact_type_id: "ft1".to_string(), role_index: 0, subset_autofill: None }],
            set_comparison_argument_length: None,
            clauses: None,
            entity: None,
            min_occurrence: None,
            max_occurrence: None,
            predicate: None,
        });

        let mut pop_state = ast::Object::phi();
        pop_state = ast::cell_push("ft1", ast::fact_from_pairs(&[("Person", "p1"), ("Person", "p1")]), &pop_state);
        let state = pop_state;

        let (_meta_state, defs, def_map) = compile_cells(cells);
        let result = eval_constraints_defs(&defs, &def_map, "", None, &state);
        assert!(!result.is_empty());
        assert!(result[0].detail.contains("Irreflexive"));
    }

    #[test]
    fn test_exclusive_or_violation() {
        let mut cells = empty_cells();
        cells = with_ft(cells, "ft1", &FactTypeDef {
            schema_id: String::new(),
            reading: "Order isPaid".to_string(),
            readings: vec![],
            roles: vec![RoleDef { noun_name: "Order".to_string(), role_index: 0 }],
        });
        cells = with_ft(cells, "ft2", &FactTypeDef {
            schema_id: String::new(),
            reading: "Order isPending".to_string(),
            readings: vec![],
            roles: vec![RoleDef { noun_name: "Order".to_string(), role_index: 0 }],
        });
        cells = with_constraint(cells, &ConstraintDef {
            id: "c1".to_string(),
            kind: "XO".to_string(),
            modality: "Alethic".to_string(),
            deontic_operator: None,
            text: "For each Order, exactly one holds".to_string(),
            spans: vec![
                SpanDef { fact_type_id: "ft1".to_string(), role_index: 0, subset_autofill: None },
                SpanDef { fact_type_id: "ft2".to_string(), role_index: 0, subset_autofill: None },
            ],
            set_comparison_argument_length: Some(2),
            clauses: Some(vec!["Order isPaid".to_string(), "Order isPending".to_string()]),
            entity: Some("Order".to_string()),
            min_occurrence: None,
            max_occurrence: None,
            predicate: None,
        });

        let mut pop_state = ast::Object::phi();
        pop_state = ast::cell_push("ft1", ast::fact_from_pairs(&[("Order", "o1")]), &pop_state);
        pop_state = ast::cell_push("ft2", ast::fact_from_pairs(&[("Order", "o1")]), &pop_state);
        let state = pop_state;

        let (_meta_state, defs, def_map) = compile_cells(cells);
        let result = eval_constraints_defs(&defs, &def_map, "", None, &state);
        assert!(!result.is_empty());
        assert!(result[0].detail.contains("Set-comparison violation"));
    }

    #[test]
    fn test_subset_violation() {
        let mut cells = empty_cells();
        cells = with_ft(cells, "ft1", &FactTypeDef {
            schema_id: String::new(),
            reading: "Person hasLicense".to_string(),
            readings: vec![],
            roles: vec![RoleDef { noun_name: "Person".to_string(), role_index: 0 }],
        });
        cells = with_ft(cells, "ft2", &FactTypeDef {
            schema_id: String::new(),
            reading: "Person hasInsurance".to_string(),
            readings: vec![],
            roles: vec![RoleDef { noun_name: "Person".to_string(), role_index: 0 }],
        });
        cells = with_constraint(cells, &ConstraintDef {
            id: "c1".to_string(),
            kind: "SS".to_string(),
            modality: "Alethic".to_string(),
            deontic_operator: None,
            text: "If some Person hasLicense then that Person hasInsurance".to_string(),
            spans: vec![
                SpanDef { fact_type_id: "ft1".to_string(), role_index: 0, subset_autofill: None },
                SpanDef { fact_type_id: "ft2".to_string(), role_index: 0, subset_autofill: None },
            ],
            set_comparison_argument_length: None,
            clauses: None,
            entity: None,
            min_occurrence: None,
            max_occurrence: None,
            predicate: None,
        });

        let mut pop_state = ast::Object::phi();
        pop_state = ast::cell_push("ft1", ast::fact_from_pairs(&[("Person", "p1")]), &pop_state);
        let state = pop_state;

        let (_meta_state, defs, def_map) = compile_cells(cells);
        let result = eval_constraints_defs(&defs, &def_map, "", None, &state);
        assert!(!result.is_empty());
        assert!(result[0].detail.contains("Subset violation"));
    }

    #[test]
    fn test_permitted_never_violates() {
        let mut cells = empty_cells();
        cells = with_constraint(cells, &ConstraintDef {
            id: "c1".to_string(),
            kind: "UC".to_string(),
            modality: "Deontic".to_string(),
            deontic_operator: Some("permitted".to_string()),
            text: "It is permitted that SupportResponse offers data retrieval".to_string(),
            spans: vec![],
            set_comparison_argument_length: None,
            clauses: None,
            entity: None,
            min_occurrence: None,
            max_occurrence: None,
            predicate: None,
        });

        let (_meta_state, defs, def_map) = compile_cells(cells);
        let result = eval_constraints_defs(&defs, &def_map, "", None, &empty_state());
        assert!(result.is_empty());
    }

    #[test]
    fn test_exclusive_choice_violation() {
        let mut cells = empty_cells();
        cells = with_ft(cells, "ft1", &FactTypeDef {
            schema_id: String::new(),
            reading: "Order isPaid".to_string(),
            readings: vec![],
            roles: vec![RoleDef { noun_name: "Order".to_string(), role_index: 0 }],
        });
        cells = with_ft(cells, "ft2", &FactTypeDef {
            schema_id: String::new(),
            reading: "Order isPending".to_string(),
            readings: vec![],
            roles: vec![RoleDef { noun_name: "Order".to_string(), role_index: 0 }],
        });
        cells = with_constraint(cells, &ConstraintDef {
            id: "c1".to_string(),
            kind: "XC".to_string(),
            modality: "Alethic".to_string(),
            deontic_operator: None,
            text: "For each Order, at most one holds".to_string(),
            spans: vec![
                SpanDef { fact_type_id: "ft1".to_string(), role_index: 0, subset_autofill: None },
                SpanDef { fact_type_id: "ft2".to_string(), role_index: 0, subset_autofill: None },
            ],
            set_comparison_argument_length: Some(2),
            clauses: Some(vec!["Order isPaid".to_string(), "Order isPending".to_string()]),
            entity: Some("Order".to_string()),
            min_occurrence: None,
            max_occurrence: None,
            predicate: None,
        });

        let mut pop_state = ast::Object::phi();
        pop_state = ast::cell_push("ft1", ast::fact_from_pairs(&[("Order", "o1")]), &pop_state);
        pop_state = ast::cell_push("ft2", ast::fact_from_pairs(&[("Order", "o1")]), &pop_state);
        let state = pop_state;

        let (_meta_state, defs, def_map) = compile_cells(cells);
        let result = eval_constraints_defs(&defs, &def_map, "", None, &state);
        assert!(!result.is_empty());
        assert!(result[0].detail.contains("Set-comparison violation"));
    }

    #[test]
    fn test_mandatory_violation() {
        let mut cells = empty_cells();
        cells = with_noun(cells, "Customer", &make_noun("entity"));
        cells = with_ft(cells, "ft1", &FactTypeDef {
            schema_id: String::new(),
            reading: "Customer has Name".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Customer".to_string(), role_index: 0 },
                RoleDef { noun_name: "Name".to_string(), role_index: 1 },
            ],
        });
        cells = with_ft(cells, "ft2", &FactTypeDef {
            schema_id: String::new(),
            reading: "Customer has Email".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Customer".to_string(), role_index: 0 },
                RoleDef { noun_name: "Email".to_string(), role_index: 1 },
            ],
        });
        cells = with_constraint(cells, &ConstraintDef {
            id: "c1".to_string(),
            kind: "MC".to_string(),
            modality: "Alethic".to_string(),
            deontic_operator: None,
            text: "Each Customer has at least one Name".to_string(),
            spans: vec![SpanDef { fact_type_id: "ft1".to_string(), role_index: 0, subset_autofill: None }],
            set_comparison_argument_length: None,
            clauses: None,
            entity: None,
            min_occurrence: None,
            max_occurrence: None,
            predicate: None,
        });

        let mut pop_state = ast::Object::phi();
        pop_state = ast::cell_push("ft2", ast::fact_from_pairs(&[("Customer", "c1"), ("Email", "a@b.com")]), &pop_state);
        let state = pop_state;

        let (_meta_state, defs, def_map) = compile_cells(cells);
        let result = eval_constraints_defs(&defs, &def_map, "", None, &state);
        assert_eq!(result.len(), 1);
        assert!(result[0].detail.contains("Mandatory violation"));
        assert!(result[0].detail.contains("c1"));
    }

    #[test]
    fn test_inclusive_or_violation() {
        let mut cells = empty_cells();
        cells = with_ft(cells, "ft1", &FactTypeDef {
            schema_id: String::new(),
            reading: "Customer hasPhone".to_string(),
            readings: vec![],
            roles: vec![RoleDef { noun_name: "Customer".to_string(), role_index: 0 }],
        });
        cells = with_ft(cells, "ft2", &FactTypeDef {
            schema_id: String::new(),
            reading: "Customer hasEmail".to_string(),
            readings: vec![],
            roles: vec![RoleDef { noun_name: "Customer".to_string(), role_index: 0 }],
        });
        cells = with_constraint(cells, &ConstraintDef {
            id: "c1".to_string(),
            kind: "OR".to_string(),
            modality: "Alethic".to_string(),
            deontic_operator: None,
            text: "For each Customer, at least one of the following holds: hasPhone, hasEmail".to_string(),
            spans: vec![
                SpanDef { fact_type_id: "ft1".to_string(), role_index: 0, subset_autofill: None },
                SpanDef { fact_type_id: "ft2".to_string(), role_index: 0, subset_autofill: None },
            ],
            set_comparison_argument_length: Some(2),
            clauses: Some(vec!["Customer hasPhone".to_string(), "Customer hasEmail".to_string()]),
            entity: Some("Customer".to_string()),
            min_occurrence: None,
            max_occurrence: None,
            predicate: None,
        });

        cells = with_ft(cells, "ft3", &FactTypeDef {
            schema_id: String::new(),
            reading: "Customer hasName".to_string(),
            readings: vec![],
            roles: vec![RoleDef { noun_name: "Customer".to_string(), role_index: 0 }],
        });
        let mut pop_state = ast::Object::phi();
        pop_state = ast::cell_push("ft3", ast::fact_from_pairs(&[("Customer", "c1")]), &pop_state);
        let state = pop_state;

        let (_meta_state, defs, def_map) = compile_cells(cells);
        let result = eval_constraints_defs(&defs, &def_map, "", None, &state);
        assert_eq!(result.len(), 1);
        assert!(result[0].detail.contains("Set-comparison violation"));
        assert!(result[0].detail.contains("at least one"));
    }

    #[test]
    fn test_obligatory_missing_enum_value() {
        let mut cells = empty_cells();
        cells = with_noun(cells, "SenderIdentityValue", &make_noun("value"));
        cells = with_enum_values(cells, "SenderIdentityValue", "value", &vec!["Support Team <support@example.com>".to_string()]);
        cells = with_noun(cells, "SupportResponse", &make_noun("entity"));
        cells = with_ft(cells, "ft1", &FactTypeDef {
            schema_id: String::new(),
            reading: "SupportResponse has SenderIdentityValue".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "SupportResponse".to_string(), role_index: 0 },
                RoleDef { noun_name: "SenderIdentityValue".to_string(), role_index: 1 },
            ],
        });
        cells = with_constraint(cells, &ConstraintDef {
            id: "c1".to_string(),
            kind: "UC".to_string(),
            modality: "Deontic".to_string(),
            deontic_operator: Some("obligatory".to_string()),
            text: "It is obligatory that each SupportResponse has SenderIdentity".to_string(),
            spans: vec![SpanDef { fact_type_id: "ft1".to_string(), role_index: 0, subset_autofill: None }],
            set_comparison_argument_length: None,
            clauses: None,
            entity: None,
            min_occurrence: None,
            max_occurrence: None,
            predicate: None,
        });

        let (_meta_state, defs, def_map) = compile_cells(cells);
        let result = eval_constraints_defs(&defs, &def_map, "Here is some help for you.", Some(""), &empty_state());
        assert!(result.len() >= 1);
        let details: Vec<String> = result.iter().map(|v| v.detail.clone()).collect();
        assert!(details.iter().any(|d: &String| d.contains("obligatory")));
    }

    #[test]
    fn test_obligatory_sender_identity_empty() {
        let mut cells = empty_cells();
        cells = with_constraint(cells, &ConstraintDef {
            id: "c1".to_string(),
            kind: "UC".to_string(),
            modality: "Deontic".to_string(),
            deontic_operator: Some("obligatory".to_string()),
            text: "It is obligatory that each SupportResponse has SenderIdentity".to_string(),
            spans: vec![],
            set_comparison_argument_length: None,
            clauses: None,
            entity: None,
            min_occurrence: None,
            max_occurrence: None,
            predicate: None,
        });

        let (_meta_state, defs, def_map) = compile_cells(cells);
        let result = eval_constraints_defs(&defs, &def_map, "Hello", Some(""), &empty_state());
        assert_eq!(result.len(), 1);
        assert!(result[0].detail.contains("SenderIdentity"));
    }

    /// Regression: constraints spanning multiple fact types that share a value-type noun
    /// must not produce duplicate violations. collect_enum_values deduplicates by noun name.
    #[test]
    fn test_no_duplicate_violations_for_multi_span_constraints() {
        let mut cells = empty_cells();
        cells = with_noun(cells, "FieldName", &make_noun("value"));
        cells = with_enum_values(cells, "FieldName", "value", &vec!["EndpointSlug".to_string(), "Title".to_string()]);
        cells = with_noun(cells, "SupportResponse", &make_noun("entity"));
        cells = with_noun(cells, "APIProduct", &make_noun("entity"));
        // Three fact types that all reference FieldName -- simulates multi-span constraint
        for i in 1..=3 {
            cells = with_ft(cells, &format!("ft{}", i), &FactTypeDef {
                schema_id: String::new(),
                reading: format!("SupportResponse names APIProduct by FieldName ({})", i),
                readings: vec![],
                roles: vec![
                    RoleDef { noun_name: "SupportResponse".to_string(), role_index: 0 },
                    RoleDef { noun_name: "APIProduct".to_string(), role_index: 1 },
                    RoleDef { noun_name: "FieldName".to_string(), role_index: 2 },
                ],
            });
        }
        cells = with_constraint(cells, &ConstraintDef {
            id: "c1".to_string(),
            kind: "UC".to_string(),
            modality: "Deontic".to_string(),
            deontic_operator: Some("obligatory".to_string()),
            text: "It is obligatory that SupportResponse names APIProduct by FieldName 'Title'.".to_string(),
            spans: vec![
                SpanDef { fact_type_id: "ft1".to_string(), role_index: 0, subset_autofill: None },
                SpanDef { fact_type_id: "ft2".to_string(), role_index: 0, subset_autofill: None },
                SpanDef { fact_type_id: "ft3".to_string(), role_index: 0, subset_autofill: None },
            ],
            set_comparison_argument_length: None,
            clauses: None,
            entity: None,
            min_occurrence: None,
            max_occurrence: None,
            predicate: None,
        });

        let (_meta_state, defs, def_map) = compile_cells(cells);
        let result = eval_constraints_defs(&defs, &def_map, "test response without required field names", None, &empty_state());
        // Should produce exactly 1 violation per unique noun, not 3 duplicates
        let field_name_violations: Vec<_> = result.iter()
            .filter(|v| v.detail.contains("FieldName"))
            .collect();
        assert_eq!(field_name_violations.len(), 1,
            "Expected 1 FieldName violation, got {}. Violations: {:?}",
            field_name_violations.len(),
            field_name_violations.iter().map(|v| &v.detail).collect::<Vec<_>>());
    }

    #[test]
    fn test_equality_violation() {
        let mut cells = empty_cells();
        cells = with_ft(cells, "ft1", &FactTypeDef {
            schema_id: String::new(),
            reading: "Person isEmployee".to_string(),
            readings: vec![],
            roles: vec![RoleDef { noun_name: "Person".to_string(), role_index: 0 }],
        });
        cells = with_ft(cells, "ft2", &FactTypeDef {
            schema_id: String::new(),
            reading: "Person hasBadge".to_string(),
            readings: vec![],
            roles: vec![RoleDef { noun_name: "Person".to_string(), role_index: 0 }],
        });
        cells = with_constraint(cells, &ConstraintDef {
            id: "c1".to_string(),
            kind: "EQ".to_string(),
            modality: "Alethic".to_string(),
            deontic_operator: None,
            text: "Person isEmployee if and only if Person hasBadge".to_string(),
            spans: vec![
                SpanDef { fact_type_id: "ft1".to_string(), role_index: 0, subset_autofill: None },
                SpanDef { fact_type_id: "ft2".to_string(), role_index: 0, subset_autofill: None },
            ],
            set_comparison_argument_length: None,
            clauses: None,
            entity: None,
            min_occurrence: None,
            max_occurrence: None,
            predicate: None,
        });

        let mut pop_state = ast::Object::phi();
        pop_state = ast::cell_push("ft1", ast::fact_from_pairs(&[("Person", "p1")]), &pop_state);
        let state = pop_state;

        let (_meta_state, defs, def_map) = compile_cells(cells);
        let result = eval_constraints_defs(&defs, &def_map, "", None, &state);
        assert!(!result.is_empty());
        assert!(result[0].detail.contains("Equality violation"));
    }

    // -- Forward Inference & Synthesis Tests ----------------------------

    #[test]
    fn test_subtype_inheritance_derivation() {
        // task 983: built via parse path so the subtype-inheritance
        // derivation rule is injected by parse_to_state_via_stage12.
        let src = "\
Vehicle(.id) is an entity type.\n\
Car(.id) is an entity type.\n\
  Car is a subtype of Vehicle.\n\
License is a value type.\n\
Color is a value type.\n\
Vehicle has License.\n\
Car has Color.\n\
Car 'my_car' has Color 'red'.\n\
";
        let state = crate::parse_forml2_stage2::parse_to_state_via_stage12(src)
            .expect("parse must succeed");
        let (defs, _def_map) = state_to_defs(&state);
        let dd = derivation_defs_from(&defs);

        // Behaviour assertion (#287 gap #10): forward chain over a
        // population with a Car instance must derive an inherited
        // fact into the supertype (Vehicle) FT. Inspect derived
        // facts directly — no cell-name substring probing.
        let (_new_state, derived) = forward_chain_defs_state(&dd, &state);

        let inheritance_facts: Vec<_> = derived.iter()
            .filter(|d| d.fact_type_id == "Vehicle_has_License"
                && d.bindings.iter().any(|(_, v)| v == "my_car"))
            .collect();
        assert!(!inheritance_facts.is_empty(),
            "Expected inherited fact in Vehicle_has_License for Car instance 'my_car'; got {:?}", derived);
    }

    #[test]
    fn test_modus_ponens_from_subset() {
        let mut cells = empty_cells();

        cells = with_noun(cells, "Person", &make_noun("entity"));
        cells = with_ft(cells, "ft1", &FactTypeDef {
            schema_id: String::new(),
            reading: "Person hasLicense".to_string(),
            readings: vec![],
            roles: vec![RoleDef { noun_name: "Person".to_string(), role_index: 0 }],
        });
        cells = with_ft(cells, "ft2", &FactTypeDef {
            schema_id: String::new(),
            reading: "Person hasInsurance".to_string(),
            readings: vec![],
            roles: vec![RoleDef { noun_name: "Person".to_string(), role_index: 0 }],
        });
        // SS constraint with autofill: hasLicense -> automatically derive hasInsurance
        cells = with_constraint(cells, &ConstraintDef {
            id: "c1".to_string(),
            kind: "SS".to_string(),
            modality: "Alethic".to_string(),
            deontic_operator: None,
            text: "If some Person hasLicense then that Person hasInsurance".to_string(),
            spans: vec![
                SpanDef { fact_type_id: "ft1".to_string(), role_index: 0, subset_autofill: Some(true) },
                SpanDef { fact_type_id: "ft2".to_string(), role_index: 0, subset_autofill: None },
            ],
            set_comparison_argument_length: None,
            clauses: None,
            entity: None,
            min_occurrence: None,
            max_occurrence: None,
            predicate: None,
        });
        // ss-autofill-retire-2: synthesiser retired — inject the reading-lift rule.
        cells = with_derivation(cells, &ss_autofill_rule());

        let (_meta_pop, defs, _def_map) = compile_cells(cells);

        // Behaviour assertion (#287 gap #10): the SS-autofill
        // derivation's presence is proven by the derived fact it
        // produces, not by its cell-name. Forward chain runs below.
        // Forward chain: p1 hasLicense -> should derive p1 hasInsurance
        let mut pop_state = ast::Object::phi();
        pop_state = ast::cell_push("ft1", ast::fact_from_pairs(&[("Person", "p1")]), &pop_state);
        let state = pop_state;

        let dd = derivation_defs_from(&defs);
        let (_new_state, derived) = forward_chain_defs_state(&dd, &state);

        let insurance_facts: Vec<_> = derived.iter()
            .filter(|d| d.fact_type_id == "ft2")
            .collect();
        assert_eq!(insurance_facts.len(), 1,
            "Expected SS autofill to derive hasInsurance for p1");
        assert_eq!(insurance_facts[0].bindings, vec![("Person".to_string(), "p1".to_string())]);
        assert_eq!(insurance_facts[0].confidence, Confidence::Definitive);
    }

    /// Whitepaper §305: the closed-world assumption is an evaluation-
    /// time semantics, not a materialized complement. A fact absent
    /// from the population is *false* under CWA (Disproven) and
    /// *unknown* under OWA. This is realized lazily by
    /// `prove_from_state`, scoped by the queried noun's
    /// world_assumption — there are no synthetic `_cwa_negation` cells.
    #[test]
    fn test_cwa_vs_owa_negation() {
        let state = ast::Object::phi();
        let absent_goal = "Permission 'read' grants access to Resource 'r1'";

        // CWA: absence ⇒ false.
        let cwa = prove_from_state(&state, absent_goal, &WorldAssumption::Closed);
        assert!(matches!(cwa.status, crate::types::ProofStatus::Disproven),
            "CWA: a fact absent from the population must be Disproven");

        // OWA: absence ⇒ unknown.
        let owa = prove_from_state(&state, absent_goal, &WorldAssumption::Open);
        assert!(matches!(owa.status, crate::types::ProofStatus::Unknown),
            "OWA: a fact absent from the population must be Unknown");
    }

    /// #287 gap #11 — focused test for the AntecedentSource::InstancesOfNoun
    /// shape in compile_explicit_derivation. Builds fixture state via
    /// parse_to_state_via_stage12 (task 983) so the subtype-inheritance
    /// derivation rule is injected by the parse path. Populates the
    /// would-be consequent cell with an "existing" fact for one instance
    /// to exercise the dedup guard (gap #12). Verifies the derivation
    /// emits one <consequent_id, reading, <<role, atom>>> fact per MISSING
    /// instance, skipping the one that already participates.
    #[test]
    fn test_instances_of_noun_antecedent_with_dedup_guard() {
        // task 983: built via parse path so the subtype-inheritance
        // derivation rule is injected by parse_to_state_via_stage12.
        let src = "\
Animal(.id) is an entity type.\n\
Dog(.id) is an entity type.\n\
  Dog is a subtype of Animal.\n\
Name is a value type.\n\
Owner is a value type.\n\
Dog has Name.\n\
Animal has Owner.\n\
Dog 'fido' has Name 'Fido'.\n\
Dog 'rex' has Name 'Rex'.\n\
Animal 'fido' has Owner 'alice'.\n\
";
        let state = crate::parse_forml2_stage2::parse_to_state_via_stage12(src)
            .expect("parse must succeed");
        let (defs, _def_map) = state_to_defs(&state);
        let dd = derivation_defs_from(&defs);

        // Two dogs; one already has an Animal-owner record (parsed in),
        // the other doesn't. The dedup guard should skip the already-
        // participating one.
        let (_s, derived) = forward_chain_defs_state(&dd, &state);

        // Inherited Animal facts in Animal_has_Owner, from Dog instances.
        let inherited: Vec<_> = derived.iter()
            .filter(|d| d.fact_type_id == "Animal_has_Owner")
            .collect();

        // fido is already in Animal_has_Owner with <Animal, fido> at
        // role 0 — dedup guard must skip it.
        let fido_inherited = inherited.iter()
            .any(|d| d.bindings.iter().any(|(_, v)| v == "fido"));
        assert!(!fido_inherited,
            "Dedup guard failed: fido already participates in Animal_has_Owner but got re-emitted: {:?}", inherited);

        // rex has no Animal record — dedup guard must emit.
        let rex_inherited = inherited.iter()
            .any(|d| d.bindings.iter().any(|(_, v)| v == "rex"));
        assert!(rex_inherited,
            "Expected inherited fact for Dog 'rex' into Animal_has_Owner; got {:?}", inherited);
    }

    #[test]
    fn test_synthesis_basic() {
        let mut cells = empty_cells();

        cells = with_noun(cells, "Customer", &NounDef {
            object_type: "entity".to_string(),
            world_assumption: WorldAssumption::Closed,
        });
        cells = with_noun(cells, "Name", &make_noun("value"));
        cells = with_noun(cells, "Email", &make_noun("value"));

        cells = with_ft(cells, "ft1", &FactTypeDef {
            schema_id: String::new(),
            reading: "Customer has Name".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Customer".to_string(), role_index: 0 },
                RoleDef { noun_name: "Name".to_string(), role_index: 1 },
            ],
        });
        cells = with_ft(cells, "ft2", &FactTypeDef {
            schema_id: String::new(),
            reading: "Customer has Email".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Customer".to_string(), role_index: 0 },
                RoleDef { noun_name: "Email".to_string(), role_index: 1 },
            ],
        });

        cells = with_constraint(cells, &ConstraintDef {
            id: "c1".to_string(),
            kind: "MC".to_string(),
            modality: "Alethic".to_string(),
            deontic_operator: None,
            text: "Each Customer has at least one Name".to_string(),
            spans: vec![SpanDef { fact_type_id: "ft1".to_string(), role_index: 0, subset_autofill: None }],
            set_comparison_argument_length: None,
            clauses: None,
            entity: None,
            min_occurrence: None,
            max_occurrence: None,
            predicate: None,
        });

        let (meta_pop, _defs, _def_map) = compile_cells(cells);
        let result = synthesize_from_state(&meta_pop, "Customer", 1);

        assert_eq!(result.noun_name, "Customer");

        // Customer participates in two fact types
        assert_eq!(result.participates_in.len(), 2,
            "Customer should participate in ft1 and ft2. Got: {:?}",
            result.participates_in);

        // One constraint applies to Customer
        assert_eq!(result.applicable_constraints.len(), 1,
            "Expected 1 constraint for Customer. Got: {:?}",
            result.applicable_constraints);
        assert_eq!(result.applicable_constraints[0].id, "c1");

        // Related nouns: Name and Email
        assert_eq!(result.related_nouns.len(), 2,
            "Expected 2 related nouns. Got: {:?}", result.related_nouns);
        let related_names: Vec<_> = result.related_nouns.iter()
            .map(|r| r.name.as_str())
            .collect();
        assert!(related_names.contains(&"Name"), "Expected Name as related noun");
        assert!(related_names.contains(&"Email"), "Expected Email as related noun");
    }

    #[test]
    fn test_synthesis_empty_noun() {
        let (meta_pop, _defs, _def_map) = compile_cells(empty_cells());
        let result = synthesize_from_state(&meta_pop, "NonExistent", 1);

        assert_eq!(result.noun_name, "NonExistent");
        assert!(result.participates_in.is_empty());
        assert!(result.applicable_constraints.is_empty());
        assert!(result.state_machines.is_empty());
        assert!(result.related_nouns.is_empty());
    }

    #[test]
    fn test_forward_chain_fixed_point() {
        // Verify forward chaining reaches a fixed point (no infinite loops)
        let mut cells = empty_cells();
        cells = with_noun(cells, "A", &make_noun("entity"));
        cells = with_ft(cells, "ft1", &FactTypeDef {
            schema_id: String::new(),
            reading: "A exists".to_string(),
            readings: vec![],
            roles: vec![RoleDef { noun_name: "A".to_string(), role_index: 0 }],
        });

        let (_meta_pop, defs, _def_map) = compile_cells(cells);

        let mut pop_state = ast::Object::phi();
        pop_state = ast::cell_push("ft1", ast::fact_from_pairs(&[("A", "a1")]), &pop_state);
        let state = pop_state;

        // Should terminate even if derivations produce facts
        let dd = derivation_defs_from(&defs);
        let (_new_state, derived) = forward_chain_defs_state(&dd, &state);
        // Just verify it terminates -- the exact count depends on CWA rules
        assert!(derived.len() < 100, "Forward chaining should reach fixed point quickly");
    }

    // task-969: the eager transitivity materialisation was REMOVED as
    // an unconsumed eager rule (no consumer ever read the
    // `_transitive_*` closure cells; SM validation joins the explicit
    // `Transition_is_from/to_Status` cells, not a closure). This test
    // (formerly `test_transitivity_derivation`, which asserted the
    // eager rule fired) now PINS THE ABSENCE on the same City → State →
    // Country fixture the old rule fanned out over: forward-chaining
    // every compiled derivation must yield NO `transitivity`-tagged
    // derived fact and NO `_transitive_*` cell. Mirrors the e2e pin
    // `transitivity_metamodel_rule_e2e::no_eager_transitive_closure_cell_is_materialised`
    // and the CWA-removal pin `test_cwa_vs_owa_negation`.
    #[test]
    fn no_eager_transitivity_materialisation() {
        let mut cells = empty_cells();

        cells = with_noun(cells, "City", &make_noun("entity"));
        cells = with_noun(cells, "State", &make_noun("entity"));
        cells = with_noun(cells, "Country", &make_noun("entity"));

        cells = with_ft(cells, "ft_city_state", &FactTypeDef {
            schema_id: String::new(),
            reading: "City isIn State".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "City".to_string(), role_index: 0 },
                RoleDef { noun_name: "State".to_string(), role_index: 1 },
            ],
        });
        cells = with_ft(cells, "ft_state_country", &FactTypeDef {
            schema_id: String::new(),
            reading: "State isIn Country".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "State".to_string(), role_index: 0 },
                RoleDef { noun_name: "Country".to_string(), role_index: 1 },
            ],
        });

        let (_meta_pop, defs, _def_map) = compile_cells(cells);

        // The classic transitivity fixture: Austin isIn Texas, Texas
        // isIn USA. The removed eager rule would have derived
        // Austin (transitively) in USA into `_transitive_<a>_<b>`.
        let mut pop_state = ast::Object::phi();
        pop_state = ast::cell_push("ft_city_state", ast::fact_from_pairs(&[("City", "Austin"), ("State", "Texas")]), &pop_state);
        pop_state = ast::cell_push("ft_state_country", ast::fact_from_pairs(&[("State", "Texas"), ("Country", "USA")]), &pop_state);

        let dd = derivation_defs_from(&defs);
        let (new_state, derived) = forward_chain_defs_state(&dd, &pop_state);

        // No derived fact may be tagged as produced by a transitivity rule.
        let transitive_facts: Vec<_> = derived.iter()
            .filter(|d| d.derived_by.contains("transitivity"))
            .collect();
        assert!(transitive_facts.is_empty(),
            "eager transitivity materialisation was removed (task-969); \
             no `transitivity`-tagged derived fact may be produced, got: {:?}",
            transitive_facts);

        // And no `_transitive_*` closure cell may exist post-chain.
        let transitive_cells: Vec<String> = ast::cells_iter(&new_state)
            .into_iter()
            .map(|(n, _)| n.to_string())
            .filter(|n| n.starts_with("_transitive_"))
            .collect();
        assert!(transitive_cells.is_empty(),
            "no eager `_transitive_*` closure cell may be materialised (task-969); got: {:?}",
            transitive_cells);
    }

    #[test]
    fn test_world_assumption_default_is_closed() {
        assert_eq!(WorldAssumption::default(), WorldAssumption::Closed);
    }

    // â”€â”€ Inline-comparator filter end-to-end (Halpin FORML Example 5) â”€â”€
    //
    // Each AntecedentFilter on a DerivationRuleDef wraps the antecedent's
    // fact-extraction in Func::filter, so only facts whose role value
    // satisfies the comparator reach the existence check. With the current
    // existence-based semantics: if every antecedent fact is filtered out,
    // NullTest on the filtered Seq returns true and the rule stops firing.
    // If at least one fact passes, the rule fires and the binding
    // extractor pulls from the first post-filter fact.

    fn city_population_cells(filter: Option<crate::types::AntecedentFilter>) -> S {
        let ft1 = FactTypeDef {
            schema_id: String::new(),
            reading: "City has Population".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "City".to_string(), role_index: 0 },
                RoleDef { noun_name: "Population".to_string(), role_index: 1 },
            ],
        };
        let ft2 = FactTypeDef {
            schema_id: String::new(),
            reading: "Big City has City".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Big City".to_string(), role_index: 0 },
                RoleDef { noun_name: "City".to_string(), role_index: 1 },
            ],
        };
        let rule = DerivationRuleDef {
            id: "big-city".to_string(),
            text: "* Big City has City iff City has Population >= 1000000".to_string(),
            antecedent_sources: vec![AntecedentSource::FactType("city_has_population".to_string())],
            consequent_cell: ConsequentCellSource::Literal("big_city".to_string()),
            consequent_instance_role: String::new(),
            kind: DerivationKind::ModusPonens,
            join_on: vec![],
            match_on: vec![],
            consequent_bindings: vec![],
            antecedent_filters: filter.into_iter().collect(),
            consequent_computed_bindings: vec![], consequent_aggregates: vec![], consequent_universals: vec![], unresolved_clauses: vec![], antecedent_role_literals: vec![], antecedent_role_comparisons: vec![], consequent_role_literals: vec![], materialization: crate::types::MaterializationPolicy::Stored, ring_join: None, skolem_head_roles: vec![], antecedent_cardinalities: vec![],
        };
        let mut cells = empty_cells();
        cells = with_ft(cells, "city_has_population", &ft1);
        cells = with_ft(cells, "big_city", &ft2);
        cells = with_derivation(cells, &rule);
        cells
    }

    #[test]
    fn inline_ge_filter_suppresses_derivation_when_no_fact_matches() {
        // Both cities well below the 1M threshold â†’ filter strips every
        // antecedent fact â†’ rule's existence check fails â†’ no derivation.
        let cells = city_population_cells(Some(crate::types::AntecedentFilter {
            antecedent_index: 0,
            role: "Population".to_string(),
            op: ">=".to_string(),
            value: 1_000_000.0,
        }));
        let (_meta_pop, defs, _def_map) = compile_cells(cells);

        let mut pop_state = ast::Object::phi();
        pop_state = ast::cell_push("city_has_population",
            ast::fact_from_pairs(&[("City", "SmallTown"), ("Population", "500000")]), &pop_state);
        pop_state = ast::cell_push("city_has_population",
            ast::fact_from_pairs(&[("City", "MidVille"), ("Population", "250000")]), &pop_state);

        let dd = derivation_defs_from(&defs);
        let (_new_state, derived) = forward_chain_defs_state(&dd, &pop_state);

        let big: Vec<_> = derived.iter().filter(|d| d.fact_type_id == "big_city").collect();
        assert!(big.is_empty(), "expected no big_city derivations, got {:?}", big);
    }

    #[test]
    fn inline_ge_filter_allows_derivation_when_a_fact_matches() {
        // One city below the threshold, one above. The filter keeps only
        // the big one, the existence check passes, and the rule fires with
        // the matching city's bindings.
        let cells = city_population_cells(Some(crate::types::AntecedentFilter {
            antecedent_index: 0,
            role: "Population".to_string(),
            op: ">=".to_string(),
            value: 1_000_000.0,
        }));
        let (_meta_pop, defs, _def_map) = compile_cells(cells);

        let mut pop_state = ast::Object::phi();
        pop_state = ast::cell_push("city_has_population",
            ast::fact_from_pairs(&[("City", "SmallTown"), ("Population", "500000")]), &pop_state);
        pop_state = ast::cell_push("city_has_population",
            ast::fact_from_pairs(&[("City", "Megapolis"), ("Population", "2000000")]), &pop_state);

        let dd = derivation_defs_from(&defs);
        let (_new_state, derived) = forward_chain_defs_state(&dd, &pop_state);

        let big: Vec<_> = derived.iter().filter(|d| d.fact_type_id == "big_city").collect();
        assert_eq!(big.len(), 1, "expected exactly one big_city derivation, got {:?}", big);
        // Bindings must come from the matching (post-filter) fact, not the
        // small-town one whose Population is below the cutoff.
        assert!(big[0].bindings.iter().any(|(k, v)| k == "City" && v == "Megapolis"),
            "expected Megapolis as the derived binding, got {:?}", big[0].bindings);
    }

    #[test]
    fn inline_lt_filter_keeps_only_smaller_values() {
        // Flip direction: derivation should fire only when some fact's
        // Population is strictly less than 1M. Exercises Func::Lt path in
        // comparator_primitive.
        let cells = city_population_cells(Some(crate::types::AntecedentFilter {
            antecedent_index: 0,
            role: "Population".to_string(),
            op: "<".to_string(),
            value: 1_000_000.0,
        }));
        let (_meta_pop, defs, _def_map) = compile_cells(cells);

        let mut pop_state = ast::Object::phi();
        pop_state = ast::cell_push("city_has_population",
            ast::fact_from_pairs(&[("City", "Megapolis"), ("Population", "2000000")]), &pop_state);
        pop_state = ast::cell_push("city_has_population",
            ast::fact_from_pairs(&[("City", "Hamlet"), ("Population", "400")]), &pop_state);

        let dd = derivation_defs_from(&defs);
        let (_new_state, derived) = forward_chain_defs_state(&dd, &pop_state);

        let big: Vec<_> = derived.iter().filter(|d| d.fact_type_id == "big_city").collect();
        assert_eq!(big.len(), 1);
        assert!(big[0].bindings.iter().any(|(k, v)| k == "City" && v == "Hamlet"),
            "expected Hamlet (pop<1M), got {:?}", big[0].bindings);
    }

    #[test]
    fn per_fact_fanout_produces_one_derivation_per_matching_fact() {
        // Four cities, three above the 1M threshold. Per-fact semantic
        // demands one derived fact per matching antecedent tuple â€” the
        // old existence-check semantic would have produced one regardless.
        let cells = city_population_cells(Some(crate::types::AntecedentFilter {
            antecedent_index: 0,
            role: "Population".to_string(),
            op: ">=".to_string(),
            value: 1_000_000.0,
        }));
        let (_meta_pop, defs, _def_map) = compile_cells(cells);

        let mut pop_state = ast::Object::phi();
        for (name, pop) in [("Alpha", "2000000"), ("Bravo", "5000000"), ("Charlie", "800000"), ("Delta", "3000000")] {
            pop_state = ast::cell_push("city_has_population",
                ast::fact_from_pairs(&[("City", name), ("Population", pop)]), &pop_state);
        }

        let dd = derivation_defs_from(&defs);
        let (_new_state, derived) = forward_chain_defs_state(&dd, &pop_state);

        let big: Vec<_> = derived.iter().filter(|d| d.fact_type_id == "big_city").collect();
        assert_eq!(big.len(), 3, "expected 3 big cities (Alpha/Bravo/Delta), got {:?}", big);

        let names: hashbrown::HashSet<&str> = big.iter()
            .flat_map(|d| d.bindings.iter()
                .filter(|(k, _)| k == "City")
                .map(|(_, v)| v.as_str()))
            .collect();
        assert!(names.contains("Alpha"));
        assert!(names.contains("Bravo"));
        assert!(names.contains("Delta"));
        assert!(!names.contains("Charlie"), "sub-threshold city must not derive");
    }

    // â”€â”€ Arithmetic definitional clauses, end-to-end â”€â”€
    //
    // A rule like `* Foo has Doubled iff Foo has Val and Doubled is Val + Val.`
    // records a ConsequentComputedBinding { role: "Doubled", expr: Val + Val }
    // which the compile side turns into a per-fact Func that appends the
    // computed pair to the antecedent's bindings.

    fn val_derived_cells(expr: crate::types::ArithExpr, derived_role: &str) -> S {
        let ft1 = FactTypeDef {
            schema_id: String::new(), reading: "Foo has Val".to_string(), readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Foo".to_string(), role_index: 0 },
                RoleDef { noun_name: "Val".to_string(), role_index: 1 },
            ],
        };
        let ft2 = FactTypeDef {
            schema_id: String::new(),
            reading: format!("Foo has {}", derived_role), readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Foo".to_string(), role_index: 0 },
                RoleDef { noun_name: derived_role.to_string(), role_index: 1 },
            ],
        };
        let rule = DerivationRuleDef {
            id: "arith-rule".to_string(),
            text: format!("* Foo has {} iff Foo has Val and ...", derived_role),
            antecedent_sources: vec![AntecedentSource::FactType("foo_has_val".to_string())],
            consequent_cell: ConsequentCellSource::Literal("foo_has_derived".to_string()),
            consequent_instance_role: String::new(),
            kind: DerivationKind::ModusPonens,
            join_on: vec![], match_on: vec![], consequent_bindings: vec![],
            antecedent_filters: vec![],
            consequent_computed_bindings: vec![crate::types::ConsequentComputedBinding {
                role: derived_role.to_string(), expr,
            }],
            consequent_aggregates: vec![], consequent_universals: vec![], unresolved_clauses: vec![], antecedent_role_literals: vec![], antecedent_role_comparisons: vec![], consequent_role_literals: vec![], materialization: crate::types::MaterializationPolicy::Stored, ring_join: None, skolem_head_roles: vec![], antecedent_cardinalities: vec![],
        };
        let mut cells = empty_cells();
        cells = with_ft(cells, "foo_has_val", &ft1);
        cells = with_ft(cells, "foo_has_derived", &ft2);
        cells = with_derivation(cells, &rule);
        cells
    }

    fn val_ref() -> crate::types::ArithExpr {
        crate::types::ArithExpr::RoleRef("Val".to_string())
    }

    fn lit(n: f64) -> crate::types::ArithExpr {
        crate::types::ArithExpr::Literal(n)
    }

    fn bin(op: &str, l: crate::types::ArithExpr, r: crate::types::ArithExpr) -> crate::types::ArithExpr {
        crate::types::ArithExpr::Op(op.to_string(), Box::new(l), Box::new(r))
    }

    #[test]
    fn arithmetic_add_computes_role_plus_role() {
        let cells = val_derived_cells(bin("+", val_ref(), val_ref()), "Doubled");
        let (_meta_pop, defs, _def_map) = compile_cells(cells);

        let mut pop_state = ast::Object::phi();
        pop_state = ast::cell_push("foo_has_val",
            ast::fact_from_pairs(&[("Foo", "f1"), ("Val", "7")]), &pop_state);

        let dd = derivation_defs_from(&defs);
        let (_new_state, derived) = forward_chain_defs_state(&dd, &pop_state);

        let out: Vec<_> = derived.iter().filter(|d| d.fact_type_id == "foo_has_derived").collect();
        assert_eq!(out.len(), 1);
        assert!(out[0].bindings.iter().any(|(k, v)| k == "Doubled" && v == "14"),
            "expected ('Doubled','14'), got {:?}", out[0].bindings);
    }

    #[test]
    fn arithmetic_sub_computes_role_minus_literal() {
        let cells = val_derived_cells(bin("-", val_ref(), lit(3.0)), "Less");
        let (_meta_pop, defs, _def_map) = compile_cells(cells);

        let mut pop_state = ast::Object::phi();
        pop_state = ast::cell_push("foo_has_val",
            ast::fact_from_pairs(&[("Foo", "f1"), ("Val", "10")]), &pop_state);

        let dd = derivation_defs_from(&defs);
        let (_new_state, derived) = forward_chain_defs_state(&dd, &pop_state);

        let out: Vec<_> = derived.iter().filter(|d| d.fact_type_id == "foo_has_derived").collect();
        assert_eq!(out.len(), 1);
        assert!(out[0].bindings.iter().any(|(k, v)| k == "Less" && v == "7"),
            "expected ('Less','7'), got {:?}", out[0].bindings);
    }

    #[test]
    fn arithmetic_mul_and_div_chain_left_associative() {
        // (Val * 3) / 2 applied to Val=10 â†’ 15.
        let expr = bin("/", bin("*", val_ref(), lit(3.0)), lit(2.0));
        let cells = val_derived_cells(expr, "Scaled");
        let (_meta_pop, defs, _def_map) = compile_cells(cells);

        let mut pop_state = ast::Object::phi();
        pop_state = ast::cell_push("foo_has_val",
            ast::fact_from_pairs(&[("Foo", "f1"), ("Val", "10")]), &pop_state);

        let dd = derivation_defs_from(&defs);
        let (_new_state, derived) = forward_chain_defs_state(&dd, &pop_state);

        let out: Vec<_> = derived.iter().filter(|d| d.fact_type_id == "foo_has_derived").collect();
        assert_eq!(out.len(), 1);
        assert!(out[0].bindings.iter().any(|(k, v)| k == "Scaled" && v == "15"),
            "expected ('Scaled','15'), got {:?}", out[0].bindings);
    }

    #[test]
    fn arithmetic_fanout_computes_per_fact_independently() {
        // Three Foo facts with different Vals â†’ three derivations, each
        // carrying its own computed value.
        let cells = val_derived_cells(bin("*", val_ref(), lit(2.0)), "Twice");
        let (_meta_pop, defs, _def_map) = compile_cells(cells);

        let mut pop_state = ast::Object::phi();
        for (id, val) in [("a", "3"), ("b", "5"), ("c", "11")] {
            pop_state = ast::cell_push("foo_has_val",
                ast::fact_from_pairs(&[("Foo", id), ("Val", val)]), &pop_state);
        }

        let dd = derivation_defs_from(&defs);
        let (_new_state, derived) = forward_chain_defs_state(&dd, &pop_state);

        let out: Vec<_> = derived.iter().filter(|d| d.fact_type_id == "foo_has_derived").collect();
        assert_eq!(out.len(), 3);
        let mut pairs: Vec<(String, String)> = out.iter().map(|d| {
            let foo = d.bindings.iter().find(|(k, _)| k == "Foo").map(|(_, v)| v.clone()).unwrap_or_default();
            let tw  = d.bindings.iter().find(|(k, _)| k == "Twice").map(|(_, v)| v.clone()).unwrap_or_default();
            (foo, tw)
        }).collect();
        pairs.sort();
        assert_eq!(pairs, vec![
            ("a".to_string(), "6".to_string()),
            ("b".to_string(), "10".to_string()),
            ("c".to_string(), "22".to_string()),
        ]);
    }

    // â”€â”€ Aggregate derivations, end-to-end (Codd image-set) â”€â”€

    fn thing_part_arity_cells() -> S {
        let ft1 = FactTypeDef {
            schema_id: String::new(), reading: "Thing has Part".to_string(), readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Thing".to_string(), role_index: 0 },
                RoleDef { noun_name: "Part".to_string(), role_index: 1 },
            ],
        };
        let ft2 = FactTypeDef {
            schema_id: String::new(), reading: "Thing has Arity".to_string(), readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Thing".to_string(), role_index: 0 },
                RoleDef { noun_name: "Arity".to_string(), role_index: 1 },
            ],
        };
        let rule = DerivationRuleDef {
            id: "thing-arity".to_string(),
            text: "* Thing has Arity iff Arity is the count of Part where Thing has Part.".to_string(),
            antecedent_sources: vec![],
            consequent_cell: ConsequentCellSource::Literal("thing_has_arity".to_string()),
            consequent_instance_role: String::new(),
            kind: DerivationKind::ModusPonens,
            join_on: vec![], match_on: vec![], consequent_bindings: vec![],
            antecedent_filters: vec![], consequent_computed_bindings: vec![],
            consequent_aggregates: vec![crate::types::ConsequentAggregate {
                role: "Arity".to_string(),
                op: "count".to_string(),
                target_role: "Part".to_string(),
                source_fact_type_id: "thing_has_part".to_string(),
                group_key_role: "Thing".to_string(),
                group_key_index: None, target_index: None, filters: vec![],
                enum_rank: false, join_fact_type_id: String::new(), enum_global: false,
            }],
            consequent_universals: vec![],
            unresolved_clauses: vec![], antecedent_role_literals: vec![], antecedent_role_comparisons: vec![], consequent_role_literals: vec![], materialization: crate::types::MaterializationPolicy::Stored, ring_join: None, skolem_head_roles: vec![], antecedent_cardinalities: vec![],
        };
        let mut cells = empty_cells();
        cells = with_ft(cells, "thing_has_part", &ft1);
        cells = with_ft(cells, "thing_has_arity", &ft2);
        cells = with_derivation(cells, &rule);
        cells
    }

    #[test]
    fn count_aggregate_computes_image_set_size_per_group() {
        // Three Parts belong to T1, one to T2. Expect two derived rows:
        // T1 has Arity=3, T2 has Arity=1.
        let cells = thing_part_arity_cells();
        let (_meta_pop, defs, _def_map) = compile_cells(cells);

        let mut pop_state = ast::Object::phi();
        for (thing, part) in [("T1", "P1"), ("T1", "P2"), ("T1", "P3"), ("T2", "PX")] {
            pop_state = ast::cell_push("thing_has_part",
                ast::fact_from_pairs(&[("Thing", thing), ("Part", part)]), &pop_state);
        }

        let dd = derivation_defs_from(&defs);
        let (_new_state, derived) = forward_chain_defs_state(&dd, &pop_state);

        let arity: Vec<_> = derived.iter().filter(|d| d.fact_type_id == "thing_has_arity").collect();
        // Collect distinct (Thing, Arity) pairs â€” the outer iteration emits
        // duplicates per group, which forward_chain is expected to dedup.
        let mut pairs: alloc::collections::BTreeSet<(String, String)> = arity.iter().map(|d| {
            let t = d.bindings.iter().find(|(k, _)| k == "Thing").map(|(_, v)| v.clone()).unwrap_or_default();
            let a = d.bindings.iter().find(|(k, _)| k == "Arity").map(|(_, v)| v.clone()).unwrap_or_default();
            (t, a)
        }).collect();
        let expected: alloc::collections::BTreeSet<(String, String)> = [
            ("T1".to_string(), "3".to_string()),
            ("T2".to_string(), "1".to_string()),
        ].into_iter().collect();
        assert_eq!(pairs, expected,
            "distinct (Thing, Arity) derivations expected T1â†’3 and T2â†’1, got {:?} (raw count = {})", pairs, arity.len());
        // Sanity â€” if dedup isn't happening, the raw list still contains
        // the right pairs somewhere.
        assert!(arity.iter().any(|d|
            d.bindings.iter().any(|(k, v)| k == "Thing" && v == "T1") &&
            d.bindings.iter().any(|(k, v)| k == "Arity" && v == "3")));
        pairs.clear();  // avoid unused warning via reset
    }

    fn order_line_item_sum_cells() -> S {
        // `LineItem has Amount for Order` is ternary-ish in Halpin's
        // example; for testing we use a simpler binary form
        // `Order has LineItem Amount`, with Order as group key and
        // Amount as target.
        let ft1 = FactTypeDef {
            schema_id: String::new(), reading: "Order has LineItem Amount".to_string(), readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Order".to_string(), role_index: 0 },
                RoleDef { noun_name: "LineItem Amount".to_string(), role_index: 1 },
            ],
        };
        let ft2 = FactTypeDef {
            schema_id: String::new(), reading: "Order has Amount".to_string(), readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Order".to_string(), role_index: 0 },
                RoleDef { noun_name: "Amount".to_string(), role_index: 1 },
            ],
        };
        let rule = DerivationRuleDef {
            id: "order-total".to_string(),
            text: "* Order has Amount iff Amount is the sum of LineItem Amount where Order has LineItem Amount.".to_string(),
            antecedent_sources: vec![],
            consequent_cell: ConsequentCellSource::Literal("order_has_total".to_string()),
            consequent_instance_role: String::new(),
            kind: DerivationKind::ModusPonens,
            join_on: vec![], match_on: vec![], consequent_bindings: vec![],
            antecedent_filters: vec![], consequent_computed_bindings: vec![],
            consequent_aggregates: vec![crate::types::ConsequentAggregate {
                role: "Amount".to_string(),
                op: "sum".to_string(),
                target_role: "LineItem Amount".to_string(),
                source_fact_type_id: "order_has_line_amount".to_string(),
                group_key_role: "Order".to_string(),
                group_key_index: None, target_index: None, filters: vec![],
                enum_rank: false, join_fact_type_id: String::new(), enum_global: false,
            }],
            consequent_universals: vec![],
            unresolved_clauses: vec![], antecedent_role_literals: vec![], antecedent_role_comparisons: vec![], consequent_role_literals: vec![], materialization: crate::types::MaterializationPolicy::Stored, ring_join: None, skolem_head_roles: vec![], antecedent_cardinalities: vec![],
        };
        let mut cells = empty_cells();
        cells = with_ft(cells, "order_has_line_amount", &ft1);
        cells = with_ft(cells, "order_has_total", &ft2);
        cells = with_derivation(cells, &rule);
        cells
    }

    #[test]
    fn sum_aggregate_folds_add_over_projected_target_values() {
        // Order O1: 10 + 25 + 5 = 40; Order O2: 7 = 7.
        let cells = order_line_item_sum_cells();
        let (_meta_pop, defs, _def_map) = compile_cells(cells);

        let mut pop_state = ast::Object::phi();
        for (order, amt) in [("O1", "10"), ("O1", "25"), ("O1", "5"), ("O2", "7")] {
            pop_state = ast::cell_push("order_has_line_amount",
                ast::fact_from_pairs(&[("Order", order), ("LineItem Amount", amt)]), &pop_state);
        }

        let dd = derivation_defs_from(&defs);
        let (_new_state, derived) = forward_chain_defs_state(&dd, &pop_state);

        let totals: Vec<_> = derived.iter().filter(|d| d.fact_type_id == "order_has_total").collect();
        let pairs: alloc::collections::BTreeSet<(String, String)> = totals.iter().map(|d| {
            let o = d.bindings.iter().find(|(k, _)| k == "Order").map(|(_, v)| v.clone()).unwrap_or_default();
            let a = d.bindings.iter().find(|(k, _)| k == "Amount").map(|(_, v)| v.clone()).unwrap_or_default();
            (o, a)
        }).collect();
        let expected: alloc::collections::BTreeSet<(String, String)> = [
            ("O1".to_string(), "40".to_string()),
            ("O2".to_string(), "7".to_string()),
        ].into_iter().collect();
        assert_eq!(pairs, expected,
            "expected O1=40, O2=7; got {:?} (raw count={})", pairs, totals.len());
    }

    fn order_amount_agg_cells(op: &str) -> S {
        // Same shape as order_line_item_sum_cells; this rebuilds with the
        // requested op in the derivation rule's aggregate clause.
        let ft1 = FactTypeDef {
            schema_id: String::new(), reading: "Order has LineItem Amount".to_string(), readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Order".to_string(), role_index: 0 },
                RoleDef { noun_name: "LineItem Amount".to_string(), role_index: 1 },
            ],
        };
        let ft2 = FactTypeDef {
            schema_id: String::new(), reading: "Order has Amount".to_string(), readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Order".to_string(), role_index: 0 },
                RoleDef { noun_name: "Amount".to_string(), role_index: 1 },
            ],
        };
        let rule = DerivationRuleDef {
            id: format!("order-{}", op),
            text: format!("* Order has Amount iff Amount is the {} of LineItem Amount where Order has LineItem Amount.", op),
            antecedent_sources: vec![],
            consequent_cell: ConsequentCellSource::Literal("order_has_total".to_string()),
            consequent_instance_role: String::new(),
            kind: DerivationKind::ModusPonens,
            join_on: vec![], match_on: vec![], consequent_bindings: vec![],
            antecedent_filters: vec![], consequent_computed_bindings: vec![],
            consequent_aggregates: vec![crate::types::ConsequentAggregate {
                role: "Amount".to_string(),
                op: op.to_string(),
                target_role: "LineItem Amount".to_string(),
                source_fact_type_id: "order_has_line_amount".to_string(),
                group_key_role: "Order".to_string(),
                group_key_index: None, target_index: None, filters: vec![],
                enum_rank: false, join_fact_type_id: String::new(), enum_global: false,
            }],
            consequent_universals: vec![],
            unresolved_clauses: vec![], antecedent_role_literals: vec![], antecedent_role_comparisons: vec![], consequent_role_literals: vec![], materialization: crate::types::MaterializationPolicy::Stored, ring_join: None, skolem_head_roles: vec![], antecedent_cardinalities: vec![],
        };
        let mut cells = empty_cells();
        cells = with_ft(cells, "order_has_line_amount", &ft1);
        cells = with_ft(cells, "order_has_total", &ft2);
        cells = with_derivation(cells, &rule);
        cells
    }

    #[test]
    fn min_aggregate_folds_pairwise_minimum() {
        let cells = order_amount_agg_cells("min");
        let (_meta_pop, defs, _def_map) = compile_cells(cells);
        let mut pop_state = ast::Object::phi();
        for (o, a) in [("O1", "10"), ("O1", "4"), ("O1", "25"), ("O2", "7")] {
            pop_state = ast::cell_push("order_has_line_amount",
                ast::fact_from_pairs(&[("Order", o), ("LineItem Amount", a)]), &pop_state);
        }
        let dd = derivation_defs_from(&defs);
        let (_new_state, derived) = forward_chain_defs_state(&dd, &pop_state);
        let totals: Vec<_> = derived.iter().filter(|d| d.fact_type_id == "order_has_total").collect();
        let pairs: alloc::collections::BTreeSet<(String, String)> = totals.iter().map(|d| {
            let o = d.bindings.iter().find(|(k, _)| k == "Order").map(|(_, v)| v.clone()).unwrap_or_default();
            let a = d.bindings.iter().find(|(k, _)| k == "Amount").map(|(_, v)| v.clone()).unwrap_or_default();
            (o, a)
        }).collect();
        let expected: alloc::collections::BTreeSet<(String, String)> = [
            ("O1".to_string(), "4".to_string()),
            ("O2".to_string(), "7".to_string()),
        ].into_iter().collect();
        assert_eq!(pairs, expected, "min: expected O1=4 O2=7, got {:?}", pairs);
    }

    #[test]
    fn max_aggregate_folds_pairwise_maximum() {
        let cells = order_amount_agg_cells("max");
        let (_meta_pop, defs, _def_map) = compile_cells(cells);
        let mut pop_state = ast::Object::phi();
        for (o, a) in [("O1", "10"), ("O1", "4"), ("O1", "25"), ("O2", "7")] {
            pop_state = ast::cell_push("order_has_line_amount",
                ast::fact_from_pairs(&[("Order", o), ("LineItem Amount", a)]), &pop_state);
        }
        let dd = derivation_defs_from(&defs);
        let (_new_state, derived) = forward_chain_defs_state(&dd, &pop_state);
        let totals: Vec<_> = derived.iter().filter(|d| d.fact_type_id == "order_has_total").collect();
        let pairs: alloc::collections::BTreeSet<(String, String)> = totals.iter().map(|d| {
            let o = d.bindings.iter().find(|(k, _)| k == "Order").map(|(_, v)| v.clone()).unwrap_or_default();
            let a = d.bindings.iter().find(|(k, _)| k == "Amount").map(|(_, v)| v.clone()).unwrap_or_default();
            (o, a)
        }).collect();
        let expected: alloc::collections::BTreeSet<(String, String)> = [
            ("O1".to_string(), "25".to_string()),
            ("O2".to_string(), "7".to_string()),
        ].into_iter().collect();
        assert_eq!(pairs, expected, "max: expected O1=25 O2=7, got {:?}", pairs);
    }

    #[test]
    fn avg_aggregate_divides_sum_by_count() {
        let cells = order_amount_agg_cells("avg");
        let (_meta_pop, defs, _def_map) = compile_cells(cells);
        let mut pop_state = ast::Object::phi();
        // O1: (9 + 12 + 15) / 3 = 12.
        for (o, a) in [("O1", "9"), ("O1", "12"), ("O1", "15"), ("O2", "7")] {
            pop_state = ast::cell_push("order_has_line_amount",
                ast::fact_from_pairs(&[("Order", o), ("LineItem Amount", a)]), &pop_state);
        }
        let dd = derivation_defs_from(&defs);
        let (_new_state, derived) = forward_chain_defs_state(&dd, &pop_state);
        let totals: Vec<_> = derived.iter().filter(|d| d.fact_type_id == "order_has_total").collect();
        let pairs: alloc::collections::BTreeSet<(String, String)> = totals.iter().map(|d| {
            let o = d.bindings.iter().find(|(k, _)| k == "Order").map(|(_, v)| v.clone()).unwrap_or_default();
            let a = d.bindings.iter().find(|(k, _)| k == "Amount").map(|(_, v)| v.clone()).unwrap_or_default();
            (o, a)
        }).collect();
        // Accept either integer or float formatting for the averaged value.
        let has_pair = |o: &str, expected_nums: &[&str]| -> bool {
            pairs.iter().any(|(actual_o, v)| actual_o == o && expected_nums.iter().any(|e| v == e))
        };
        assert!(has_pair("O1", &["12", "12.0"]), "avg: expected O1 to average to 12, got {:?}", pairs);
        assert!(has_pair("O2", &["7", "7.0"]), "avg: expected O2=7, got {:?}", pairs);
    }

    #[test]
    fn rule_without_filter_fires_for_any_fact_regression() {
        // Regression: when antecedent_filters is empty, behavior is
        // unchanged from pre-#192 â€” any fact makes the rule fire.
        let cells = city_population_cells(None);
        let (_meta_pop, defs, _def_map) = compile_cells(cells);

        let mut pop_state = ast::Object::phi();
        pop_state = ast::cell_push("city_has_population",
            ast::fact_from_pairs(&[("City", "SmallTown"), ("Population", "500000")]), &pop_state);

        let dd = derivation_defs_from(&defs);
        let (_new_state, derived) = forward_chain_defs_state(&dd, &pop_state);

        let big: Vec<_> = derived.iter().filter(|d| d.fact_type_id == "big_city").collect();
        assert_eq!(big.len(), 1, "unfiltered rule must still fire");
    }

    #[test]
    fn join_derivation_equi_join_on_shared_key() {
        let mut cells = empty_cells();
        cells = with_ft(cells, "a_key", &FactTypeDef {
            schema_id: String::new(), reading: "A has Key".to_string(), readings: vec![],
            roles: vec![
                RoleDef { noun_name: "A".to_string(), role_index: 0 },
                RoleDef { noun_name: "Key".to_string(), role_index: 1 },
            ],
        });
        cells = with_ft(cells, "b_key", &FactTypeDef {
            schema_id: String::new(), reading: "B has Key".to_string(), readings: vec![],
            roles: vec![
                RoleDef { noun_name: "B".to_string(), role_index: 0 },
                RoleDef { noun_name: "Key".to_string(), role_index: 1 },
            ],
        });
        cells = with_ft(cells, "derived", &FactTypeDef {
            schema_id: String::new(), reading: "A is matched to B".to_string(), readings: vec![],
            roles: vec![
                RoleDef { noun_name: "A".to_string(), role_index: 0 },
                RoleDef { noun_name: "B".to_string(), role_index: 1 },
            ],
        });
        cells = with_derivation(cells, &DerivationRuleDef {
            id: "join1".to_string(),
            text: "A matches B on Key".to_string(),
            antecedent_sources: vec![AntecedentSource::FactType("a_key".to_string()), AntecedentSource::FactType("b_key".to_string())],
            consequent_cell: ConsequentCellSource::Literal("derived".to_string()),
            consequent_instance_role: String::new(),
            kind: DerivationKind::Join,
            join_on: vec!["Key".to_string()],
            match_on: vec![],
            consequent_bindings: vec!["A".to_string(), "B".to_string()],
            antecedent_filters: vec![], consequent_computed_bindings: vec![], consequent_aggregates: vec![], consequent_universals: vec![], unresolved_clauses: vec![], antecedent_role_literals: vec![], antecedent_role_comparisons: vec![], consequent_role_literals: vec![], materialization: crate::types::MaterializationPolicy::Stored, ring_join: None, skolem_head_roles: vec![], antecedent_cardinalities: vec![],
        });

        let (_meta_pop, defs, _def_map) = compile_cells(cells);

        let mut pop_state = ast::Object::phi();
        pop_state = ast::cell_push("a_key", ast::fact_from_pairs(&[("A", "a1"), ("Key", "k1")]), &pop_state);
        pop_state = ast::cell_push("a_key", ast::fact_from_pairs(&[("A", "a2"), ("Key", "k2")]), &pop_state);
        pop_state = ast::cell_push("b_key", ast::fact_from_pairs(&[("B", "b1"), ("Key", "k1")]), &pop_state);
        pop_state = ast::cell_push("b_key", ast::fact_from_pairs(&[("B", "b2"), ("Key", "k3")]), &pop_state);

        let dd = derivation_defs_from(&defs);
        let (_new_state, derived) = forward_chain_defs_state(&dd, &pop_state);

        let derived_facts: Vec<_> = derived.iter().filter(|d| d.fact_type_id == "derived").collect();
        // Only a1<->b1 (both Key="k1"). a2 has Key="k2" which doesn't match any B.
        assert_eq!(derived_facts.len(), 1);
        assert!(derived_facts[0].bindings.contains(&("A".to_string(), "a1".to_string())));
        assert!(derived_facts[0].bindings.contains(&("B".to_string(), "b1".to_string())));
    }

    #[test]
    fn join_derivation_entity_consistency_across_fact_types() {
        let mut cells = empty_cells();
        cells = with_ft(cells, "x_key", &FactTypeDef {
            schema_id: String::new(), reading: "X has Key".to_string(), readings: vec![],
            roles: vec![
                RoleDef { noun_name: "X".to_string(), role_index: 0 },
                RoleDef { noun_name: "Key".to_string(), role_index: 1 },
            ],
        });
        cells = with_ft(cells, "x_label", &FactTypeDef {
            schema_id: String::new(), reading: "X has Label".to_string(), readings: vec![],
            roles: vec![
                RoleDef { noun_name: "X".to_string(), role_index: 0 },
                RoleDef { noun_name: "Label".to_string(), role_index: 1 },
            ],
        });
        cells = with_ft(cells, "y_key", &FactTypeDef {
            schema_id: String::new(), reading: "Y has Key".to_string(), readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Y".to_string(), role_index: 0 },
                RoleDef { noun_name: "Key".to_string(), role_index: 1 },
            ],
        });
        cells = with_ft(cells, "result", &FactTypeDef {
            schema_id: String::new(), reading: "Y is resolved to X".to_string(), readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Y".to_string(), role_index: 0 },
                RoleDef { noun_name: "X".to_string(), role_index: 1 },
            ],
        });
        cells = with_derivation(cells, &DerivationRuleDef {
            id: "join2".to_string(),
            text: "Y resolves to X via Key".to_string(),
            antecedent_sources: vec![AntecedentSource::FactType("y_key".to_string()), AntecedentSource::FactType("x_key".to_string()), AntecedentSource::FactType("x_label".to_string())],
            consequent_cell: ConsequentCellSource::Literal("result".to_string()),
            consequent_instance_role: String::new(),
            kind: DerivationKind::Join,
            join_on: vec!["Key".to_string(), "X".to_string()],
            match_on: vec![],
            consequent_bindings: vec!["Y".to_string(), "X".to_string()],
            antecedent_filters: vec![], consequent_computed_bindings: vec![], consequent_aggregates: vec![], consequent_universals: vec![], unresolved_clauses: vec![], antecedent_role_literals: vec![], antecedent_role_comparisons: vec![], consequent_role_literals: vec![], materialization: crate::types::MaterializationPolicy::Stored, ring_join: None, skolem_head_roles: vec![], antecedent_cardinalities: vec![],
        });

        let (_meta_pop, defs, _def_map) = compile_cells(cells);

        let mut pop_state = ast::Object::phi();
        pop_state = ast::cell_push("x_key", ast::fact_from_pairs(&[("X", "x1"), ("Key", "k1")]), &pop_state);
        pop_state = ast::cell_push("x_key", ast::fact_from_pairs(&[("X", "x2"), ("Key", "k1")]), &pop_state);
        pop_state = ast::cell_push("x_label", ast::fact_from_pairs(&[("X", "x1"), ("Label", "L1")]), &pop_state);
        pop_state = ast::cell_push("x_label", ast::fact_from_pairs(&[("X", "x2"), ("Label", "L2")]), &pop_state);
        pop_state = ast::cell_push("y_key", ast::fact_from_pairs(&[("Y", "y1"), ("Key", "k1")]), &pop_state);

        let dd = derivation_defs_from(&defs);
        let (_new_state, derived) = forward_chain_defs_state(&dd, &pop_state);

        let resolved: Vec<_> = derived.iter().filter(|d| d.fact_type_id == "result").collect();
        assert_eq!(resolved.len(), 2);
    }

    #[test]
    fn join_derivation_match_on_containment() {
        let mut cells = empty_cells();
        cells = with_ft(cells, "a_name", &FactTypeDef {
            schema_id: String::new(), reading: "A has Full Name".to_string(), readings: vec![],
            roles: vec![
                RoleDef { noun_name: "A".to_string(), role_index: 0 },
                RoleDef { noun_name: "Full Name".to_string(), role_index: 1 },
            ],
        });
        cells = with_ft(cells, "b_name", &FactTypeDef {
            schema_id: String::new(), reading: "B has Short Name".to_string(), readings: vec![],
            roles: vec![
                RoleDef { noun_name: "B".to_string(), role_index: 0 },
                RoleDef { noun_name: "Short Name".to_string(), role_index: 1 },
            ],
        });
        cells = with_ft(cells, "matched", &FactTypeDef {
            schema_id: String::new(), reading: "B is matched to A".to_string(), readings: vec![],
            roles: vec![
                RoleDef { noun_name: "B".to_string(), role_index: 0 },
                RoleDef { noun_name: "A".to_string(), role_index: 1 },
            ],
        });
        cells = with_derivation(cells, &DerivationRuleDef {
            id: "match1".to_string(),
            text: "B matches A by name containment".to_string(),
            antecedent_sources: vec![AntecedentSource::FactType("a_name".to_string()), AntecedentSource::FactType("b_name".to_string())],
            consequent_cell: ConsequentCellSource::Literal("matched".to_string()),
            consequent_instance_role: String::new(),
            kind: DerivationKind::Join,
            join_on: vec![],
            match_on: vec![("Full Name".to_string(), "Short Name".to_string())],
            consequent_bindings: vec!["B".to_string(), "A".to_string()],
            antecedent_filters: vec![], consequent_computed_bindings: vec![], consequent_aggregates: vec![], consequent_universals: vec![], unresolved_clauses: vec![], antecedent_role_literals: vec![], antecedent_role_comparisons: vec![], consequent_role_literals: vec![], materialization: crate::types::MaterializationPolicy::Stored, ring_join: None, skolem_head_roles: vec![], antecedent_cardinalities: vec![],
        });

        let (_meta_pop, defs, _def_map) = compile_cells(cells);

        let mut pop_state = ast::Object::phi();
        pop_state = ast::cell_push("a_name", ast::fact_from_pairs(&[("A", "a1"), ("Full Name", "Alpha Bravo")]), &pop_state);
        pop_state = ast::cell_push("a_name", ast::fact_from_pairs(&[("A", "a2"), ("Full Name", "Charlie Delta")]), &pop_state);
        pop_state = ast::cell_push("b_name", ast::fact_from_pairs(&[("B", "b1"), ("Short Name", "Alpha")]), &pop_state);

        let dd = derivation_defs_from(&defs);
        let (_new_state, derived) = forward_chain_defs_state(&dd, &pop_state);

        let matched: Vec<_> = derived.iter().filter(|d| d.fact_type_id == "matched").collect();
        assert_eq!(matched.len(), 1);
        assert!(matched[0].bindings.contains(&("A".to_string(), "a1".to_string())));
        assert!(matched[0].bindings.contains(&("B".to_string(), "b1".to_string())));
    }

    #[test]
    fn join_derivation_no_match_produces_nothing() {
        let mut cells = empty_cells();
        cells = with_ft(cells, "a_key", &FactTypeDef {
            schema_id: String::new(), reading: "A has Key".to_string(), readings: vec![],
            roles: vec![
                RoleDef { noun_name: "A".to_string(), role_index: 0 },
                RoleDef { noun_name: "Key".to_string(), role_index: 1 },
            ],
        });
        cells = with_ft(cells, "b_key", &FactTypeDef {
            schema_id: String::new(), reading: "B has Key".to_string(), readings: vec![],
            roles: vec![
                RoleDef { noun_name: "B".to_string(), role_index: 0 },
                RoleDef { noun_name: "Key".to_string(), role_index: 1 },
            ],
        });
        cells = with_ft(cells, "derived", &FactTypeDef {
            schema_id: String::new(), reading: "A matches B".to_string(), readings: vec![],
            roles: vec![
                RoleDef { noun_name: "A".to_string(), role_index: 0 },
                RoleDef { noun_name: "B".to_string(), role_index: 1 },
            ],
        });
        cells = with_derivation(cells, &DerivationRuleDef {
            id: "j".to_string(),
            text: "join".to_string(),
            antecedent_sources: vec![AntecedentSource::FactType("a_key".to_string()), AntecedentSource::FactType("b_key".to_string())],
            consequent_cell: ConsequentCellSource::Literal("derived".to_string()),
            consequent_instance_role: String::new(),
            kind: DerivationKind::Join,
            join_on: vec!["Key".to_string()],
            match_on: vec![],
            consequent_bindings: vec!["A".to_string(), "B".to_string()],
            antecedent_filters: vec![], consequent_computed_bindings: vec![], consequent_aggregates: vec![], consequent_universals: vec![], unresolved_clauses: vec![], antecedent_role_literals: vec![], antecedent_role_comparisons: vec![], consequent_role_literals: vec![], materialization: crate::types::MaterializationPolicy::Stored, ring_join: None, skolem_head_roles: vec![], antecedent_cardinalities: vec![],
        });

        let (_meta_pop, defs, _def_map) = compile_cells(cells);

        let mut pop_state = ast::Object::phi();
        pop_state = ast::cell_push("a_key", ast::fact_from_pairs(&[("A", "a1"), ("Key", "k1")]), &pop_state);
        pop_state = ast::cell_push("b_key", ast::fact_from_pairs(&[("B", "b1"), ("Key", "k2")]), &pop_state);

        let dd = derivation_defs_from(&defs);
        let (_new_state, derived) = forward_chain_defs_state(&dd, &pop_state);

        let derived_count = derived.iter().filter(|d| d.fact_type_id == "derived").count();
        assert_eq!(derived_count, 0, "No match should produce no derivation");
    }

    fn make_forbidden_text_cells(enum_vals: Vec<String>) -> S {
        let mut cells = empty_cells();
        let pt = "ProhibitedText";
        let sr = "SupportResponse";
        cells = with_enum_values(cells, pt, "value", &enum_vals);
        cells = with_noun(cells, sr, &make_noun("entity"));
        cells = with_ft(cells, "ft1", &FactTypeDef {
            schema_id: String::new(),
            reading: format!("{} contains {}", sr, pt),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: sr.to_string(), role_index: 0 },
                RoleDef { noun_name: pt.to_string(), role_index: 1 },
            ],
        });
        cells = with_constraint(cells, &ConstraintDef {
            id: "c1".to_string(),
            kind: "UC".to_string(),
            modality: "Deontic".to_string(),
            deontic_operator: Some("forbidden".to_string()),
            text: format!("It is forbidden that {} contains {}", sr, pt),
            spans: vec![SpanDef { fact_type_id: "ft1".to_string(), role_index: 0, subset_autofill: None }],
            set_comparison_argument_length: None,
            clauses: None,
            entity: None,
            min_occurrence: None,
            max_occurrence: None,
            predicate: None,
        });
        cells
    }

    #[test]
    fn test_forbidden_text_detected() {
        let endash = core::char::from_u32(0x2013).unwrap().to_string();
        let emdash_s = core::char::from_u32(0x2014).unwrap().to_string();
        let cells = make_forbidden_text_cells(vec![endash, emdash_s]);
        let (_meta_state, defs, def_map) = compile_cells(cells);
        let emdash = core::char::from_u32(0x2014).unwrap();
        let text: String = ['H','e','l','l','o',' ',emdash,' ','h','o','w',' ','c','a','n',' ','I',' ','h','e','l','p','?'].iter().collect();
        let result = eval_constraints_defs(&defs, &def_map, &text, None, &empty_state());
        assert!(!result.is_empty());
        assert!(result[0].detail.contains(emdash));
    }

    #[test]
    fn test_forbidden_text_clean() {
        let endash = core::char::from_u32(0x2013).unwrap().to_string();
        let cells = make_forbidden_text_cells(vec![endash]);
        let (_meta_state, defs, def_map) = compile_cells(cells);
        let result = eval_constraints_defs(&defs, &def_map, "Hello, how can I help you today?", None, &empty_state());
        assert!(result.is_empty());
    }

    // ── Literal-in-consequent derivation (#286) ──────────────────────
    //
    // Grammar readings take the shape:
    //   Statement has Classification 'Entity Type Declaration'
    //     iff Statement has Trailing Marker 'is an entity type'.
    // The antecedent role `Trailing Marker` must EQUAL a string literal
    // (not a numeric comparator), and the consequent role `Classification`
    // must be BOUND to a string literal (not inherited from antecedent).
    // Both paths are required to make Stage-2 meta-circular.

    fn stmt_classification_cells(ant_literal: &str, cons_literal: &str) -> S {
        let ant_ft = FactTypeDef {
            schema_id: String::new(),
            reading: "Statement has Trailing Marker".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Statement".to_string(), role_index: 0 },
                RoleDef { noun_name: "Trailing Marker".to_string(), role_index: 1 },
            ],
        };
        let cons_ft = FactTypeDef {
            schema_id: String::new(),
            reading: "Statement has Classification".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Statement".to_string(), role_index: 0 },
                RoleDef { noun_name: "Classification".to_string(), role_index: 1 },
            ],
        };
        let rule = DerivationRuleDef {
            id: "entity-type-recognizer".to_string(),
            text: format!(
                "Statement has Classification '{}' iff Statement has Trailing Marker '{}'",
                cons_literal, ant_literal),
            antecedent_sources: vec![AntecedentSource::FactType("stmt_has_trailing_marker".to_string())],
            consequent_cell: ConsequentCellSource::Literal("stmt_has_classification".to_string()),
            consequent_instance_role: String::new(),
            kind: DerivationKind::ModusPonens,
            join_on: vec![], match_on: vec![], consequent_bindings: vec![],
            antecedent_filters: vec![],
            consequent_computed_bindings: vec![], consequent_aggregates: vec![],
            consequent_universals: vec![],
            unresolved_clauses: vec![],
            antecedent_role_literals: vec![crate::types::AntecedentRoleLiteral {
                antecedent_index: 0,
                role: "Trailing Marker".to_string(),
                value: ant_literal.to_string(),
            }],
            antecedent_role_comparisons: vec![],
            consequent_role_literals: vec![crate::types::ConsequentRoleLiteral {
                role: "Classification".to_string(),
                value: cons_literal.to_string(),
            }],
            materialization: crate::types::MaterializationPolicy::Stored,
            ring_join: None, skolem_head_roles: vec![], antecedent_cardinalities: vec![],
        };
        let mut cells = empty_cells();
        cells = with_ft(cells, "stmt_has_trailing_marker", &ant_ft);
        cells = with_ft(cells, "stmt_has_classification", &cons_ft);
        cells = with_derivation(cells, &rule);
        cells
    }

    #[test]
    fn literal_in_consequent_fires_when_antecedent_literal_matches() {
        // Stage-2 must see the derived classification fact when a
        // Statement carries the exact trailing-marker literal the
        // grammar rule names. Binding keys preserve noun-name spaces
        // to match the parser convention (`Trailing Marker`), looked
        // up by compile::role_value_by_name verbatim — see the sibling
        // suppressed_when_mismatches test which already used the
        // space-keyed form.
        let cells = stmt_classification_cells(
            "is an entity type", "Entity Type Declaration");
        let (_meta, defs, _def_map) = compile_cells(cells);

        let mut pop = ast::Object::phi();
        pop = ast::cell_push("stmt_has_trailing_marker",
            ast::fact_from_pairs(&[
                ("Statement", "s1"),
                ("Trailing Marker", "is an entity type"),
            ]), &pop);

        let dd = derivation_defs_from(&defs);
        let (_new_state, derived) = forward_chain_defs_state(&dd, &pop);

        let hits: Vec<_> = derived.iter()
            .filter(|d| d.fact_type_id == "stmt_has_classification")
            .collect();
        assert_eq!(hits.len(), 1,
            "expected exactly one classification fact, got {:?}", derived);
        let bindings = &hits[0].bindings;
        assert!(bindings.iter().any(|(k, v)| k == "Statement" && v == "s1"),
            "Statement binding missing: {:?}", bindings);
        assert!(bindings.iter().any(|(k, v)|
            k == "Classification" && v == "Entity Type Declaration"),
            "Classification literal binding missing: {:?}", bindings);
    }

    #[test]
    fn literal_in_consequent_suppressed_when_antecedent_literal_mismatches() {
        // If the trailing-marker value on the Statement does NOT match
        // the grammar rule's literal, no classification fact should be
        // emitted. Same-shaped rule with a different literal remains
        // inert for this statement.
        let cells = stmt_classification_cells(
            "is an entity type", "Entity Type Declaration");
        let (_meta, defs, _def_map) = compile_cells(cells);

        let mut pop = ast::Object::phi();
        pop = ast::cell_push("stmt_has_trailing_marker",
            ast::fact_from_pairs(&[
                ("Statement", "s1"),
                ("Trailing Marker", "is a value type"),
            ]), &pop);

        let dd = derivation_defs_from(&defs);
        let (_new_state, derived) = forward_chain_defs_state(&dd, &pop);

        let hits: Vec<_> = derived.iter()
            .filter(|d| d.fact_type_id == "stmt_has_classification")
            .collect();
        assert!(hits.is_empty(),
            "expected no classification facts, got {:?}", hits);
    }

    // MC3b-e (#763) retired the JSON-blob path. The MC3b-d parity tests
    // (cell-driven ⇋ JSON-blob, legacy_only / cells_only helpers) were
    // load-bearing only as long as both paths existed; they're gone with
    // the JSON-blob path. The remaining SM eval coverage above
    // (`test_run_machine_via_ast`, `test_initial_status_*`,
    // `test_run_machine_support_request_lifecycle`,
    // `test_guarded_transition_blocks_on_violation`, etc.) all build SMs
    // via `with_state_machine`, which now writes only the normalized
    // cells — so each of those tests already exercises the cell-driven
    // compile path end-to-end through `run_machine_defs`.

    // ─── task-744 phase 4: forward-chain Map-backed cell storage ────
    //
    // When a fact-type cell carries an alethic UC, the compiler emits
    // a `_CellKeyRoles` metadata cell with that FT's key-role names.
    // The forward-chain emit path consults that metadata and routes
    // derived facts through `cell_put_keyed`, producing `Object::Map`
    // contents instead of `Object::Seq`.
    //
    // Acceptance: a UC-keyed consequent cell ends up as `Object::Map`
    // after `compile_to_defs_state` + `forward_chain_defs_state`,
    // while an un-keyed cell in the same compile remains `Object::Seq`.

    #[test]
    fn forward_chain_routes_keyed_cells_through_map_storage_for_alethic_uc() {
        // Two FTs with a shared antecedent driver:
        //   ft_keyed:   "Person has Email"  — alethic UC on Person (role 0)
        //                                      → cell stores as Map keyed by Person.
        //   ft_seq:     "Person likes Topic" — no UC at all
        //                                      → cell stays as Seq.
        // A derivation rule "Person has Email <id> iff Person likes <id>"
        // fires for every Person/Topic pair, materialising the keyed cell
        // through forward-chain.
        let mut cells = empty_cells();
        cells = with_noun(cells, "Person", &make_noun("entity"));
        cells = with_noun(cells, "Email", &make_noun("value"));
        cells = with_noun(cells, "Topic", &make_noun("value"));
        cells = with_ft(cells, "ft_keyed", &FactTypeDef {
            schema_id: String::new(),
            reading: "Person has Email".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Person".to_string(), role_index: 0 },
                RoleDef { noun_name: "Email".to_string(), role_index: 1 },
            ],
        });
        cells = with_ft(cells, "ft_seq", &FactTypeDef {
            schema_id: String::new(),
            reading: "Person likes Topic".to_string(),
            readings: vec![],
            roles: vec![
                RoleDef { noun_name: "Person".to_string(), role_index: 0 },
                RoleDef { noun_name: "Topic".to_string(), role_index: 1 },
            ],
        });
        // Alethic UC on Person (role 0) of ft_keyed: each Person has at
        // most one Email. `resolve_key_roles_for_ft` picks `vec![0]` →
        // `_CellKeyRoles` registers `ft_keyed = ["Person"]`.
        cells = with_constraint(cells, &ConstraintDef {
            id: "uc_email".to_string(),
            kind: "UC".to_string(),
            modality: "Alethic".to_string(),
            text: "Each Person has at most one Email".to_string(),
            spans: vec![
                SpanDef { fact_type_id: "ft_keyed".to_string(), role_index: 0, subset_autofill: None },
            ],
            ..Default::default()
        });
        let state = build(cells);

        // Compile through the production path so `_CellKeyRoles` lands
        // in the def overlay alongside everything else.
        let defs = crate::compile::compile_to_defs_state(&state);
        let d = ast::defs_to_state(&defs, &state);

        // Verify the metadata cell actually carries ft_keyed.
        let key_roles_cell = ast::fetch_or_phi("_CellKeyRoles", &d);
        let entries = key_roles_cell.as_seq().and_then(|items| {
            if items.len() == 2 && items[0].as_atom() == Some("'") {
                items[1].as_seq().map(|s| s.to_vec())
            } else { Some(items.to_vec()) }
        }).expect("_CellKeyRoles must be present after compile_to_defs_state");
        assert!(entries.iter().any(|f| {
            ast::binding(f, "ftId") == Some("ft_keyed")
                && ast::binding(f, "keyRoles") == Some("Person")
        }), "_CellKeyRoles must register ft_keyed → Person; got {:?}", entries);
        // ft_seq has no UC → must NOT appear in the metadata.
        assert!(!entries.iter().any(|f| ast::binding(f, "ftId") == Some("ft_seq")),
            "ft_seq (no UC) must be absent from _CellKeyRoles; got {:?}", entries);

        // Hand-author a derivation that writes both into ft_keyed and ft_seq
        // so the round emits at least one fact for each consequent. We
        // skip the full DerivationRuleDef wiring here and just push the
        // facts directly via a Func::Constant — the forward-chain
        // integration path is what's under test, not the derivation
        // compiler. Distinct Person values + distinct value-role values
        // dodge the dedup in `derive_one_round` (cells are seeded empty).
        let person_facts: [(&str, &str, &str); 2] = [
            ("p1", "p1@example.com", "topic-a"),
            ("p2", "p2@example.com", "topic-b"),
        ];
        let mut emit_items: Vec<ast::Object> = Vec::new();
        for (p, email, topic) in &person_facts {
            emit_items.push(ast::Object::seq(vec![
                ast::Object::atom("ft_keyed"),
                ast::Object::atom("Person has Email"),
                ast::Object::seq(vec![
                    ast::Object::seq(vec![ast::Object::atom("Person"), ast::Object::atom(p)]),
                    ast::Object::seq(vec![ast::Object::atom("Email"), ast::Object::atom(email)]),
                ]),
            ]));
            emit_items.push(ast::Object::seq(vec![
                ast::Object::atom("ft_seq"),
                ast::Object::atom("Person likes Topic"),
                ast::Object::seq(vec![
                    ast::Object::seq(vec![ast::Object::atom("Person"), ast::Object::atom(p)]),
                    ast::Object::seq(vec![ast::Object::atom("Topic"), ast::Object::atom(topic)]),
                ]),
            ]));
        }
        let synth_func = ast::Func::constant(ast::Object::Seq(emit_items.into()));
        let dd: Vec<(&str, &ast::Func)> = vec![("derivation:test", &synth_func)];

        let (new_d, derived) = forward_chain_defs_state(&dd, &d);
        assert_eq!(derived.len(), 4,
            "all four candidate facts (2 per cell) must land as DerivedFacts; got {}",
            derived.len());

        // ft_keyed: expect Object::Map keyed by Person value.
        let keyed_cell = ast::fetch_or_phi("ft_keyed", &new_d);
        let map = keyed_cell.as_map().cloned().unwrap_or_else(|| panic!(
            "ft_keyed must be Object::Map after forward-chain (alethic UC keys it); \
             got {:?}", keyed_cell));
        assert_eq!(map.len(), 2, "two distinct Persons → two map entries");
        assert!(map.contains_key("p1"), "p1 must be a map key; keys={:?}",
            map.keys().collect::<Vec<_>>());
        assert!(map.contains_key("p2"), "p2 must be a map key; keys={:?}",
            map.keys().collect::<Vec<_>>());

        // ft_seq: no narrower UC, so #932 phase-2 folds it to a Map keyed
        // by the full tuple (synthesize_fact_id, via cell_put_folded) — the
        // SAME Map shape as a keyed cell, differing only in the key. The
        // two distinct (Person, Topic) tuples give two entries (set
        // semantics per eq:cellfold); the Seq-append path is retired.
        let seq_cell = ast::fetch_or_phi("ft_seq", &new_d);
        let folded = seq_cell.as_map().cloned().unwrap_or_else(|| panic!(
            "ft_seq must be Object::Map after forward-chain (#932 phase-2: a \
             keyless cell folds by full tuple); got {:?}", seq_cell));
        assert_eq!(folded.len(), 2,
            "two distinct (Person,Topic) tuples → two folded entries; got {}",
            folded.len());
    }
}


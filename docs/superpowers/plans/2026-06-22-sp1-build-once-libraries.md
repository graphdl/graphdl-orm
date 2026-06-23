# SP1: Build-once Libraries — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Pay each library's derivation LFP **once per (content, binary)** and load it on app compile, so app compiles delta-derive only their own additions — eliminating the per-app `Function` supertype-union reconstitution storm (root cause: `Chain Cost Driver 'supertype-union-reconstitution'`).

**Architecture:** Extend the existing **metamodel parse-cache** (`store/load_metamodel_parse_cache`, cli/entry.rs:1166-1192) into a **derived-state cache** — the "blank library db." App compile loads the pre-built derived library state as the *prior*, then runs the existing **seeded-delta semi-naive chain** (`forward_chain_defs_state_seeded_with_delta`, evaluate.rs:879) over the app's delta. The `#836` drop (`derived_wipe_set`, cli/entry.rs:1030) is scoped to app-owned cells so library-derived cells are reused, not re-derived. Pure caching of the LFP — no FP/lambda/WASM semantic change.

**Tech Stack:** Rust, the `arest` crate (`cli/entry.rs`, `evaluate.rs`, `loadcache.rs`), rusqlite, the `Func`/`Object` algebra.

## Global Constraints

- **No FP/WASM semantic change.** Library artifacts are `Object` cells + `Func` defs — the same the engine already persists. Derivation semantics unchanged.
- **`cold == warm` is the hard gate.** A warm compile (on pre-built libraries) MUST produce an identity-aware-identical final state to a cold full recompile. Every task touching the compile path adds/extends this assertion. Divergence is a release blocker.
- **Reversible by env.** `AREST_NO_LIB_CACHE=1` bypasses all SP1 caching → the exact current full-compile path. It is both the fallback and the "cold" reference for the gate.
- **Green per task.** `cargo tall` stays green after every task. Use **debug** builds for iteration (~2min); one **release** build for the final timing check.
- **Key = (content + dep keys + binary).** Every cache key includes the library's readings content, its dependency library keys, and `binary_self_hash()` (cli/entry.rs:1126) — any change invalidates. Mirrors the existing parse-cache.
- **Identity-aware comparison.** State equality uses the engine's existing identity-aware cell comparison (the same `same_identity`/`merge_states` notion in ast.rs), NOT raw string equality, because cell row order can differ.

---

### Task 0: Establish the cold-vs-warm equivalence harness

**Files:**
- Create: `crates/arest/tests/sp1_equivalence.rs`
- Reference: existing integration tests under `crates/arest/tests/` for the app-compile invocation pattern (how a test compiles a readings set to a state).

**Interfaces:**
- Produces: a test helper `fn compile_app_to_state(readings: &[(&str,&str)], no_lib_cache: bool) -> ast::Object` that compiles a readings set to its final derived state, honoring `AREST_NO_LIB_CACHE`. Later tasks consume this.
- Produces: `fn assert_states_equivalent(a: &ast::Object, b: &ast::Object)` — identity-aware per-cell comparison; panics with the first differing cell name + a diff.

- [ ] **Step 1: Find the existing compile-to-state entry used by tests.** Read 2-3 tests in `crates/arest/tests/` and `compile_to_defs_state` / the public compile entry in `cli/entry.rs`. Identify the smallest function that takes readings and returns the final `ast::Object` (post forward-chain). Document it at the top of the new test file.

- [ ] **Step 2: Write `compile_app_to_state` + `assert_states_equivalent`.** `compile_app_to_state` sets/clears `AREST_NO_LIB_CACHE` then calls the entry from Step 1. `assert_states_equivalent` iterates `ast::cells_iter(a)`, looks up each in `b`, and compares with the identity-aware helper; assert both have the same cell-name set.

- [ ] **Step 3: Sanity test — a trivial app compiled twice (both `no_lib_cache=true`) is self-equivalent.**

```rust
#[test]
fn cold_compile_is_self_consistent() {
    let app = &[("probe.md", "Widget(.id) is an entity type.\nWidget has Label.\n  Each Widget has at most one Label.\nLabel is a value type.\n")];
    let a = compile_app_to_state(app, /*no_lib_cache=*/true);
    let b = compile_app_to_state(app, true);
    assert_states_equivalent(&a, &b);
}
```

- [ ] **Step 4: Run → PASS.** `cargo test -p arest --test sp1_equivalence cold_compile_is_self_consistent`

- [ ] **Step 5: `cargo tall` green; commit.** `feat(sp1): cold-vs-warm equivalence test harness`

---

### Task 1: Build the metamodel library (derived-LFP cache)

Build the metamodel **alone** to its LFP and cache the derived state, keyed by content+binary. This is the "blank metamodel library db." It does not change app compile yet — it produces + caches the artifact, with an `AREST_NO_LIB_CACHE` bypass.

**Files:**
- Modify: `cli/entry.rs` — add `load_metamodel_derived_cache`/`store_metamodel_derived_cache` (mirror `load/store_metamodel_parse_cache` at 1166-1192, filename `arest-metamodel-derived-{sig}.db`) and `build_metamodel_library() -> ast::Object`.
- Test: `crates/arest/tests/sp1_metamodel_library.rs`

**Interfaces:**
- Produces: `fn build_metamodel_library() -> ast::Object` — the metamodel's derived LFP over the metamodel population only (no app). On `AREST_NO_LIB_CACHE`, always rebuild (no store). Implementation: parsed = `metamodel_parsed_state_seeded()` (or `load_metamodel_parse_cache().unwrap_or_else(...)`); defs = `compile_to_defs_state(&parsed)`; derived = `forward_chain_defs_state_stratified(&refs, &defs_state, 100).0`; return derived. Cache key = `metamodel_readings_signature()` (content+binary, cli/entry.rs:1142).
- Consumes: `metamodel_parsed_state_seeded`, `compile_to_defs_state`, `forward_chain_defs_state_stratified`, `db::persist_state`/`db::load_state` (for the cache file, as the parse-cache does).

- [ ] **Step 1: Failing test — the built metamodel has its supertype derivations materialized.**

```rust
#[test]
fn metamodel_library_has_function_domain_derived() {
    let mm = arest::cli::entry::build_metamodel_library(); // expose via pub(crate) + a test shim if needed
    let fbd = ast::fetch_cell_seq("Function_belongs_to_Domain", &mm);
    assert!(fbd.as_seq().map_or(false, |s| !s.is_empty()),
        "Function_belongs_to_Domain must be derived in the built metamodel library");
    let func = ast::fetch_cell_seq("Function", &mm);
    assert!(func.as_seq().map_or(false, |s| !s.is_empty()),
        "Function (supertype union) must be populated with the metamodel's nouns");
}
```

- [ ] **Step 2: Run → FAIL** (`build_metamodel_library` not defined). `cargo test -p arest --test sp1_metamodel_library`

- [ ] **Step 3: Implement `build_metamodel_library` + the derived cache.** Mirror the parse-cache exactly (the deep-read gives the verbatim pattern): `metamodel_derived_cache_path()` = temp dir + `arest-metamodel-derived-{:016x}.db` of `metamodel_readings_signature()`; `load_*` opens the sqlite + `db::load_state` + a "Function_belongs_to_Domain populated" usability guard; `store_*` persists via a tmp + atomic rename. `build_metamodel_library()`: if `AREST_NO_LIB_CACHE` unset and `load_metamodel_derived_cache()` hits → return it; else derive (parse → compile_to_defs_state → forward_chain_defs_state_stratified) and `store_*`.

- [ ] **Step 4: Run → PASS.**

- [ ] **Step 5: Cache-hit test — second call does not re-derive.** Add a test that calls `build_metamodel_library()` twice and asserts the second is fast (e.g. wrap with a coarse `Instant` budget, or assert the cache file exists after the first call). Run → PASS.

- [ ] **Step 6: `cargo tall` green; commit.** `feat(sp1): build + cache the metamodel library derived LFP`

---

### Task 2: App compile — warm-load metamodel library + delta-derive

The scale win. App compile loads the pre-built metamodel-derived state as the prior; computes the app's delta; runs the seeded-delta chain; scopes the `#836` drop to app-owned cells. `Function` is *extended* with the app's nouns (monotone add), not re-derived.

**Files:**
- Modify: `cli/entry.rs` — the multi-dir compile path (~2909-3060) and the `#836` drop (~3488-3535).
- Test: `crates/arest/tests/sp1_warm_compile.rs`

**Interfaces:**
- Consumes: `build_metamodel_library()` (Task 1); `forward_chain_defs_state_seeded_with_delta` (evaluate.rs:879); `derived_wipe_set` (cli/entry.rs:1030).
- Produces: a warm compile path gated by `!AREST_NO_LIB_CACHE`.

**Mechanism (the integration):**
1. When `AREST_NO_LIB_CACHE` is unset: `let prior = build_metamodel_library();` — the metamodel's `Function`, `Function_belongs_to_Domain`, etc. arrive materialized for the metamodel's own nouns.
2. Parse the app readings and `merge_states(&prior, &app_parsed)` → the merged schema/population. The **delta** = cells present/changed vs `prior` (the app's new nouns/FTs/instances), computed with the existing snapshot-diff pattern (deep-read item 3, entry.rs:2034-2048).
3. `seed_delta` = the app's new rows per cell. `seed` = the changed cell names.
4. Call `forward_chain_defs_state_seeded_with_delta(&refs, seed, seed_delta, &merged, 100)` → the chain *extends* `Function`/`Function_belongs_to_Domain`/etc. with the app's nouns without re-deriving the metamodel's contribution.
5. **Scope the `#836` drop:** compute `library_derived = derived_wipe_set(&prior)` (the metamodel's derived cells) and `app_derived = derived_wipe_set(&merged) \ library_derived`. The drop wipes only `app_derived`; library cells are kept (the seeded chain extends them). This is the provenance the deep-read flagged as missing — derived by *set difference against the pre-built library*, no new metadata needed.

- [ ] **Step 1: Failing test — warm Widget compile equals cold.**

```rust
#[test]
fn warm_widget_equals_cold() {
    let app = &[("probe.md", "Widget(.id) is an entity type.\nWidget has Label.\n  Each Widget has at most one Label.\nLabel is a value type.\n")];
    let cold = compile_app_to_state(app, /*no_lib_cache=*/true);
    let warm = compile_app_to_state(app, /*no_lib_cache=*/false);
    assert_states_equivalent(&cold, &warm); // identity-aware over ALL cells incl. Function, Function_belongs_to_Domain
}
```

- [ ] **Step 2: Run → FAIL** (warm path not implemented, or diverges).

- [ ] **Step 3: Implement the warm-load + delta-derive + scoped drop** per the Mechanism above. Keep the `AREST_NO_LIB_CACHE` branch routing to the exact current full path.

- [ ] **Step 4: Run → PASS** (cold == warm). If it diverges, the failing cell name from `assert_states_equivalent` localizes the bug (most likely a library cell the app extends — defer that exact case to Task 4 by adding it to the scoped drop; for Widget there should be none).

- [ ] **Step 5: Timing — warm Widget converges fast.** A test (or a manual `AREST_TIMEOUT_SECS=30` release run on the baseprobe dir) asserting the warm compile finishes (it was >90s non-converging). Record the number.

- [ ] **Step 6: `cargo tall` green; commit.** `feat(sp1): warm-load metamodel library + delta-derive on app compile`

---

### Task 3: Generalize to dependency libraries (chained builds)

Build each dependency dir (e.g. kernel, spd-1, sherlock) once on its predecessors; the app loads the chain. Brings the win to claude/support/arc.

**Files:**
- Modify: `cli/entry.rs` — generalize `build_metamodel_library` → `build_library`; resolve the app's dependency dirs (the dirs before the app dir, per the positional model, deep-read item 5) → build each (topological, cached) → fold the union as the prior.
- Test: `crates/arest/tests/sp1_library_chain.rs`

**Interfaces:**
- Produces: `fn build_library(readings: &[(String,String)], dep_state: &ast::Object, key: u64) -> ast::Object` — builds a library's derived LFP on `dep_state` (its pre-built deps as prior), cached by `key` = FNV(readings content + dep keys + binary). `build_metamodel_library()` becomes the base case (`dep_state` = empty, readings = `metamodel_readings()`).
- Consumes: Task 1/2 machinery.

- [ ] **Step 1: Failing test — a 2-layer app (one dep dir + app dir) warm == cold.**

```rust
#[test]
fn warm_two_layer_equals_cold() {
    let lib = &[("lib.md", "Gadget(.id) is an entity type.\nGadget has Tag.\n  Each Gadget has at most one Tag.\nTag is a value type.\n")];
    let app = &[("app.md", "Widget(.id) is an entity type.\nWidget refers to Gadget.\n  Each Widget refers to at most one Gadget.\n")];
    let cold = compile_layers_to_state(&[lib, app], true);
    let warm = compile_layers_to_state(&[lib, app], false);
    assert_states_equivalent(&cold, &warm);
}
```
(Add `compile_layers_to_state` to the harness: compiles an ordered list of dirs, each-before-app a library.)

- [ ] **Step 2: Run → FAIL.**
- [ ] **Step 3: Implement `build_library` + topological dep resolution + chained warm load.** Each dir before the app: `prior = fold(build_library over predecessors)`; the app delta-derives on the union.
- [ ] **Step 4: Run → PASS.**
- [ ] **Step 5: Real apps — support.auto.dev warm == cold and compiles in seconds.** Run the support compile warm vs `AREST_NO_LIB_CACHE=1` cold; assert-equivalent (or diff-localize); record warm time. Repeat for a minimal arc-agi-3 compile if feasible.
- [ ] **Step 6: `cargo tall` green; commit.** `feat(sp1): chained dependency-library builds`

---

### Task 4: Negation / app-extends-library carve-out

The edge the deep-read and spec flagged: an app adds facts/rules that feed a **library** derivation (e.g. a new Transition affecting a library SM's negation-derived `Status_is_terminal`/`rooted`). The pre-built library cell is then stale and must re-derive.

**Files:**
- Modify: `cli/entry.rs` — the scoped-drop logic from Task 2 Step 3.
- Test: `crates/arest/tests/sp1_extends_library.rs`

**Interfaces:**
- Consumes: the Task 2 `library_derived`/`app_derived` partition.
- Produces: an `affected_library_cells(merged, prior)` set — library derived cells whose antecedent cells the app wrote to → added to the drop+rederive.

- [ ] **Step 1: Failing test — app adds a Transition to a library SM; warm `terminal`/`rooted` == cold.**

```rust
#[test]
fn warm_app_extends_library_sm_equals_cold() {
    // lib defines an SM with statuses S0->S1; app adds a transition S1->S2 (new terminal/rooted set)
    let lib = &[("sm.md", /* a State Machine Definition with two statuses + one transition */ "...")];
    let app = &[("ext.md", /* a third status + a transition into it, changing terminal/rooted */ "...")];
    let cold = compile_layers_to_state(&[lib, app], true);
    let warm = compile_layers_to_state(&[lib, app], false);
    assert_states_equivalent(&cold, &warm); // terminal/rooted must match
}
```
(The implementer fills the two readings with a concrete SM per `readings/core/state.md`'s `Status is terminal/rooted` rules.)

- [ ] **Step 2: Run → FAIL** (warm keeps the stale pre-built `terminal`/`rooted`).
- [ ] **Step 3: Implement `affected_library_cells`** — for each library derived cell, if any of its antecedent cells (from the rule's reads-sidecar, `read_derivation_reads`) received app rows in the delta, add it to the drop+rederive set. Conservative: when uncertain, include it.
- [ ] **Step 4: Run → PASS.**
- [ ] **Step 5: Re-run Tasks 2-3 equivalence tests** (no regression). `cargo tall` green; commit. `feat(sp1): re-derive library cells an app extends (negation carve-out)`

---

## Self-Review

**Spec coverage:** SP1 spec §3 build → Task 1/3; §3 app compile warm-load+delta → Task 2; §5 common case → Task 2, edge case → Task 4; §5 equivalence gate → Task 0 + every task; §6 timing → Task 2 Step 5, Task 3 Step 5. Non-goals (cross-db, release tree-shake, shared libs) correctly absent. ✓

**Placeholder scan:** Task 4's two readings are described, not literal — flagged inline for the implementer to fill from `state.md` (a concrete SM is domain-specific; the test *shape* is complete). All other steps have runnable code/commands. The `compile_app_to_state` harness (Task 0) is defined before use. No "TODO/handle errors/etc."

**Type consistency:** `build_metamodel_library() -> ast::Object` (Task 1) → consumed in Task 2; generalized to `build_library(readings, dep_state, key) -> ast::Object` (Task 3) with `build_metamodel_library` as its base case. `derived_wipe_set(&ast::Object) -> HashSet<String>` used consistently (deep-read signature). `forward_chain_defs_state_seeded_with_delta(&refs, seed, delta, &d, 100)` signature matches evaluate.rs:879. ✓

**Risk note carried from spec:** Task 2 Step 4 and Task 4 are where delta-soundness is proven by the `cold==warm` gate; if Task 2 diverges on a cell beyond Widget's scope, that cell is an app-extends-library case → handled by Task 4's mechanism.

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-06-22-sp1-build-once-libraries.md`.

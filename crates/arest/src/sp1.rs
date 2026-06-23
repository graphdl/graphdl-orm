//! SP1 — build-once libraries: warm-base compilation.
//!
//! # Problem
//!
//! Every app compile re-derives the metamodel's self-derived cells from
//! scratch. Root-caused 2026-06-22 (`Chain Cost Driver
//! 'supertype-union-reconstitution'`): the wall is the `Function`
//! supertype-union reconstitution — `Function` is the root supertype
//! (Noun/Verb/Fact Type/Resource < Function), so its membership is the
//! union of every subtype, reconstituted ~24ms PER READ; a trivial
//! 1-entity app fails to converge in 90s.
//!
//! # Fix
//!
//! Pay each library's derivation LFP **once per (content, binary)** and
//! warm-load it on app compile, so app compiles delta-derive ONLY their
//! own additions. This module is the SP1 core (plan tasks 1–2):
//!
//! * [`build_metamodel_library`] — the metamodel's derived LFP over the
//!   metamodel population alone, cached (content+binary keyed) via the
//!   derived-LFP sidecar in [`crate::loadcache`]. The "blank metamodel
//!   library db." Built once; reused across app compiles.
//! * [`compile_app_with_library`] — compile an app's readings to their
//!   final derived state, either COLD (full recompile — the exact current
//!   drop-all + full-chain semantics, the equivalence reference) or WARM
//!   (load the pre-built metamodel library as the prior, then
//!   delta-derive ONLY the app's new cells via the seeded-delta
//!   semi-naive chain, scoping the `#836` derived-cell drop to app-owned
//!   cells so library cells are reused, not re-derived).
//!
//! # Correctness — the cold==warm gate
//!
//! WARM MUST produce an identity-aware-identical final state to COLD. The
//! soundness argument: the seeded-delta view-swap is sound for
//! monotone-positive deltas (AREST.tex semi-naive [bancilhon86]) — any
//! `Function`/`Function_belongs_to_Domain`/etc. fact derivable with no
//! antecedent in the app delta was derivable from the metamodel alone and
//! is ALREADY in the pre-built library; the delta path derives exactly the
//! app's new contribution. The gate (`tests/sp1_equivalence.rs`) verifies
//! it. `AREST_NO_LIB_CACHE=1` selects the cold path and is both the
//! fallback and the gate's reference.
//!
//! # No FP/WASM semantic change
//!
//! A library artifact is pre-computed `Object` cells + `Func` defs — the
//! same the engine already persists. This module caches the LFP and loads
//! it; derivation semantics are unchanged.

#![cfg(feature = "local")]

use crate::ast::{self, Object};

/// `AREST_NO_LIB_CACHE=1` bypasses all SP1 caching → the exact current
/// full-compile path. Both the operator fallback and the "cold" reference
/// for the cold==warm equivalence gate.
pub fn no_lib_cache() -> bool {
    std::env::var("AREST_NO_LIB_CACHE").map(|v| v == "1").unwrap_or(false)
}

/// Whether the `cli/entry.rs` dirs-compile path should take the SP1 WARM path
/// (load the pre-built library + delta-derive) instead of the cold full
/// recompile. DEFAULT OFF.
///
/// Why off by default: SP1 optimizes the forward-CHAIN (it loads the
/// metamodel's derived LFP instead of re-deriving it), targeting the
/// `Function` supertype-union reconstitution storm. That storm has already
/// been eliminated in the working tree by the convergence fix (the disabled
/// absorbed-`Function` bridge rules), so post-fix the chain is NOT the
/// compile bottleneck — measured on `apps/claude` the chain is ~2.4s of a
/// ~20s compile, the rest being parse-fold + persist + GC + reflect, none of
/// which SP1 touches. With nothing to save, the warm machinery (library
/// cache decode + derived-cell restore + per-cell delta diff) makes the
/// chain phase slightly SLOWER, so forcing warm on the live MCP compile path
/// would be a small regression on a memory/disk-constrained host. The
/// implementation is kept (correct, cold==warm gated, the foundation for SP2
/// cross-db / SP3 release tree-shake, and a real win for any future workload
/// whose metamodel derivation IS expensive); `AREST_LIB_CACHE=1` opts in.
///
/// `AREST_NO_LIB_CACHE=1` still hard-disables (and is the gate's cold
/// reference); it wins over this opt-in.
pub fn lib_cache_enabled() -> bool {
    !no_lib_cache() && std::env::var("AREST_LIB_CACHE").map(|v| v == "1").unwrap_or(false)
}

/// The content+binary signature of the metamodel library — the cache key.
/// Reuses the metamodel parse-cache signature (FNV of the bundled readings
/// content ⊕ the binary self-hash), so the derived cache invalidates on
/// EXACTLY the same events the parse cache does: a readings edit or a
/// rebuild. A `_d` library-specific salt distinguishes the derived-LFP
/// artifact from the parse artifact under the same content (different file
/// namespace anyway, but the salt keeps the keys provably disjoint).
fn metamodel_library_signature() -> u64 {
    // Salt the parse signature so the derived-LFP key can never collide
    // with any future cache that reuses the raw parse signature.
    crate::cli::entry::metamodel_readings_signature() ^ 0x5350_3144_4552_4956 // "SP1DERIV"
}

/// Build the metamodel library: the metamodel's derived LFP over the
/// metamodel population ALONE (no app), content+binary keyed and cached.
/// This is the "blank metamodel library db" — the warm base every app
/// compile loads.
///
/// On a cache HIT (and `AREST_NO_LIB_CACHE` unset) the pre-built derived
/// state is returned directly — the `Function` reconstitution and every
/// base derivation arrive already materialized. On a MISS (first compile
/// per binary) the metamodel is derived to its LFP and stored. Under
/// `AREST_NO_LIB_CACHE` the library is always rebuilt and never stored
/// (so a cold reference never reads or writes the cache).
///
/// Derivation mirrors the production full-compile core EXACTLY (the same
/// `derive_metamodel_lfp` body the cold app path runs over the metamodel
/// portion), so the warm base is byte-identical (identity-aware) to the
/// metamodel contribution of a cold recompile.
pub fn build_metamodel_library() -> Object {
    let bypass = no_lib_cache();
    let sig = metamodel_library_signature();
    if !bypass {
        if let Some(cached) = crate::loadcache::load_derived(sig) {
            // A populated `Function_belongs_to_Domain` confirms a usable
            // derived cache (guards a torn/partial artifact); on the
            // unusable path fall through and rebuild.
            let usable = ast::fetch_cell_seq("Function_belongs_to_Domain", &cached)
                .as_seq().map_or(false, |s| !s.is_empty());
            if usable {
                return cached;
            }
        }
    }
    let derived = derive_metamodel_lfp();
    if !bypass {
        crate::loadcache::store_derived(sig, &derived);
    }
    derived
}

/// Derive the metamodel-alone LFP: the cached SEEDED metamodel parse →
/// `compile_to_defs_state` → defs overlay → reflect → `#836` drop-all →
/// stratified semi-naive chain → reflect-tail. This is the metamodel
/// portion of the cold full-compile core (`derive_to_lfp`), run with NO
/// app readings — so its output is exactly what a cold recompile derives
/// for the metamodel's own nouns/FTs/instances.
fn derive_metamodel_lfp() -> Object {
    // Cached, app-independent seeded parse (resolves the metamodel's
    // circular cross-slice noun refs; the same parse the CLI compile path
    // folds app readings onto). `metamodel_parsed_state_seeded` is
    // memoized per-process; we clone the cell graph (Arc bumps).
    let parsed = crate::metamodel_parsed_state_seeded().clone();
    derive_to_lfp(&parsed, None)
}

/// Compile an app's readings to their final derived state.
///
/// `no_lib_cache`: when `true`, COLD — full recompile (drop ALL derived
/// cells, full stratified chain over the metamodel+app union). This is the
/// exact current semantics and the equivalence reference. When `false`,
/// WARM — load the pre-built metamodel library as the prior, merge the
/// app's parsed delta, and delta-derive only the app's new cells (scoped
/// `#836` drop + seeded-delta chain).
///
/// Both paths share the same parse → defs → reflect spine; they differ
/// ONLY in the starting derived population (parse-only vs pre-built
/// library) and the drop scope + chain seeding. The cold==warm gate
/// (`tests/sp1_equivalence.rs`) proves the two converge to the same LFP.
///
/// This is the LIBRARY-/TEST-facing pipeline: it compiles readings to a
/// derived `Object` with no SQLite persistence and no prior-DB population
/// (the CLI dirs path layers persistence + cor:closure on top and reuses
/// [`derive_to_lfp`] for the SP1-relevant drop+chain decision).
pub fn compile_app_with_library(app_readings: &[(&str, &str)], no_lib_cache: bool) -> Object {
    // Parse + merge the app readings into one state (app schema +
    // population). Mirrors the CLI per-file fold (in-domain parse +
    // ns-3/ns-4 domain stamping) so cross-file refs resolve and keyed
    // identity is preserved.
    let app_parsed = fold_app_readings(app_readings);

    // BOTH paths merge the app onto the SEEDED metamodel PARSE — so every
    // PARSE-PURE cell (schema cells, and the `stamp_file_domain` outputs like
    // `Function_belongs_to_Domain`) is computed over the IDENTICAL base on
    // both paths. WARM differs ONLY in supplying the pre-built library's
    // DERIVED cells as the chain prior (so the expensive supertype-union
    // reconstitution is loaded, not recomputed). Using the bare parse (not
    // the reconciled library) as the merge base is what keeps cold==warm for
    // order-dependent parse-stamp cells: a noun like the ubiquitous `id`
    // refscheme gets the same domain attribution on both paths, instead of
    // warm freezing the library-context value while cold re-derives the
    // app-context one.
    let merged = ast::merge_states(crate::metamodel_parsed_state_seeded(), &app_parsed);

    if no_lib_cache {
        // COLD: derive the whole union to its LFP with the drop-ALL + full
        // stratified chain — the production full-compile core, the reference.
        derive_to_lfp(&merged, None)
    } else {
        // WARM: pass the pre-built metamodel library as the prior. It is used
        // ONLY to (a) identify the library's derived cells (`derived_wipe_set`)
        // so the `#836` drop is scoped to app-derived cells, and (b) restore
        // those derived rows as the chain prior after reflect-pre. Parse-pure
        // cells come from `merged` (== cold), NOT the library.
        let prior = build_metamodel_library();
        derive_to_lfp(&merged, Some(&prior))
    }
}

/// SP1 Task 3 — compile an ORDERED list of reading layers (each but the last a
/// dependency library; the last the app) to the final derived state.
///
/// COLD (`no_lib_cache=true`): flatten every layer and full-recompile the
/// union — the cold==warm equivalence reference. WARM: build the dependency
/// chain's derived LFP once (each layer derived on its predecessors via
/// [`derive_to_lfp`], the metamodel library as the base) and delta-derive the
/// whole union on that chain prior, so the app's additions EXTEND the chain
/// rather than re-deriving it. Generalizes [`compile_app_with_library`] from a
/// single metamodel-library prior to a dependency-chain prior.
pub fn compile_layers_with_library(layers: &[&[(&str, &str)]], no_lib_cache: bool) -> Object {
    // Flatten all layers' files in order; `fold_app_readings` parses each file
    // against the accumulating context, so a later layer (the app) resolves
    // references into an earlier one (its deps). Both paths share this base.
    let all: Vec<(&str, &str)> = layers.iter().flat_map(|l| l.iter().copied()).collect();
    let merged = ast::merge_states(
        crate::metamodel_parsed_state_seeded(), &fold_app_readings(&all));
    if no_lib_cache {
        derive_to_lfp(&merged, None)
    } else {
        let prior = build_chain(&layers[..layers.len().saturating_sub(1)]);
        derive_to_lfp(&merged, Some(&prior))
    }
}

/// Content+binary signature of a dependency-chain prefix — the per-layer
/// derived-LFP cache key. FNV-1a over the accumulated layer filenames+text,
/// salted by `metamodel_library_signature()` (which folds in the binary
/// self-hash), so a readings edit OR a rebuild invalidates it — the same
/// regime as the metamodel library, extended with the dependency content.
fn dep_chain_signature(acc: &[(&str, &str)]) -> u64 {
    let mut h: u64 = metamodel_library_signature();
    for (name, text) in acc {
        for b in name.bytes().chain(text.bytes()) {
            h ^= b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3); // FNV-1a-64 prime
        }
    }
    h ^ 0x4445_5043_4841_494e // "DEPCHAIN" salt
}

/// Build the derived LFP of a list of dependency layers, chained and CACHED:
/// each layer's cells derive on its predecessors' derived state (the metamodel
/// library is the base), and each chain prefix's LFP is cached by
/// `dep_chain_signature` so a repeated / cross-app compile reuses it ("build
/// once"). Returns the metamodel library when `deps` is empty. Under
/// `AREST_NO_LIB_CACHE` it always rebuilds and never stores (the cold
/// reference). This is the warm prior the dependent layer delta-derives on.
fn build_chain(deps: &[&[(&str, &str)]]) -> Object {
    let bypass = no_lib_cache();
    let mut prior = build_metamodel_library();
    let mut acc: Vec<(&str, &str)> = Vec::new();
    for dep in deps {
        acc.extend(dep.iter().copied());
        let sig = dep_chain_signature(&acc);
        // Cache hit: a usable pre-built chain prefix (guarded by a populated
        // Function_belongs_to_Domain, like the metamodel library) becomes the
        // prior for the next layer without re-deriving.
        if !bypass {
            if let Some(cached) = crate::loadcache::load_derived(sig) {
                if ast::fetch_cell_seq("Function_belongs_to_Domain", &cached)
                    .as_seq().map_or(false, |s| !s.is_empty())
                {
                    prior = cached;
                    continue;
                }
            }
        }
        // Parse the dependency layers SO FAR on the metamodel base (the same
        // base the cold path parses on), derive on the accumulated prior, and
        // cache the prefix LFP; the next layer derives on the result.
        let merged = ast::merge_states(
            crate::metamodel_parsed_state_seeded(), &fold_app_readings(&acc));
        prior = derive_to_lfp(&merged, Some(&prior));
        if !bypass {
            crate::loadcache::store_derived(sig, &prior);
        }
    }
    prior
}

/// CLI WARM drop+chain — the production integration point for the
/// `cli/entry.rs` dirs-compile path. Given that path's already-built `d`
/// (defs compiled, keyed-cells reconciled, schema-as-facts reflected over
/// the PARSE-ONLY base — i.e. the same reflect-pre input COLD has, since the
/// CLI folds app readings onto the metamodel PARSE, not the derived library),
/// this:
///
///   1. loads the pre-built metamodel library (content+binary cached),
///   2. RESTORES the library's derived cells onto `d` as the warm chain prior
///      (so the `Function` supertype-union reconstitution arrives materialized,
///      not recomputed — the storm SP1 eliminates),
///   3. SCOPES the `#836` drop to app-owned derived cells (library cells kept),
///   4. runs the seeded-delta semi-naive chain over the app's delta.
///
/// Returns `(state, converged)` mirroring the CLI's existing
/// `(d, chain_converged)` contract so the `_CompileSig` persistence gate is
/// unchanged. The CLI keeps its EXACT current cold path under
/// `AREST_NO_LIB_CACHE=1` (the equivalence reference + fallback); this is the
/// `!no_lib_cache()` branch.
///
/// Equivalence: `cli_warm_derive(d)` over the CLI's `d` is the same warm
/// drop+chain the cold==warm gate (`tests/sp1_equivalence.rs`) proves
/// identity-equal to the cold full recompile — the CLI's `d` reaches this
/// point having taken the SAME parse→defs→reconcile→reflect-pre spine the gate's
/// COLD path takes (entry.rs mirrors `derive_to_lfp`'s prefix), so the
/// gate-proven invariant carries to production.
pub fn cli_warm_derive(d: &Object) -> (Object, bool) {
    let prior = build_metamodel_library();
    let lib_derived = crate::cli::entry::derived_wipe_set(&prior);
    // Restore the library's derived rows as the warm prior (after the CLI's
    // reflect-pre, which already ran over the bare base — matching the gate's
    // ordering). Empty library cells are skipped (see `restore_cells`).
    let d = restore_cells(d, &prior, &lib_derived);
    let rules = collect_derivation_rules(&d);
    derive_warm(d, &prior, &lib_derived, rules)
}

/// CLI WARM drop+chain over a DEPENDENCY CHAIN (SP1 Task 3) — the production
/// integration for a multi-dir app whose `dirs` before the app dir are
/// dependency libraries. Builds the chained dependency prior once (each dep
/// dir's LFP derived on its predecessors, cached), then runs the SAME warm
/// restore + scoped-drop + seeded-delta chain `cli_warm_derive` uses, but with
/// the dependency-inclusive prior — so the app delta-derives on (metamodel +
/// all deps) instead of the metamodel alone. `dep_layers` are the dependency
/// dirs' readings in order (the app dir is `d`, already merged + reflect-pre'd
/// by the CLI). Identity-equal to the cold recompile (the cold==warm gate
/// generalizes — see `warm_two_layer_equals_cold`).
pub fn cli_warm_derive_chained(d: &Object, dep_layers: &[Vec<(String, String)>]) -> (Object, bool) {
    // Borrow the owned (String,String) readings as the &str slices build_chain wants.
    let borrowed: Vec<Vec<(&str, &str)>> = dep_layers.iter()
        .map(|layer| layer.iter().map(|(n, t)| (n.as_str(), t.as_str())).collect())
        .collect();
    let slices: Vec<&[(&str, &str)]> = borrowed.iter().map(|l| l.as_slice()).collect();
    let prior = build_chain(&slices);
    let lib_derived = crate::cli::entry::derived_wipe_set(&prior);
    let d = restore_cells(d, &prior, &lib_derived);
    let rules = collect_derivation_rules(&d);
    derive_warm(d, &prior, &lib_derived, rules)
}

/// Fold a list of `(filename, text)` app readings into one parsed state,
/// in order, each parsed against the accumulating context (so cross-file
/// references resolve) with ns-3 noun-domain + ns-4 file-domain stamping —
/// the same per-file fold the CLI dirs path runs.
///
/// The fold STARTS from the metamodel's Noun/FactType/Role catalog (the
/// same `app_noun_seed` role the CLI dirs path provides, arc issues
/// 14/14b) so an app declaration referencing core vocabulary (`Case
/// observes Fact` — `Fact` is a metamodel noun) resolves. The seed cells
/// dedupe by identity when the metamodel is re-merged downstream (via
/// `prior` / the seeded parse), so this contributes parse CONTEXT only,
/// not a second copy of the metamodel population. Returned state is the
/// app's own parsed cells layered on that catalog.
fn fold_app_readings(app_readings: &[(&str, &str)]) -> Object {
    let mm = crate::metamodel_parsed_state_seeded();
    let seed: Object = {
        let mut m: hashbrown::HashMap<String, Object> = hashbrown::HashMap::new();
        for c in ["Noun", "FactType", "Role"] {
            m.insert(c.to_string(), ast::fetch_cell_seq(c, mm));
        }
        Object::map(m)
    };
    app_readings.iter().fold(seed, |merged, (name, text)| {
        let this = crate::parse_forml2::parse_to_state_from_in_domain(text, &merged, name)
            .unwrap_or_else(|e| panic!("{}: {}", name, e));
        let this = ast::annotate_noun_domain(&this, name);
        let this = ast::merge_states(&this, &ast::stamp_file_domain(&this, name));
        ast::merge_states(&merged, &this)
    })
}

/// The SP1-relevant compile core: from a merged schema+population state,
/// produce the final derived LFP. `prior` selects the path:
///
/// * `prior == None` — COLD: drop ALL derived consequent cells (the
///   `#836` wipe) and run the full stratified semi-naive chain. The
///   merged state's derived cells are assumed parse-only or stale; the
///   chain recomputes the complete LFP from primary facts (AREST.tex
///   §4.3, Knaster-Tarski). This is byte-identical to the current
///   production full-compile derive.
///
/// * `prior == Some(lib)` — WARM: the merged state already carries
///   `lib`'s fully-derived library cells. SCOPE the `#836` drop to
///   APP-derived cells (`derived_wipe_set(merged) \ derived_wipe_set(lib)`)
///   — library-derived cells are kept (already at LFP) and EXTENDED by the
///   chain, not re-derived. Seed the chain with the app's changed cells
///   and their new rows (the delta vs `lib`), then run the seeded-delta
///   semi-naive chain: it fires only rules touching the app delta and, for
///   sidecar-complete positive rules, evaluates over per-antecedent delta
///   views (|ΔA|×|B|) — the monotone extension of `Function` etc. with the
///   app's nouns.
///
/// The parse→defs→reflect spine is IDENTICAL on both paths; only the drop
/// scope + chain entry differ. The CLI dirs path calls this with its own
/// already-built `d` (so it inherits the same warm/cold decision).
pub fn derive_to_lfp(merged: &Object, prior: Option<&Object>) -> Object {
    // NOTE: the platform dispatch defs (`apply`/`compile`/`audit`/
    // `verify_signature`/`induce`) are deliberately NOT overlaid here —
    // they are SYSTEM-dispatch entries, not part of the derived LFP or the
    // population, and the CLI dirs path adds them on its own `d` before
    // calling into the drop+chain core. Omitting them keeps this pipeline's
    // output to exactly the schema + def + derived-population cells the
    // cold==warm gate compares.

    // Compile defs over the UNION schema (metamodel+app). The app's rules,
    // per-noun validate defs, sql DDL, etc. are minted here; the metamodel
    // defs re-mint identically (deterministic function of the schema).
    let compile_defs = crate::compile::compile_to_defs_state(merged);
    let d = ast::defs_to_state(&compile_defs, merged);

    // WARM clean-base normalization. The reflect passes below are SET-REPLACE
    // pure functions of the population, but `reflect_schema_cells` reads the
    // population of EVERY declared fact-type cell — including DERIVED cells
    // and its OWN reflection-output cells (which carry entity-noun roles like
    // `Fact`). So its result depends on which derived/reflection rows are
    // present in its INPUT. The COLD path's reflect-pre input is parse-only
    // (`merged` carries NO derived rows and NO reflection rows — those are
    // produced later), so COLD reflects over the bare primary population. The
    // WARM `merged` carries the pre-built library's FULLY-derived +
    // saturated-reflection cells, which would (a) make reflect-pre reflect
    // the library's derived rows that COLD's reflect-pre cannot see, and
    // (b) cascade the reflection cells into an ever-larger set.
    //
    // Normalize WARM to COLD: snapshot the library's derived rows, then drop
    // BOTH the derived cells and the reflection cells so reflect-pre runs
    // over the SAME bare primary base COLD uses. The library's derived rows
    // are RESTORED right after reflect-pre (below) as the warm chain prior —
    // so the expensive supertype-union reconstitution is still loaded, not
    // recomputed; only the reflect INPUT is normalized.
    let warm_lib_derived: Option<(hashbrown::HashSet<String>, Object)> =
        prior.map(|lib| (crate::cli::entry::derived_wipe_set(lib), lib.clone()));
    let d = if let Some((ref lib_derived, _)) = warm_lib_derived {
        let mut wipe = reflection_cell_names();
        wipe.extend(lib_derived.iter().cloned());
        drop_cells(&d, &wipe)
    } else {
        d
    };

    // UC upsert (984-B parity): corrected single-valued facts displace
    // stale priors at the same key BEFORE the chain reads the population.
    let d = {
        let key_roles = crate::evaluate::read_cell_key_roles(&d);
        let (next, _displaced) = ast::reconcile_keyed_cells(&d, &key_roles);
        next
    };

    // Reflect schema-as-facts BEFORE the chain so metamodel rules whose
    // antecedents are reflection cells fire on the first pass
    // (compile-chain-before-reflect-lag). Both paths now reflect over the
    // bare primary base → identical reflect-pre output.
    let d = apply_reflect(&d);

    // WARM: restore the library's derived rows as the chain prior (the warm
    // base the seeded-delta chain extends, NOT recomputes). Reflect-pre has
    // already run over the bare base, so this only affects the chain input.
    let d = if let Some((ref lib_derived, ref lib)) = warm_lib_derived {
        restore_cells(&d, lib, lib_derived)
    } else {
        d
    };

    // Collect the derivation rule pack: user `rule_*` + the synthetic SM
    // family (init / event-fold / for-resource-backfill / instance-of-def-
    // backfill). EXACTLY the prefixes the CLI dirs path collects.
    let rules = collect_derivation_rules(&d);

    let (d, _converged) = if let Some((ref lib_derived, ref lib)) = warm_lib_derived {
        derive_warm(d, lib, lib_derived, rules)
    } else {
        derive_cold(d, rules)
    };

    // Reflect-tail: re-canonicalize the set-replace reflection layers over
    // whatever the chain added. NOTE we deliberately do NOT reset the
    // reflection cells here: COLD's reflect-tail runs over its post-chain
    // state whose reflection cells are reflect-PRE's output (R1), and the
    // tail reflects (primary + derived + R1) → R2 (the cascade's 2nd step).
    // WARM's reflect-pre output already equals COLD's (the clean-base
    // normalization above guaranteed identical reflect-pre input), so
    // leaving R1 in place makes WARM's reflect-tail reflect the SAME
    // (primary + derived + R1) → the SAME R2. Resetting here would drop R1
    // and under-reflect relative to COLD.
    apply_reflect(&d)
}

/// The cells `reflect_schema_cells` SET-REPLACES (its 11 output cells). The
/// WARM path resets these to φ before each reflect pass so reflecting a
/// state that carries the pre-built library's saturated reflection rows
/// cannot cascade (these cells hold entity-noun roles, so reflecting them
/// re-reflects their own rows into a larger set). COLD never needs this:
/// its input's reflection cells are already empty (parse-only `merged`).
/// Kept in sync with `reflect_schema_cells`'s return tuple (compile.rs).
fn reflection_cell_names() -> hashbrown::HashSet<String> {
    [
        "Fact_Type_has_Role",
        "Noun_plays_Role",
        "Noun_has_Object_Type",
        "Noun_has_Conceptual_Data_Type",
        "Noun_has_World_Assumption",
        "Fact_Type_has_Reading",
        "Reading_has_Text",
        "Role_is_used_in_Reading",
        "Reading_is_used_by_Verb",
        "Resource_is_instance_of_Noun",
        "Fact_is_of_Fact_Type",
    ].iter().map(|s| s.to_string()).collect()
}

/// COLD derive: drop ALL derived consequent cells, then run the full
/// stratified semi-naive chain to the LFP. The production full-compile
/// semantics — the equivalence reference. Returns `(state, converged)`.
fn derive_cold(d: Object, rules: Vec<(String, ast::Func)>) -> (Object, bool) {
    let drop_set = crate::cli::entry::derived_wipe_set(&d);
    let d = drop_cells(&d, &drop_set);
    if rules.is_empty() {
        return (d, true);
    }
    let packed = pack_rules(&rules, &d);
    let refs: Vec<(&str, &ast::Func, Option<&[String]>)> = packed.iter()
        .map(|(name, func, reads)| (name.as_str(), func, reads.as_deref()))
        .collect();
    let (new_d, _derived) = crate::evaluate::forward_chain_defs_state_stratified(&refs, &d, 100);
    let converged = !crate::evaluate::take_chain_abort();
    (new_d, converged)
}

/// WARM derive: scope the `#836` drop to app-owned derived cells (keep the
/// library's, already restored as the chain prior), seed the chain with the
/// app's delta vs `lib`, and run the seeded-delta semi-naive chain — the
/// monotone extension of the library cells with the app's contribution.
///
/// `lib_derived` = `derived_wipe_set(lib)` (the library's derived consequent
/// cells), computed once by the caller. `d` already carries the library's
/// derived rows (restored after reflect-pre) plus the app's parsed delta and
/// the bare reflect-pre output.
fn derive_warm(
    d: Object,
    lib: &Object,
    lib_derived: &hashbrown::HashSet<String>,
    rules: Vec<(String, ast::Func)>,
) -> (Object, bool) {
    // Scope the drop: app-derived = all derived consequents MINUS the
    // library's derived consequents. The library cells stay (restored as the
    // warm prior); the seeded chain EXTENDS them with the app's delta rather
    // than re-deriving them. Derivation by set difference against the
    // pre-built library — the provenance the design calls for, no new
    // metadata.
    let all_derived = crate::cli::entry::derived_wipe_set(&d);
    let app_derived: hashbrown::HashSet<String> = all_derived
        .difference(lib_derived).cloned().collect();
    let d = drop_cells(&d, &app_derived);

    if rules.is_empty() {
        return (d, true);
    }

    // Seed = the app-derived cells we just dropped (so their producing rules
    // re-fire and re-materialize them) PLUS every PRIMARY cell whose rows
    // changed vs the library prior (the app's new nouns/FTs/instances). The
    // seed_delta carries those cells' NEW rows (current `d` minus `lib`,
    // identity by encoding) so the delta-join path can evaluate
    // sidecar-complete positive rules over per-antecedent delta views
    // (|ΔA|×|B|) — extending `Function` etc. with the app's nouns.
    //
    // DEF cells are not chain inputs. PRIMARY cells get a TARGETED delta (the
    // app's new rows vs `lib`) so the delta-view path varies only |ΔA|.
    let reflection = reflection_cell_names();
    let mut seed: hashbrown::HashSet<String> = app_derived.clone();
    let mut seed_delta: hashbrown::HashMap<String, Vec<Object>> = hashbrown::HashMap::new();
    for (cell, contents) in ast::cells_iter(&d) {
        if cell.contains(':') || reflection.contains(cell) {
            continue;
        }
        let prior_rows: hashbrown::HashSet<String> = ast::fetch_cell_seq(cell, lib)
            .as_seq()
            .map(|s| s.iter().map(|f| f.to_string()).collect())
            .unwrap_or_default();
        let new_rows: Vec<Object> = ast::cell_facts_iter(contents)
            .filter(|f| !prior_rows.contains(&f.to_string()))
            .cloned()
            .collect();
        if !new_rows.is_empty() {
            seed.insert(cell.to_string());
            seed_delta.insert(cell.to_string(), new_rows);
        }
    }
    // REFLECTION cells get a FULL delta = ALL their current rows. They were
    // reset+recomputed fresh by reflect-pre (bare-primary), so their value
    // intentionally differs from `lib`'s saturated value — a diff against
    // `lib` is meaningless. Crucially, a rule the delta-join path routes to
    // the VIEW branch (e.g. the `Fact_Type_has_Arity` aggregate, which reads
    // the reflection cell `Fact_Type_has_Role`) derives ONLY over read-cells
    // present in `seed_delta`; a seeded-but-delta-less read cell makes such a
    // rule derive NOTHING (it is excluded from the full-eval batch because
    // `view_ok` holds, yet the view loop skips cells absent from the delta
    // map — the silent under-derive that left app FT arities missing). Giving
    // the reflection cell its FULL rows makes the view ΔA == A == a full
    // evaluation for that occurrence: SOUND (every candidate is deduped
    // against `existing_keys`) and complete (the aggregate sees the entire
    // population). COLD reaches the same result because its non-delta
    // stratified chain full-evaluates every rule in round 0.
    for (cell, contents) in ast::cells_iter(&d) {
        if !reflection.contains(cell) {
            continue;
        }
        let rows: Vec<Object> = ast::cell_facts_iter(contents).cloned().collect();
        if !rows.is_empty() {
            seed.insert(cell.to_string());
            seed_delta.insert(cell.to_string(), rows);
        }
    }

    let packed = pack_rules(&rules, &d);
    let refs: Vec<(&str, &ast::Func, Option<&[String]>)> = packed.iter()
        .map(|(name, func, reads)| (name.as_str(), func, reads.as_deref()))
        .collect();
    if std::env::var("AREST_SP1_DIAG").is_ok() {
        eprintln!("[sp1] warm delta-derive: {} rules, seed={} cells, \
                   seed_delta={} cells, app-derived(dropped)={} cells",
            refs.len(), seed.len(), seed_delta.len(), app_derived.len());
    }
    let (new_d, _derived) = crate::evaluate::forward_chain_defs_state_seeded_with_delta(
        &refs, seed, seed_delta, &d, 100);
    let converged = !crate::evaluate::take_chain_abort();
    (new_d, converged)
}

/// Collect the derivation rule defs from `d`: user `rule_*` plus the
/// synthetic SM family. EXACTLY the prefixes the CLI dirs-compile path
/// packs (entry.rs ~3575-3591). `_cwa_negation_*` per-FT expansions are
/// deliberately excluded (they can spike the fixpoint), matching the CLI.
fn collect_derivation_rules(d: &Object) -> Vec<(String, ast::Func)> {
    let collect = |prefix: &str| -> Vec<(String, ast::Func)> {
        ast::cells_iter(d).into_iter()
            .filter(|(n, _)| n.starts_with(prefix))
            .map(|(n, contents)| (n.to_string(), ast::metacompose(contents, d)))
            .collect()
    };
    let mut rules = collect("derivation:rule_");
    rules.extend(collect("derivation:_sm_init_"));
    rules.extend(collect("derivation:_sm_event_fold_"));
    rules.extend(collect("derivation:_sm_for_resource_backfill_"));
    rules.extend(collect("derivation:_sm_instance_of_def_backfill_"));
    rules
}

/// Pack each rule with its `derivation_reads:<id>` sidecar (the antecedent
/// cells the semi-naive gate / delta-view path consult). A rule with no
/// sidecar maps to `None` and runs every round — identical to the naive
/// chainer.
fn pack_rules(rules: &[(String, ast::Func)], d: &Object)
    -> Vec<(String, ast::Func, Option<Vec<String>>)>
{
    rules.iter()
        .map(|(name, func)| {
            let id = name.split_once(':').map(|(_, id)| id).unwrap_or(name.as_str());
            (name.clone(), func.clone(), crate::evaluate::read_derivation_reads(d, id))
        })
        .collect()
}

/// Overlay the named cells' contents FROM `src` ONTO `d`, returning a fresh
/// state. Used by the WARM path to restore the pre-built library's derived
/// rows after reflect-pre has run over the bare primary base — re-attaching
/// the warm chain prior without re-deriving it.
fn restore_cells(d: &Object, src: &Object, names: &hashbrown::HashSet<String>) -> Object {
    if names.is_empty() {
        return d.clone();
    }
    let mut map: hashbrown::HashMap<String, Object> = ast::cells_iter(d)
        .into_iter()
        .map(|(name, contents)| (name.to_string(), contents.clone()))
        .collect();
    for name in names {
        let cell = ast::fetch_or_phi(name, src);
        // Skip EMPTY library cells: a declared-but-unpopulated derived FT
        // contributes nothing as a warm prior, and materializing it would
        // create a cell the COLD parse-only base never has (cold==warm treats
        // empty ≡ absent, but not creating it keeps warm's output minimal and
        // matches cold's cell set exactly). Restore the RAW cell shape
        // (Map-backed keyed cells stay Map; Seq cells stay Seq) so a populated
        // warm prior is byte-faithful to the library.
        if ast::cell_facts_iter(&cell).next().is_some() {
            map.insert(name.clone(), cell);
        }
    }
    Object::map(map)
}

/// Drop the named cells to `φ` (the `#836` wipe), returning a fresh state.
fn drop_cells(d: &Object, drop: &hashbrown::HashSet<String>) -> Object {
    if drop.is_empty() {
        return d.clone();
    }
    let mut map: hashbrown::HashMap<String, Object> = hashbrown::HashMap::new();
    for (name, contents) in ast::cells_iter(d) {
        if drop.contains(name) {
            map.insert(name.to_string(), Object::phi());
        } else {
            map.insert(name.to_string(), contents.clone());
        }
    }
    Object::map(map)
}

/// Set-replace the schema-as-facts reflection cells over `d` (idempotent).
fn apply_reflect(d: &Object) -> Object {
    let mut map: hashbrown::HashMap<String, Object> = ast::cells_iter(d)
        .into_iter()
        .map(|(name, contents)| (name.to_string(), contents.clone()))
        .collect();
    for (name, contents) in crate::compile::reflect_schema_cells(d) {
        map.insert(name, contents);
    }
    Object::map(map)
}

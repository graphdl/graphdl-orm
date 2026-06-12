// CLI subcommand handlers — std-only.
//
// Submodules implement the `arest <verb> <args…>` family of CLI
// subcommands. main.rs (and the bin target) dispatches to them after
// argv parsing; each submodule owns its own help text + exit codes.
//
// Currently:
//   * `run` — `arest run "App Name"` dispatches to
//             `crate::command::wine_app_by_name` to resolve the slug +
//             prefix, then calls `wine_bootstrap::bootstrap_prefix`
//             to apply winetricks recipes / DLL overrides / registry
//             keys derived from the FORML facts in
//             `readings/compat/wine.md`, then calls
//             `wine_install::install_app` to fetch + run the
//             installer binary under wine.
//   * `wine_bootstrap` — orchestrates the prefix bootstrap by walking
//             `Wine_App_requires_Required_Component` cells (winetricks
//             recipes), `requires DLL Override of` legacy cells (DLL
//             overrides) and `requires Registry Key at` legacy cells
//             (registry keys) for a given app id.
//   * `winetricks` — wraps the `winetricks` shell script as a
//             subprocess; reads the prefix's `winetricks.log` to
//             short-circuit already-applied recipes for idempotency.
//   * `wine_overrides` — DLL override + registry-key writers; emits
//             `[Software\\Wine\\DllOverrides]` blocks into the
//             prefix's `system.reg` and `@="<value>"` keys into
//             `system.reg` / `user.reg` per the registry root.
//   * `wine_install` (#505) — installer fetch + install orchestrator.
//             Resolves Installer URL / Filename from the FORML facts,
//             fetches the binary into `<prefix>/drive_c/_install/`,
//             runs it under wine, transitions the install state
//             machine. Idempotent via `_install_complete` marker.
//   * `installer_fetch` (#505) — subprocess wrapper around curl /
//             PowerShell `Invoke-WebRequest` for the binary download;
//             also handles local-path copies for pre-staged
//             installers.
//   * `installer_run` (#505) — subprocess wrapper for `wine
//             <installer>`; captures stdout + stderr to
//             `<prefix>/drive_c/_install_log` for debugging.
//   * `wine_launch` (#506) — main app launch + monitor. Resolves the
//             Main Exe Path from FORML facts, spawns wine on it under
//             `WINEPREFIX=<prefix>` with `WINEDEBUG=-all`, samples
//             the monitor after a short settle delay, and walks the
//             outcome through the `Wine_App_run_status` SM cell
//             (Running → Paused | Exited | Crashed). Captures
//             stdout+stderr to `<prefix>/drive_c/_run_log`.
//             Idempotent: refuses to relaunch when the cell's
//             most-recent transition for the app is `Running`.
//   * `process_monitor` (#506) — non-blocking `Child::try_wait`
//             wrapper translating into a `MonitorOutcome` enum
//             (`StillRunning`, `Exited(i32)`, `Crashed { exit_code }`,
//             `Errored`). Used by `wine_launch` for the post-spawn
//             settle poll and by the future `arest watch` flow for
//             ongoing observation.
//
// Future verbs (`arest install`, `arest exec`, …) plug in here so
// main.rs doesn't grow another giant `match` arm per subcommand.

// Wine compat surface (#481-#506), factored behind `feature = "wine"`
// (task 633): the `run` subcommand, prefix bootstrap, winetricks /
// overrides, installer fetch+run, launch+monitor. `installer_*` and
// `process_monitor` are wine-only by usage (their sole consumers are
// wine_install / wine_launch), so they ride the same gate. Non-wine
// builds drop the whole surface; `cli::entry`'s `run` verb already
// carries a `not(wine)` arm that names the rebuild flag.
#[cfg(all(not(feature = "no_std"), feature = "wine"))]
pub mod run;
#[cfg(all(not(feature = "no_std"), feature = "wine"))]
pub mod wine_bootstrap;
#[cfg(all(not(feature = "no_std"), feature = "wine"))]
pub mod wine_overrides;
#[cfg(all(not(feature = "no_std"), feature = "wine"))]
pub mod winetricks;
#[cfg(all(not(feature = "no_std"), feature = "wine"))]
pub mod wine_install;
#[cfg(all(not(feature = "no_std"), feature = "wine"))]
pub mod installer_fetch;
#[cfg(all(not(feature = "no_std"), feature = "wine"))]
pub mod installer_run;
#[cfg(all(not(feature = "no_std"), feature = "wine"))]
pub mod process_monitor;
#[cfg(all(not(feature = "no_std"), feature = "wine"))]
pub mod wine_launch;
// `entropy_host` (#574) — host-OS `EntropySource` adapter delegating to
// `getrandom` (Linux/FreeBSD getrandom(2), macOS arc4random_buf, Windows
// BCryptGenRandom). Installed by callers that need RNG before any
// `csprng::random_*` path fires; the CLI itself doesn't auto-install
// today (per-target adapter job, see #575/#578).
#[cfg(not(feature = "no_std"))]
pub mod entropy_host;
// `tenant_master_host` (#663) — host-CLI tenant master installer.
// Generates 32 random bytes on first run, persists to
// `~/.arest/tenant_master.bin` (mode 0600 on Unix, restricted ACL on
// Windows), reads on subsequent runs. Wires into the cell_aead global
// slot via `arest::cell_aead::install_tenant_master`. Boot order:
// `entropy_host::install` first (csprng needs it for the seed), then
// `tenant_master_host::install` (uses csprng to generate the master
// on first run).
#[cfg(not(feature = "no_std"))]
pub mod tenant_master_host;
// `reload` (#561) — `arest reload <file.md>` runtime reading load.
// Routes through `crate::load_reading_core::load_reading` with
// `LoadReadingPolicy::AllowAll` and persists the merged state to the
// configured `--db`. Companion `arest watch <dir>` shares the same
// `dispatch_with_state` core.
#[cfg(not(feature = "no_std"))]
pub mod reload;
// `watch` (#561 followup / DynRdg-T2) — `arest watch <dir>` polls a
// directory for `.md` changes and re-applies each modified file via
// the same `LoadReading` pipeline as `arest reload`. Pure scan core
// (`scan_once_with_state`) is testable without DB; the DB-backed
// `dispatch` enters an infinite poll loop with per-reload persist.
#[cfg(not(feature = "no_std"))]
pub mod watch;
// `entry` (#684/#650b) — main CLI dispatcher extracted from src/main.rs.
// Pre-extract, src/main.rs declared `mod ast; mod compile; ...` for
// every lib module independently of lib.rs, forcing cargo to recompile
// the entire crate twice (once for the lib's rlib, once for the bin's
// compilation unit). Profile (cargo-timing 2026-05-01) showed ~120s of
// duplicate cumulative compile across `arest-cli "bin"` and
// `arest-cli "bin" (test)`. Now `cli::entry::main_entry` carries the
// dispatcher inside the lib's compilation, src/main.rs is a 6-line
// shim, and each source file compiles exactly once.
#[cfg(not(feature = "no_std"))]
pub mod entry;

// compile-gc-orphaned-derived-facts (asserted-cell dup-fact bloat): the
// pattern matches `cli/entry.rs:1144-1160` — the bake-time compile path
// applies an identity-aware dedup pass over the final state before
// persisting, so cells like `Task_is_epic` that accrue one extra
// identity-equal copy per recompile (312 bindings for 8 distinct tasks
// observed live) are scrubbed before the row hits SQLite.
//
// `arest reload <file.md>` and `arest watch <dir>` bypass that site —
// they thread through `load_reading_core::load_reading`, which merges
// the new reading into the prior state via `ast::merge_states` /
// `ast::concat_dedup`. `concat_dedup` dedups the INCOMING side against
// the accumulator but never the accumulator's OWN internal duplicates
// (documented at `ast::dedup_cell_facts`), so a bloated prior cell
// loaded from disk stays bloated through the merge and re-persists at
// the same size on every reload. This helper applies the same dedup
// pattern to the runtime-load paths so their persisted result self-
// heals identically to the dirs-compile path.
//
// Layout mirrors entry.rs:1144-1160:
//   * declared-FT data cells (in `FactType.id`) get the full
//     arity+subject GC plus identity dedup;
//   * non-`:` non-meta cells get the arity-free empty-subject drop plus
//     identity dedup (safe without a uniformity assumption for
//     synthetic SM outputs etc.);
//   * `:` view / meta cells pass through (they regenerate from data).
//
// TODO(arest#TBD): extract the matching block in `cli/entry.rs:1144-
// 1160` to call this helper too, so the three sites stop drifting.
#[cfg(all(not(feature = "no_std"), feature = "local"))]
pub(crate) fn dedup_state_for_persist(d: &crate::ast::Object) -> crate::ast::Object {
    dedup_state_for_persist_inner(d, None)
}

/// 987-A.3 (delta tail): scoped variant for the leaf-ingest path —
/// GC+dedup ONLY the named cells, pass every other cell through
/// UNTOUCHED. The full sweep re-encodes unchanged cells
/// (φ-canonicalization, identity dedup), which would mark them
/// changed in the leaf path's post-tail diff and defeat the delta
/// persist (every cell would look dirty every ingest).
#[cfg(all(not(feature = "no_std"), feature = "local"))]
pub(crate) fn dedup_state_for_persist_scoped(
    d: &crate::ast::Object,
    scope: &hashbrown::HashSet<String>,
) -> crate::ast::Object {
    dedup_state_for_persist_inner(d, Some(scope))
}

// Shares the cfg of its callers: the body calls the local-only
// case-collision detector (the cfg the refactor briefly dropped —
// gate-2/default went red on E0425; this is the restore).
#[cfg(all(not(feature = "no_std"), feature = "local"))]
fn dedup_state_for_persist_inner(
    d: &crate::ast::Object,
    scope: Option<&hashbrown::HashSet<String>>,
) -> crate::ast::Object {
    use crate::ast;
    let ft_ids: hashbrown::HashSet<String> =
        ast::fetch_cell_seq("FactType", d).as_seq()
            .map(|facts| facts.iter()
                .filter_map(|f| ast::binding(f, "id").map(|s| s.to_string()))
                .collect())
            .unwrap_or_default();
    let ft_arity: hashbrown::HashMap<String, usize> =
        ast::fetch_cell_seq("FactType", d).as_seq()
            .map(|facts| facts.iter()
                .filter_map(|f| Some((
                    ast::binding(f, "id")?.to_string(),
                    ast::binding(f, "arity")?.parse::<usize>().ok()?)))
            .collect())
            .unwrap_or_default();
    // engine-casing-skew-cell-name-regression safeguard: surface any
    // case-only cell-name collisions BEFORE persist so a re-introduced
    // regression (or a stale relic carried in via cor:closure) is
    // visible at the boundary it would otherwise corrupt. Warns rather
    // than panics: an existing app DB carrying a relic from a pre-fix
    // compile must still re-persist successfully so a subsequent
    // recompile (now seeded by `cli/entry.rs::noun_seed` / `rebuild.rs`
    // / `compile::bundled_domain_fact_type_ids`) can scrub it. The
    // mitigation guarantees fresh compiles emit only the canonical
    // name; this detector pins that contract end-to-end and turns any
    // future skew (e.g., a new role-noun-shadowing reading shape that
    // the seed misses) into a build-time eprint instead of silent SQL
    // materialization breakage. FT-cell-scoped because they are the
    // only cells whose names come from `fact_type_id_from_reading`'s
    // role-noun walk; `:` meta/view cells and schema cells (`Noun`,
    // `Role`, …) are not at risk.
    let collisions = detect_case_only_ft_cell_collisions(d, &ft_ids);
    for (lower, names) in &collisions {
        eprintln!(
            "[persist] WARNING: case-only FT cell-name collision under '{}': {:?} \
             — likely engine-casing-skew-cell-name-regression relic; recompile \
             with seeded noun catalog should canonicalize.",
            lower, names);
    }
    let map: hashbrown::HashMap<String, ast::Object> =
        ast::cells_iter(d).into_iter()
            .map(|(name, contents)| if scope.map_or(false, |s| !s.contains(name)) {
                // scoped pass-through (987-A.3): untouched cells keep
                // their exact prior encoding so the delta diff stays
                // delta-sized.
                (name.to_string(), contents.clone())
            } else if ft_ids.contains(name) {
                (name.to_string(), ast::dedup_cell_facts(
                    &ast::drop_subjectless_facts_with_arity(contents, ft_arity.get(name).copied())))
            } else if !name.contains(':') {
                (name.to_string(), ast::dedup_cell_facts(&ast::drop_empty_subject_facts(contents)))
            } else {
                (name.to_string(), contents.clone())
            })
            .collect();
    ast::Object::map(map)
}

/// engine-casing-skew-cell-name-regression detector. Returns groups of
/// FT cell names that share a lowercase form but differ in casing —
/// the live shape of a `fact_type_id_from_reading` skew (e.g.
/// `Verb_is_performed_during_transition` colliding with the canonical
/// `Verb_is_performed_during_Transition`). Empty when no collisions.
///
/// Scoped to FT cells (names present in the supplied `ft_ids`): only
/// they derive from the readings walk that can produce skew. Schema
/// cells (`Noun`, `Role`, `FactType`, …) and `:` meta cells are
/// excluded — they are not minted by `fact_type_id_from_reading` and
/// would yield false positives if someone declared an FT whose id
/// happens to lowercase-collide with a schema cell.
///
/// Returned as `(lowercase_key, sorted_distinct_names)` pairs, sorted
/// by key for deterministic diagnostic output.
#[cfg(all(not(feature = "no_std"), feature = "local"))]
pub(crate) fn detect_case_only_ft_cell_collisions(
    d: &crate::ast::Object,
    ft_ids: &hashbrown::HashSet<String>,
) -> Vec<(String, Vec<String>)> {
    let mut by_lower: hashbrown::HashMap<String, hashbrown::HashSet<String>> =
        hashbrown::HashMap::new();
    for (name, _) in crate::ast::cells_iter(d) {
        if !ft_ids.contains(name) { continue; }
        by_lower.entry(name.to_lowercase()).or_default().insert(name.to_string());
    }
    let mut out: Vec<(String, Vec<String>)> = by_lower.into_iter()
        .filter(|(_, names)| names.len() > 1)
        .map(|(k, names)| {
            let mut v: Vec<String> = names.into_iter().collect();
            v.sort();
            (k, v)
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

#[cfg(all(not(feature = "no_std"), feature = "local"))]
#[cfg(test)]
mod tests {
    use crate::ast;

    /// engine-casing-skew-cell-name-regression pin: when two FT cells
    /// share a lowercase form but differ in casing (the exact shape of
    /// the original regression — `Verb_is_performed_during_transition`
    /// vs canonical `…_Transition`), the detector surfaces them so the
    /// persist boundary can warn before they hit SQLite. This pins the
    /// safeguard the noun_seed mitigation (`cli/entry.rs:757-765`)
    /// relies on as a defense-in-depth: even if a future reading shape
    /// or seed gap re-introduces the skew, the build emits a visible
    /// warning instead of silently corrupting SQL materialization.
    #[test]
    fn detect_case_only_ft_cell_collisions_finds_canonical_vs_skewed() {
        let ft_canonical = ast::fact_from_pairs(&[
            ("id", "Verb_is_performed_during_Transition"),
            ("arity", "2"),
        ]);
        let ft_skewed = ast::fact_from_pairs(&[
            ("id", "Verb_is_performed_during_transition"),
            ("arity", "2"),
        ]);
        let state = {
            let s = ast::Object::phi();
            let s = ast::store("FactType",
                ast::Object::seq(vec![ft_canonical, ft_skewed]), &s);
            let s = ast::store("Verb_is_performed_during_Transition",
                ast::Object::seq(vec![]), &s);
            ast::store("Verb_is_performed_during_transition",
                ast::Object::seq(vec![]), &s)
        };
        let ft_ids: hashbrown::HashSet<String> = [
            "Verb_is_performed_during_Transition".to_string(),
            "Verb_is_performed_during_transition".to_string(),
        ].into_iter().collect();
        let collisions = super::detect_case_only_ft_cell_collisions(&state, &ft_ids);
        assert_eq!(collisions.len(), 1,
            "expected exactly one case-only collision group, got {:?}", collisions);
        let (lower, names) = &collisions[0];
        assert_eq!(lower, "verb_is_performed_during_transition");
        assert_eq!(names, &vec![
            "Verb_is_performed_during_Transition".to_string(),
            "Verb_is_performed_during_transition".to_string(),
        ]);
    }

    /// Negative pin: a clean state (only the canonical-cased FT cell)
    /// must yield NO collisions. Guards against the detector firing on
    /// the every-day post-noun_seed compile output and spamming
    /// `[persist] WARNING` lines on healthy builds.
    #[test]
    fn detect_case_only_ft_cell_collisions_silent_on_canonical_state() {
        let ft = ast::fact_from_pairs(&[
            ("id", "Verb_is_performed_during_Transition"),
            ("arity", "2"),
        ]);
        let state = {
            let s = ast::Object::phi();
            let s = ast::store("FactType", ast::Object::seq(vec![ft]), &s);
            ast::store("Verb_is_performed_during_Transition",
                ast::Object::seq(vec![]), &s)
        };
        let ft_ids: hashbrown::HashSet<String> =
            ["Verb_is_performed_during_Transition".to_string()].into_iter().collect();
        let collisions = super::detect_case_only_ft_cell_collisions(&state, &ft_ids);
        assert!(collisions.is_empty(),
            "clean state must yield no collisions, got {:?}", collisions);
    }

    /// Schema cells (`Noun`, `Role`, `FactType`, `:`-prefixed view
    /// cells, etc.) are NOT FT cells and must be excluded from the
    /// collision scan — they are not minted by
    /// `fact_type_id_from_reading`'s role-noun walk, so a hypothetical
    /// lowercase clash between them and an FT id (or among themselves)
    /// is not a casing-skew relic. Scoping prevents false positives.
    #[test]
    fn detect_case_only_ft_cell_collisions_ignores_non_ft_cells() {
        // A canonical FT cell + a non-FT cell whose name happens to
        // lowercase-collide with the FT id. ft_ids contains only the
        // FT id, so the non-FT cell is filtered out and no collision
        // is reported.
        let ft = ast::fact_from_pairs(&[
            ("id", "Verb_is_performed_during_Transition"),
            ("arity", "2"),
        ]);
        let state = {
            let s = ast::Object::phi();
            let s = ast::store("FactType", ast::Object::seq(vec![ft]), &s);
            let s = ast::store("Verb_is_performed_during_Transition",
                ast::Object::seq(vec![]), &s);
            // Non-FT cell with a colliding lowercase form.
            ast::store("verb_is_performed_during_transition",
                ast::Object::seq(vec![]), &s)
        };
        let ft_ids: hashbrown::HashSet<String> =
            ["Verb_is_performed_during_Transition".to_string()].into_iter().collect();
        let collisions = super::detect_case_only_ft_cell_collisions(&state, &ft_ids);
        assert!(collisions.is_empty(),
            "non-FT cells must be ignored by the FT-scoped detector, got {:?}",
            collisions);
    }
}

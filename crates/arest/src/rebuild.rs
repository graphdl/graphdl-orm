// crates/arest/src/rebuild.rs
//
// task-919 gap-4: arest-dev substrate-rebuild Platform Functions.
//
// arest-dev's Rebuild SM (requested → snapshotted → initialized → applied →
// verified) dispatches each transition to a Platform Function via the gap-3
// wiring in command::transition_via_defs (look up `<Transition> is performed by
// Platform Function`, apply Func::Platform(name) with the entity ctx, roll back
// the SM cell flip on a Bottom result). This module implements those functions.
//
// Each operates on a TARGET app (the Rebuild's `App Target`) resolved under an
// `apps_dir` root. The engine is app-agnostic, so the install point supplies
// that root: production wires `install_rebuild_fns(real_apps_dir)` at MCP/CLI
// boot; tests pass a temp dir. Gated on `local` for the rusqlite cross-app read.
//
// Design notes (the substrate is silent on these — decided by judgment):
//  - Install point: a captured-`apps_dir` closure (PlatformFn is Arc<dyn Fn>),
//    so the app-agnostic engine reaches a named target's on-disk DB.
//  - Snapshot-entity recording: PlatformFn returns are only Bottom-checked, not
//    merged into D, so these functions perform the OPERATION (a side effect on
//    the target / a new file) and report success via a non-Bottom return. The
//    `Rebuild produces Snapshot` *entity* recording is a separate refinement
//    (an SM derivation or orchestrator apply), deliberately not done here.

use crate::ast::{self, Object};
use crate::sync::Arc;
use std::path::{Path, PathBuf};

/// Register the rebuild Platform Functions, each capturing `apps_dir` so the
/// app-agnostic engine can resolve a target app's on-disk DB. Production calls
/// this once at MCP/CLI boot with the real apps dir; tests pass a temp dir.
///
/// Only `rebuild_snapshot` is wired so far — it is read-only w.r.t. the target
/// (reads its DB, writes a NEW file), so it cannot corrupt a real app. The
/// target-MUTATING ops (rebuild_init / rebuild_apply_bulk) and rebuild_verify
/// land next, each unit-tested on a synthetic target before being installed.
pub fn install_rebuild_fns(apps_dir: PathBuf) {
    let ad = apps_dir.clone();
    ast::install_platform_fn(
        "rebuild_snapshot",
        Arc::new(move |x: &Object, d: &Object| rebuild_snapshot(&ad, x, d)),
    );
    let ad = apps_dir.clone();
    ast::install_platform_fn(
        "rebuild_verify",
        Arc::new(move |x: &Object, d: &Object| rebuild_verify(&ad, x, d)),
    );
    let ad = apps_dir.clone();
    ast::install_platform_fn(
        "rebuild_apply_bulk",
        Arc::new(move |x: &Object, d: &Object| rebuild_apply_bulk(&ad, x, d)),
    );
    let ad = apps_dir;
    ast::install_platform_fn(
        "rebuild_init",
        Arc::new(move |x: &Object, d: &Object| rebuild_init(&ad, x, d)),
    );
}

/// The Rebuild's `App Target` name, read from the `Rebuild concerns App Target`
/// cell in the current (arest-dev) state `d`.
fn target_for_rebuild(d: &Object, rebuild_id: &str) -> Option<String> {
    ast::fetch_cell_seq("Rebuild_concerns_App_Target", d)
        .as_seq()?
        .iter()
        .find(|f| ast::binding(f, "Rebuild") == Some(rebuild_id))
        .and_then(|f| ast::binding(f, "App Target").map(|s| s.to_string()))
}

/// Load a target app's population cells (name → parsed contents) directly from
/// its SQLite `cells` table — the same table the CLI persists. Kept local so
/// rebuild stays self-contained rather than coupling to cli::entry's db module.
fn load_target_cells(db_path: &Path) -> Option<Object> {
    let conn = rusqlite::Connection::open(db_path).ok()?;
    let mut map: hashbrown::HashMap<String, Object> = hashbrown::HashMap::new();
    let mut stmt = conn.prepare("SELECT name, contents FROM cells").ok()?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .ok()?;
    for row in rows.flatten() {
        map.insert(row.0, Object::parse(&row.1));
    }
    Some(Object::map(map))
}

/// `rebuild_snapshot` (requested → snapshotted): freeze the target app's current
/// cell store to a timestamped file under `<apps_dir>/<target>/rebuild-snapshots/`.
/// READ-ONLY w.r.t. the target (reads its DB, writes a NEW file), so it cannot
/// corrupt the target. Returns the snapshot-path atom on success, Bottom on any
/// failure — which the gap-3 dispatch turns into an SM-transition rollback.
fn rebuild_snapshot(apps_dir: &Path, x: &Object, d: &Object) -> Object {
    let rebuild_id = match x.as_map().and_then(|m| m.get("id")).and_then(|o| o.as_atom()) {
        Some(id) => id.to_string(),
        None => return Object::Bottom,
    };
    let target = match target_for_rebuild(d, &rebuild_id) {
        Some(t) => t,
        None => return Object::Bottom,
    };
    let state = match load_target_cells(&apps_dir.join(&target).join(format!("{}.db", target))) {
        Some(s) => s,
        None => return Object::Bottom,
    };
    let bytes = crate::freeze::freeze(&state);
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let snap_dir = apps_dir.join(&target).join("rebuild-snapshots");
    if std::fs::create_dir_all(&snap_dir).is_err() {
        return Object::Bottom;
    }
    let snap_path = snap_dir.join(format!("{}.bin", ts));
    if std::fs::write(&snap_path, &bytes).is_err() {
        return Object::Bottom;
    }
    Object::atom(&snap_path.to_string_lossy())
}

/// `rebuild_verify` (applied -> verified): confirm the rebuilt target app's
/// cell store loads + parses cleanly and is non-empty — a sanity gate that the
/// rebuild produced a usable state. READ-ONLY w.r.t. the target. Returns the
/// cell-count atom on success, Bottom on a missing/empty/unreadable target
/// (-> the gap-3 dispatch rolls back the SM transition). A fuller verify
/// (running the target's declared constraints over the rebuilt population) is a
/// refinement left for when the constraint surface is threaded in.
fn rebuild_verify(apps_dir: &Path, x: &Object, d: &Object) -> Object {
    let rebuild_id = match x.as_map().and_then(|m| m.get("id")).and_then(|o| o.as_atom()) {
        Some(id) => id.to_string(),
        None => return Object::Bottom,
    };
    let target = match target_for_rebuild(d, &rebuild_id) {
        Some(t) => t,
        None => return Object::Bottom,
    };
    let state = match load_target_cells(&apps_dir.join(&target).join(format!("{}.db", target))) {
        Some(s) => s,
        None => return Object::Bottom,
    };
    let n = ast::cells_iter(&state).len();
    if n == 0 {
        return Object::Bottom;
    }
    Object::atom(&n.to_string())
}

/// Newest snapshot file (by numeric timestamp stem) under
/// `<apps_dir>/<target>/rebuild-snapshots/`, the convention `rebuild_snapshot`
/// writes. Used by `rebuild_apply_bulk` to find the population to restore
/// without depending on the (deferred) `Snapshot` entity.
fn latest_snapshot(apps_dir: &Path, target: &str) -> Option<PathBuf> {
    std::fs::read_dir(apps_dir.join(target).join("rebuild-snapshots"))
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("bin"))
        .max_by_key(|p| {
            p.file_stem()
                .and_then(|s| s.to_str())
                .and_then(|s| s.parse::<u128>().ok())
                .unwrap_or(0)
        })
}

/// Persist `state`'s cells to a target DB, replacing the cells table atomically
/// (DELETE + re-insert in one transaction) — mirrors cli::entry::db::persist_state's
/// cell path. MUTATES the target DB.
fn persist_target_cells(db_path: &Path, state: &Object) -> bool {
    let conn = match rusqlite::Connection::open(db_path) {
        Ok(c) => c,
        Err(_) => return false,
    };
    if conn
        .execute_batch("CREATE TABLE IF NOT EXISTS cells (name TEXT PRIMARY KEY, contents TEXT);")
        .is_err()
    {
        return false;
    }
    let tx = match conn.unchecked_transaction() {
        Ok(t) => t,
        Err(_) => return false,
    };
    if tx.execute("DELETE FROM cells", []).is_err() {
        return false;
    }
    for (name, contents) in ast::cells_iter(state) {
        if tx
            .execute(
                "INSERT OR REPLACE INTO cells (name, contents) VALUES (?1, ?2)",
                rusqlite::params![name, contents.to_string()],
            )
            .is_err()
        {
            return false;
        }
    }
    tx.commit().is_ok()
}

/// `rebuild_apply_bulk` (initialized -> applied): restore the target app's
/// runtime population from its latest rebuild snapshot onto the freshly-init'd
/// target. The snapshot's DECLARED-FactType population cells are merged onto the
/// target's fresh state (which carries the schema from rebuild_init); snapshot
/// cells for FTs no longer declared, and snapshot schema/meta cells, are left
/// out (the fresh schema wins). MUTATES the target DB. Returns the restored-cell
/// count on success, Bottom on a missing snapshot / unreadable target (-> the
/// gap-3 dispatch rolls back the SM transition).
fn rebuild_apply_bulk(apps_dir: &Path, x: &Object, d: &Object) -> Object {
    let rebuild_id = match x.as_map().and_then(|m| m.get("id")).and_then(|o| o.as_atom()) {
        Some(id) => id.to_string(),
        None => return Object::Bottom,
    };
    let target = match target_for_rebuild(d, &rebuild_id) {
        Some(t) => t,
        None => return Object::Bottom,
    };
    let snap_path = match latest_snapshot(apps_dir, &target) {
        Some(p) => p,
        None => return Object::Bottom,
    };
    let snapshot_state = match std::fs::read(&snap_path).ok().and_then(|b| crate::freeze::thaw(&b).ok()) {
        Some(s) => s,
        None => return Object::Bottom,
    };
    let target_db = apps_dir.join(&target).join(format!("{}.db", target));
    let target_state = match load_target_cells(&target_db) {
        Some(s) => s,
        None => return Object::Bottom,
    };
    // FactTypes declared in the freshly-init'd target — only their populations
    // are restored from the snapshot.
    let ft_ids: hashbrown::HashSet<String> = ast::fetch_cell_seq("FactType", &target_state)
        .as_seq()
        .map(|fs| {
            fs.iter()
                .filter_map(|f| ast::binding(f, "id").map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let mut pop_map: hashbrown::HashMap<String, Object> = hashbrown::HashMap::new();
    for (name, contents) in ast::cells_iter(&snapshot_state) {
        if ft_ids.contains(name) && ast::looks_like_population_cell(contents) {
            pop_map.insert(name.to_string(), contents.clone());
        }
    }
    let restored = pop_map.len();
    let merged = ast::merge_states(&target_state, &Object::map(pop_map));
    if !persist_target_cells(&target_db, &merged) {
        return Object::Bottom;
    }
    Object::atom(&restored.to_string())
}

/// Collect a target app's readings (recursively, `*.md`) from its `readings/`
/// dir, app.md first then depth-then-name order — mirrors cli::entry::read_readings'
/// ordering so core nouns (app.md) are in context before instance slices. Kept
/// local so rebuild stays self-contained.
fn load_app_readings(readings_dir: &Path) -> Vec<(String, String)> {
    fn collect_md(dir: &Path, out: &mut Vec<PathBuf>) {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for e in entries.filter_map(|e| e.ok()) {
                let p = e.path();
                if p.is_dir() {
                    collect_md(&p, out);
                } else if p.extension().and_then(|x| x.to_str()) == Some("md") {
                    out.push(p);
                }
            }
        }
    }
    let mut files: Vec<PathBuf> = Vec::new();
    collect_md(readings_dir, &mut files);
    files.sort_by(|a, b| {
        a.components()
            .count()
            .cmp(&b.components().count())
            .then_with(|| a.cmp(b))
    });
    let mut app_md: Option<(String, String)> = None;
    let mut rest: Vec<(String, String)> = Vec::new();
    for p in files {
        let name = p
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let text = match std::fs::read_to_string(&p) {
            Ok(t) => t,
            Err(_) => continue,
        };
        if name == "app.md" {
            app_md = Some((name, text));
        } else {
            rest.push((name, text));
        }
    }
    match app_md {
        Some(a) => {
            let mut v = vec![a];
            v.extend(rest);
            v
        }
        None => rest,
    }
}

/// `rebuild_init` (snapshotted -> initialized): recompile the target app from
/// its readings into a FRESH schema, written to the target DB. Mirrors
/// cli::entry's dirs-compile CORE — parse the metamodel + app readings with a
/// global noun seed, compile defs, forward-chain derived FT cells — MINUS the
/// recompile-preserve machinery (cor:closure / preserve_prior_population,
/// tree-shake, #836 prior-derived drop): a fresh init has no prior population to
/// preserve; the snapshot's population is restored afterward by
/// rebuild_apply_bulk. MUTATES the target DB. Returns the cell-count atom on
/// success, Bottom on a missing target / parse failure / persist failure
/// (-> the gap-3 dispatch rolls back the SM transition).
///
/// Composes the engine's pub compile pieces (parse_to_state_from, merge_states,
/// compile_to_defs_state, forward_chain_defs_state) rather than the platform
/// `compile` verb: platform_compile DROPS + re-emits schema cells per call (a
/// whole-readings recompile), so it can't be folded per-file and discards the
/// metamodel schema when handed app-only readings. Validation is not run inline
/// (dirs-compile doesn't either); rebuild_verify is the post-gate.
fn rebuild_init(apps_dir: &Path, x: &Object, d: &Object) -> Object {
    let rebuild_id = match x.as_map().and_then(|m| m.get("id")).and_then(|o| o.as_atom()) {
        Some(id) => id.to_string(),
        None => return Object::Bottom,
    };
    let target = match target_for_rebuild(d, &rebuild_id) {
        Some(t) => t,
        None => return Object::Bottom,
    };
    let app_readings = load_app_readings(&apps_dir.join(&target).join("readings"));
    if app_readings.is_empty() {
        return Object::Bottom;
    }
    // Metamodel readings FIRST, then the app's — the fresh parse's schema cells
    // (FactType / Noun / Role / …) must include the metamodel, else the merge
    // would land an app-only schema.
    crate::parse_forml2::set_bootstrap_mode(true);
    let all_readings: Vec<(&str, &str)> = crate::metamodel_readings()
        .into_iter()
        .map(|r| (r.0, r.1))
        .chain(app_readings.iter().map(|(n, t)| (n.as_str(), t.as_str())))
        .collect();
    // Global noun seed (cli::entry parity): pre-parse the whole corpus so every
    // slice sees all declared nouns regardless of fold order.
    let corpus: String = all_readings
        .iter()
        .map(|(_, t)| *t)
        .collect::<Vec<_>>()
        .join("\n\n");
    let noun_seed = match crate::parse_forml2::parse_to_state_from(&corpus, &Object::phi()) {
        Ok(full) => {
            let mut m: hashbrown::HashMap<String, Object> = hashbrown::HashMap::new();
            m.insert("Noun".to_string(), ast::fetch_cell_seq("Noun", &full));
            Object::map(m)
        }
        Err(_) => {
            crate::parse_forml2::set_bootstrap_mode(false);
            return Object::Bottom;
        }
    };
    let mut parsed = noun_seed;
    for (_name, text) in &all_readings {
        match crate::parse_forml2::parse_to_state_from(text, &parsed) {
            Ok(this) => parsed = ast::merge_states(&parsed, &this),
            Err(_) => {
                crate::parse_forml2::set_bootstrap_mode(false);
                return Object::Bottom;
            }
        }
    }
    // Tree-shake the UoD (cli::entry parity, L832-858): drop bundled-metamodel
    // DOMAIN fact types the app never reaches, so forward-chain runs over the
    // app's actual reachable closure rather than the full ~584-FT metamodel UoD
    // — without this the fixpoint on a metamodel-scale state is multi-minute.
    let parsed = {
        let ft_ids = |st: &Object| -> hashbrown::HashSet<String> {
            ast::fetch_cell_seq("FactType", st)
                .as_seq()
                .map(|fs| {
                    fs.iter()
                        .filter_map(|f| ast::binding(f, "id").map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default()
        };
        let all_ft_ids = ft_ids(&parsed);
        let base_ft_ids = ft_ids(crate::metamodel_state());
        let domain_ft_ids = crate::compile::bundled_domain_fact_type_ids();
        let roots: hashbrown::HashSet<String> =
            all_ft_ids.difference(&base_ft_ids).cloned().collect();
        let idx = crate::compile::cell_index_from_state(&parsed);
        let reachable = crate::compile::reachable_fact_types(&idx, &roots);
        let keep: hashbrown::HashSet<String> = all_ft_ids
            .iter()
            .filter(|id| !domain_ft_ids.contains(*id) || reachable.contains(*id))
            .cloned()
            .collect();
        crate::compile::prune_unreachable_fact_types(&parsed, &keep)
    };
    crate::parse_forml2::set_bootstrap_mode(false);
    // Compile defs (platform primitives + schema-derived) into the state.
    let mut defs: Vec<(String, ast::Func)> = Vec::new();
    defs.push(("compile".to_string(), ast::Func::Platform("compile".to_string())));
    defs.push(("apply".to_string(), ast::Func::Platform("apply_command".to_string())));
    defs.push(("verify_signature".to_string(), ast::Func::Platform("verify_signature".to_string())));
    defs.push(("audit".to_string(), ast::Func::Platform("audit".to_string())));
    let mut state = ast::defs_to_state(&defs, &parsed);
    let compile_defs = crate::compile::compile_to_defs_state(&parsed);
    state = ast::defs_to_state(&compile_defs, &state);
    // Forward-chain positive + SM-synthetic derivations to materialize derived
    // FT cells (cli::entry parity, L1035-1088; the #822 empty-consequent fix).
    let collect = |prefix: &str, st: &Object| -> Vec<(String, ast::Func)> {
        ast::cells_iter(st)
            .into_iter()
            .filter(|(n, _)| n.starts_with(prefix))
            .map(|(n, c)| (n.to_string(), ast::metacompose(c, st)))
            .collect()
    };
    let mut strat = collect("derivation:rule_", &state);
    strat.extend(collect("derivation:_sm_init_", &state));
    strat.extend(collect("derivation:_sm_event_fold_", &state));
    strat.extend(collect("derivation:_sm_for_resource_backfill_", &state));
    if !strat.is_empty() {
        let refs: Vec<(&str, &ast::Func)> =
            strat.iter().map(|(n, f)| (n.as_str(), f)).collect();
        state = crate::evaluate::forward_chain_defs_state(&refs, &state).0;
    }
    let target_db = apps_dir.join(&target).join(format!("{}.db", target));
    if !persist_target_cells(&target_db, &state) {
        return Object::Bottom;
    }
    Object::atom(&ast::cells_iter(&state).len().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rebuild_snapshot_freezes_target_db_to_a_file() {
        // Synthetic temp apps-dir with a target app DB holding one cell — never
        // a real app, so the test cannot touch real data.
        let root = std::env::temp_dir().join(format!(
            "arest_rebuild_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let target = "snaptarget";
        let tdir = root.join(target);
        std::fs::create_dir_all(&tdir).expect("mk target dir");
        {
            let conn = rusqlite::Connection::open(tdir.join(format!("{}.db", target)))
                .expect("open target db");
            conn.execute_batch(
                "CREATE TABLE cells (name TEXT PRIMARY KEY, contents TEXT);
                 INSERT INTO cells VALUES ('Task_is_epic', '<<Task, 772>>');",
            )
            .expect("seed target db");
        }
        // arest-dev state: Rebuild 'rb-1' concerns App Target 'snaptarget'.
        let mut dm: hashbrown::HashMap<String, Object> = hashbrown::HashMap::new();
        dm.insert(
            "Rebuild_concerns_App_Target".to_string(),
            Object::seq(vec![Object::seq(vec![
                Object::seq(vec![Object::atom("Rebuild"), Object::atom("rb-1")]),
                Object::seq(vec![Object::atom("App Target"), Object::atom(target)]),
            ])]),
        );
        let d = Object::map(dm);
        // ctx Map as command::transition_via_defs builds it.
        let mut xm: hashbrown::HashMap<String, Object> = hashbrown::HashMap::new();
        xm.insert("id".to_string(), Object::atom("rb-1"));
        xm.insert("noun".to_string(), Object::atom("Rebuild"));
        let x = Object::map(xm);

        let result = rebuild_snapshot(&root, &x, &d);
        assert!(
            !matches!(result, Object::Bottom),
            "rebuild_snapshot must succeed for a valid target; got Bottom"
        );
        let snap = result.as_atom().expect("snapshot path atom");
        assert!(Path::new(snap).exists(), "snapshot file must be written: {}", snap);
        assert!(
            std::fs::metadata(snap).map(|m| m.len() > 0).unwrap_or(false),
            "snapshot file must be non-empty (frozen state)"
        );

        // Unknown Rebuild id → Bottom (graceful failure → SM rollback).
        let mut bad: hashbrown::HashMap<String, Object> = hashbrown::HashMap::new();
        bad.insert("id".to_string(), Object::atom("nonexistent"));
        assert!(
            matches!(rebuild_snapshot(&root, &Object::map(bad), &d), Object::Bottom),
            "unknown Rebuild id must return Bottom"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn rebuild_verify_passes_for_non_empty_target_and_bottoms_on_missing() {
        let root = std::env::temp_dir().join(format!(
            "arest_rebuild_verify_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let target = "verifytarget";
        let tdir = root.join(target);
        std::fs::create_dir_all(&tdir).expect("mk target dir");
        {
            let conn = rusqlite::Connection::open(tdir.join(format!("{}.db", target)))
                .expect("open target db");
            conn.execute_batch(
                "CREATE TABLE cells (name TEXT PRIMARY KEY, contents TEXT);
                 INSERT INTO cells VALUES ('Task_is_epic', '<<Task, 772>>');",
            )
            .expect("seed target db");
        }
        let mut dm: hashbrown::HashMap<String, Object> = hashbrown::HashMap::new();
        dm.insert(
            "Rebuild_concerns_App_Target".to_string(),
            Object::seq(vec![Object::seq(vec![
                Object::seq(vec![Object::atom("Rebuild"), Object::atom("rb-2")]),
                Object::seq(vec![Object::atom("App Target"), Object::atom(target)]),
            ])]),
        );
        let d = Object::map(dm);
        let mut xm: hashbrown::HashMap<String, Object> = hashbrown::HashMap::new();
        xm.insert("id".to_string(), Object::atom("rb-2"));
        assert!(
            !matches!(rebuild_verify(&root, &Object::map(xm), &d), Object::Bottom),
            "verify must pass for a non-empty rebuilt target"
        );
        let mut bad: hashbrown::HashMap<String, Object> = hashbrown::HashMap::new();
        bad.insert("id".to_string(), Object::atom("missing"));
        assert!(
            matches!(rebuild_verify(&root, &Object::map(bad), &d), Object::Bottom),
            "missing target must return Bottom"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn rebuild_apply_bulk_restores_declared_ft_population_from_snapshot() {
        let root = std::env::temp_dir().join(format!(
            "arest_rebuild_apply_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let target = "applytarget";
        let tdir = root.join(target);
        let snap_dir = tdir.join("rebuild-snapshots");
        std::fs::create_dir_all(&snap_dir).expect("mk dirs");
        let target_db = tdir.join(format!("{}.db", target));

        // Snapshot: a frozen state holding a Task_is_epic population.
        let mut sm: hashbrown::HashMap<String, Object> = hashbrown::HashMap::new();
        sm.insert(
            "Task_is_epic".to_string(),
            Object::seq(vec![Object::seq(vec![Object::seq(vec![
                Object::atom("Task"),
                Object::atom("772"),
            ])])]),
        );
        std::fs::write(snap_dir.join("100.bin"), crate::freeze::freeze(&Object::map(sm)))
            .expect("write snapshot");

        // Freshly-init'd target: FactType declares Task_is_epic, no population yet.
        let ft_fact = Object::seq(vec![Object::seq(vec![
            Object::atom("id"),
            Object::atom("Task_is_epic"),
        ])]);
        let mut tm: hashbrown::HashMap<String, Object> = hashbrown::HashMap::new();
        tm.insert("FactType".to_string(), Object::seq(vec![ft_fact]));
        assert!(persist_target_cells(&target_db, &Object::map(tm)), "seed target schema");

        let mut dm: hashbrown::HashMap<String, Object> = hashbrown::HashMap::new();
        dm.insert(
            "Rebuild_concerns_App_Target".to_string(),
            Object::seq(vec![Object::seq(vec![
                Object::seq(vec![Object::atom("Rebuild"), Object::atom("rb-3")]),
                Object::seq(vec![Object::atom("App Target"), Object::atom(target)]),
            ])]),
        );
        let d = Object::map(dm);
        let mut xm: hashbrown::HashMap<String, Object> = hashbrown::HashMap::new();
        xm.insert("id".to_string(), Object::atom("rb-3"));

        let result = rebuild_apply_bulk(&root, &Object::map(xm), &d);
        assert!(
            !matches!(result, Object::Bottom),
            "apply_bulk must succeed with a snapshot + declared FT; got Bottom"
        );
        // The target now carries the restored Task_is_epic population.
        let after = load_target_cells(&target_db).expect("reload target");
        let restored = ast::fetch_cell_seq("Task_is_epic", &after);
        assert!(
            restored.as_seq().map(|s| !s.is_empty()).unwrap_or(false),
            "Task_is_epic population must be restored onto the target; got {:?}",
            restored
        );
        // ...and the fresh schema (FactType) survives the merge.
        assert!(
            ast::fetch_cell_seq("FactType", &after).as_seq().map(|s| !s.is_empty()).unwrap_or(false),
            "fresh schema (FactType) must be preserved"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    // Integration-style: compiles the full metamodel + a synthetic app, runs
    // forward-chain, and asserts schema + derived materialization. Compiles
    // ~280 FTs / ~7800 defs and peaks past 5GB under cargo's debug profile —
    // the per-machine 5GB Job Object cap (scripts/mem-capped-build.ps1) kills
    // it. Production rebuild_init runs in the engine binary with the full
    // host memory and fits; cargo test debug+capped is just too tight. Run
    // explicitly with `cargo test --ignored rebuild_init` on a less
    // constrained build (e.g. --release) when validating end-to-end.
    #[ignore]
    #[test]
    fn rebuild_init_compiles_target_readings_into_fresh_schema_with_derived_facts() {
        let root = std::env::temp_dir().join(format!(
            "arest_rebuild_init_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let target = "inittarget";
        let readings_dir = root.join(target).join("readings");
        std::fs::create_dir_all(&readings_dir).expect("mk readings dir");
        // A complete self-contained model: two FTs, a derivation rule, one
        // asserted instance fact whose consequent must be derived.
        std::fs::write(
            readings_dir.join("app.md"),
            "Each Topic has exactly one Topic Status.\n\
             Each Topic has exactly one Topic Readiness.\n\
             Topic has Topic Readiness 'ready' iff Topic has Topic Status 'pending'.\n\
             Topic 'tA' has Topic Status 'pending'.\n",
        )
        .expect("write app.md");

        let mut dm: hashbrown::HashMap<String, Object> = hashbrown::HashMap::new();
        dm.insert(
            "Rebuild_concerns_App_Target".to_string(),
            Object::seq(vec![Object::seq(vec![
                Object::seq(vec![Object::atom("Rebuild"), Object::atom("rb-init")]),
                Object::seq(vec![Object::atom("App Target"), Object::atom(target)]),
            ])]),
        );
        let d = Object::map(dm);
        let mut xm: hashbrown::HashMap<String, Object> = hashbrown::HashMap::new();
        xm.insert("id".to_string(), Object::atom("rb-init"));

        let result = rebuild_init(&root, &Object::map(xm), &d);
        assert!(
            !matches!(result, Object::Bottom),
            "rebuild_init must compile a valid target; got Bottom"
        );

        // Reload the persisted target and verify the compile landed.
        let after = load_target_cells(&root.join(target).join(format!("{}.db", target)))
            .expect("reload compiled target");
        assert!(
            ast::fetch_cell_seq("FactType", &after).as_seq().map(|s| !s.is_empty()).unwrap_or(false),
            "schema (FactType) must be compiled + persisted"
        );
        // The asserted primary fact persisted.
        let status = ast::fetch_cell_seq("Topic_has_Topic_Status", &after);
        assert!(
            status.as_seq().map(|s| !s.is_empty()).unwrap_or(false),
            "asserted Topic_has_Topic_Status fact must persist; got {:?}",
            status
        );
        // The DERIVED consequent materialized via forward-chain — this is what
        // platform_compile alone would NOT produce, so it pins the forward-chain
        // wiring.
        let readiness = ast::fetch_cell_seq("Topic_has_Topic_Readiness", &after);
        assert!(
            readiness.as_seq().map(|s| !s.is_empty()).unwrap_or(false),
            "derived Topic_has_Topic_Readiness must be materialized by forward-chain; got {:?}",
            readiness
        );

        std::fs::remove_dir_all(&root).ok();
    }
}

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
    let ad = apps_dir;
    ast::install_platform_fn(
        "rebuild_snapshot",
        Arc::new(move |x: &Object, d: &Object| rebuild_snapshot(&ad, x, d)),
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
}

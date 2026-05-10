// crates/arest/tests/sql_against_tasks_db.rs
//
// End-to-end pin (#864 acceptance) of the read-only `sql` verb against
// the apps/tasks DB. The test loads the real SQLite cell store, runs
// the parallel-paths query verbatim from the issue, and asserts:
//
//   1. Single-FT regression: SELECT … FROM ft_Task_has_Task_Priority
//      WHERE "Task_Priority" = 'p0' returns ≥ 60 rows. (Spec: "we have
//      60+ p0 tasks".)
//
//   2. Parallel-paths shape: SELECT DISTINCT … with NOT EXISTS over
//      ft_Task_touches_Source_File ⨝ ft_Task_has_Task_Status returns
//      a non-empty set of Task ids that does NOT intersect the
//      in_progress set.
//
// The test is gated on the local feature (rusqlite is local-only) and
// skips gracefully when the apps/tasks DB isn't on disk — CI runners
// without the apps repo cloned won't fail the build, they just don't
// exercise this acceptance.
//
// Path is hardcoded to `C:\Users\lippe\Repos\apps\tasks\tasks.db`
// (the spec's target). Override via `AREST_TASKS_DB` env var if the
// apps repo lives elsewhere.

#![cfg(feature = "local")]

use std::path::PathBuf;
use rusqlite::Connection;

use arest::ast::{self, Object};
use arest::sql::sql_query;

fn tasks_db_path() -> PathBuf {
    if let Ok(p) = std::env::var("AREST_TASKS_DB") {
        return PathBuf::from(p);
    }
    PathBuf::from(r"C:\Users\lippe\Repos\apps\tasks\tasks.db")
}

/// Load the tasks.db cell store WITHOUT recompiling defs. We only need
/// the population cells (Task_has_*, Task_touches_*, FactType, Role)
/// to materialize the per-FT SQLite tables — `sql_query` doesn't read
/// any def. Mirrors the `db::load_state` body in `cli/entry.rs::db`
/// but without the def round-trip (defs aren't needed and parsing
/// thousands of Func trees on a cold DB takes ~3s).
fn load_population_state(db_path: &std::path::Path) -> Object {
    let conn = Connection::open(db_path).expect("open tasks.db");
    let mut stmt = conn.prepare("SELECT name, contents FROM cells").expect("prepare");
    let rows: Vec<(String, String)> = stmt.query_map([], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    }).expect("query_map").filter_map(|r| r.ok()).collect();

    let mut state = Object::phi();
    for (name, contents) in rows {
        let obj = Object::parse(&contents);
        state = ast::store(&name, obj, &state);
    }
    state
}

#[test]
fn task_864_p0_count_is_at_least_sixty() {
    let path = tasks_db_path();
    if !path.exists() {
        eprintln!("[#864] skip: {} not present (set AREST_TASKS_DB to override)",
            path.display());
        return;
    }
    let state = load_population_state(&path);

    let envelope = sql_query(&state,
        r#"SELECT COUNT(*) AS n FROM ft_Task_has_Task_Priority WHERE "Task_Priority" = 'p0'"#);
    let v: serde_json::Value = serde_json::from_str(&envelope).expect("envelope is JSON");
    let rows = v.get("rows").and_then(|r| r.as_array())
        .unwrap_or_else(|| panic!("expected rows in envelope: {}", envelope));
    let n = rows.first()
        .and_then(|row| row.get("n"))
        .and_then(|v| v.as_i64())
        .unwrap_or_else(|| panic!("expected n in row: {}", envelope));
    assert!(n >= 60, "p0 task count should be ≥ 60 (spec), got {}: {}", n, envelope);
}

#[test]
fn task_864_parallel_paths_query_returns_non_empty_excluding_in_progress() {
    let path = tasks_db_path();
    if !path.exists() {
        eprintln!("[#864] skip: {} not present (set AREST_TASKS_DB to override)",
            path.display());
        return;
    }
    let state = load_population_state(&path);

    // The parallel-paths query verbatim from the issue.
    let parallel_paths = r#"
        SELECT DISTINCT r."Task" AS task
        FROM ft_Task_has_Task_Readiness r
        JOIN ft_Task_has_Task_Priority p ON p."Task" = r."Task"
        WHERE r."Task_Readiness" = 'ready'
          AND p."Task_Priority" = 'p0'
          AND NOT EXISTS (
            SELECT 1
            FROM ft_Task_touches_Source_File mine
            JOIN ft_Task_touches_Source_File theirs ON theirs."Source_File" = mine."Source_File"
            JOIN ft_Task_has_Task_Status s ON s."Task" = theirs."Task"
            WHERE mine."Task" = r."Task"
              AND s."Task_Status" = 'in_progress'
          )
    "#;
    let env = sql_query(&state, parallel_paths);
    let v: serde_json::Value = serde_json::from_str(&env).expect("envelope is JSON");
    let rows = v.get("rows").and_then(|r| r.as_array())
        .unwrap_or_else(|| panic!("expected rows in envelope: {}", env));
    let task_ids: std::collections::HashSet<String> = rows.iter()
        .filter_map(|r| r.get("task").and_then(|v| v.as_str()).map(String::from))
        .collect();
    assert!(!task_ids.is_empty(),
        "parallel-paths query should return ≥ 1 ready+p0 task: {}", env);

    // Cross-check: none of the returned tasks should be in the
    // in_progress set. That's the whole point of the NOT EXISTS guard
    // (the AND mine."Task" = r."Task" wires the parallel-source-file
    // check to the candidate row, but a task in_progress on its OWN
    // file would still appear in `theirs` via the same join — and so
    // gets correctly excluded).
    let in_progress_env = sql_query(&state,
        r#"SELECT "Task" FROM ft_Task_has_Task_Status WHERE "Task_Status" = 'in_progress'"#);
    let in_progress_v: serde_json::Value = serde_json::from_str(&in_progress_env).expect("envelope");
    let in_progress: std::collections::HashSet<String> = in_progress_v.get("rows")
        .and_then(|r| r.as_array())
        .map(|arr| arr.iter()
            .filter_map(|r| r.get("Task").and_then(|v| v.as_str()).map(String::from))
            .collect())
        .unwrap_or_default();
    let intersection: Vec<&String> = task_ids.intersection(&in_progress).collect();
    assert!(intersection.is_empty(),
        "parallel-paths result must not include in_progress tasks; intersection: {:?}",
        intersection);
}

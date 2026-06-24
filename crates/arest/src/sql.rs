// crates/arest/src/sql.rs
//
// Read-only SQL SELECT over the relational substrate (#864, hard-cut
// to 3NF in rmap-3nf-tables Stage 2).
//
// Cells ARE relations (whitepaper §3 / RMAP) — and the relations the
// verb exposes are the SAME 3NF schema the persisted app db carries:
// entity tables (task, resource, status, …) with functional
// absorptions as columns, junction tables for m:n / UC-less fact
// types (task_blocks_task, …), and unary occurrence tables for event
// cells (task_is_started + nullable timestamp). One projection plan
// (`rmap::projection_plan`) drives both this per-call `:memory:`
// materialization and the persist path's Phase 4 (cli/entry.rs), so a
// SELECT here and a SELECT against the app db file mean the same
// thing. The historic per-FT `ft_<FactType_id>` virtual layer is GONE
// (user-ratified pre-1.0 hard cut — no compat views).
//
//   - SELECT only. INSERT / UPDATE / DELETE are refused with an error
//     envelope (mutating SQL goes through the apply pipeline).
//   - The tool surface is `system(h, "sql", "<query>")` returning JSON
//     `{"rows":[{col: val, ...}, ...]}` on success or `{"error":"..."}`
//     on parse / exec failure.
//
// Acceptance: the parallel-paths question (issue #864) stays ONE SQL
// query — see `parallel_paths_acceptance_pinned_shape` below, now
// phrased over the 3NF junctions.

#![cfg(feature = "local")]

use crate::ast::Object;
use rusqlite::Connection;

// ── Public entry point ─────────────────────────────────────────────

/// Run a read-only SQL SELECT against the cell graph.
///
/// `state` is the live cell store (typically `tenant.read().snapshot_d()`
/// in the engine path or the loaded `D` in the CLI path). Returns a
/// JSON envelope:
///
///   { "rows": [ {"col": "val", ...}, ... ] }   on success
///   { "error": "<message>" }                   on parse / exec failure
///
/// Refuses any statement whose first non-comment, non-whitespace token
/// is not `SELECT` (case-insensitive). The full SQLite SELECT grammar
/// — JOINs, WHERE, GROUP BY, HAVING, subqueries, window functions —
/// is otherwise available because the underlying engine is SQLite.
pub fn sql_query(state: &Object, query: &str) -> String {
    if !is_select_statement(query) {
        return error_envelope("only SELECT statements are permitted in this read-only verb");
    }

    let conn = match Connection::open_in_memory() {
        Ok(c) => c,
        Err(e) => return error_envelope(&format!("could not open in-memory SQLite: {}", e)),
    };

    if let Err(e) = materialize_3nf_tables(&conn, state) {
        return error_envelope(&format!("materialize: {}", e));
    }

    execute_select(&conn, query)
}

// ── Materialization: projection plan → 3NF SQLite tables ───────────
//
// rmap-3nf-tables Stage 2 (HARD CUT, user-ratified pre-1.0): the sql
// verb queries the SAME 3NF schema the persisted app db carries —
// entity tables (task, resource, status, …), junction tables
// (task_blocks_task, fact_type_has_role, …), and unary occurrence
// tables (task_is_started + timestamp). The per-FT `ft_<id>` virtual
// layer is GONE. One projection plan (`rmap::projection_plan`) drives
// both this per-call :memory: materialization and the persist path's
// Phase 4 (cli/entry.rs), so a SELECT here and a SELECT against the
// app db file mean the same thing — including constraint enforcement:
// the same DDL (NOT NULL / REFERENCES / PK / UNIQUE / CHECK) plus
// `PRAGMA foreign_keys=ON` makes the per-row skip set identical.

/// SQLite identifier quoting — shared with the persist path (see
/// `rmap::qid`; junk prose-minted FT names can carry literal `"`,
/// arc issue 11).
use crate::rmap::{create_table_sql, qid};

fn materialize_3nf_tables(conn: &Connection, state: &Object) -> rusqlite::Result<()> {
    let plan = crate::rmap::projection_plan(state);
    // #23 Stage 1 (flag-gated; mirrors entry::db::domain_namespaces_enabled):
    // under AREST_DOMAIN_NAMESPACES the :memory: tables are namespaced so the
    // sql verb matches the persisted namespaced 3NF tables (user SQL then uses
    // `<domain>__<noun>` names). Off by default -> flat path byte-identical.
    let plan = if std::env::var("AREST_DOMAIN_NAMESPACES").map(|v| v == "1").unwrap_or(false) {
        crate::rmap::namespace_plan(plan, &crate::rmap::build_table_domain(state))
    } else {
        plan
    };
    let by_name: std::collections::HashMap<&str, &crate::rmap::TableDef> =
        plan.tables.iter().map(|t| (t.name.as_str(), t)).collect();

    conn.execute_batch("PRAGMA foreign_keys=ON;")?;
    for name in &plan.order {
        let Some(def) = by_name.get(name.as_str()) else { continue };
        // A junk-named table that still fails to create must not sink
        // every other table (the verb-wholesale failure of issue 11).
        let _ = conn.execute_batch(&create_table_sql(def));
    }
    for name in &plan.order {
        let Some(rows) = plan.rows.get(name) else { continue };
        for row in rows {
            let mut names: Vec<String> = Vec::new();
            let mut values: Vec<&String> = Vec::new();
            for (k, v) in row.iter() {
                names.push(qid(k));
                values.push(v);
            }
            let placeholders: Vec<String> =
                (1..=values.len()).map(|i| format!("?{}", i)).collect();
            let sql = format!(
                "INSERT OR REPLACE INTO {} ({}) VALUES ({})",
                qid(name), names.join(", "), placeholders.join(", "));
            // Per-row skip on constraint failure — the same rows the
            // persist path warn-skips simply aren't present here.
            // Quiet by design: population diagnostics belong to
            // persist, not to every read.
            let _ = conn.execute(&sql, rusqlite::params_from_iter(values.iter()));
        }
    }
    Ok(())
}

// ── SELECT-only gate ───────────────────────────────────────────────

/// Detect whether the leading non-comment, non-whitespace token of
/// `query` is `SELECT` or `WITH` (CTE form). Conservative: if the
/// scanner can't find a leading keyword we refuse.
///
/// Strips:
///   - leading whitespace
///   - `--` line comments
///   - `/* ... */` block comments
///
/// Anything else — INSERT, UPDATE, DELETE, CREATE, DROP, PRAGMA,
/// ATTACH, ALTER, REINDEX, VACUUM — is refused. SQLite's PRAGMA in
/// particular can read filesystem state (`pragma database_list`) and
/// must stay off the read-only verb's surface.
fn is_select_statement(query: &str) -> bool {
    let trimmed = strip_leading_comments(query);
    let upper: String = trimmed.chars().take_while(|c| c.is_ascii_alphabetic())
        .map(|c| c.to_ascii_uppercase()).collect();
    matches!(upper.as_str(), "SELECT" | "WITH")
}

fn strip_leading_comments(input: &str) -> &str {
    let mut s = input.trim_start();
    loop {
        if let Some(rest) = s.strip_prefix("--") {
            s = match rest.find('\n') {
                Some(idx) => rest[idx + 1..].trim_start(),
                None => return "",
            };
            continue;
        }
        if let Some(rest) = s.strip_prefix("/*") {
            s = match rest.find("*/") {
                Some(idx) => rest[idx + 2..].trim_start(),
                None => return "",
            };
            continue;
        }
        return s;
    }
}

// ── Execute + serialize ────────────────────────────────────────────

fn execute_select(conn: &Connection, query: &str) -> String {
    let mut stmt = match conn.prepare(query) {
        Ok(s) => s,
        Err(e) => return error_envelope(&e.to_string()),
    };
    let column_names: Vec<String> = stmt.column_names().into_iter().map(String::from).collect();
    let rows_iter = stmt.query_map([], |row| {
        let mut map = serde_json::Map::with_capacity(column_names.len());
        for (i, name) in column_names.iter().enumerate() {
            let value: serde_json::Value = match row.get_ref(i)? {
                rusqlite::types::ValueRef::Null => serde_json::Value::Null,
                rusqlite::types::ValueRef::Integer(n) => serde_json::Value::from(n),
                rusqlite::types::ValueRef::Real(f) => serde_json::Value::from(f),
                rusqlite::types::ValueRef::Text(t) => serde_json::Value::String(
                    String::from_utf8_lossy(t).into_owned()),
                rusqlite::types::ValueRef::Blob(b) => serde_json::Value::String(
                    format!("0x{}", hex_lower(b))),
            };
            map.insert(name.clone(), value);
        }
        Ok(serde_json::Value::Object(map))
    });
    let rows: Vec<serde_json::Value> = match rows_iter {
        Ok(it) => match it.collect::<Result<Vec<_>, _>>() {
            Ok(v) => v,
            Err(e) => return error_envelope(&e.to_string()),
        },
        Err(e) => return error_envelope(&e.to_string()),
    };
    let mut envelope = serde_json::Map::with_capacity(1);
    envelope.insert("rows".into(), serde_json::Value::Array(rows));
    serde_json::Value::Object(envelope).to_string()
}

fn error_envelope(message: &str) -> String {
    let mut map = serde_json::Map::with_capacity(1);
    map.insert("error".into(), serde_json::Value::String(message.to_string()));
    serde_json::Value::Object(map).to_string()
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use std::fmt::Write;
        let _ = write!(&mut out, "{:02x}", b);
    }
    out
}

// ── Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{self, cell_push, cell_put_keyed, fact_from_pairs, Object};

    fn parse_rows(envelope: &str) -> Vec<serde_json::Value> {
        let v: serde_json::Value = serde_json::from_str(envelope)
            .unwrap_or_else(|_| panic!("envelope must be JSON, got: {}", envelope));
        v.get("rows").and_then(|r| r.as_array()).cloned()
            .unwrap_or_else(|| panic!("envelope must have rows, got: {}", envelope))
    }

    fn parse_error(envelope: &str) -> String {
        let v: serde_json::Value = serde_json::from_str(envelope)
            .unwrap_or_else(|_| panic!("envelope must be JSON, got: {}", envelope));
        v.get("error").and_then(|r| r.as_str()).map(String::from)
            .unwrap_or_else(|| panic!("envelope must have error, got: {}", envelope))
    }

    /// Build a state with a FactType cell + per-FT cells + Role cell so
    /// the projection plan has the full metadata context. The reading
    /// is derived from the id (underscores → spaces) so the junction
    /// table name is snake(ft_id) — `Task_has_Task_Priority` →
    /// `task_has_task_priority`.
    fn state_with(
        ft_specs: &[(&str, &[&str])],
        cells: &[(&str, &[&[(&str, &str)]])],
    ) -> Object {
        let mut state = Object::phi();
        for (ft_id, _roles) in ft_specs {
            let reading = ft_id.replace('_', " ");
            state = cell_push("FactType", fact_from_pairs(&[
                ("id", ft_id), ("reading", reading.as_str()),
            ]), &state);
        }
        for (ft_id, roles) in ft_specs {
            for (idx, role) in roles.iter().enumerate() {
                let pos = idx.to_string();
                state = cell_push("Role", fact_from_pairs(&[
                    ("factType", ft_id),
                    ("nounName", role),
                    ("position", pos.as_str()),
                ]), &state);
            }
        }
        for (cell_name, facts) in cells {
            for fact in *facts {
                state = cell_push(cell_name, fact_from_pairs(fact), &state);
            }
        }
        state
    }

    #[test]
    fn select_only_no_inserts() {
        let state = Object::phi();
        let env = sql_query(&state, "INSERT INTO ft_x VALUES (1)");
        let err = parse_error(&env);
        assert!(err.contains("SELECT"), "expected SELECT-only refusal, got: {}", err);
    }

    #[test]
    fn select_only_no_drops() {
        let state = Object::phi();
        let env = sql_query(&state, "DROP TABLE ft_x");
        assert!(parse_error(&env).contains("SELECT"));
    }

    // ── 3NF contract (rmap-3nf-tables Stage 2, HARD CUT) ────────────
    //
    // The sql verb queries the SAME 3NF schema the persisted app db
    // carries — one projection plan (`rmap::projection_plan`) drives
    // both this :memory: materialization and the persist path. Entity
    // tables appear when the schema declares nouns + functional UCs;
    // UC-less / m:n FTs map to junction tables named snake(reading);
    // unary event FTs map to occurrence tables with a trailing
    // nullable `timestamp`. The hand-built fixtures below declare NO
    // nouns, so every binary FT maps to a junction with fk-style
    // columns (task_id, task_priority_id) — the minimal shape that
    // pins the query contract without the full FORML parse.

    #[test]
    fn select_only_no_pragmas() {
        // PRAGMA can read filesystem state via `database_list`. The
        // gate must reject it even though SQLite accepts it as a
        // read-style statement.
        let state = Object::phi();
        let env = sql_query(&state, "PRAGMA database_list");
        assert!(parse_error(&env).contains("SELECT"));
    }

    #[test]
    fn invalid_sql_returns_error_envelope() {
        let state = Object::phi();
        let env = sql_query(&state, "SELECT * FROM "); // truncated
        let err = parse_error(&env);
        assert!(!err.is_empty(), "should surface SQL parse error");
    }

    #[test]
    fn unknown_table_returns_error_envelope() {
        let state = Object::phi();
        let env = sql_query(&state, "SELECT * FROM nonexistent");
        let err = parse_error(&env);
        assert!(err.to_lowercase().contains("no such table"),
            "expected no-such-table error, got: {}", err);
    }

    /// task-924 parity through the 3NF cut: a `*`-marked (view /
    /// derived) fact type must resolve its derivation when read via
    /// SQL — the projection plan's `effective_cell` resolves the
    /// never-materialized stored cell to the derivation output, so the
    /// junction carries the derived rows.
    #[test]
    fn view_fact_type_resolves_derivation_on_sql_read() {
        // bridge-identity-binding-untyped: head-noun membership row (identity
        // renames are typed now) — `Thing has Name.` + `Thing 'b1' has Name 'n1'.`
        // stage b1 as an instance of head noun Thing.
        let src = "Thing(.id) is an entity type.\n\
Base(.id) is an entity type.\n\
Tag is a value type.\n\
Thing Tag is a value type.\n\
Name is a value type.\n\
\n\
## Fact Types\n\
Base has Tag.\n\
Thing has Thing Tag. *\n\
Thing has Name.\n\
\n\
## Derivation Rules\n\
* Thing has Thing Tag iff that Base has some Tag and Thing Tag is Tag and Thing is Base.\n\
\n\
## Instance Facts\n\
Base 'b1' has Tag 'hot'.\n\
Thing 'b1' has Name 'n1'.\n";
        let state = crate::parse_forml2_stage2::parse_to_state_via_stage12(src).expect("parse");
        let defs = crate::compile::compile_to_defs_state(&state);
        let d = crate::ast::defs_to_state(&defs, &state);
        let env = sql_query(&d,
            "SELECT thing_id, thing_tag FROM thing_has_thing_tag");
        let rows = parse_rows(&env);
        assert!(rows.iter().any(|r|
            r.get("thing_id").and_then(|v| v.as_str()) == Some("b1")
                && r.get("thing_tag").and_then(|v| v.as_str()) == Some("hot")),
            "view FT must resolve to derived facts on SQL read; got: {:?}", rows);
    }

    /// Unary (event) fact types map to occurrence tables — Halpin
    /// §10.3 open-world unaries — with the entity role column plus the
    /// nullable SM occurred-at `timestamp` (the undeclared trailing
    /// `<Timestamp, …>` pair `transition_via_defs` stamps; pre-stamp
    /// historical facts project NULL). Replaces the ft_-layer's
    /// appended-Timestamp-column tests.
    #[test]
    fn unary_event_fact_type_maps_to_occurrence_table_with_timestamp() {
        let src = "Hop(.hid) is an entity type.\n\
hid is a value type.\n\
\n\
## Fact Types\n\
Hop is started.\n";
        let mut state = crate::parse_forml2_stage2::parse_to_state_via_stage12(src)
            .expect("parse");
        state = cell_push("Hop_is_started",
            fact_from_pairs(&[("Hop", "h-new"), ("Timestamp", "0000000000099-000000000001")]),
            &state);
        state = cell_push("Hop_is_started",
            fact_from_pairs(&[("Hop", "h-old")]), &state);

        let env = sql_query(&state,
            "SELECT hop_id, timestamp FROM hop_is_started ORDER BY hop_id");
        let rows = parse_rows(&env);
        assert_eq!(rows.len(), 2, "envelope: {}", env);
        assert_eq!(rows[0].get("hop_id").and_then(|v| v.as_str()), Some("h-new"));
        assert_eq!(rows[0].get("timestamp").and_then(|v| v.as_str()),
            Some("0000000000099-000000000001"));
        assert_eq!(rows[1].get("hop_id").and_then(|v| v.as_str()), Some("h-old"));
        assert!(rows[1].get("timestamp").map_or(false, |v| v.is_null()),
            "pre-stamp facts must project NULL timestamp; envelope: {}", env);
    }

    #[test]
    fn junction_select_with_filter() {
        let state = state_with(
            &[("Task_has_Task_Priority", &["Task", "Task Priority"])],
            &[("Task_has_Task_Priority", &[
                &[("Task", "1"), ("Task Priority", "p0")],
                &[("Task", "2"), ("Task Priority", "p1")],
                &[("Task", "3"), ("Task Priority", "p0")],
            ])],
        );
        let env = sql_query(&state,
            "SELECT task_id FROM task_has_task_priority WHERE task_priority_id = 'p0' ORDER BY task_id");
        let rows = parse_rows(&env);
        assert_eq!(rows.len(), 2, "envelope: {}", env);
        assert_eq!(rows[0].get("task_id").and_then(|v| v.as_str()), Some("1"));
        assert_eq!(rows[1].get("task_id").and_then(|v| v.as_str()), Some("3"));
    }

    #[test]
    fn join_across_two_junctions() {
        let state = state_with(
            &[
                ("Task_has_Task_Priority", &["Task", "Task Priority"]),
                ("Task_has_Task_Readiness", &["Task", "Task Readiness"]),
            ],
            &[
                ("Task_has_Task_Priority", &[
                    &[("Task", "1"), ("Task Priority", "p0")],
                    &[("Task", "2"), ("Task Priority", "p1")],
                    &[("Task", "3"), ("Task Priority", "p0")],
                ]),
                ("Task_has_Task_Readiness", &[
                    &[("Task", "1"), ("Task Readiness", "ready")],
                    &[("Task", "2"), ("Task Readiness", "ready")],
                    &[("Task", "3"), ("Task Readiness", "blocked")],
                ]),
            ],
        );
        let env = sql_query(&state, r#"
            SELECT DISTINCT r.task_id AS task
            FROM task_has_task_readiness r
            JOIN task_has_task_priority p ON p.task_id = r.task_id
            WHERE r.task_readiness_id = 'ready' AND p.task_priority_id = 'p0'
            ORDER BY task
        "#);
        let rows = parse_rows(&env);
        assert_eq!(rows.len(), 1, "envelope: {}", env);
        assert_eq!(rows[0].get("task").and_then(|v| v.as_str()), Some("1"));
    }

    #[test]
    fn parallel_paths_acceptance_pinned_shape() {
        // Issue #864 acceptance: the parallel-paths question becomes
        // ONE SQL query — now phrased over the 3NF junctions. The
        // "ready, p0, and not on a file someone in_progress is
        // touching" set keeps Task 1 and excludes Task 5 (touches
        // src/b.rs which in_progress Task 9 also touches).
        let state = state_with(
            &[
                ("Task_has_Task_Readiness", &["Task", "Task Readiness"]),
                ("Task_has_Task_Priority", &["Task", "Task Priority"]),
                ("Task_has_Task_Status", &["Task", "Task Status"]),
                ("Task_touches_Source_File", &["Task", "Source File"]),
            ],
            &[
                ("Task_has_Task_Readiness", &[
                    &[("Task", "1"), ("Task Readiness", "ready")],
                    &[("Task", "5"), ("Task Readiness", "ready")],
                    &[("Task", "7"), ("Task Readiness", "blocked")],
                ]),
                ("Task_has_Task_Priority", &[
                    &[("Task", "1"), ("Task Priority", "p0")],
                    &[("Task", "5"), ("Task Priority", "p0")],
                    &[("Task", "7"), ("Task Priority", "p0")],
                    &[("Task", "9"), ("Task Priority", "p1")],
                ]),
                ("Task_has_Task_Status", &[
                    &[("Task", "1"), ("Task Status", "pending")],
                    &[("Task", "5"), ("Task Status", "pending")],
                    &[("Task", "9"), ("Task Status", "in_progress")],
                ]),
                ("Task_touches_Source_File", &[
                    &[("Task", "1"), ("Source File", "src/a.rs")],
                    &[("Task", "5"), ("Source File", "src/b.rs")],
                    &[("Task", "9"), ("Source File", "src/b.rs")],
                ]),
            ],
        );
        let env = sql_query(&state, r#"
            SELECT DISTINCT r.task_id AS task
            FROM task_has_task_readiness r
            JOIN task_has_task_priority p ON p.task_id = r.task_id
            WHERE r.task_readiness_id = 'ready'
              AND p.task_priority_id = 'p0'
              AND NOT EXISTS (
                SELECT 1
                FROM task_touches_source_file mine
                JOIN task_touches_source_file theirs
                  ON theirs.source_file_id = mine.source_file_id
                JOIN task_has_task_status s ON s.task_id = theirs.task_id
                WHERE mine.task_id = r.task_id
                  AND s.task_status_id = 'in_progress'
              )
            ORDER BY task
        "#);
        let rows = parse_rows(&env);
        let tasks: Vec<&str> = rows.iter()
            .filter_map(|r| r.get("task").and_then(|v| v.as_str()))
            .collect();
        assert_eq!(tasks, vec!["1"], "envelope: {}", env);
    }

    #[test]
    fn ring_fact_type_duplicate_role_names_are_disambiguated() {
        // Ring FTs (`Theme is the default Theme` — both roles "Theme")
        // disambiguate POSITIONALLY in the junction: theme_id,
        // theme_id_2, with values landing in declaration order.
        let state = state_with(
            &[("Theme_is_the_default_Theme", &["Theme", "Theme"])],
            &[("Theme_is_the_default_Theme", &[
                &[("Theme", "dark"), ("Theme", "light")],
                &[("Theme", "light"), ("Theme", "dark")],
            ])],
        );
        let env = sql_query(&state,
            "SELECT theme_id AS a, theme_id_2 AS b FROM theme_is_the_default_theme ORDER BY a");
        let rows = parse_rows(&env);
        assert_eq!(rows.len(), 2, "envelope: {}", env);
        assert_eq!(rows[0].get("a").and_then(|v| v.as_str()), Some("dark"));
        assert_eq!(rows[0].get("b").and_then(|v| v.as_str()), Some("light"));
        assert_eq!(rows[1].get("a").and_then(|v| v.as_str()), Some("light"));
        assert_eq!(rows[1].get("b").and_then(|v| v.as_str()), Some("dark"));
    }

    #[test]
    fn map_keyed_cell_projects_to_sql_table() {
        // task-922 parity through the 3NF cut: Map-keyed cells (RMAP
        // storage; #744) populated via cell_put_keyed MUST surface in
        // the projection — `projection_plan` reads via cell_facts_iter,
        // which walks Seq and Map shapes identically.
        let mut state = Object::phi();
        state = cell_push("FactType", fact_from_pairs(&[
            ("id", "Task_has_Task_Status"),
            ("reading", "Task has Task Status"),
        ]), &state);
        state = cell_push("Role", fact_from_pairs(&[
            ("factType", "Task_has_Task_Status"),
            ("nounName", "Task"),
            ("position", "0"),
        ]), &state);
        state = cell_push("Role", fact_from_pairs(&[
            ("factType", "Task_has_Task_Status"),
            ("nounName", "Task Status"),
            ("position", "1"),
        ]), &state);
        for (task, status) in [("t-1", "pending"), ("t-2", "done"), ("t-3", "in_progress")] {
            state = cell_put_keyed(
                "Task_has_Task_Status",
                &["Task"],
                fact_from_pairs(&[("Task", task), ("Task Status", status)]),
                &state,
            ).expect("distinct keys must not collide");
        }

        let contents = ast::fetch_or_phi("Task_has_Task_Status", &state);
        assert!(matches!(contents, Object::Map(_)),
            "fixture must produce Map-shape contents to pin the regression");

        let env = sql_query(&state,
            "SELECT COUNT(*) AS n FROM task_has_task_status");
        let rows = parse_rows(&env);
        assert_eq!(rows.len(), 1, "envelope: {}", env);
        let n = rows[0].get("n").and_then(|v| v.as_i64()).unwrap_or(-1);
        assert_eq!(n, 3, "Map-keyed cell must project all 3 facts; envelope: {}", env);

        let env = sql_query(&state,
            "SELECT task_status_id AS s FROM task_has_task_status WHERE task_id = 't-2'");
        let rows = parse_rows(&env);
        assert_eq!(rows.len(), 1, "envelope: {}", env);
        assert_eq!(rows[0].get("s").and_then(|v| v.as_str()), Some("done"));
    }

    #[test]
    fn map_keyed_factype_and_role_cells_still_resolve() {
        // Map-shaped FactType / Role cells must still drive the
        // projection (cell_facts_iter shape duality, #744/#932).
        let mut state = Object::phi();
        state = cell_put_keyed(
            "FactType",
            &["id"],
            fact_from_pairs(&[
                ("id", "Task_has_Task_Priority"),
                ("reading", "Task has Task Priority"),
            ]),
            &state,
        ).expect("first FactType");
        state = cell_put_keyed(
            "Role",
            &["factType", "nounName"],
            fact_from_pairs(&[
                ("factType", "Task_has_Task_Priority"),
                ("nounName", "Task"),
                ("position", "0"),
            ]),
            &state,
        ).expect("Task role");
        state = cell_put_keyed(
            "Role",
            &["factType", "nounName"],
            fact_from_pairs(&[
                ("factType", "Task_has_Task_Priority"),
                ("nounName", "Task Priority"),
                ("position", "1"),
            ]),
            &state,
        ).expect("Task Priority role");
        state = cell_put_keyed(
            "Task_has_Task_Priority",
            &["Task"],
            fact_from_pairs(&[("Task", "1"), ("Task Priority", "p0")]),
            &state,
        ).expect("FT row");

        assert!(matches!(ast::fetch_or_phi("FactType", &state), Object::Map(_)),
            "FactType fixture must be Map-shape");
        assert!(matches!(ast::fetch_or_phi("Role", &state), Object::Map(_)),
            "Role fixture must be Map-shape");

        let env = sql_query(&state,
            "SELECT task_id, task_priority_id AS p FROM task_has_task_priority");
        let rows = parse_rows(&env);
        assert_eq!(rows.len(), 1, "envelope: {}", env);
        assert_eq!(rows[0].get("task_id").and_then(|v| v.as_str()), Some("1"));
        assert_eq!(rows[0].get("p").and_then(|v| v.as_str()), Some("p0"));
    }

    // ── Crate round-trip: apply-create keyed cells → persist → load → sql ──

    /// Faithful reproduction of `cli/entry.rs::db::persist_state` +
    /// `load_state` for the POPULATION cells (the ones `sql` reads).
    /// Persist serializes each cell via `Display` (`to_string`); load
    /// re-parses via `Object::parse`. Mirrors the exact `INSERT OR
    /// REPLACE` / `SELECT name, contents FROM cells` the MCP path runs.
    fn persist_then_load(state: &Object) -> Object {
        let conn = Connection::open_in_memory().expect("in-memory sqlite");
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS cells (name TEXT PRIMARY KEY, contents TEXT);",
        ).expect("create cells table");
        for (name, contents) in ast::cells_iter(state) {
            if name.contains(':')
                || ["validate", "compile", "apply", "verify_signature",
                    "debug", "_defs_compiled"].contains(&name)
            {
                continue;
            }
            conn.execute(
                "INSERT OR REPLACE INTO cells (name, contents) VALUES (?1, ?2)",
                rusqlite::params![name, contents.to_string()],
            ).expect("insert cell");
        }
        let mut map: hashbrown::HashMap<String, Object> = hashbrown::HashMap::new();
        let mut stmt = conn.prepare("SELECT name, contents FROM cells").expect("prepare");
        let rows = stmt.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        }).expect("query_map");
        for r in rows.filter_map(|r| r.ok()) {
            map.insert(r.0, Object::parse(&r.1));
        }
        Object::map(map)
    }

    fn count_for(state: &Object, table: &str, where_clause: &str) -> i64 {
        let env = sql_query(state, &format!(
            "SELECT COUNT(*) AS n FROM {} WHERE {}", table, where_clause));
        let rows = parse_rows(&env);
        rows.first().and_then(|r| r.get("n")).and_then(|v| v.as_i64())
            .unwrap_or_else(|| panic!("expected n in {}: {}", table, env))
    }

    #[test]
    fn apply_create_keyed_map_cell_survives_persist_load_into_sql() {
        // apply-create keyed Map-cell fields must surface in sql after
        // the exact persist (Display) → SQLite → load (Object::parse)
        // round-trip the MCP path runs.
        let mut state = Object::phi();
        for (ft, reading) in [
            ("Task_has_Task_Subject", "Task has Task Subject"),
            ("Task_has_Task_Priority", "Task has Task Priority"),
            ("Task_has_Task_Description", "Task has Task Description"),
            ("Task_has_Task_Status", "Task has Task Status"),
        ] {
            state = cell_push("FactType",
                fact_from_pairs(&[("id", ft), ("reading", reading)]), &state);
        }
        for (ft, vrole) in [
            ("Task_has_Task_Subject", "Task Subject"),
            ("Task_has_Task_Priority", "Task Priority"),
            ("Task_has_Task_Description", "Task Description"),
            ("Task_has_Task_Status", "Task Status"),
        ] {
            state = cell_push("Role", fact_from_pairs(&[
                ("factType", ft), ("nounName", "Task"), ("position", "0")]), &state);
            state = cell_push("Role", fact_from_pairs(&[
                ("factType", ft), ("nounName", vrole), ("position", "1")]), &state);
        }
        for (task, subj, prio, desc) in [
            ("task-001", "first subject", "p0", "first description"),
            ("task-002", "second subject", "p1", "second description"),
        ] {
            state = cell_put_keyed("Task_has_Task_Subject", &["Task"],
                fact_from_pairs(&[("Task", task), ("Task Subject", subj)]), &state).unwrap();
            state = cell_put_keyed("Task_has_Task_Priority", &["Task"],
                fact_from_pairs(&[("Task", task), ("Task Priority", prio)]), &state).unwrap();
            state = cell_put_keyed("Task_has_Task_Description", &["Task"],
                fact_from_pairs(&[("Task", task), ("Task Description", desc)]), &state).unwrap();
            state = ast::cell_put_folded("Task_has_Task_Status",
                fact_from_pairs(&[("Task", task), ("Task Status", "pending")]), &state);
        }

        const NEW: &str = "task-NEW";
        state = cell_put_keyed("Task_has_Task_Subject", &["Task"],
            fact_from_pairs(&[("Task", NEW), ("Task Subject", "brand new subject")]), &state).unwrap();
        state = cell_put_keyed("Task_has_Task_Priority", &["Task"],
            fact_from_pairs(&[("Task", NEW), ("Task Priority", "p0")]), &state).unwrap();
        state = cell_put_keyed("Task_has_Task_Description", &["Task"],
            fact_from_pairs(&[("Task", NEW), ("Task Description", "brand new description")]), &state).unwrap();
        state = ast::cell_put_folded("Task_has_Task_Status",
            fact_from_pairs(&[("Task", NEW), ("Task Status", "pending")]), &state);

        let subj_cell = ast::fetch_or_phi("Task_has_Task_Subject", &state);
        assert!(matches!(subj_cell, Object::Map(_)),
            "keyed cell must be Map-shaped after apply-create");
        assert_eq!(ast::cell_fact_count(&subj_cell), 3,
            "in-memory Subject cell must hold 3 rows (2 seeded + new)");

        let reloaded = persist_then_load(&state);

        let subj_env = sql_query(&reloaded,
            &format!("SELECT task_subject_id AS s FROM task_has_task_subject WHERE task_id = '{}'", NEW));
        let subj_rows = parse_rows(&subj_env);
        assert_eq!(subj_rows.len(), 1,
            "NEW task Subject must surface in sql after persist+load; envelope: {}", subj_env);
        assert_eq!(subj_rows[0].get("s").and_then(|v| v.as_str()), Some("brand new subject"),
            "NEW task Subject value must round-trip; envelope: {}", subj_env);

        for table in ["task_has_task_subject", "task_has_task_priority",
                      "task_has_task_description", "task_has_task_status"] {
            assert_eq!(count_for(&reloaded, table, &format!("task_id = '{}'", NEW)), 1,
                "{} must have the new row", table);
            assert_eq!(count_for(&reloaded, table, "1=1"), 3,
                "{} must materialize all 3 rows post round-trip", table);
        }
    }
}

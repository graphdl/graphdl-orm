// crates/arest/src/sql.rs
//
// Read-only SQL SELECT over the relational substrate (#864).
//
// Cells ARE relations (whitepaper §3 / RMAP). Each FactType cell maps
// to a SQL table named `ft_<FactType_id>` whose columns are the role
// names (spaces normalized to underscores). Rows come from the cell's
// in-flight FFP-encoded contents — `<<role, value>, ...>` per fact.
//
// First cut scope (#864):
//   - SELECT only. INSERT / UPDATE / DELETE are refused with an error
//     envelope (mutating SQL goes through the apply pipeline; separate
//     task).
//   - Per-FT virtual tables are materialized into an in-memory SQLite
//     connection on each call. No state mutation. Read-only by design.
//   - The tool surface is `system(h, "sql", "<query>")` returning JSON
//     `{"rows":[{col: val, ...}, ...]}` on success or `{"error":"..."}`
//     on parse / exec failure.
//
// Why materialize to SQLite per-call instead of holding a long-lived
// connection? Cells already round-trip through `cells_iter` cheaply.
// SQLite's `:memory:` handle is microseconds to open. The diff is
// smaller than a virtual-table extension that has to track cell
// versions, and the call surface stays state-free — exactly the
// "read-only projection" shape `is_read_only_op` already documents
// for `query:` / `get:`. A virtual-table follow-up can land later
// without changing this MCP verb's contract.
//
// Acceptance: the parallel-paths question (issue #864) becomes ONE
// SQL query — see `parallel_paths_acceptance` test below.

#![cfg(feature = "local")]

use crate::ast::{self, Object};
use rusqlite::Connection;
use std::collections::{BTreeMap, HashSet};

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

    if let Err(e) = materialize_fact_type_tables(&conn, state) {
        return error_envelope(&format!("materialize: {}", e));
    }

    execute_select(&conn, query)
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

// ── Materialization: cells → per-FT SQLite tables ──────────────────

/// Read the FactType cell to learn which cell names are FT cells, then
/// for each FT create an `ft_<sanitize(ft_id)>` table whose columns
/// are the role names (sanitized: spaces → underscores). Insert one
/// row per fact in the cell's contents.
///
/// Cells with no `Role` rows fall back to whatever role names appear
/// in the first fact — the parallel-paths acceptance test loads the
/// raw `tasks.db` cells directly without re-deriving the metamodel,
/// so the Role cell may be absent. Falling back keeps the verb useful
/// against any cell store; the FactType cell is consulted only to
/// scope which cells get tables (so user-namespaced housekeeping
/// cells like `_loaded_reading:foo` aren't promoted).
fn materialize_fact_type_tables(conn: &Connection, state: &Object) -> rusqlite::Result<()> {
    let ft_ids: HashSet<String> = collect_fact_type_ids(state);
    let role_map: BTreeMap<String, Vec<String>> = collect_role_map(state);

    // Iterate declared FT ids (not just cells that happen to exist) so a
    // view (derived) FT with no stored cell still gets a table.
    for ft_id in &ft_ids {
        // task-924: a view (derived) fact type's population IS its
        // derivation's output, not the stored cell — the `view:` marker
        // chooses lazy materialization, not different semantics. Resolve
        // the view on read so a query reflects the derived facts;
        // a never-materialized view otherwise reads as an empty cell.
        let resolved = ast::resolve_view(ft_id, state, state);
        let stored = resolved.is_none().then(|| ast::fetch_or_phi(ft_id, state));
        let effective: &Object = resolved.as_ref().or(stored.as_ref())
            .expect("resolved or stored is always Some");
        let table = format!("ft_{}", sanitize_identifier(ft_id));
        let columns = column_names_for(ft_id, effective, &role_map);
        if columns.is_empty() {
            continue;
        }
        create_and_populate_table(conn, &table, &columns, effective)?;
    }
    Ok(())
}

/// Collect the set of FT ids from the FactType cell's `id` bindings.
/// If FactType is absent (raw test fixture / partially-bootstrapped
/// state) every cell whose name passes `looks_like_fact_type` is
/// included so callers can still query.
///
/// Reads facts via `cell_facts_iter` so Map-keyed FactType cells
/// (RMAP storage shape; #744) iterate identically to legacy Seq cells.
/// Direct `as_seq()` would return None for Map storage and silently
/// drop every FT id — which is exactly the regression task-922 fixes
/// for the downstream cell projector.
fn collect_fact_type_ids(state: &Object) -> HashSet<String> {
    let cell = ast::fetch_or_phi("FactType", state);
    let parsed: HashSet<String> = ast::cell_facts_iter(&cell)
        .filter_map(|f| ast::binding(f, "id").map(String::from))
        .collect();
    if !parsed.is_empty() {
        return parsed;
    }
    // Fallback: any cell whose name looks like a FactType id (has at
    // least one underscore, no `:` namespace separator, isn't all-
    // lowercase entity-table form).
    ast::cells_iter(state).into_iter()
        .map(|(name, _)| name.to_string())
        .filter(|name| looks_like_fact_type(name))
        .collect()
}

fn looks_like_fact_type(name: &str) -> bool {
    !name.contains(':')
        && name.contains('_')
        && name.chars().next().map_or(false, |c| c.is_ascii_uppercase())
}

/// Collect role-name lists per FT id from the Role cell. Position-
/// ordered. Empty when Role is absent.
///
/// Reads facts via `cell_facts_iter` (Seq + Map shapes; #744) so a
/// Map-keyed Role cell still surfaces every role.
fn collect_role_map(state: &Object) -> BTreeMap<String, Vec<String>> {
    let cell = ast::fetch_or_phi("Role", state);
    let mut tagged: BTreeMap<String, Vec<(usize, String)>> = BTreeMap::new();
    for fact in ast::cell_facts_iter(&cell) {
        let Some(ft) = ast::binding(fact, "factType") else { continue };
        let Some(name) = ast::binding(fact, "nounName") else { continue };
        let pos: usize = ast::binding(fact, "position")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        tagged.entry(ft.to_string()).or_default().push((pos, name.to_string()));
    }
    tagged.into_iter()
        .map(|(ft, mut rows)| {
            rows.sort_by_key(|(p, _)| *p);
            (ft, rows.into_iter().map(|(_, n)| n).collect())
        })
        .collect()
}

/// Determine the column list for a cell. Prefers the Role cell; falls
/// back to inspecting the first fact's bindings (in seen order). Both
/// shapes get sanitized so role names with spaces (`Task Priority`)
/// become valid SQL identifiers (`Task_Priority`); collisions after
/// sanitization (e.g. ring fact types `Theme_is_the_default_Theme`
/// where both roles are named "Theme") get a `_<position>` suffix on
/// the second-and-later occurrence so SQLite's
/// "duplicate column name" error doesn't sink the whole materializer.
fn column_names_for(
    cell_name: &str,
    contents: &Object,
    role_map: &BTreeMap<String, Vec<String>>,
) -> Vec<String> {
    let raw: Vec<String> = if let Some(roles) = role_map.get(cell_name) {
        if !roles.is_empty() {
            roles.iter().map(|r| sanitize_identifier(r)).collect()
        } else {
            Vec::new()
        }
    } else {
        // Fallback: first fact's bindings determine the columns by
        // declaration order. Used when the Role cell isn't populated
        // (raw test fixtures, partially-bootstrapped state).
        //
        // Reads via `cell_facts_iter` so Map-keyed cells (#744) still
        // expose a "first" fact; HashMap iteration order is arbitrary
        // but the per-fact pair order is preserved by `fact_from_pairs`,
        // so column names match the fact's binding order regardless of
        // which fact happens to be visited first.
        ast::cell_facts_iter(contents).next()
            .and_then(|fact| fact.as_seq())
            .map(|pairs| pairs.iter()
                .filter_map(|p| {
                    let kv = p.as_seq()?;
                    if kv.len() != 2 { return None; }
                    kv[0].as_atom().map(|s| sanitize_identifier(s))
                })
                .collect::<Vec<_>>())
            .unwrap_or_default()
    };
    disambiguate_columns(&raw)
}

/// Append `_<n>` to repeated names so the resulting list is unique
/// while preserving the first occurrence verbatim. So `["Theme",
/// "Theme"]` becomes `["Theme", "Theme_2"]`. Identifiers stay
/// snake-case-with-underscore so users can read them back from the
/// original FT id without surprise.
fn disambiguate_columns(raw: &[String]) -> Vec<String> {
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    raw.iter().map(|name| {
        let count = seen.entry(name.clone()).and_modify(|n| *n += 1).or_insert(1);
        if *count == 1 {
            name.clone()
        } else {
            format!("{}_{}", name, count)
        }
    }).collect()
}

/// Replace every non-alphanumeric character with `_`. SQL identifiers
/// quoted with `"` accept arbitrary Unicode but the generator emitted
/// table / column names `ft_Task_has_Task_Priority` / `Task_Priority`
/// (snake_case_with_underscore) for years — keep that convention so
/// hand-typed SQL matches what users see in `cells` listings.
fn sanitize_identifier(raw: &str) -> String {
    raw.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect()
}

/// CREATE TABLE with TEXT columns (everything is an atom in the cell
/// graph; SQLite will accept comparisons regardless), then INSERT one
/// row per fact.
///
/// Insert strategy: walk each fact's `<role,val>` pairs by position
/// and assign them to columns in declaration order. Position-aware
/// because ring fact types (e.g. `Theme_is_the_default_Theme` with
/// roles `Theme` at index 0 and `Theme` at index 1) repeat the
/// role name; a name-keyed lookup would always pick the first
/// occurrence and silently drop subsequent values. The column list
/// already has uniqueness baked in by `disambiguate_columns`, so
/// position-mapping puts each value in the right slot.
///
/// Facts whose pair count is shorter than the column list get NULLs
/// in the trailing columns; longer facts get the surplus dropped.
/// Both should be rare — schemas declare a fixed arity — but neither
/// is fatal and the projection still surfaces the available values.
fn create_and_populate_table(
    conn: &Connection,
    table: &str,
    columns: &[String],
    contents: &Object,
) -> rusqlite::Result<()> {
    let cols_ddl = columns.iter()
        .map(|c| format!("\"{}\" TEXT", c))
        .collect::<Vec<_>>()
        .join(", ");
    let create = format!("CREATE TABLE \"{}\" ({})", table, cols_ddl);
    conn.execute_batch(&create)?;

    let placeholders = (0..columns.len()).map(|_| "?").collect::<Vec<_>>().join(", ");
    let cols_list = columns.iter()
        .map(|c| format!("\"{}\"", c))
        .collect::<Vec<_>>()
        .join(", ");
    let insert = format!("INSERT INTO \"{}\" ({}) VALUES ({})", table, cols_list, placeholders);
    let mut stmt = conn.prepare(&insert)?;

    // Iterate the cell uniformly across Seq and Map storage shapes.
    // Pre-task-922 this was `contents.as_seq()` which returned None for
    // Map-keyed cells (RMAP storage; #744) and silently produced an
    // empty SQL table — `Task_has_Task_Status` and every other
    // bridge/SM-driven Map-keyed cell projected zero rows even when
    // populated. `cell_facts_iter` walks both shapes; the per-fact
    // role-pair structure inside each fact is still Seq-encoded by
    // `fact_from_pairs`, so the position-mapping below is unchanged.
    for fact in ast::cell_facts_iter(contents) {
        let pairs = match fact.as_seq() {
            Some(p) => p,
            None => continue,
        };
        let values: Vec<Option<String>> = (0..columns.len())
            .map(|idx| pairs.get(idx)
                .and_then(|p| p.as_seq())
                .filter(|kv| kv.len() == 2)
                .and_then(|kv| kv[1].as_atom().map(String::from)))
            .collect();
        let params: Vec<&dyn rusqlite::ToSql> = values.iter()
            .map(|v| v as &dyn rusqlite::ToSql).collect();
        stmt.execute(rusqlite::params_from_iter(params))?;
    }
    Ok(())
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
    use crate::ast::{cell_push, cell_put_keyed, fact_from_pairs, Object};

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
    /// the materializer has the full metadata context.
    fn state_with(
        ft_specs: &[(&str, &[&str])],
        cells: &[(&str, &[&[(&str, &str)]])],
    ) -> Object {
        let mut state = Object::phi();
        for (ft_id, _roles) in ft_specs {
            state = cell_push("FactType", fact_from_pairs(&[("id", ft_id)]), &state);
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
        let env = sql_query(&state, "SELECT * FROM ft_nonexistent");
        let err = parse_error(&env);
        assert!(err.to_lowercase().contains("no such table"),
            "expected no-such-table error, got: {}", err);
    }

    /// task-924: a `*`-marked (view / derived) fact type must resolve its
    /// derivation when read via SQL — querying it returns the derived
    /// facts, not the never-materialized (empty) stored cell. Pre-fix the
    /// materializer read the stored cell and returned zero rows.
    #[test]
    fn view_fact_type_resolves_derivation_on_sql_read() {
        let src = "Thing(.id) is an entity type.\n\
Base(.id) is an entity type.\n\
Tag is a value type.\n\
Thing Tag is a value type.\n\
\n\
## Fact Types\n\
Base has Tag.\n\
Thing has Thing Tag. *\n\
\n\
## Derivation Rules\n\
* Thing has Thing Tag iff that Base has some Tag and Thing Tag is Tag and Thing is Base.\n\
\n\
## Instance Facts\n\
Base 'b1' has Tag 'hot'.\n";
        let state = crate::parse_forml2_stage2::parse_to_state_via_stage12(src).expect("parse");
        let defs = crate::compile::compile_to_defs_state(&state);
        let d = crate::ast::defs_to_state(&defs, &state);
        let env = sql_query(&d, r#"SELECT "Thing", "Thing_Tag" FROM ft_Thing_has_Thing_Tag"#);
        let rows = parse_rows(&env);
        assert!(rows.iter().any(|r|
            r.get("Thing").and_then(|v| v.as_str()) == Some("b1")
                && r.get("Thing_Tag").and_then(|v| v.as_str()) == Some("hot")),
            "view FT must resolve to derived facts on SQL read; got: {:?}", rows);
    }

    #[test]
    fn single_ft_select_returns_rows_with_role_columns() {
        let state = state_with(
            &[("Task_has_Task_Priority", &["Task", "Task Priority"])],
            &[("Task_has_Task_Priority", &[
                &[("Task", "1"), ("Task Priority", "p0")],
                &[("Task", "2"), ("Task Priority", "p1")],
                &[("Task", "3"), ("Task Priority", "p0")],
            ])],
        );
        let env = sql_query(&state,
            r#"SELECT "Task" FROM ft_Task_has_Task_Priority WHERE "Task_Priority" = 'p0' ORDER BY "Task""#);
        let rows = parse_rows(&env);
        assert_eq!(rows.len(), 2, "envelope: {}", env);
        assert_eq!(rows[0].get("Task").and_then(|v| v.as_str()), Some("1"));
        assert_eq!(rows[1].get("Task").and_then(|v| v.as_str()), Some("3"));
    }

    #[test]
    fn role_names_with_spaces_become_underscored_columns() {
        // "Task Priority" → "Task_Priority". Verify both the column
        // exists post-materialization (the SELECT below would error
        // otherwise) and that selecting it returns the original value.
        let state = state_with(
            &[("Task_has_Task_Priority", &["Task", "Task Priority"])],
            &[("Task_has_Task_Priority", &[
                &[("Task", "9"), ("Task Priority", "p0")],
            ])],
        );
        let env = sql_query(&state,
            r#"SELECT "Task_Priority" AS p FROM ft_Task_has_Task_Priority"#);
        let rows = parse_rows(&env);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].get("p").and_then(|v| v.as_str()), Some("p0"));
    }

    #[test]
    fn join_across_two_fts() {
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
            SELECT DISTINCT r."Task" AS task
            FROM ft_Task_has_Task_Readiness r
            JOIN ft_Task_has_Task_Priority p ON p."Task" = r."Task"
            WHERE r."Task_Readiness" = 'ready' AND p."Task_Priority" = 'p0'
            ORDER BY task
        "#);
        let rows = parse_rows(&env);
        assert_eq!(rows.len(), 1, "envelope: {}", env);
        assert_eq!(rows[0].get("task").and_then(|v| v.as_str()), Some("1"));
    }

    #[test]
    fn parallel_paths_acceptance_pinned_shape() {
        // Issue #864 acceptance: the parallel-paths question becomes
        // ONE SQL query. Builds a hand-curated state mirroring the
        // shape the apps/tasks DB has — three readiness/priority/
        // status FTs plus a Source-File touch FT. The "ready, p0,
        // and not on a file someone in_progress is touching" set
        // should leave Task 1 (touches f1, no other in_progress
        // touches f1) and exclude Task 5 (touches f1 which Task 9 is
        // in_progress on).
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
            SELECT DISTINCT r."Task" AS task
            FROM ft_Task_has_Task_Readiness r
            JOIN ft_Task_has_Task_Priority p ON p."Task" = r."Task"
            WHERE r."Task_Readiness" = 'ready'
              AND p."Task_Priority" = 'p0'
              AND NOT EXISTS (
                SELECT 1
                FROM ft_Task_touches_Source_File mine
                JOIN ft_Task_touches_Source_File theirs
                  ON theirs."Source_File" = mine."Source_File"
                JOIN ft_Task_has_Task_Status s ON s."Task" = theirs."Task"
                WHERE mine."Task" = r."Task"
                  AND s."Task_Status" = 'in_progress'
              )
            ORDER BY task
        "#);
        let rows = parse_rows(&env);
        let tasks: Vec<&str> = rows.iter()
            .filter_map(|r| r.get("task").and_then(|v| v.as_str()))
            .collect();
        // Task 1 is ready+p0 and only touches files no in_progress
        // task touches; Task 5 is ready+p0 but touches src/b.rs which
        // in_progress Task 9 also touches → excluded.
        assert_eq!(tasks, vec!["1"], "envelope: {}", env);
    }

    #[test]
    fn role_fallback_when_role_cell_absent() {
        // Bare FactType cell + per-FT cell, no Role cell. The
        // materializer should fall back to inspecting the first fact's
        // bindings to learn the column names. Mirrors the apps/tasks
        // DB shape after the CLI loads only the population cells (no
        // metamodel re-derivation in the integration test).
        let mut state = Object::phi();
        state = cell_push("FactType", fact_from_pairs(&[("id", "Task_has_Task_Priority")]), &state);
        state = cell_push("Task_has_Task_Priority",
            fact_from_pairs(&[("Task", "42"), ("Task Priority", "p0")]), &state);
        let env = sql_query(&state,
            r#"SELECT "Task" FROM ft_Task_has_Task_Priority WHERE "Task_Priority" = 'p0'"#);
        let rows = parse_rows(&env);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].get("Task").and_then(|v| v.as_str()), Some("42"));
    }

    #[test]
    fn ring_fact_type_duplicate_role_names_are_disambiguated() {
        // Real-world hazard surfaced against apps/tasks: the metamodel
        // includes ring FTs like `Theme_is_the_default_Theme` whose
        // roles are both named "Theme". Pre-fix: SQLite refused
        // `CREATE TABLE … (Theme TEXT, Theme TEXT)` with "duplicate
        // column name". The materializer now appends `_<position>` to
        // collisions: roles `[Theme, Theme]` become columns
        // `[Theme, Theme_2]`, and positional inserts put the right
        // value in each slot.
        let state = state_with(
            &[("Theme_is_the_default_Theme", &["Theme", "Theme"])],
            &[("Theme_is_the_default_Theme", &[
                &[("Theme", "dark"), ("Theme", "light")],
                &[("Theme", "light"), ("Theme", "dark")],
            ])],
        );
        let env = sql_query(&state,
            r#"SELECT "Theme" AS a, "Theme_2" AS b FROM ft_Theme_is_the_default_Theme ORDER BY a"#);
        let rows = parse_rows(&env);
        assert_eq!(rows.len(), 2, "envelope: {}", env);
        assert_eq!(rows[0].get("a").and_then(|v| v.as_str()), Some("dark"));
        assert_eq!(rows[0].get("b").and_then(|v| v.as_str()), Some("light"));
        assert_eq!(rows[1].get("a").and_then(|v| v.as_str()), Some("light"));
        assert_eq!(rows[1].get("b").and_then(|v| v.as_str()), Some("dark"));
    }

    #[test]
    fn map_keyed_cell_projects_to_sql_table() {
        // Regression for task-922-sql-projector-map-cells: Map-keyed
        // cells (RMAP storage; #744) populated via cell_put_keyed
        // MUST surface their facts in the SQL projection. Pre-fix the
        // projector called `contents.as_seq()` which returned None for
        // Object::Map and silently produced an empty `ft_<name>` table
        // even though the cell was populated. The bridge derivation
        // (task-922-sql-projection-rolemismatch / commit 9c446787) emits
        // FT-aligned bindings into Map cells; the apps/tasks
        // `Task_has_Task_Status` cell carried 660 entries while
        // `SELECT COUNT(*) FROM ft_Task_has_Task_Status` returned 0.
        // After this fix the SQL count must equal the cell's fact count.

        // Build a Map-shape Task_has_Task_Status cell via cell_put_keyed
        // — same path the bridge derivation takes for downstream cells.
        let mut state = Object::phi();
        state = cell_push("FactType", fact_from_pairs(&[("id", "Task_has_Task_Status")]), &state);
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

        // Confirm the cell really is Map-shaped — defends against a
        // future refactor flipping cell_put_keyed back to Seq storage,
        // which would silently make the test pass for the wrong reason.
        let contents = ast::fetch_or_phi("Task_has_Task_Status", &state);
        assert!(matches!(contents, Object::Map(_)),
            "fixture must produce Map-shape contents to pin the regression");

        let env = sql_query(&state,
            r#"SELECT COUNT(*) AS n FROM ft_Task_has_Task_Status"#);
        let rows = parse_rows(&env);
        assert_eq!(rows.len(), 1, "envelope: {}", env);
        let n = rows[0].get("n").and_then(|v| v.as_i64()).unwrap_or(-1);
        assert_eq!(n, 3, "Map-keyed cell must project all 3 facts; envelope: {}", env);

        // Round-trip a value to make sure the position-mapped insert
        // also carries the role binding correctly out of Map storage.
        let env = sql_query(&state, r#"
            SELECT "Task_Status" AS s
            FROM ft_Task_has_Task_Status
            WHERE "Task" = 't-2'
        "#);
        let rows = parse_rows(&env);
        assert_eq!(rows.len(), 1, "envelope: {}", env);
        assert_eq!(rows[0].get("s").and_then(|v| v.as_str()), Some("done"));
    }

    #[test]
    fn map_keyed_factype_and_role_cells_still_resolve() {
        // Sibling fix audit: collect_fact_type_ids and collect_role_map
        // both used to call `cell.as_seq()` directly on the FactType /
        // Role cells. If either of THOSE cells is Map-shaped (a real
        // possibility once metamodel cells migrate to keyed storage)
        // the projector would lose the FT id catalog or the column
        // map, and every downstream `ft_<name>` table would either
        // disappear or fall back to the bare-binding column inference.
        // This test pins both via Map storage.
        let mut state = Object::phi();
        state = cell_put_keyed(
            "FactType",
            &["id"],
            fact_from_pairs(&[("id", "Task_has_Task_Priority")]),
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

        // Pin the storage shape — sibling regression must fail loudly
        // if FactType/Role flip back to Seq.
        assert!(matches!(ast::fetch_or_phi("FactType", &state), Object::Map(_)),
            "FactType fixture must be Map-shape");
        assert!(matches!(ast::fetch_or_phi("Role", &state), Object::Map(_)),
            "Role fixture must be Map-shape");

        let env = sql_query(&state,
            r#"SELECT "Task", "Task_Priority" AS p FROM ft_Task_has_Task_Priority"#);
        let rows = parse_rows(&env);
        assert_eq!(rows.len(), 1, "envelope: {}", env);
        assert_eq!(rows[0].get("Task").and_then(|v| v.as_str()), Some("1"));
        assert_eq!(rows[0].get("p").and_then(|v| v.as_str()), Some("p0"));
    }

    #[test]
    fn cells_outside_factype_are_skipped() {
        // A non-FT cell (housekeeping, e.g. `_loaded_reading:foo`)
        // should NOT be promoted to a SQL table even if its name
        // happens to look like one.
        let state = state_with(
            &[("Task_has_Task_Priority", &["Task", "Task Priority"])],
            &[
                ("Task_has_Task_Priority", &[&[("Task", "1"), ("Task Priority", "p0")]]),
            ],
        );
        // The materializer succeeded only for Task_has_Task_Priority.
        // Querying any cell name not in FactType should error.
        let env = sql_query(&state, "SELECT * FROM ft_FactType");
        assert!(parse_error(&env).to_lowercase().contains("no such table"));
    }
}

// `arest reload <file.md>` — CLI subcommand for runtime reading load (#561).
//
// Reads the markdown body from disk, derives the reading name from the
// file stem (so `categories.md` → name `categories`), and routes through
// `crate::load_reading_core::load_reading` with `LoadReadingPolicy::AllowAll`.
//
// The pure dispatch (`dispatch_with_state`) takes the existing cell graph
// directly and returns the new graph alongside the exit code, so unit
// tests can assert on the merged state without touching SQLite. The DB
// wrapper (`dispatch`) opens the configured database, loads state,
// invokes `dispatch_with_state`, then persists on success.
//
// Exit codes:
//   * 0 — load succeeded; the report is on stdout, the new state has
//         been returned to the caller (and persisted by `dispatch`).
//   * 1 — read failure, empty body, parse error, or constraint violation.
//   * 2 — usage error (no file path supplied).
//
// `arest watch <dir>` is a follow-up commit; the per-file load logic
// lives here so the watcher will share `dispatch_with_state` directly.

use crate::ast::Object;
use crate::load_reading_core::{
    load_reading, LoadError, LoadReadingPolicy, LoadReport,
};
use std::io::Write;
use std::path::Path;

/// Pure dispatch: takes the current state, returns `(exit_code, new_state)`.
///
/// `new_state` equals `state.clone()` on any non-success path so the caller
/// can unconditionally proceed without branching on success/failure for
/// state replacement — they only need to persist when `exit_code == 0`.
pub fn dispatch_with_state<O: Write, E: Write>(
    args: &[String],
    state: &Object,
    out: &mut O,
    err: &mut E,
) -> (i32, Object) {
    let path = match args.first() {
        Some(p) => Path::new(p),
        None => {
            let _ = writeln!(err, "{}", usage_text());
            return (2, state.clone());
        }
    };

    let body = match std::fs::read_to_string(path) {
        Ok(b) => b,
        Err(e) => {
            let _ = writeln!(err, "Failed to read {}: {}", path.display(), e);
            return (1, state.clone());
        }
    };

    let name = reading_name_from_path(path);

    match load_reading(state, &name, &body, LoadReadingPolicy::AllowAll) {
        Ok(outcome) => {
            let _ = write!(out, "{}", format_success(&name, &outcome.report));
            (0, outcome.new_state)
        }
        Err(load_err) => {
            let _ = writeln!(err, "{}", format_failure(&name, &load_err));
            (1, state.clone())
        }
    }
}

/// File stem with the `.md` extension stripped — e.g. `readings/foo.md` →
/// `foo`. Mirrors the bake-time naming convention `metamodel_readings()`
/// uses (`("core", core_md_text)`), so a reload of `core.md` produces
/// the same `name` the bake-time path would.
pub fn reading_name_from_path(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| path.display().to_string())
}

pub fn usage_text() -> &'static str {
    "Usage: arest-cli reload <file.md>\n\
     \n\
     Re-applies a single FORML 2 reading to the persisted state.\n\
     The reading name is the file stem (categories.md → 'categories').\n\
     \n\
     Equivalent to `arest <dir>` in the bake-time path, but applied to\n\
     a single file at runtime via SystemVerb::LoadReading."
}

/// Single-line success summary plus per-cell-class growth. Format:
/// ```text
/// reload <name>: +N nouns, +M fact types, +K derivations
/// ```
/// `+0` lines are omitted to keep the line short for the common
/// idempotent re-load case.
pub fn format_success(name: &str, report: &LoadReport) -> String {
    let mut parts: Vec<String> = Vec::new();
    if !report.added_nouns.is_empty() {
        parts.push(format!("+{} nouns", report.added_nouns.len()));
    }
    if !report.added_fact_types.is_empty() {
        parts.push(format!("+{} fact types", report.added_fact_types.len()));
    }
    if !report.added_derivations.is_empty() {
        parts.push(format!("+{} derivations", report.added_derivations.len()));
    }
    let suffix = if parts.is_empty() {
        " (no new cells — body is idempotent against current state)".to_string()
    } else {
        format!(": {}", parts.join(", "))
    };
    format!("reload {}{}\n", name, suffix)
}

pub fn format_failure(name: &str, err: &LoadError) -> String {
    match err {
        LoadError::Disallowed => format!("reload {} rejected: runtime LoadReading is disabled.", name),
        LoadError::EmptyBody => format!("reload {} rejected: body is empty.", name),
        LoadError::InvalidName(why) => format!("reload {} rejected: invalid name — {}", name, why),
        LoadError::ParseError(msg) => format!("reload {} parse error: {}", name, msg),
        LoadError::DeonticViolation(diags) => {
            let mut out = format!("reload {} rejected: {} constraint violation(s).", name, diags.len());
            for d in diags.iter().take(5) {
                out.push_str(&format!("\n  {:?}", d));
            }
            out
        }
        // #559 / DynRdg-5: alethic violations from the load-time
        // validation gate (parse / resolve errors against the merged
        // state). Same render shape as DeonticViolation; different
        // top line so the operator sees which gate fired.
        LoadError::AlethicViolation(diags) => {
            let mut out = format!("reload {} rejected: {} structural violation(s).", name, diags.len());
            for d in diags.iter().take(5) {
                out.push_str(&format!("\n  {:?}", d));
            }
            out
        }
    }
}

/// DB-backed wrapper. Opens the configured SQLite database, loads the
/// persisted state, threads through `dispatch_with_state`, and persists
/// the new state on success. Returns the process exit code only — the
/// new state lives in the database after this returns.
#[cfg(feature = "local")]
pub fn dispatch<O: Write, E: Write>(
    args: &[String],
    db_path: &str,
    out: &mut O,
    err: &mut E,
) -> i32 {
    use rusqlite::Connection;
    let conn = match Connection::open(db_path) {
        Ok(c) => c,
        Err(e) => {
            let _ = writeln!(err, "Failed to open database {}: {}", db_path, e);
            return 1;
        }
    };
    if let Err(e) = conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS cells (name TEXT PRIMARY KEY, contents TEXT);
         CREATE TABLE IF NOT EXISTS defs (name TEXT PRIMARY KEY, func TEXT);"
    ) {
        let _ = writeln!(err, "Failed to ensure tables: {}", e);
        return 1;
    }
    let state = load_state_from_conn(&conn);

    let (code, new_state) = dispatch_with_state(args, &state, out, err);
    if code == 0 {
        if let Err(e) = persist_state_to_conn(&conn, &new_state) {
            let _ = writeln!(err, "Failed to persist new state: {}", e);
            return 1;
        }
    }
    code
}

#[cfg(feature = "local")]
fn load_state_from_conn(conn: &rusqlite::Connection) -> Object {
    let mut state = Object::phi();
    let mut stmt = match conn.prepare("SELECT name, contents FROM cells") {
        Ok(s) => s,
        Err(_) => return state,
    };
    let rows = stmt.query_map([], |row| {
        let name: String = row.get(0)?;
        let contents: String = row.get(1)?;
        Ok((name, contents))
    });
    if let Ok(iter) = rows {
        for r in iter.flatten() {
            let (name, json) = r;
            // Object::parse infallibly returns Object::Bottom on
            // malformed input, so a corrupt cell row simply lands as
            // Bottom in that slot rather than crashing the load.
            let parsed = crate::ast::Object::parse(&json);
            state = crate::ast::store(&name, parsed, &state);
        }
    }
    state
}

#[cfg(feature = "local")]
fn persist_state_to_conn(conn: &rusqlite::Connection, d: &Object) -> Result<(), rusqlite::Error> {
    use rusqlite::params;
    let tx = conn.unchecked_transaction()?;
    for (name, contents) in crate::ast::cells_iter(d) {
        let is_def = name.contains(':')
            || ["compile", "apply", "verify_signature", "validate", "debug"].contains(&name);
        if is_def {
            tx.execute(
                "INSERT OR REPLACE INTO defs (name, func) VALUES (?1, ?2)",
                params![name, contents.to_string()],
            )?;
        } else if !["validate", "compile", "apply", "verify_signature", "debug", "_defs_compiled"]
            .contains(&name)
        {
            tx.execute(
                "INSERT OR REPLACE INTO cells (name, contents) VALUES (?1, ?2)",
                params![name, contents.to_string()],
            )?;
        }
    }
    tx.commit()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{self, Object};

    /// Mirror of load_reading_core::tests::seed_state — minimal cell
    /// graph the parse path can fold against.
    fn seed_state() -> Object {
        let nouns = ast::Object::seq(vec![ast::fact_from_pairs(&[
            ("name", "Order"),
            ("objectType", "entity"),
        ])]);
        ast::store("Noun", nouns, &Object::phi())
    }

    fn write_temp(label: &str, body: &str) -> std::path::PathBuf {
        use std::io::Write as _;
        let dir = std::env::temp_dir().join("arest-reload-tests");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join(format!("{}-{}.md",
            label,
            std::process::id()));
        let mut f = std::fs::File::create(&path).expect("create temp");
        f.write_all(body.as_bytes()).expect("write temp");
        path
    }

    #[test]
    fn dispatch_missing_arg_returns_usage_code_2() {
        let state = seed_state();
        let mut out: Vec<u8> = Vec::new();
        let mut err: Vec<u8> = Vec::new();
        let (code, new) = dispatch_with_state(&[], &state, &mut out, &mut err);
        assert_eq!(code, 2);
        assert_eq!(new, state, "missing-arg path must not mutate state");
        let err_text = String::from_utf8(err).unwrap();
        assert!(err_text.contains("Usage: arest-cli reload"), "got: {}", err_text);
    }

    #[test]
    fn dispatch_nonexistent_file_returns_1_and_read_error() {
        let state = seed_state();
        let path = std::env::temp_dir().join("arest-reload-tests/this-file-does-not-exist.md");
        let mut out: Vec<u8> = Vec::new();
        let mut err: Vec<u8> = Vec::new();
        let (code, new) = dispatch_with_state(
            &[path.to_string_lossy().to_string()], &state, &mut out, &mut err);
        assert_eq!(code, 1);
        assert_eq!(new, state);
        let err_text = String::from_utf8(err).unwrap();
        assert!(err_text.contains("Failed to read"), "got: {}", err_text);
    }

    #[test]
    fn dispatch_empty_file_returns_1_and_empty_body_error() {
        let state = seed_state();
        let path = write_temp("empty", "");
        let mut out: Vec<u8> = Vec::new();
        let mut err: Vec<u8> = Vec::new();
        let (code, new) = dispatch_with_state(
            &[path.to_string_lossy().to_string()], &state, &mut out, &mut err);
        assert_eq!(code, 1);
        assert_eq!(new, state);
        let err_text = String::from_utf8(err).unwrap();
        assert!(err_text.contains("body is empty"), "got: {}", err_text);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn dispatch_valid_reading_returns_0_and_grown_state() {
        let state = seed_state();
        let body = "\
Product(.SKU) is an entity type.
Category(.Name) is an entity type.
";
        let path = write_temp("valid", body);
        let mut out: Vec<u8> = Vec::new();
        let mut err: Vec<u8> = Vec::new();
        let (code, new) = dispatch_with_state(
            &[path.to_string_lossy().to_string()], &state, &mut out, &mut err);
        assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&err));
        // New state must contain Order + Product + Category nouns.
        let nouns_after = ast::fetch_or_phi("Noun", &new);
        let names: Vec<&str> = nouns_after.as_seq()
            .map(|s| s.iter().filter_map(|f| ast::binding(f, "name")).collect())
            .unwrap_or_default();
        assert!(names.contains(&"Order"), "Order should still be present, got: {:?}", names);
        assert!(names.contains(&"Product"), "Product should be added, got: {:?}", names);
        assert!(names.contains(&"Category"), "Category should be added, got: {:?}", names);
        // Stdout report announces the growth.
        let out_text = String::from_utf8(out).unwrap();
        let expected_name = path.file_stem().unwrap().to_string_lossy().to_string();
        assert!(out_text.contains(&format!("reload {}", expected_name)),
            "got: {}", out_text);
        assert!(out_text.contains("+2 nouns"), "got: {}", out_text);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn dispatch_invalid_reading_returns_1_and_parse_error() {
        let state = seed_state();
        // Reserved-keyword noun declaration triggers the hard-error path
        // (#309) inside parse_to_state_from. Mirrors the
        // `malformed_forml_yields_parse_error` case in load_reading_core.
        let path = write_temp("bad", "each(.X) is an entity type.\n");
        let mut out: Vec<u8> = Vec::new();
        let mut err: Vec<u8> = Vec::new();
        let (code, new) = dispatch_with_state(
            &[path.to_string_lossy().to_string()], &state, &mut out, &mut err);
        assert_eq!(code, 1, "stdout: {}, stderr: {}",
            String::from_utf8_lossy(&out), String::from_utf8_lossy(&err));
        assert_eq!(new, state, "parse-error path must not mutate state");
        let err_text = String::from_utf8(err).unwrap();
        assert!(err_text.contains("parse error"), "got: {}", err_text);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn reading_name_strips_md_extension() {
        let p = std::path::Path::new("/some/dir/categories.md");
        assert_eq!(reading_name_from_path(p), "categories");
    }

    #[test]
    fn reading_name_strips_any_extension() {
        let p = std::path::Path::new("foo.bar");
        assert_eq!(reading_name_from_path(p), "foo");
    }

    #[test]
    fn format_success_omits_zero_buckets() {
        // Minimal explicit-field init; #558 added content_hash /
        // version_stamp — using `..Default::default()` here keeps
        // future field additions to `LoadReport` from forcing
        // another edit to this CLI test.
        let report = LoadReport {
            added_nouns: vec!["A".to_string(), "B".to_string()],
            ..Default::default()
        };
        let s = format_success("test", &report);
        assert_eq!(s, "reload test: +2 nouns\n");
    }

    #[test]
    fn format_success_lists_all_three_buckets() {
        let report = LoadReport {
            added_nouns: vec!["A".to_string()],
            added_fact_types: vec!["B".to_string(), "C".to_string()],
            added_derivations: vec!["D".to_string(), "E".to_string(), "F".to_string()],
            ..Default::default()
        };
        let s = format_success("test", &report);
        assert!(s.contains("+1 nouns"));
        assert!(s.contains("+2 fact types"));
        assert!(s.contains("+3 derivations"));
    }

    #[test]
    fn format_success_empty_report_signals_idempotence() {
        let report = LoadReport::default();
        let s = format_success("test", &report);
        assert!(s.contains("idempotent"), "got: {}", s);
    }
}

// `arest watch <dir>` — CLI subcommand for polling a directory for `.md`
// changes and re-applying each modified file via the same
// `LoadReading` pipeline that `arest reload <file.md>` uses (#561 /
// DynRdg-T2 second half; first half landed in c1a5ed5c).
//
// Polling-only — we deliberately don't pull in `notify`. The
// subcommand is a developer convenience for "edit a reading, see the
// state update without restarting"; throughput is bounded by human
// edit speed, so a 500ms `read_dir + metadata().modified()` sweep is
// adequate. Keeps the dependency surface (and the Cargo.lock churn)
// small.
//
// Like `cli::reload`, the testable core is the pure
// `scan_once_with_state` — it walks a directory's `*.md` files once,
// threads each through `reload::dispatch_with_state`, aggregates the
// worst exit code seen, and returns the post-scan state. The watch
// loop (`watch_loop_with_state`) and the DB-backed `dispatch` wrap
// that core; tests only exercise the pure scan path so the suite
// stays under a second and the infinite-loop path stays untested
// (intentionally — there's nothing to assert after `loop {}`).
//
// Top-level only for v1: nested directories are skipped. The
// metamodel-compile path (`arest <dir>`) recurses, but for runtime
// reload-on-edit a flat watched directory is the common case (a
// readings folder under active edit). Recursion can land later
// without breaking the API — the test fixtures pass a flat dir and
// don't assert on nesting either way.
//
// Transactional model on mixed-success scans: per-file. A successful
// reload inside a scan that also includes a failing reload still
// writes its state forward; the failing file leaves the state at its
// pre-failure value (matching `dispatch_with_state`'s "on error,
// return state.clone()" semantics). The aggregate exit code is the
// worst seen (1 if any file fails, 0 if all succeed). This matches
// the reload-per-file mental model — each file is its own atomic
// load — rather than scan-as-transaction. Documented in the test
// `scan_once_mixed_keeps_valid_changes`.
//
// Exit codes:
//   * 0 — all `*.md` in the dir loaded successfully (or the dir was
//         empty); state is the merged result, persisted by `dispatch`.
//   * 1 — read failure on the dir, OR at least one `*.md` failed to
//         load. Successfully-loaded files inside a mixed scan still
//         have their state changes persisted.
//   * 2 — usage error (no dir path supplied).

use crate::ast::Object;
use crate::cli::reload;
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Pure dispatch: scan `dir` once for `*.md` files, thread each
/// through `reload::dispatch_with_state`, return `(worst_exit_code,
/// new_state)`.
///
/// Top-level only — subdirectories are ignored. Files are visited in
/// alphabetical order so the per-file reload report is deterministic
/// across platforms (Linux's `read_dir` is unordered without an
/// explicit sort).
pub fn scan_once_with_state<O: Write, E: Write>(
    dir: &Path,
    state: &Object,
    out: &mut O,
    err: &mut E,
) -> (i32, Object) {
    let entries = match collect_md_files(dir) {
        Ok(v) => v,
        Err(e) => {
            let _ = writeln!(err, "Failed to read {}: {}", dir.display(), e);
            return (1, state.clone());
        }
    };

    if entries.is_empty() {
        let _ = writeln!(out, "watch {}: 0 .md files found", dir.display());
        return (0, state.clone());
    }

    let mut current = state.clone();
    let mut worst: i32 = 0;
    for path in entries {
        let arg = path.to_string_lossy().to_string();
        let (code, next) = reload::dispatch_with_state(
            &[arg], &current, out, err);
        // Per-file transactional model: dispatch_with_state already
        // returns `state.clone()` on failure, so the assignment is
        // unconditional and a failing file leaves `current`
        // untouched while a succeeding sibling threads its diff
        // forward.
        current = next;
        if code > worst { worst = code; }
    }
    (worst, current)
}

/// Top-level `*.md` listing, alphabetically sorted. Returns an
/// `io::Error` if the directory itself can't be opened (the dir
/// missing or unreadable is a hard error; a dir with zero `*.md`
/// children is a non-error empty `Vec`).
fn collect_md_files(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut out: Vec<PathBuf> = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_file()
            && path.extension().and_then(|e| e.to_str()) == Some("md")
        {
            out.push(path);
        }
    }
    out.sort();
    Ok(out)
}

pub fn usage_text() -> &'static str {
    "Usage: arest-cli watch <dir>\n\
     \n\
     Polls a directory for `.md` file changes and re-applies each\n\
     modified file via the same LoadReading pipeline as\n\
     `arest reload <file.md>`. Top-level only (no recursion in v1).\n\
     \n\
     Runs until interrupted (Ctrl-C). Sleeps 500ms between scans.\n\
     The reading name for each file is its file stem\n\
     (categories.md → 'categories')."
}

/// Snapshot of `(path → mtime)` for every `*.md` in `dir`. Files
/// without a readable mtime are omitted (we treat unknown-mtime as
/// "no change" — re-running a load that already succeeded is cheap
/// and safe via `LoadReadingPolicy::AllowAll`'s idempotence).
fn snapshot_mtimes(dir: &Path) -> HashMap<PathBuf, SystemTime> {
    let mut out: HashMap<PathBuf, SystemTime> = HashMap::new();
    let entries = match collect_md_files(dir) {
        Ok(v) => v,
        Err(_) => return out,
    };
    for path in entries {
        if let Ok(meta) = std::fs::metadata(&path) {
            if let Ok(mt) = meta.modified() {
                out.insert(path, mt);
            }
        }
    }
    out
}

/// Polling watch loop. Performs an initial scan, then enters a
/// 500ms-sleep loop, re-running `dispatch_with_state` only for files
/// whose mtime has advanced since the last seen value (or for
/// newly-created `*.md`).
///
/// `on_state_change` is invoked exactly once per successful reload
/// of a file whose mtime moved forward, with the new merged state.
/// The DB-backed `dispatch` uses this to persist after each
/// individual reload (so a long-running watch session doesn't lose
/// hours of edits if interrupted).
///
/// Returns `!`-shaped: there's no clean exit (Ctrl-C / SIGTERM
/// terminates the process). Untested by design — see the module
/// header.
#[allow(dead_code)]
pub fn watch_loop_with_state<O: Write, E: Write, F: FnMut(&Object)>(
    dir: &Path,
    initial_state: Object,
    out: &mut O,
    err: &mut E,
    mut on_state_change: F,
) -> ! {
    // Initial scan: load every `*.md` in the dir against the seed
    // state, persist once, then transition into change-only mode.
    let (_code, mut current) = scan_once_with_state(dir, &initial_state, out, err);
    on_state_change(&current);
    let mut seen = snapshot_mtimes(dir);

    loop {
        std::thread::sleep(Duration::from_millis(500));
        let now = snapshot_mtimes(dir);
        // Files whose mtime moved forward (or that didn't exist at
        // the previous tick).
        let changed: Vec<PathBuf> = now.iter()
            .filter(|(p, mt)| match seen.get(*p) {
                Some(prev) => *mt > prev,
                None => true,
            })
            .map(|(p, _)| p.clone())
            .collect();

        if !changed.is_empty() {
            for path in changed {
                let arg = path.to_string_lossy().to_string();
                let (code, next) = reload::dispatch_with_state(
                    &[arg], &current, out, err);
                current = next;
                if code == 0 {
                    on_state_change(&current);
                }
            }
        }
        seen = now;
    }
}

/// DB-backed wrapper. Opens the configured SQLite database, loads
/// the persisted state, runs an initial `scan_once_with_state` +
/// persists the merged state, then enters `watch_loop_with_state`
/// with a per-reload persist callback. Mirrors `reload::dispatch`.
///
/// Returns the process exit code only — but in practice this only
/// returns when the initial scan can't even start (missing dir,
/// missing arg, etc.). The watch loop itself runs until SIGTERM.
#[cfg(feature = "local")]
pub fn dispatch<O: Write, E: Write>(
    args: &[String],
    db_path: &str,
    out: &mut O,
    err: &mut E,
) -> i32 {
    let dir_path = match args.first() {
        Some(p) => PathBuf::from(p),
        None => {
            let _ = writeln!(err, "{}", usage_text());
            return 2;
        }
    };
    if !dir_path.is_dir() {
        let _ = writeln!(err, "Not a directory: {}", dir_path.display());
        return 1;
    }

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

    let (code, scanned) = scan_once_with_state(&dir_path, &state, out, err);
    if let Err(e) = persist_state_to_conn(&conn, &scanned) {
        let _ = writeln!(err, "Failed to persist initial scan state: {}", e);
        return 1;
    }
    let _ = writeln!(out, "watch {}: initial scan complete (exit={}); entering loop",
        dir_path.display(), code);

    // Enter the polling loop. `watch_loop_with_state` returns `!`,
    // so the only exit from here is SIGTERM. We hand it a persist
    // callback so each successful reload writes through to disk.
    watch_loop_with_state(&dir_path, scanned, out, err, |new_state| {
        if let Err(e) = persist_state_to_conn(&conn, new_state) {
            // Persist failures don't abort the loop — log and keep
            // watching so a transient I/O hiccup doesn't lose the
            // session.
            let _ = writeln!(std::io::stderr(),
                "Warning: failed to persist after reload: {}", e);
        }
    });
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

    /// Mirror of `cli::reload::tests::seed_state`.
    fn seed_state() -> Object {
        let nouns = ast::Object::seq(vec![ast::fact_from_pairs(&[
            ("name", "Order"),
            ("objectType", "entity"),
        ])]);
        ast::store("Noun", nouns, &Object::phi())
    }

    /// Allocate a fresh per-test fixture directory under
    /// `temp_dir()/arest-watch-tests/<label>-<pid>`. The PID suffix
    /// keeps parallel `cargo test` runs (and concurrent CI shards
    /// against the same tmp) from stomping each other; the label
    /// disambiguates tests inside a single run. Returns the absolute
    /// directory path; caller is responsible for `remove_dir_all` at
    /// the end of the test.
    fn fresh_dir(label: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir()
            .join("arest-watch-tests")
            .join(format!("{}-{}", label, std::process::id()));
        // Clean any leftover from a prior test failure so we start
        // empty.
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create fixture dir");
        dir
    }

    fn write_md(dir: &Path, stem: &str, body: &str) -> std::path::PathBuf {
        use std::io::Write as _;
        let path = dir.join(format!("{}.md", stem));
        let mut f = std::fs::File::create(&path).expect("create md");
        f.write_all(body.as_bytes()).expect("write md");
        path
    }

    #[test]
    fn dispatch_missing_dir_arg_returns_usage_code_2() {
        // Pure helper isn't reachable for the missing-arg case — that
        // lives in `dispatch` (DB-backed). Verify the usage_text
        // string itself reflects the watch verb so regressions are
        // visible.
        let usage = usage_text();
        assert!(usage.contains("Usage: arest-cli watch"), "got: {}", usage);
        assert!(usage.contains("<dir>"), "got: {}", usage);
    }

    #[test]
    fn scan_once_nonexistent_dir_returns_1_and_read_error() {
        let state = seed_state();
        let dir = std::env::temp_dir()
            .join("arest-watch-tests")
            .join(format!("does-not-exist-{}", std::process::id()));
        // Make sure it really doesn't exist.
        let _ = std::fs::remove_dir_all(&dir);
        let mut out: Vec<u8> = Vec::new();
        let mut err: Vec<u8> = Vec::new();
        let (code, new) = scan_once_with_state(&dir, &state, &mut out, &mut err);
        assert_eq!(code, 1);
        assert_eq!(new, state, "read-failure path must not mutate state");
        let err_text = String::from_utf8(err).unwrap();
        assert!(err_text.contains("Failed to read"), "got: {}", err_text);
    }

    #[test]
    fn scan_once_empty_dir_returns_0_and_zero_files_message() {
        let dir = fresh_dir("empty");
        let state = seed_state();
        let mut out: Vec<u8> = Vec::new();
        let mut err: Vec<u8> = Vec::new();
        let (code, new) = scan_once_with_state(&dir, &state, &mut out, &mut err);
        assert_eq!(code, 0);
        assert_eq!(new, state, "empty-dir scan must not mutate state");
        let out_text = String::from_utf8(out).unwrap();
        assert!(out_text.contains("0 .md files"), "got: {}", out_text);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scan_once_skips_non_md_files() {
        let dir = fresh_dir("non-md");
        // Write a non-md file alongside; it must be ignored.
        use std::io::Write as _;
        let txt = dir.join("notes.txt");
        std::fs::File::create(&txt).unwrap()
            .write_all(b"this is not a reading\n").unwrap();
        let state = seed_state();
        let mut out: Vec<u8> = Vec::new();
        let mut err: Vec<u8> = Vec::new();
        let (code, new) = scan_once_with_state(&dir, &state, &mut out, &mut err);
        assert_eq!(code, 0);
        assert_eq!(new, state);
        let out_text = String::from_utf8(out).unwrap();
        assert!(out_text.contains("0 .md files"), "got: {}", out_text);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scan_once_single_valid_md_returns_0_and_grown_state() {
        let dir = fresh_dir("single");
        write_md(&dir, "products", "\
Product(.SKU) is an entity type.
");
        let state = seed_state();
        let mut out: Vec<u8> = Vec::new();
        let mut err: Vec<u8> = Vec::new();
        let (code, new) = scan_once_with_state(&dir, &state, &mut out, &mut err);
        assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&err));
        let nouns_after = ast::fetch_or_phi("Noun", &new);
        let names: Vec<&str> = nouns_after.as_seq()
            .map(|s| s.iter().filter_map(|f| ast::binding(f, "name")).collect())
            .unwrap_or_default();
        assert!(names.contains(&"Order"), "Order should still be present, got: {:?}", names);
        assert!(names.contains(&"Product"), "Product should be added, got: {:?}", names);
        let out_text = String::from_utf8(out).unwrap();
        // The reload report mentions the file stem.
        assert!(out_text.contains("reload products"), "got: {}", out_text);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scan_once_multiple_valid_md_threads_state_forward() {
        let dir = fresh_dir("multi");
        // Use distinct noun names so the two files compose cleanly.
        write_md(&dir, "a-products", "Product(.SKU) is an entity type.\n");
        write_md(&dir, "b-categories", "Category(.Name) is an entity type.\n");
        let state = seed_state();
        let mut out: Vec<u8> = Vec::new();
        let mut err: Vec<u8> = Vec::new();
        let (code, new) = scan_once_with_state(&dir, &state, &mut out, &mut err);
        assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&err));
        let nouns_after = ast::fetch_or_phi("Noun", &new);
        let names: Vec<&str> = nouns_after.as_seq()
            .map(|s| s.iter().filter_map(|f| ast::binding(f, "name")).collect())
            .unwrap_or_default();
        assert!(names.contains(&"Order"));
        assert!(names.contains(&"Product"));
        assert!(names.contains(&"Category"),
            "Category should be threaded forward from the second file, got: {:?}", names);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scan_once_mixed_keeps_valid_changes() {
        // Documents the per-file transactional model: valid files
        // commit their state changes; the failing file leaves state
        // at its pre-failure value; aggregate exit code is the
        // worst seen.
        let dir = fresh_dir("mixed");
        // Alphabetical visit order ensures the valid file runs
        // first; both orders must yield the same final state, but
        // we want a deterministic out-stream for assertions.
        write_md(&dir, "a-good", "Product(.SKU) is an entity type.\n");
        // Reserved-keyword noun → parse error (#309), same trigger
        // reload's parse-error test uses.
        write_md(&dir, "b-bad", "each(.X) is an entity type.\n");
        let state = seed_state();
        let mut out: Vec<u8> = Vec::new();
        let mut err: Vec<u8> = Vec::new();
        let (code, new) = scan_once_with_state(&dir, &state, &mut out, &mut err);
        assert_eq!(code, 1, "worst-case exit must surface from the failing file");
        // Valid file's state change MUST persist.
        let nouns_after = ast::fetch_or_phi("Noun", &new);
        let names: Vec<&str> = nouns_after.as_seq()
            .map(|s| s.iter().filter_map(|f| ast::binding(f, "name")).collect())
            .unwrap_or_default();
        assert!(names.contains(&"Product"),
            "valid file's state change must persist across a mixed scan, got: {:?}", names);
        // Stderr surfaces the parse-error from the bad file.
        let err_text = String::from_utf8(err).unwrap();
        assert!(err_text.contains("parse error"),
            "stderr should mention parse error, got: {}", err_text);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scan_once_visits_files_in_alphabetical_order() {
        // Determinism guard: cross-platform `read_dir` is
        // unordered. We sort inside `collect_md_files` so the per-
        // file reload reports come out in the same order on every
        // platform; the multi-file state-threading test relies on
        // this.
        let dir = fresh_dir("order");
        write_md(&dir, "z-last", "ZLast(.X) is an entity type.\n");
        write_md(&dir, "a-first", "AFirst(.X) is an entity type.\n");
        let state = seed_state();
        let mut out: Vec<u8> = Vec::new();
        let mut err: Vec<u8> = Vec::new();
        let (code, _) = scan_once_with_state(&dir, &state, &mut out, &mut err);
        assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&err));
        let out_text = String::from_utf8(out).unwrap();
        let a_pos = out_text.find("reload a-first").unwrap_or(usize::MAX);
        let z_pos = out_text.find("reload z-last").unwrap_or(usize::MAX);
        assert!(a_pos < z_pos,
            "a-first must report before z-last, got:\n{}", out_text);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn collect_md_files_skips_subdirectories() {
        // V1 is top-level only — nested dirs must not appear in the
        // returned list, and a `*.md` file *inside* a subdirectory
        // must not be picked up.
        let dir = fresh_dir("nested");
        write_md(&dir, "top", "Product(.SKU) is an entity type.\n");
        let sub = dir.join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        write_md(&sub, "buried", "Buried(.X) is an entity type.\n");
        let files = collect_md_files(&dir).expect("read dir");
        assert_eq!(files.len(), 1, "only top.md should be visible, got: {:?}", files);
        assert!(files[0].file_name().unwrap().to_string_lossy().ends_with("top.md"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

// CLI driver — std-only. Extracted from src/main.rs as part of
// #685 (#650b) so the bin no longer compiles every lib module a
// second time. Pre-extract, src/main.rs declared `mod ast;
// mod compile; …` for all 31 lib modules independently of lib.rs's
// `pub mod ast; …`, forcing cargo to recompile each source file
// twice. Profile (cargo-timing 2026-05-01) showed
// `arest-cli "bin" (test)` at 85.2s and `arest-cli "bin"` at 37.2s
// of duplicate cumulative compile.
//
// Post-extract, this file lives inside the lib's compilation unit
// (`pub mod entry;` in `cli/mod.rs`). `crate::ast`, `crate::compile`,
// etc. resolve to the lib's already-compiled modules — no second
// pass over their source. main.rs is now a 6-line shim that calls
// `cli::entry::main_entry()`.
//
// Usage (unchanged from pre-extract):
//   arest-cli <readings_dir> [<readings_dir2> ...] [--db <path>]
//   arest-cli --db <path> <key> <input>
//
// Reads .md files from each directory, feeds them through
// system(h, 'compile', text), then persists state to SQLite.
// Subsequent system calls load state from the database.
//
// Everything goes through SYSTEM. No separate bootstrap, synthesize,
// or forward-chain commands. Per AREST paper: SYSTEM:x = ⟨o, D'⟩.

#[cfg(feature = "local")]
use crate::{ast, compile};
#[cfg(feature = "local")]
use crate::parse_forml2;

// =========================================================================
// SQLite persistence (feature = "local")
// =========================================================================

#[cfg(feature = "local")]
mod db {
    use rusqlite::{Connection, params};
    use crate::ast;

    pub fn open(path: &str) -> Connection {
        Connection::open(path)
            .unwrap_or_else(|e| { eprintln!("Failed to open database {}: {}", path, e); std::process::exit(1); })
    }

    /// Ensure the cells + defs meta-tables exist.
    pub fn ensure_meta_tables(conn: &Connection) {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS cells (name TEXT PRIMARY KEY, contents TEXT);
             CREATE TABLE IF NOT EXISTS defs (name TEXT PRIMARY KEY, func TEXT);"
        ).unwrap_or_else(|e| { eprintln!("Failed to create tables: {}", e); std::process::exit(1); });
    }

    /// Execute DDL from sql:sqlite:* defs.
    ///
    /// rmap-3nf-tables: the generator stores each DDL def as a
    /// `Func::Constant(Object::atom(ddl))` — `func_to_object`-encoded,
    /// so the cell contents is the constant-func WRAPPER, not a bare
    /// atom. The old `.as_atom()` filter therefore matched NOTHING and
    /// every CREATE TABLE was silently skipped — the 3NF RMAP tables
    /// the sqlite generator has been emitting never landed in any app
    /// db (population persisted only into the cells/defs blobs).
    /// Unwrap through `metacompose` and take the constant's atom.
    pub fn apply_ddl(conn: &Connection, d: &ast::Object) {
        let ddl_of = |contents: &ast::Object| -> Option<String> {
            // Bare atom (legacy shape) first, then the encoded
            // constant-func wrapper.
            if let Some(s) = contents.as_atom() {
                return Some(s.to_string());
            }
            match ast::metacompose(contents, d) {
                ast::Func::Constant(obj) => obj.as_atom().map(|s| s.to_string()),
                _ => None,
            }
        };
        // The RMAP tables are a PROJECTION of the cell graph (readings
        // are the source of truth), so a recompile REPLACES them: the
        // generated DDL uses CREATE TABLE IF NOT EXISTS, which would
        // silently keep a stale shape from a prior compile (observed:
        // a fixed column rename never landing because the bad table
        // survived). Drop each projection object first; Stage 1b's row
        // projection re-populates from cells on persist.
        ast::cells_iter(d).into_iter()
            .filter_map(|(name, _)| name.strip_prefix("sql:trigger:"))
            .for_each(|trigger| {
                let _ = conn.execute_batch(
                    &format!("DROP TRIGGER IF EXISTS \"{}\";", trigger.replace('"', "")));
            });
        ast::cells_iter(d).into_iter()
            .filter_map(|(name, _)| name.strip_prefix("sql:sqlite:"))
            .for_each(|table| {
                let _ = conn.execute_batch(
                    &format!("DROP TABLE IF EXISTS \"{}\";", table.replace('"', "")));
            });
        // rmap-3nf-tables Stage 2 (ddl-plan-drift): the PROJECTION PLAN
        // is the authoritative table shape. The sql:sqlite: defs are
        // baked from a mid-compile rmap pass and can lag the plan the
        // row projection computes at persist (live hit: the plan's
        // NORMA collision suffixes produced noun.has_object_type_3
        // while the defs' table stopped at _2 — every noun row
        // warn-skipped "no column named", cascading through
        // state_machine_definition → state_machine). Drop + create
        // every plan table from rmap::create_table_sql FIRST; the defs
        // pass below then only creates NON-plan tables (its CREATE ...
        // IF NOT EXISTS no-ops on plan-covered names) and the triggers
        // attach last, after every table exists.
        // Children-first DROP / parents-first CREATE: with SQLite FK
        // enforcement on, dropping a parent that still has populated
        // referencing children fails — an unsorted drop pass stranded
        // OLD-shaped tables (live hit: noun kept its pre-dedup
        // has_object_type shape, every new-shape insert failed
        // "no column named object_type", cascading through resource /
        // role). The projection plan's Kahn order gives both
        // directions.
        let plan = crate::rmap::projection_plan(d);
        let plan_by_name: hashbrown::HashMap<&str, &crate::rmap::TableDef> =
            plan.tables.iter().map(|t| (t.name.as_str(), t)).collect();
        // DDL reshaping must not be hostage to FK enforcement or
        // populated children: a silently-failing DROP strands the OLD
        // table shape and every new-shape insert fails "no column
        // named" (the live noun.has_object_type_4 fossil). FKs off for
        // the DDL phase only; Phase 4's delete+insert re-establishes
        // row-level consistency under whatever enforcement the
        // connection has. Drop failures are LOUD now.
        let _ = conn.execute_batch("PRAGMA foreign_keys=OFF;");
        for name in plan.order.iter().rev() {
            if let Err(e) = conn.execute_batch(
                &format!("DROP TABLE IF EXISTS {};", crate::rmap::qid(name))) {
                eprintln!("Warning: plan DROP failed for {}: {}", name, e);
            }
        }
        for name in &plan.order {
            let Some(t) = plan_by_name.get(name.as_str()) else { continue };
            conn.execute_batch(&crate::rmap::create_table_sql(t)).unwrap_or_else(|e| {
                eprintln!("Warning: plan DDL failed for {}: {}", t.name, e);
            });
        }
        // rmap-3nf-tables Stage 3 (fossil sweep): drop tables whose NAMES
        // cannot be the output of any current emission convention —
        // prose-leak relics from the pre-gate era (backticks, pipes,
        // dots, commas, spaces, '>' …; arc-agi-3's db carried dozens,
        // and one broke the sql verb's materialize wholesale, issue 11).
        // rmap names and sql:sqlite: def names are [A-Za-z0-9_] by
        // construction, so anything outside that alphabet is provably
        // junk WE minted historically — never a user scratch table worth
        // preserving. Sane-named non-plan tables are LEFT ALONE (they
        // may be user scratch); engine storage (cells/defs), sqlite
        // internals, plan tables, and this compile's def tables are
        // excluded by name. Loud per-drop report.
        {
            let def_tables: hashbrown::HashSet<String> = ast::cells_iter(d)
                .into_iter()
                .filter_map(|(name, _)| name.strip_prefix("sql:sqlite:"))
                .map(|s| s.to_string())
                .collect();
            let fossil_names: Vec<String> = conn
                .prepare("SELECT name FROM sqlite_master WHERE type='table'")
                .and_then(|mut stmt| {
                    let names = stmt.query_map([], |row| row.get::<_, String>(0))?
                        .filter_map(|r| r.ok())
                        .collect::<Vec<String>>();
                    Ok(names)
                })
                .unwrap_or_default()
                .into_iter()
                .filter(|n| n != "cells" && n != "defs" && !n.starts_with("sqlite_"))
                .filter(|n| !plan_by_name.contains_key(n.as_str()))
                .filter(|n| !def_tables.contains(n.as_str()))
                .filter(|n| n.chars().any(|c| !(c.is_ascii_alphanumeric() || c == '_')))
                .collect();
            for name in &fossil_names {
                match conn.execute_batch(
                    &format!("DROP TABLE IF EXISTS {};", crate::rmap::qid(name))) {
                    Ok(()) => eprintln!("[ddl] fossil table dropped (impossible name, \
                                         pre-gate relic): {}", name),
                    Err(e) => eprintln!("Warning: fossil DROP failed for {}: {}", name, e),
                }
            }
        }
        let _ = conn.execute_batch("PRAGMA foreign_keys=ON;");
        // CREATE TABLE from sql:sqlite:* cells
        ast::cells_iter(d).into_iter()
            .filter(|(name, _)| name.starts_with("sql:sqlite:"))
            .filter_map(|(_, contents)| ddl_of(contents))
            .for_each(|ddl| {
                conn.execute_batch(&ddl).unwrap_or_else(|e| {
                    eprintln!("Warning: DDL failed: {}", e);
                });
            });
        // CREATE TRIGGER from sql:trigger:* cells
        ast::cells_iter(d).into_iter()
            .filter(|(name, _)| name.starts_with("sql:trigger:"))
            .filter_map(|(_, contents)| ddl_of(contents))
            .for_each(|ddl| {
                conn.execute_batch(&ddl).unwrap_or_else(|e| {
                    eprintln!("Warning: Trigger failed: {}", e);
                });
            });
    }

    /// Persist the full state D to SQLite.
    pub fn persist_state(conn: &Connection, d: &ast::Object) {
        let tx = conn.unchecked_transaction()
            .unwrap_or_else(|e| { eprintln!("Transaction failed: {}", e); std::process::exit(1); });

        // Replace the persisted snapshot atomically. INSERT OR REPLACE
        // updates cells that still exist, but it cannot remove cells
        // whose facts were retracted or whose derivation rule stopped
        // producing them. Deleting inside the transaction keeps the DB
        // equal to the current cell graph while preserving crash safety.
        tx.execute("DELETE FROM cells", [])
            .unwrap_or_else(|e| { eprintln!("Failed to clear cells: {}", e); std::process::exit(1); });
        tx.execute("DELETE FROM defs", [])
            .unwrap_or_else(|e| { eprintln!("Failed to clear defs: {}", e); std::process::exit(1); });

        // Store population cells only — compiled defs are recomputed
        // on each session start (452ms). Persisting Func trees as display
        // strings is slow to reload (Object::parse on thousands of nested
        // bracket expressions). Population cells are small and fast.
        ast::cells_iter(d).into_iter()
            .filter(|(name, _)| !name.contains(':') && !["validate", "compile", "apply",
                "verify_signature", "debug", "_defs_compiled"].contains(name))
            .for_each(|(name, contents)| {
                let json = contents.to_string();
                tx.execute(
                    "INSERT OR REPLACE INTO cells (name, contents) VALUES (?1, ?2)",
                    params![name, json],
                ).unwrap_or_else(|e| { eprintln!("Failed to store cell {}: {}", name, e); std::process::exit(1); });
            });

        // Store defs.
        ast::cells_iter(d).into_iter()
            .filter(|(name, _)| name.contains(':') || ["compile", "apply", "verify_signature", "validate", "debug"].contains(&name))
            .for_each(|(name, contents)| {
                let text = contents.to_string();
                tx.execute(
                    "INSERT OR REPLACE INTO defs (name, func) VALUES (?1, ?2)",
                    params![name, text],
                ).unwrap_or_else(|e| { eprintln!("Failed to store def {}: {}", name, e); std::process::exit(1); });
            });

        // rmap-3nf-tables Stage 1b: refresh the 3NF projection rows in
        // the same transaction. Cells remain the source of truth.
        project_population_rows(&tx, d);

        tx.commit()
            .unwrap_or_else(|e| { eprintln!("Commit failed: {}", e); std::process::exit(1); });
    }

    /// 987-A.3 increment 1 — delta persist for the leaf path. The full
    /// `persist_state` is DELETE-ALL + rewrite of every cell, every
    /// def, and every 3NF projection table — O(population) per persist
    /// (the dominant share of the measured ~700s leaf tail at 171MB).
    /// The leaf path knows its exact delta, so:
    ///   - cells: upsert ONLY the changed ones (the caller verified
    ///     nothing vanished — it falls back to the full persist
    ///     otherwise, loudly);
    ///   - defs: untouched (schema unchanged ⇒ defs unchanged by
    ///     construction on the leaf path);
    ///   - 3NF projection: scoped to tables sourcing a changed cell.
    /// Returns (cells upserted, projection tables refreshed).
    #[cfg(feature = "local")]
    pub fn persist_state_delta(
        conn: &Connection,
        d: &ast::Object,
        changed: &hashbrown::HashSet<String>,
    ) -> (usize, usize) {
        let tx = conn.unchecked_transaction()
            .unwrap_or_else(|e| { eprintln!("Transaction failed: {}", e); std::process::exit(1); });
        let mut n_cells = 0usize;
        for (name, contents) in ast::cells_iter(d).into_iter() {
            if !changed.contains(name) { continue; }
            if name.contains(':') || ["validate", "compile", "apply",
                "verify_signature", "debug", "_defs_compiled"].contains(&name) {
                continue; // def surface — unchanged by construction
            }
            let json = contents.to_string();
            tx.execute(
                "INSERT OR REPLACE INTO cells (name, contents) VALUES (?1, ?2)",
                params![name, json],
            ).unwrap_or_else(|e| { eprintln!("Failed to store cell {}: {}", name, e); std::process::exit(1); });
            n_cells += 1;
        }
        let n_tables = project_population_rows_scoped(&tx, d, changed);
        tx.commit()
            .unwrap_or_else(|e| { eprintln!("Commit failed: {}", e); std::process::exit(1); });
        (n_cells, n_tables)
    }

    /// rmap-3nf-tables Stage 1b — project population cells into the
    /// 3NF RMAP tables (wholesale refresh, mirroring the cells
    /// DELETE+reINSERT above; the tables are a PROJECTION, cells are
    /// the source of truth).
    ///
    /// Row assembly per `TableDef`, driven by the Stage-1b provenance
    /// the rmap columns now carry (`source_cell` / `source_subject_role`
    /// / `source_value_role` — the same final DECORATED names the DDL
    /// used, so projection can never drift from the schema):
    ///
    ///   * entity tables (synthetic `id` PK): one row per subject id,
    ///     each provenance column's value joined from its source cell
    ///     by (subject binding = id, value binding = column value);
    ///   * junction/compound tables: one row per fact of the (shared)
    ///     source cell — extracted POSITIONALLY so same-noun rings
    ///     (`Task blocks Task`: two `Task` bindings) land both roles,
    ///     with a by-name fallback when the fact's arity differs.
    ///
    /// Best-effort per row: a NOT NULL miss or constraint failure
    /// warns and skips that row (mandatory enforcement lives in
    /// ρ(validate), not here). Tables without provenance (independent
    /// id-only tables) project the distinct subject ids found across
    /// the cell graph's references — deferred until a consumer needs
    /// them; today they stay empty.
    pub fn project_population_rows(conn: &Connection, d: &ast::Object) {
        // SAVEPOINT isolation: the projection must NEVER poison the
        // cells/defs persist — a deferred-FK commit failure here once
        // rolled back the WHOLE transaction (caught live). Any terminal
        // projection error rolls back to this savepoint and the persist
        // proceeds without the projection.
        if conn.execute_batch("SAVEPOINT rmap_projection;").is_err() { return; }
        let ok = project_population_rows_inner(conn, d, None);
        if ok {
            let _ = conn.execute_batch("RELEASE rmap_projection;");
        } else {
            eprintln!("Warning: 3NF row projection rolled back (cells/defs persist unaffected)");
            let _ = conn.execute_batch("ROLLBACK TO rmap_projection; RELEASE rmap_projection;");
        }
    }

    /// 987-A.3 (delta tail): refresh ONLY the 3NF tables one of whose
    /// columns sources a changed cell. Row assembly still builds the
    /// full plan (v1 — measure before pushing the filter into
    /// `rmap::projection_plan`); the DELETE+reINSERT is scoped, which
    /// is where the wholesale-refresh sqlite time went. Deferred FKs
    /// make in-transaction parent rewrites safe: the scoped tables are
    /// fully re-assembled from the complete plan, so every key a child
    /// row references is back before commit. Returns the number of
    /// affected tables.
    #[cfg(feature = "local")]
    pub fn project_population_rows_scoped(
        conn: &Connection,
        d: &ast::Object,
        changed: &hashbrown::HashSet<String>,
    ) -> usize {
        let affected: hashbrown::HashSet<String> = crate::rmap::rmap(d).iter()
            .filter(|t| t.columns.iter().any(|c|
                c.source_cell.as_deref().map_or(false, |s| changed.contains(s))))
            .map(|t| t.name.clone())
            .collect();
        if affected.is_empty() { return 0; }
        if conn.execute_batch("SAVEPOINT rmap_projection;").is_err() { return 0; }
        let ok = project_population_rows_inner(conn, d, Some(&affected));
        if ok {
            let _ = conn.execute_batch("RELEASE rmap_projection;");
        } else {
            eprintln!("Warning: scoped 3NF row projection rolled back (cells persist unaffected)");
            let _ = conn.execute_batch("ROLLBACK TO rmap_projection; RELEASE rmap_projection;");
        }
        affected.len()
    }

    fn project_population_rows_inner(
        conn: &Connection,
        d: &ast::Object,
        only: Option<&hashbrown::HashSet<String>>,
    ) -> bool {
        // rmap-3nf-tables Stage 2: Phases 1–3 (collect, parent-fill
        // fixpoint, Kahn order) live in `rmap::projection_plan` — the
        // SAME plan the `sql` verb materializes into :memory:. This fn
        // is Phase 4 only: delete children-first, insert parents-first,
        // warn-skip per row.
        // 987-A.3 increment 2: the SCOPED plan assembles rows only for
        // the named tables (plus Phase-2 parent id-rows) — the full
        // assembly was ~60s of the 65s leaf persist phase at 171MB.
        let plan = match only {
            Some(o) => crate::rmap::projection_plan_scoped(d, o),
            None => crate::rmap::projection_plan(d),
        };

        for name in plan.order.iter().rev() {
            // DELETE only the scoped tables — unscoped tables keep
            // their rows (their cells did not change).
            if only.map_or(false, |o| !o.contains(name)) { continue; }
            let exists: bool = conn.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                params![name], |r| r.get::<_, i64>(0)).map(|n| n > 0).unwrap_or(false);
            if !exists { continue; }
            if let Err(e) = conn.execute(&format!("DELETE FROM \"{}\"", name), []) {
                eprintln!("Warning: projection clear failed for {}: {}", name, e);
            }
        }
        for name in &plan.order {
            // INSERT everything the (scoped) plan carries: scoped
            // tables fully, plus parent id-rows from Phase 2 — upsert
            // is idempotent for existing parents and required for a
            // first-referenced new parent id.
            let Some(rows) = plan.rows.get(name) else { continue };
            if rows.is_empty() { continue; }
            let exists: bool = conn.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                params![name], |r| r.get::<_, i64>(0)).map(|n| n > 0).unwrap_or(false);
            if !exists { continue; }
            for row in rows {
                let mut names: Vec<String> = Vec::new();
                let mut values: Vec<&String> = Vec::new();
                for (k, v) in row.iter() {
                    names.push(format!("\"{}\"", k.replace('"', "")));
                    values.push(v);
                }
                let placeholders: Vec<String> =
                    (1..=values.len()).map(|i| format!("?{}", i)).collect();
                let sql = format!(
                    "INSERT OR REPLACE INTO \"{}\" ({}) VALUES ({})",
                    name, names.join(", "), placeholders.join(", "));
                if let Err(e) = conn.execute(&sql, rusqlite::params_from_iter(values.iter())) {
                    eprintln!("Warning: row projection failed for {}: {}", name, e);
                }
            }
        }
        true
    }

    /// Load state D from SQLite.
    ///
    /// Builds an `Object::Map` directly while iterating rows. The
    /// previous shape folded via `ast::store` starting from
    /// `Object::phi()` (empty Seq), making each insert O(N) and the
    /// whole load O(N²). For a tasks-app DB with ~10K combined cells
    /// + defs, the cold-start `load_state` cost dominated every CLI
    /// invocation at ~18s — pinning a CPU core for the duration of
    /// every MCP shell-out. The direct-Map path is O(N) total.
    pub fn load_state(conn: &Connection) -> ast::Object {
        let mut map: hashbrown::HashMap<String, ast::Object> =
            hashbrown::HashMap::new();

        // Load cells (population facts).
        if let Ok(mut stmt) = conn.prepare("SELECT name, contents FROM cells") {
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            }).unwrap_or_else(|e| { eprintln!("Failed to read cells: {}", e); std::process::exit(1); });
            for r in rows.filter_map(|r| r.ok()) {
                map.insert(r.0, ast::Object::parse(&r.1));
            }
        }

        // Load defs.
        if let Ok(mut stmt) = conn.prepare("SELECT name, func FROM defs") {
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            }).unwrap_or_else(|e| { eprintln!("Failed to read defs: {}", e); std::process::exit(1); });
            for r in rows.filter_map(|r| r.ok()) {
                map.insert(r.0, ast::Object::parse(&r.1));
            }
        }

        ast::Object::map(map)
    }

    /// read-path-defprune: walk an FFP `Object` body and push every atom
    /// that names a known def into `out`. This is how a def body
    /// REFERENCES another def — `metacompose_atom` resolves a bare atom by
    /// `fetch(name, d)` first (ast.rs:6009), so any atom matching a def key
    /// is a live edge in the def-dependency graph (the by-name reference
    /// `Func::Def`/cell-fetch both serialize to a plain `Object::Atom`, see
    /// `func_to_object` ast.rs:6213/6191). Scanning ALL atoms against the
    /// def-name set is a sound over-approximation of reachability: it can
    /// only ever pull in MORE defs than a read could touch, never fewer, so
    /// the loaded snapshot is byte-identical to the full load on every
    /// reachable path. (Population cells referenced by a body are already
    /// loaded verbatim, so they need no edge here — `names` is the def set.)
    fn collect_name_refs(
        obj: &ast::Object,
        names: &std::collections::HashSet<String>,
        out: &mut Vec<String>,
    ) {
        match obj {
            ast::Object::Atom(s) => {
                if names.contains(s.as_str()) {
                    out.push(s.clone());
                }
            }
            ast::Object::Seq(items) => {
                for it in items.iter() {
                    collect_name_refs(it, names, out);
                }
            }
            ast::Object::Map(m) => {
                for v in m.values() {
                    collect_name_refs(v, names, out);
                }
            }
            ast::Object::Bottom => {}
        }
    }

    /// read-path-defprune: load the transitive-closure of defs REACHABLE
    /// from a seed, instead of all of them.
    ///
    /// A tasks-scale DB persists ~8.6k compiled-def cells (generator
    /// output: `query:`/`schema:`/`shard:`/`html:`/… families) but a
    /// read-only verb resolves at most the 9 `view:` defs plus whatever
    /// THOSE transitively reference. `load_state` parses every row up
    /// front (~0.9s, dominated by `Object::parse` over the bulk); this
    /// parses only the reachable set (~14 defs on tasks.db), cutting a
    /// representative read roughly in half again.
    ///
    /// Closure, not a blunt prune: starting from the defs `seed_is`
    /// selects, every atom in a loaded body that names another def is
    /// followed (BFS) until no new def is discovered — so the snapshot
    /// holds exactly the defs a read can reach and is byte-identical to
    /// the full load on every reachable path. ALL population/schema cells
    /// are loaded unconditionally (only ~221 rows, all the read verbs may
    /// touch them, and they are cheap), so only the def table is pruned.
    ///
    /// Safe in general: it relies on no fragile "views never embed Def
    /// refs" invariant — if a view's derivation DID reference a
    /// `derivation:`/`schema:` def, the walk would simply load it too.
    pub fn load_state_closure(
        conn: &Connection,
        seed_is: impl Fn(&str) -> bool,
    ) -> ast::Object {
        let mut map: hashbrown::HashMap<String, ast::Object> =
            hashbrown::HashMap::new();

        // All population/schema cells, verbatim (cheap; any read may read
        // any of them, e.g. `sql` materializes FT tables from them).
        if let Ok(mut stmt) = conn.prepare("SELECT name, contents FROM cells") {
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            }).unwrap_or_else(|e| { eprintln!("Failed to read cells: {}", e); std::process::exit(1); });
            for r in rows.filter_map(|r| r.ok()) {
                map.insert(r.0, ast::Object::parse(&r.1));
            }
        }

        // The full set of def NAMES (key column only — no body parse), so
        // the walk can recognise which atoms are def references and the
        // seed predicate has the universe to select from.
        let mut def_names: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        if let Ok(mut stmt) = conn.prepare("SELECT name FROM defs") {
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))
                .unwrap_or_else(|e| { eprintln!("Failed to read def names: {}", e); std::process::exit(1); });
            for n in rows.filter_map(|r| r.ok()) {
                def_names.insert(n);
            }
        }

        // BFS the def-dependency graph from the seed. `loaded` guards
        // against re-parsing (and cycles); `frontier` holds names whose
        // bodies still need scanning. One prepared statement, point
        // lookups by primary key.
        let mut body_stmt = match conn.prepare("SELECT func FROM defs WHERE name = ?1") {
            Ok(s) => s,
            Err(e) => { eprintln!("Failed to prepare def lookup: {}", e); std::process::exit(1); }
        };
        let mut loaded: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        let mut frontier: Vec<String> = def_names.iter()
            .filter(|n| seed_is(n))
            .cloned()
            .collect();
        while let Some(name) = frontier.pop() {
            if !loaded.insert(name.clone()) {
                continue;
            }
            let body: Option<String> = body_stmt
                .query_row([&name], |row| row.get::<_, String>(0))
                .ok();
            let Some(body) = body else { continue };
            let obj = ast::Object::parse(&body);
            let mut refs: Vec<String> = Vec::new();
            collect_name_refs(&obj, &def_names, &mut refs);
            for r in refs {
                if !loaded.contains(&r) {
                    frontier.push(r);
                }
            }
            map.insert(name, obj);
        }

        ast::Object::map(map)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn persist_state_removes_rows_absent_from_next_snapshot() {
            let conn = Connection::open_in_memory().expect("in-memory sqlite");
            ensure_meta_tables(&conn);

            let first = ast::store(
                "query:Ticket",
                ast::Object::atom("old def"),
                &ast::store("Ticket", ast::Object::atom("old cell"), &ast::Object::phi()),
            );
            persist_state(&conn, &first);

            let cell_count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM cells WHERE name = 'Ticket'",
                [],
                |row| row.get(0),
            ).expect("cell count");
            let def_count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM defs WHERE name = 'query:Ticket'",
                [],
                |row| row.get(0),
            ).expect("def count");
            assert_eq!(cell_count, 1);
            assert_eq!(def_count, 1);

            persist_state(&conn, &ast::Object::phi());

            let cell_count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM cells WHERE name = 'Ticket'",
                [],
                |row| row.get(0),
            ).expect("cell count after replacement");
            let def_count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM defs WHERE name = 'query:Ticket'",
                [],
                |row| row.get(0),
            ).expect("def count after replacement");
            assert_eq!(cell_count, 0);
            assert_eq!(def_count, 0);
        }

        /// 987-A increment 1: the per-file signature registry round-trips
        /// through a cells row, and leaf-only delta detection passes
        /// exactly the instance-only-modification case — declining on
        /// missing priors, file-set changes, schema-bearing edits, and
        /// the no-change case (the delta-LFP skip owns that one).
        #[test]
        fn leaf_only_delta_detection_gates() {
            let conn = Connection::open_in_memory().expect("in-memory sqlite");
            ensure_meta_tables(&conn);
            let v1 = vec![
                ("app.md".to_string(), "# App\n## Fact Types\nCase observes Fact Note.\n".to_string()),
                ("run-1.md".to_string(), "## Instance Facts\nCase 'c1' observes Fact Note 'n1'.\n".to_string()),
            ];
            // No priors → None (first compile is always full).
            assert_eq!(super::super::leaf_only_changed_files(&conn, &v1), None);
            // Store the registry the way the compile tail does.
            conn.execute("INSERT OR REPLACE INTO cells (name, contents) VALUES ('_FileSigs', ?1)",
                [&super::super::encode_file_sigs(&v1)]).unwrap();
            // No change → None (delta-LFP skip's territory).
            assert_eq!(super::super::leaf_only_changed_files(&conn, &v1), None);
            // Instance-only change → Some([run-1.md]).
            let mut v2 = v1.clone();
            v2[1].1.push_str("Case 'c1' observes Fact Note 'n2'.\n");
            assert_eq!(super::super::leaf_only_changed_files(&conn, &v2),
                Some(vec!["run-1.md".to_string()]));
            // Schema-bearing change → None.
            let mut v3 = v1.clone();
            v3[0].1.push_str("Case has Confidence.\n");
            assert_eq!(super::super::leaf_only_changed_files(&conn, &v3), None,
                "app.md carries ## Fact Types — schema edit must decline");
            // Added instance-only file → eligible (987-A.2 relaxation:
            // a brand-new instance file is the safest delta — every row
            // is new; this is the arc one-file-per-run percept shape).
            let mut v4 = v1.clone();
            v4.push(("run-2.md".to_string(), "## Instance Facts\nCase 'c2' observes Fact Note 'n3'.\n".to_string()));
            assert_eq!(super::super::leaf_only_changed_files(&conn, &v4),
                Some(vec!["run-2.md".to_string()]));
            // Added SCHEMA-bearing file → None (section gate catches it).
            let mut v5 = v1.clone();
            v5.push(("more.md".to_string(), "## Fact Types\nCase has Confidence.\n".to_string()));
            assert_eq!(super::super::leaf_only_changed_files(&conn, &v5), None);
            // Removed file → None (removal can retract schema or facts).
            let v6 = vec![v1[0].clone()];
            assert_eq!(super::super::leaf_only_changed_files(&conn, &v6), None);
            // `## Description` + `## Instance Facts` → eligible (the arc
            // run-file shape; prose-leak safety is the STRUCTURAL gate in
            // try_leaf_ingest, not a textual section decline).
            let mut v7 = v1.clone();
            v7[1].1 = format!("## Description\n\nPercept log prose.\n\n{}", v7[1].1);
            assert_eq!(super::super::leaf_only_changed_files(&conn, &v7),
                Some(vec!["run-1.md".to_string()]));
        }

        /// 987-A.2: the leaf EXECUTION path ingests an instance-only
        /// delta into a real (mini) prior db — the new fact lands in
        /// its FT cell, prior facts and the def surface survive, and
        /// the sig registries advance so the NEXT no-change recompile
        /// can take the delta-LFP skip. Decline paths persist nothing.
        #[test]
        fn leaf_ingest_executes_instance_only_delta() {
            let conn = Connection::open_in_memory().expect("in-memory sqlite");
            ensure_meta_tables(&conn);
            let v1 = vec![
                ("app.md".to_string(),
                 "# App\n\n## Entity Types\n\nWidget(.id) is an entity type.\n\n\
                  ## Value Types\n\nColor is a value type.\n\n\
                  ## Fact Types\n\nWidget has Color.\n".to_string()),
                // The arc run-file shape: a Description (prose) section
                // alongside the Instance Facts — the leaf path must take
                // it (textual gate allows it; the structural schema gate
                // verifies the prose emitted no schema rows).
                ("data.md".to_string(),
                 "## Description\n\nPercept log for the widget run.\n\n\
                  ## Instance Facts\n\nWidget 'w1' has Color 'blue'.\n".to_string()),
            ];
            // Prior db = a real mini full compile: corpus parse → compile
            // defs → registry stores → persist (the dirs arm distilled).
            let corpus: String = v1.iter().map(|(_, t)| t.as_str())
                .collect::<Vec<_>>().join("\n\n");
            let parsed = crate::parse_forml2::parse_to_state_from(
                &corpus, &crate::ast::Object::phi()).expect("v1 corpus parses");
            let compile_defs = crate::compile::compile_to_defs_state(&parsed);
            let d = crate::ast::defs_to_state(&compile_defs, &parsed);
            let d = crate::ast::store("_FileSigs",
                crate::ast::Object::atom(&super::super::encode_file_sigs(&v1)), &d);
            persist_state(&conn, &d);
            let prior_def_count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM defs", [], |r| r.get(0)).expect("defs count");
            // The delta: one ADDITIVE instance edit.
            let mut v2 = v1.clone();
            v2[1].1.push_str("Widget 'w2' has Color 'red'.\n");
            let changed = super::super::leaf_only_changed_files(&conn, &v2)
                .expect("instance-only edit is eligible");
            assert_eq!(changed, vec!["data.md".to_string()]);
            // Decline path: a generator opt-in in the changed file falls
            // back loudly and persists nothing.
            let mut v2_gen = v2.clone();
            v2_gen[1].1.push_str("App 'x' uses Generator 'sqlite'.\n");
            assert!(!super::super::try_leaf_ingest(&conn, None, &v2_gen, &changed),
                "generator opt-in must decline the leaf path");
            // Execute the leaf ingest.
            assert!(super::super::try_leaf_ingest(&conn, None, &v2, &changed),
                "leaf ingest should run to completion on an additive delta");
            // The new fact landed; the prior fact survived.
            let reloaded = super::super::db::load_state(&conn);
            let cell = crate::ast::fetch_cell_seq("Widget_has_Color", &reloaded);
            let rows = cell.as_seq().expect("Widget_has_Color is a populated cell");
            let has = |id: &str, color: &str| rows.iter().any(|f|
                crate::ast::binding(f, "Widget") == Some(id)
                    && crate::ast::binding(f, "Color") == Some(color));
            assert!(has("w1", "blue"), "prior fact must survive the leaf ingest");
            assert!(has("w2", "red"), "the appended fact must land");
            // The def surface survived (compile_to_defs_state was skipped,
            // so the prior defs must carry over verbatim).
            let post_def_count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM defs", [], |r| r.get(0)).expect("defs count");
            assert_eq!(post_def_count, prior_def_count,
                "leaf ingest must not lose (or re-mint) the def surface");
            // The registries advanced: the next no-change recompile of v2
            // sees current sigs (None = nothing changed → delta-LFP skip).
            assert_eq!(super::super::leaf_only_changed_files(&conn, &v2), None,
                "_FileSigs must reflect v2 after the leaf ingest");
        }

        /// rmap-3nf-tables Stage 3 (fossil sweep): apply_ddl drops
        /// tables whose names fall outside the [A-Za-z0-9_] emission
        /// alphabet (pre-prose-leak-gate relics — arc-agi-3's backtick
        /// junk, one of which broke the sql verb wholesale, issue 11)
        /// while leaving engine storage AND sane-named non-plan tables
        /// (potential user scratch) untouched.
        #[test]
        fn apply_ddl_drops_impossible_name_fossils_keeps_sane_scratch() {
            let conn = Connection::open_in_memory().expect("in-memory sqlite");
            ensure_meta_tables(&conn);
            conn.execute_batch(
                r#"CREATE TABLE "0..63,_color_over_0..15" (x TEXT);"#).unwrap();
            conn.execute_batch(
                r#"CREATE TABLE ">_`fact_type_has_role`_(:164)" (x TEXT);"#).unwrap();
            conn.execute_batch("CREATE TABLE my_scratch (x TEXT);").unwrap();

            apply_ddl(&conn, &ast::Object::phi());

            let names: Vec<String> = conn
                .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
                .unwrap()
                .query_map([], |r| r.get::<_, String>(0)).unwrap()
                .filter_map(|r| r.ok())
                .collect();
            assert!(names.contains(&"cells".to_string())
                && names.contains(&"defs".to_string()),
                "engine storage tables must survive the sweep; got {names:?}");
            assert!(names.contains(&"my_scratch".to_string()),
                "sane-named non-plan tables are potential user scratch and \
                 must survive; got {names:?}");
            assert!(!names.iter().any(|n| n.contains('`') || n.contains(',') || n.contains('>')),
                "impossible-alphabet fossils must be dropped; got {names:?}");
        }
    }
}

// =========================================================================
// perf-metamodel-parse-cache (cross-process). Grounded in AREST.tex
// §Conclusion: "cacheability because the representation is a deterministic
// function of P and S" (Thm. Derivability); the FILE store of eq:pop
// (`P = ⋃ ↑FILE:D_n`) is the persistence medium; and "versioning is the
// event stream" — the cache is CONTENT-ADDRESSED by a hash of the bundled
// metamodel readings, so it auto-invalidates the instant those readings
// change (≈ a rebuild). The seeded metamodel parse is a deterministic
// function of those (binary-constant) readings, so a cold `arest-cli` spawn
// can LOAD it (~1-2s) instead of re-folding it (~15s). Determinism
// (Constraint Consensus — deterministic replay) is what guarantees the warm
// load equals the cold fold; the cold-vs-warm 6249/838 gate verifies it.
// =========================================================================

/// The #836 derived-cell wipe set for the LOAD path: DerivationRule
/// consequents ∪ SyntheticDerivedCells, MINUS the SM trigger cells.
///
/// run-load-chain-wipes-sm-trigger-cells: SM trigger cells hold REAL
/// transition events and must never be wiped — the same exclusion the
/// apply path applies (`command::sm_trigger_cell_set`). They land in
/// the derived set because the event→event migration backfills
/// (`Task is started iff Task is finished`) make them DerivationRule
/// consequents; wiping them here re-derived starts only for FINISHED
/// entities (duplicates) and LOST them for started-not-finished ones,
/// folding those back to the initial status on every recompile (the
/// live board's in-progress tasks reset to pending twice on 2026-06-10
/// before this fix).
#[cfg(feature = "local")]
pub(crate) fn derived_wipe_set(d: &ast::Object) -> hashbrown::HashSet<String> {
    let mut out: hashbrown::HashSet<String> = hashbrown::HashSet::new();
    let drule_cell = ast::fetch_cell_seq("DerivationRule", d);
    if let Some(facts) = drule_cell.as_seq() {
        for fact in facts.iter() {
            let Some(encoded) = ast::binding(fact, "consequentFactTypeId") else { continue };
            let cell_name = crate::types::ConsequentCellSource::decode(encoded)
                .literal_id().to_string();
            if !cell_name.is_empty() { out.insert(cell_name); }
        }
    }
    // #905/task-740: synthetic-rule consequents (SM init, SM event-fold,
    // etc.) declared in the SyntheticDerivedCells meta cell from
    // compile.rs. User rules contribute via DerivationRule above;
    // synthetic rules via this cell. No hand-curated list here.
    //
    // The cell is emitted via `defs.push(..., Func::constant(seq))` which
    // `func_to_object` stores as a 2-elem Seq `<atom("'"), entries>` —
    // the FFP const-fn wrapper. Unwrap before iterating.
    let synth_cell = ast::fetch_cell_seq("SyntheticDerivedCells", d);
    let synth_entries = synth_cell.as_seq()
        .and_then(|items| {
            if items.len() == 2 && items[0].as_atom() == Some("'") {
                items[1].as_seq().map(|s| s.to_vec())
            } else {
                Some(items.to_vec())
            }
        })
        .unwrap_or_default();
    for fact in synth_entries.iter() {
        let Some(name) = ast::binding(fact, "name") else { continue };
        if !name.is_empty() { out.insert(name.to_string()); }
    }
    let sm_triggers = crate::command::sm_trigger_cell_set(d);
    let excluded: Vec<&String> = out.iter()
        .filter(|c| sm_triggers.contains(c.as_str())).collect();
    if !excluded.is_empty() {
        eprintln!("[load] excluding {} SM trigger cell(s) from the #836 wipe \
                   (real events, never re-derivable): {:?}",
            excluded.len(), excluded);
    }
    out.retain(|c| !sm_triggers.contains(c.as_str()));
    out
}

#[cfg(all(test, feature = "local"))]
mod derived_wipe_set_tests {
    use super::*;
    use crate::ast;

    /// The load-path wipe set excludes SM trigger cells: a backfill
    /// rule makes `Task_is_started` a DerivationRule consequent, but
    /// the SM trio (transition → SMD → noun, transition → event type)
    /// marks it a trigger — so it must survive the wipe while an
    /// ordinary derived cell is wiped. This is the load-path twin of
    /// command.rs's apply-path guard
    /// (apply_update_does_not_wipe_sm_trigger_cell_collapsing_status).
    #[test]
    fn excludes_sm_trigger_cells_from_the_wipe() {
        let push = |s: ast::Object, cell: &str, pairs: &[(&str, &str)]|
            ast::cell_push(cell, ast::fact_from_pairs(pairs), &s);
        let d = {
            let s = ast::Object::phi();
            // Two derivation consequents: the backfilled trigger + an
            // ordinary derived marker.
            let s = push(s, "DerivationRule",
                &[("id", "rule_backfill"), ("consequentFactTypeId", "Task_is_started")]);
            let s = push(s, "DerivationRule",
                &[("id", "rule_marker"), ("consequentFactTypeId", "Task_is_dependency_blocked")]);
            // The SM trio that makes 'Task is started' a trigger cell.
            let s = push(s, "Transition_is_defined_in_State_Machine_Definition",
                &[("Transition", "start"), ("State Machine Definition", "Task SM")]);
            let s = push(s, "State_Machine_Definition_is_for_Noun",
                &[("State Machine Definition", "Task SM"), ("Noun", "Task")]);
            push(s, "Transition_is_triggered_by_Event_Type",
                &[("Transition", "start"), ("Event Type", "Task is started")])
        };
        let set = derived_wipe_set(&d);
        assert!(set.contains("Task_is_dependency_blocked"),
            "ordinary derived cells stay in the wipe set; got {:?}", set);
        assert!(!set.contains("Task_is_started"),
            "SM trigger cells must be excluded from the wipe (real events); got {:?}", set);
    }
}

/// FNV-1a of THIS BINARY's bytes — the definitive engine identity.
///
/// rmap-3nf-tables Stage 3 (cache-key hardening): both the metamodel
/// parse cache and the `_CompileSig` delta-LFP skip used to key on
/// content that does NOT change for working-tree rebuilds
/// (`AREST_GIT_SHA` moves only on commit; the readings hash only on
/// readings edits) — so a rebuilt parser served STALE cached parses
/// and skipped re-derivation until a commit landed or the caches were
/// hand-cleared. Bit twice live this session (a new backfill
/// derivation silently didn't fire; a parser fix didn't take). The
/// binary hashing itself is f(code) exactly; ~10ms once per process
/// via OnceLock.
#[cfg(feature = "local")]
fn binary_self_hash() -> u64 {
    static HASH: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *HASH.get_or_init(|| {
        let bytes = std::env::current_exe().ok()
            .and_then(|p| std::fs::read(p).ok())
            .unwrap_or_default();
        let mut h: u64 = 0xcbf29ce484222325;
        for b in bytes { h ^= b as u64; h = h.wrapping_mul(0x100000001b3); }
        h
    })
}

/// Content hash (FNV-1a) of the bundled metamodel readings PLUS the
/// binary self-hash — the cache key. A parse cache is a function of
/// (readings, parser); either changing must miss.
#[cfg(feature = "local")]
fn metamodel_readings_signature() -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for entry in crate::metamodel_readings() {
        for b in entry.0.bytes() { h ^= b as u64; h = h.wrapping_mul(0x100000001b3); }
        h ^= 0x1f; h = h.wrapping_mul(0x100000001b3);
        for b in entry.1.bytes() { h ^= b as u64; h = h.wrapping_mul(0x100000001b3); }
        h ^= 0x1e; h = h.wrapping_mul(0x100000001b3);
    }
    h ^ binary_self_hash()
}

/// FILE path of the metamodel parse cache for the current readings signature.
#[cfg(feature = "local")]
fn metamodel_parse_cache_path() -> std::path::PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!("arest-metamodel-parse-{:016x}.db", metamodel_readings_signature()));
    dir
}

/// Load the cached deterministic metamodel parse from FILE, or None if absent
/// / unreadable / empty (→ caller rebuilds). The signature is encoded in the
/// filename, so a present, populated file IS by construction the parse of the
/// current readings — no separate version check needed.
#[cfg(feature = "local")]
fn load_metamodel_parse_cache() -> Option<ast::Object> {
    let path = metamodel_parse_cache_path();
    if !path.exists() { return None; }
    let conn = rusqlite::Connection::open(&path).ok()?;
    let loaded = db::load_state(&conn);
    // A populated Noun cell confirms a usable cache (guards a torn/empty file).
    let usable = ast::fetch_cell_seq("Noun", &loaded).as_seq().map_or(false, |s| !s.is_empty());
    usable.then_some(loaded)
}

/// Persist the deterministic metamodel parse to FILE for cross-process reuse.
/// Writes to a per-process temp then atomically renames, so concurrent
/// compilers (different apps, same binary) never observe a torn file; the
/// content is deterministic, so racing writers emit byte-identical files.
#[cfg(feature = "local")]
fn store_metamodel_parse_cache(cells: &ast::Object) {
    let path = metamodel_parse_cache_path();
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    let _ = std::fs::remove_file(&tmp);
    if let Ok(conn) = rusqlite::Connection::open(&tmp) {
        db::ensure_meta_tables(&conn);
        db::persist_state(&conn, cells);
        drop(conn);
        let _ = std::fs::rename(&tmp, &path);
    }
    let _ = std::fs::remove_file(&tmp);
}

// =========================================================================
// SYSTEM is the only function
// =========================================================================

/// system(key, input, D) → (output, D')
/// Pure ρ-dispatch. Same as lib.rs system_impl but operates on an
/// owned state instead of a global handle registry.
#[cfg(feature = "local")]
fn system(key: &str, input: &str, d: &ast::Object) -> (String, ast::Object) {
    // #864 — read-only SQL SELECT intercept. Mirrors the engine-side
    // intercept in `lib.rs::system_impl` so the CLI shell-out path
    // (`arest-cli --db <path> sql "<SELECT ...>"`) the MCP shim uses
    // produces the same JSON envelope as the in-process engine call.
    // Bypassing the standard `apply(Func::Def(key), …)` path is
    // necessary because `sql` is not a Def — it's a host-only verb
    // that materializes per-FT SQLite tables from the cell graph and
    // queries them. State is unchanged on this branch (read-only).
    if key == "sql" {
        let raw = input.to_string();
        return (crate::sql::sql_query(d, &raw), d.clone());
    }

    // #870 — read-only cells introspection intercept. Same shape as
    // the `sql` arm: mirror the engine-side intercept so
    // `arest-cli --db <path> cells "<JSON>"` produces the same envelope
    // the MCP shim sees from the in-process engine call. Read-only;
    // state is unchanged on this branch.
    if key == "cells" {
        let raw = input.to_string();
        return (crate::cells_introspect::cells_query(d, &raw), d.clone());
    }

    // #871 — read-only session re-orientation intercept. Same shape:
    // `arest-cli --db <path> orient "<JSON>"` produces the same
    // envelope the MCP shim sees from the in-process engine call.
    // Read-only; state is unchanged on this branch.
    if key == "orient" {
        let raw = input.to_string();
        return (crate::orient::orient(d, &raw), d.clone());
    }

    // task-738 — `retract:<ft_name>` removes one exact fact tuple from
    // the named FactType cell. Mirrors the engine-side intercept in
    // `lib.rs::system_impl` so the CLI shell-out path the MCP shim
    // uses produces the same updated state envelope. Write path:
    // returns ("ok", new_d) so the caller persists. ⊥ returns leave
    // D unchanged.
    if let Some(ft_name) = key.strip_prefix("retract:") {
        let input_obj = ast::Object::parse(input);
        let pairs: Vec<(String, String)> = match input_obj.as_seq() {
            Some(items) if !items.is_empty() => items.iter()
                .filter_map(|item| {
                    let kv = item.as_seq()?;
                    if kv.len() != 2 { return None; }
                    let role = kv[0].as_atom()?.to_string();
                    let value = kv[1].as_atom()?.to_string();
                    Some((role, value))
                })
                .collect(),
            _ => return ("⊥".into(), d.clone()),
        };
        if pairs.is_empty() {
            return ("⊥".into(), d.clone());
        }
        // Row match runs over the Map->Seq flattening so a folded FT-image
        // cell is searchable; existence check first (no match → ⊥, D
        // unchanged).
        let cell = ast::fetch_cell_seq(ft_name, d);
        let items: Vec<ast::Object> = match cell.as_seq() {
            Some(it) => it.to_vec(),
            None => return ("⊥".into(), d.clone()),
        };
        let found_idx = items
            .iter()
            .position(|fact| ast::fact_matches_pairs(fact, &pairs));
        let idx = match found_idx {
            Some(i) => i,
            None => return ("⊥".into(), d.clone()),
        };
        // #932 W7-b: shape-preserving write-back. A folded FT-image cell is
        // `Object::Map`; `cell_filter` drops the matching row by filtering
        // Map VALUES and re-wrapping as Map, so the cell stays Map (no
        // demotion to Seq). A genuine legacy Seq cell keeps the
        // remove-first-index + Seq delta through `merge_delta`.
        let new_d = match ast::fetch_or_phi(ft_name, d) {
            ast::Object::Map(_) => {
                let pairs_for_pred = pairs.clone();
                ast::cell_filter(
                    ft_name,
                    move |f| !ast::fact_matches_pairs(f, &pairs_for_pred),
                    d,
                )
            }
            _ => {
                let mut new_items = items;
                new_items.remove(idx);
                let new_cell = ast::Object::Seq(new_items.into());
                let mut delta_map: hashbrown::HashMap<String, ast::Object> =
                    hashbrown::HashMap::new();
                delta_map.insert(ft_name.to_string(), new_cell);
                let delta = ast::Object::map(delta_map);
                ast::merge_delta(d, &delta, None)
            }
        };
        return ("ok".into(), new_d);
    }

    // task-971 — `assert:<ft_name>` appends one exact fact tuple to the
    // named FactType cell. The SYMMETRIC counterpart to `retract:` above,
    // and the CLI mirror of the engine-side intercept in
    // `lib.rs::system_impl` so the CLI shell-out path the MCP shim uses
    // lands the fact instead of bottoming. Without this branch the key
    // falls through to `apply(Func::Def("assert:<ft>"), …)` below — there
    // is no such Def, so it returns ⊥ and the armed bottom-trace surfaces
    // `⊥ origin: … in rule `assert:<ft>`` (the reported same-noun-ring
    // failure). Repeated role names ARE allowed (ring facts). Write path:
    // returns ("ok", new_d) on success so the caller persists; an alethic
    // violation (e.g. an irreflexive self-loop) is rejected with D'=D
    // ("⊥", D unchanged).
    if let Some(ft_name) = key.strip_prefix("assert:") {
        let input_obj = ast::Object::parse(input);
        let pairs: Vec<crate::command::RolePair> = match input_obj.as_seq() {
            Some(items) if !items.is_empty() => items.iter()
                .filter_map(|item| {
                    let kv = item.as_seq()?;
                    if kv.len() != 2 { return None; }
                    let role = kv[0].as_atom()?.to_string();
                    let value = kv[1].as_atom()?.to_string();
                    Some(crate::command::RolePair { role, value })
                })
                .collect(),
            _ => return ("⊥".into(), d.clone()),
        };
        if pairs.is_empty() {
            return ("⊥".into(), d.clone());
        }
        // Dispatch through the full assert_fact pipeline (derive+validate),
        // exactly as `system_impl` does. An alethic violation rejects with
        // D'=D; otherwise merge the delta so chain entries land and persist.
        let cmd = crate::command::Command::AssertFact {
            fact_type: ft_name.to_string(),
            pairs,
            sender: None,
            signature: None,
        };
        let result = crate::command::apply_command_defs(d, &cmd, d);
        if result.rejected {
            return ("⊥".into(), d.clone());
        }
        let new_d = ast::merge_delta(d, &result.state, None);
        return ("ok".into(), new_d);
    }

    let obj = ast::Object::parse(input);
    // ⊥-trace: arm why-NOT provenance around the dispatch apply. ZERO
    // cost on the success path — the trace materializes only if the
    // computation structurally bottoms out (see ast::with_bottom_trace).
    // De-opaques the bare "⊥" the `_` arm below would otherwise print
    // into "⊥ origin: <binding> in rule `…` over cell `…`".
    let (result, bottom_trace) =
        ast::with_bottom_trace(|| ast::apply(&ast::Func::Def(key.to_string()), &obj, d));

    // Three result shapes the dispatcher must distinguish:
    //
    //   (a) Compile-style: `Object::Seq` of cells with a Noun entry.
    //       The apply'd function returned the entire new D directly;
    //       we replace D wholesale.
    //
    //   (b) Apply-style (#766): `Object::Map` carrier with a
    //       `__state_delta` cell and a `__result` JSON atom — what
    //       `platform_create` / `platform_update` /
    //       `platform_transition` emit via `encode_command_result`.
    //       The delta covers only the cells the command modified;
    //       merge it onto D so the chain entries land and persist.
    //       (#831(a) follow-up: this branch was missing, so apply
    //       results from the CLI shell-out were stringified and
    //       discarded — the in-process WASM-side `system_impl` never
    //       had the bug because it routes via
    //       `classify_writer_result` → `CommitDelta` →
    //       `merge_delta`. The CLI now mirrors that.)
    //
    //   (c) Display-only: every other shape is a query/explain/audit
    //       response that doesn't change D.
    //
    // For (b) we also surface the JSON envelope under `__result` as
    // the printed output so callers (MCP server, scripts) see the
    // same response shape `system_impl` produces.
    let (display, new_d) = match &result {
        ast::Object::Map(m) if m.contains_key("__state_delta") => {
            let delta = m.get("__state_delta").cloned().unwrap_or(ast::Object::phi());
            let merged = ast::merge_delta(d, &delta, None);
            let result_atom = m.get("__result").cloned();
            let output = result_atom
                .and_then(|o| o.as_atom().map(|s| s.to_string()))
                .unwrap_or_else(|| result.to_string());
            (output, merged)
        }
        ast::Object::Seq(_) if ast::fetch("Noun", &result) != ast::Object::Bottom => {
            (result.to_string(), result.clone())
        }
        // ⊥-trace surfacing: a top-level ⊥ is provenance-lossless on its
        // own. If an armed frame captured the origin, print the traced
        // form instead of a bare "⊥". State is unchanged either way.
        ast::Object::Bottom => {
            let rendered = bottom_trace
                .as_ref()
                .and_then(|t| t.describe())
                .unwrap_or_else(|| result.to_string());
            (rendered, d.clone())
        }
        // task-985 (arc issue 12.2): induce returns a Seq of Hypothesis
        // Candidates and the MCP shim parses JSON — the generic Display
        // arm printed the FFP form (`<confidenceScore, >` …), which the
        // shim flagged "malformed induce envelope". Mirror lib.rs's
        // read-only path: JSON-encode (empty atoms become "" cleanly).
        _ if key == "induce" => (result.to_json_string(), d.clone()),
        _ => (result.to_string(), d.clone()),
    };

    (display, new_d)
}

/// Read .md files from directories, sorted alphabetically, app.md first.
/// Also checks the parent directory of each readings dir for app.md.
#[cfg(feature = "local")]
fn read_readings(dirs: &[String]) -> Vec<(String, String)> {
    let (readings, app_md) = dirs.iter().flat_map(|dir| {
        let dir_path = std::path::Path::new(dir);
        (!dir_path.is_dir()).then(|| {
            eprintln!("Not a directory: {}", dir);
            std::process::exit(1);
        });
        // Check parent for app.md (app root vs readings subdir convention)
        let parent_app = dir_path.parent()
            .map(|p| p.join("app.md"))
            .filter(|p| p.exists());
        let parent_entry = parent_app.map(|path| {
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| { eprintln!("Failed to read {}: {}", path.display(), e); std::process::exit(1); });
            ("app.md".to_string(), text)
        });
        // Collect .md files recursively (readings may be in subdirectories).
        fn collect_md(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            let entries = std::fs::read_dir(dir)
                .unwrap_or_else(|e| { eprintln!("Failed to read {}: {}", dir.display(), e); std::process::exit(1); });
            entries.filter_map(|e| e.ok()).map(|e| e.path()).for_each(|p| {
                if p.is_dir() { collect_md(&p, out); }
                else if p.extension().and_then(|e| e.to_str()) == Some("md") { out.push(p); }
            });
        }
        let mut entries: Vec<std::path::PathBuf> = Vec::new();
        collect_md(dir_path, &mut entries);
        // Sort: files before subdirectories at each level, then alphabetically.
        // This ensures parent domain files (cases.md) load before subdirectory
        // files (cases/speckled-band.md) so nouns are in context.
        entries.sort_by(|a, b| {
            let a_depth = a.components().count();
            let b_depth = b.components().count();
            a_depth.cmp(&b_depth).then_with(|| a.cmp(b))
        });
        entries.into_iter().map(|path| {
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| { eprintln!("Failed to read {}: {}", path.display(), e); std::process::exit(1); });
            (name, text)
        }).chain(parent_entry).collect::<Vec<_>>()
    }).fold((Vec::new(), None::<(String, String)>), |(mut readings, app), (name, text)| {
        match name.as_str() {
            "app.md" => (readings, Some((name, text))),
            _ => { readings.push((name, text)); (readings, app) }
        }
    });

    app_md.into_iter().chain(readings).collect()
}

// task-951-b: source-file → ORM-element provenance.
//
// Compiled cells (Noun / FactType / Constraint) carry NO record of which
// readings file declared each element — the per-file fold at `main_entry`
// concatenates every file's parse into one state via `merge_states`, so by
// the time `compile_to_defs_state` builds the NORMA model the file boundary
// is gone. The NORMA exporter needs that boundary to emit one ORMDiagram
// (a NORMA "tab") per source file: a domain = a readings file.
//
// The AREST-aligned fix is a model-carried cell: `Provenance`, a Seq of
// facts `<<element, X>, <kind, K>, <sourceFile, F.md>>` where K ∈ {Noun,
// FactType, Constraint} and `element` is that element's identity (Noun
// `name`, FactType/Constraint `id`). It is built by replaying the SAME
// fold the loader uses — each file is parsed in the accumulated context,
// and every Noun/FactType/Constraint identity the file INTRODUCES (absent
// from the running accumulator) is attributed to that file. First-declarer
// wins, mirroring `merge_states`' identity-keyed dedup, so the provenance
// of a noun referenced by many files is the file that first declared it.
//
// `compile_to_defs_state` reads this cell (when present) to partition ORM
// elements into per-file diagrams; absent it (e.g. the single-blob
// `parse_to_state_via_stage12` test path) the exporter falls back to one
// diagram, so this is purely additive.

/// The identity-binding key for a fact in cell `cell` — the value the
/// provenance map keys on. Mirrors `merge_states`/`concat_dedup`'s
/// identity discipline (Noun by `name`, everything else by `id`).
#[cfg(feature = "local")]
fn provenance_id_key(cell: &str) -> &'static str {
    match cell {
        "Noun" => "name",
        _ => "id",
    }
}

/// Replay the loader's fold purely to attribute each ORM element to the
/// readings file that first declared it. `all_readings` is the SAME
/// `(name, text)` sequence the loader folds (metamodel slices + user
/// files), and `seed` is the same global-noun seed, so the per-file parse
/// context matches the real load exactly. Returns a `Provenance` cell
/// (a Seq of `<<element,..>,<kind,..>,<sourceFile,..>>` facts).
///
/// Cost: one extra parse per file. Only paid on the dirs-compile path
/// (the only caller that has file boundaries), and the parses are the
/// same ones the loader already does — provenance is the delta-capture,
/// not new parsing semantics.
#[cfg(feature = "local")]
fn build_provenance_cell(all_readings: &[(&str, &str)], seed: &ast::Object) -> ast::Object {
    const KINDS: [&str; 3] = ["Noun", "FactType", "Constraint"];
    // Per kind: the set of element identities already seen, so a file is
    // only credited with elements it INTRODUCES (first-declarer wins).
    //
    // `seen` is NOT pre-seeded from `seed`'s noun catalog: that catalog is
    // the WHOLE-corpus noun parse (every noun in every file), used only as
    // parse CONTEXT so cross-file references resolve regardless of fold
    // order. Pre-seeding from it would mark every noun "already seen" and
    // leave every Noun unattributed (no file ever credited). A per-file
    // parse's output Noun cell holds only the nouns that file's own text
    // declares (parse_to_state_via_stage12 builds it from `raw_nouns`, not
    // from `extra_nouns`), so the unseeded first-declarer walk attributes
    // each noun to the file that introduces it.
    let mut seen: hashbrown::HashMap<&'static str, hashbrown::HashSet<String>> =
        KINDS.iter().map(|k| (*k, hashbrown::HashSet::new())).collect();
    let mut facts: Vec<ast::Object> = Vec::new();
    // Accumulate parse context across files exactly as the loader's fold
    // does (later files see earlier files' nouns + fact types), seeded with
    // the global noun catalog so references resolve fold-order-independently.
    let mut merged = seed.clone();
    for (name, text) in all_readings {
        let this = match parse_forml2::parse_to_state_from(text, &merged) {
            Ok(s) => s,
            // A parse error here is reported (fatally) by the real fold the
            // caller runs right after; provenance must not be the thing that
            // exits the process, so skip this file and let the loader surface
            // the diagnostic with its own message.
            Err(_) => continue,
        };
        for kind in KINDS {
            let key = provenance_id_key(kind);
            let set = seen.get_mut(kind).expect("seen has an entry per KIND");
            if let Some(cell_facts) = ast::fetch_cell_seq(kind, &this).as_seq() {
                for f in cell_facts {
                    if let Some(id) = ast::binding(f, key) {
                        if set.insert(id.to_string()) {
                            facts.push(ast::fact_from_pairs(&[
                                ("element", id),
                                ("kind", kind),
                                ("sourceFile", name),
                            ]));
                        }
                    }
                }
            }
        }
        merged = ast::merge_states(&merged, &this);
    }
    ast::Object::Seq(facts.into())
}

/// Load population from SQLite, compile defs in memory.
/// Compile takes ~500ms and produces the full D for SYSTEM calls.
///
/// cli-apply-large-tasksdb-nonterminating: `persist_state` writes the
/// compiled defs (every `:`-named cell — `derivation:`, `schema:`,
/// `validate:`, …) to the `defs` table alongside the population, and
/// `load_state` reads them back. Those defs are RECOMPUTED here on every
/// load, so the persisted copies are pure cache — but a STALE one whose
/// name the current compiler no longer emits is an orphan that survives
/// the recompile (`defs_to_state` only overwrites SAME-named defs) and is
/// then picked up by the apply-time forward chain as a real rule.
///
/// On the live tasks.db this is exactly what happened: an older engine
/// persisted `derivation:_subtype_inheritance` — a 101 KB pre-expanded
/// Concat-of-`InstancesOfNoun` func (the subtype-inheritance reading-lift
/// before task-982/983 re-keyed it to `derivation:rule_bdabc589693e5cb5`).
/// The current compiler emits the small `rule_bdabc…` form, but the stale
/// 101 KB `_subtype_inheritance` orphan lingered in `defs` with NO
/// `derivation_reads:` sidecar, so the seeded chain treated it as a
/// run-every-round rule. ONE round of it over the ~870-entity population
/// was ~20 s / ~95 k candidate facts — by itself the difference between a
/// create/update that converges in ~2 s and one that runs for minutes
/// (tripping the chain's wall-clock ⊥ guard).
///
/// Fix: the compiled defs are authoritative as freshly compiled. Drop
/// every persisted COMPILED-def cell from the loaded snapshot before
/// recompiling — population cells never contain `:` (verified: zero
/// `:`-named rows in the `cells` table), so "name contains `:`" cleanly
/// selects the def families, and the platform singletons are dropped by
/// name. `compile_to_defs_state` then regenerates all of them, so a stale
/// orphan can never reach the apply chain. (This mirrors the existing
/// `ast::preserve_prior_population` "drop sidecar `:` cells on load"
/// discipline the recompile-watch path at L828 already follows.)
///
/// `population_only` is the (testable) heart of the fix: from a loaded
/// snapshot it keeps ONLY the population cells — every cell whose name is
/// `:`-free and not a platform-singleton def — dropping all persisted
/// compiled-def families (`derivation:`, `schema:`, `validate:`,
/// `resolve:`, `view:`, the generator prefixes, …). The recompile then
/// regenerates the live set, so a stale orphan in `defs` cannot survive.
#[cfg(feature = "local")]
fn population_only(loaded: &ast::Object) -> ast::Object {
    // Platform singletons `persist_state` writes to the `defs` table by a
    // bare (colon-free) name; everything else compiled carries a `:`.
    const PLATFORM_DEFS: [&str; 6] =
        ["compile", "apply", "verify_signature", "validate", "audit", "induce"];
    let kept: hashbrown::HashMap<String, ast::Object> = ast::cells_iter(loaded)
        .into_iter()
        .filter(|(name, _)| !name.contains(':') && !PLATFORM_DEFS.contains(name))
        .map(|(name, contents)| (name.to_string(), contents.clone()))
        .collect();
    ast::Object::map(kept)
}

/// 987-A increment 1 (leaf-only ingest): per-file content signature.
#[cfg(feature = "local")]
fn file_sig(text: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in text.bytes() { h ^= b as u64; h = h.wrapping_mul(0x100000001b3); }
    h
}

/// The delta-LFP compile signature — fnv over every reading (name +
/// text) folded with the binary self-hash. ONE definition shared by
/// the full pipeline and the leaf-ingest path (987-A.2) so the two
/// can never drift: a leaf ingest stores the SAME sig a full compile
/// of the same inputs would, which is what lets the next no-change
/// recompile take the delta-LFP no-op skip regardless of which path
/// produced the db.
///
/// Stage-3 cache-key hardening note (carried from the inline
/// original): the BINARY SELF-HASH, not AREST_GIT_SHA — the SHA only
/// moves on commit, so working-tree rebuilds (new derivations, parser
/// fixes) skipped the re-derive and served stale state until a commit
/// landed (bit twice live).
#[cfg(feature = "local")]
fn compile_input_sig(readings: &[(String, String)]) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for r in readings {
        for b in r.0.bytes().chain(r.1.bytes()) { h ^= b as u64; h = h.wrapping_mul(0x100000001b3); }
        h ^= 0x1e; h = h.wrapping_mul(0x100000001b3);
    }
    h ^= binary_self_hash();
    alloc::format!("{:016x}", h)
}

/// Encode the per-file signature registry as one atom:
/// `name\x1f{hash:016x}\x1e…` (US/RS separators — filenames are sane).
#[cfg(feature = "local")]
fn encode_file_sigs(readings: &[(String, String)]) -> String {
    let mut out = String::new();
    for (name, text) in readings {
        out.push_str(name);
        out.push('\u{1f}');
        out.push_str(&format!("{:016x}", file_sig(text)));
        out.push('\u{1e}');
    }
    out
}

/// Read the prior `_FileSigs` registry with ONE targeted SQL read —
/// never a full load_state (the whole point of the leaf path is to
/// avoid whole-store costs before the mode decision).
#[cfg(feature = "local")]
fn read_prior_file_sigs(conn: &rusqlite::Connection) -> hashbrown::HashMap<String, String> {
    let raw: Option<String> = conn
        .query_row("SELECT contents FROM cells WHERE name='_FileSigs'", [], |r| r.get(0))
        .ok();
    let mut out = hashbrown::HashMap::new();
    if let Some(raw) = raw {
        for entry in raw.trim_matches(|c| c == '<' || c == '>').split('\u{1e}') {
            if let Some((name, hash)) = entry.split_once('\u{1f}') {
                out.insert(name.to_string(), hash.to_string());
            }
        }
    }
    out
}

/// Leaf-only delta detection (987-A increment 1, arc ask #3): when a
/// recompile's per-file delta is confined to INSTANCE-ONLY files (no
/// schema sections — the arc percept-file shape), the schema is
/// provably unchanged and the full corpus re-parse + #836 wipe + full
/// forward chain are unnecessary. Returns the changed file names when
/// EVERY structural gate passes; None falls through to the full path.
///
/// Conservative by construction:
///   - prior registry must exist (a first compile is always full);
///   - a REMOVED file declines (removal can retract schema or facts —
///     only a full recompile proves which);
///   - an ADDED file is eligible when it is itself instance-only
///     (987-A.2 relaxation: a brand-new instance file is the SAFEST
///     delta of all — every row it contributes is new, so there is no
///     stale-row risk; this is the arc one-file-per-run percept shape);
///   - every changed/added file must contain `## Instance Facts` and
///     NONE of the schema section headers (Fact Types / Entity Types /
///     Value Types / Derivation Rules / State Machine / Constraints).
#[cfg(feature = "local")]
fn leaf_only_changed_files(
    conn: &rusqlite::Connection,
    readings: &[(String, String)],
) -> Option<Vec<String>> {
    let prior = read_prior_file_sigs(conn);
    if prior.is_empty() { return None; }
    // Removed file → full path (readings carries every current file,
    // so a prior name missing from it is a deletion).
    let current: hashbrown::HashSet<&str> =
        readings.iter().map(|(n, _)| n.as_str()).collect();
    if prior.keys().any(|n| !current.contains(n.as_str())) { return None; }
    let mut changed: Vec<&(String, String)> = Vec::new();
    for r in readings {
        match prior.get(&r.0) {
            None => changed.push(r), // added file — section-gated below
            Some(h) if *h == format!("{:016x}", file_sig(&r.1)) => {}
            Some(_) => changed.push(r),
        }
    }
    if changed.is_empty() {
        // No file changed at all — the existing delta-LFP no-op skip
        // already covers this; let the full path take it.
        return None;
    }
    // `## Description` is deliberately NOT here (987-A.2): every arc
    // run file carries one (prose + `## Instance Facts`), so declining
    // on it would exclude the exact percept workload this path exists
    // for. The risk a Description section poses is PROSE LEAKING INTO
    // SCHEMA CELLS (the junk-DerivationRule class) — and that is gated
    // STRUCTURALLY in `try_leaf_ingest`: the partial parse must emit
    // ZERO new rows into any schema-critical cell or the leaf path
    // declines loudly. Gating on what the parse EMITS subsumes gating
    // on what sections the file declares.
    const SCHEMA_HEADERS: [&str; 6] = [
        "## Fact Types", "## Entity Types", "## Value Types",
        "## Derivation Rules", "## State Machine", "## Constraints",
    ];
    for (name, text) in &changed {
        if !text.contains("## Instance Facts") {
            eprintln!("[load] leaf-ingest declined: {} changed without an \
                       Instance Facts section", name);
            return None;
        }
        if let Some(h) = SCHEMA_HEADERS.iter().find(|h| text.contains(*h)) {
            eprintln!("[load] leaf-ingest declined: {} carries schema section \
                       {:?}", name, h);
            return None;
        }
    }
    Some(changed.into_iter().map(|(n, _)| n.clone()).collect())
}

/// 987-A.2 → DEFAULT-ON (user directive 2026-06-12, after arc hit the
/// detection-only line in production: "that shouldn't be necessary").
/// The v1 opt-in served its purpose: equivalence proven at fixture
/// scale (app-UoD byte-identical, divergence fully classified) and at
/// arc scale (canary series), every decline path falls back LOUDLY to
/// the full pipeline, and eligibility is narrow by construction
/// (instance-only deltas, structural schema-cell gate). The env var
/// inverts to an escape hatch: AREST_LEAF_INGEST=0 forces the full
/// pipeline (e.g. to reconcile a removed instance line immediately
/// rather than at the next schema-touching compile).
#[cfg(feature = "local")]
fn leaf_ingest_enabled() -> bool {
    std::env::var("AREST_LEAF_INGEST").map(|v| v != "0").unwrap_or(true)
}

/// 987-A.2 — the leaf-ingest EXECUTION path (arc ask #3, percept-only
/// bulk load). For an ELIGIBLE delta (instance-only files, schema
/// provably unchanged — `leaf_only_changed_files`), ingest the changed
/// files as ONE BIG APPLY instead of a full recompile:
///
///   - skip the corpus pre-parse: the prior db's schema cells (Noun /
///     FactType / Role and everything else) ARE the parse context —
///     the prior compile already paid for them;
///   - parse + fold ONLY the changed files (same in-domain parse and
///     ns-3/ns-4 stamping as the full path's per-file fold);
///   - skip `compile_to_defs_state`: schema unchanged ⇒ defs unchanged
///     (the Generator-opt-in guard declines the one counterexample);
///   - skip the #836 wipe: the wipe exists for support REMOVAL; this
///     path's contract is ADDITIVE (a removed instance line does not
///     retract until the next full compile — printed loudly), and
///     aggregate/superlative supersession rides the keyed upsert
///     (`_UpsertSafeCells` / `_CellKeyRoles`) exactly as it does on
///     every `apply` mutation;
///   - run the SAME seeded semi-naive chain the apply path runs — all
///     rule defs, sidecar-gated, seeded with the cells the changed
///     files wrote; sidecar-less rules (aggregates, the SM fold
///     family) run every round, conservatively — NOT an SM-only
///     stratum, so a percept that triggers an SM event still updates
///     every downstream bridge exactly as an apply would;
///   - tail parity: converged-sig stores → persist-dedup → schema
///     reflection (new instances need their membership sweep) → DDL →
///     persist.
///
/// Returns true when the leaf path ran to completion and persisted;
/// false declines LOUDLY with nothing persisted (caller falls through
/// to the full pipeline). Known v1 staleness (documented, full compile
/// reconciles): the `Provenance` cell (NORMA export tabs) is not
/// rebuilt here — `--export-norma` invocations decline at the caller.
#[cfg(feature = "local")]
fn try_leaf_ingest(
    conn: &rusqlite::Connection,
    db_path: Option<&str>,
    readings: &[(String, String)],
    changed: &[String],
) -> bool {
    let t0 = std::time::Instant::now();
    // Generator opt-ins change the def surface (sql:sqlite DDL, norma
    // model, openapi cells) — that's a compile_to_defs_state matter.
    if let Some((name, _)) = readings.iter()
        .filter(|(n, _)| changed.iter().any(|c| c == n))
        .find(|(_, t)| t.contains("uses Generator '")) {
        eprintln!("[load] leaf-ingest declined: {} adds or edits a Generator \
                   opt-in (the def surface changes — full compile)", name);
        return false;
    }
    // ONE load: full prior state — schema cells, def cells
    // (`derivation:*`, `derivation_reads:*`, `_CellKeyRoles`,
    // `_UpsertSafeCells`, …) and every population. Serves as both the
    // parse context and the merge base. Rides the loadcache sidecar
    // when fresh (459b3900).
    let loaded = load_state_cached(conn, db_path);
    // 987-A.3: keep the load-time snapshot for the end-of-tail delta
    // diff (Object clones are Arc bumps — shallow and cheap).
    let snapshot = loaded.clone();
    let t_load = t0.elapsed();
    // 987-A.3 increment 2: per-phase wall clocks. Canary-2 showed
    // ingest-2 at 773s with load 18.3s + chain 20 facts + persist 21
    // cells — i.e. ~740s in the un-instrumented middle. This summary
    // line is what increment 2's next kill gets chosen from.
    let mut laps: Vec<(&str, std::time::Duration)> = Vec::new();
    let mut t_phase = std::time::Instant::now();
    // Parse ONLY the changed files, in readings order, each against
    // the ACCUMULATED context (prior state + earlier changed files).
    // The prior state carries the complete metamodel + app schema
    // catalog, which is exactly what the full path's corpus pre-parse
    // + noun/FT/Role seed exist to provide (arc issues 14/14b).
    // Structural schema gate (987-A.2): snapshot the row counts of
    // every schema-critical cell BEFORE the fold. The textual section
    // gate upstream can't see prose leaking into schema cells (the
    // junk-DerivationRule class — `## Description` sections are
    // allowed through for exactly the arc run-file shape), so after
    // the fold we require these cells to have grown by ZERO rows.
    // merge_states is identity-aware: a context echo adds nothing, so
    // any growth is a genuinely new schema row → decline. `Migration`
    // is here because compile_migration_defs mints one def per
    // Migration instance — a new instance changes the def surface.
    const LEAF_SCHEMA_CELLS: [&str; 11] = [
        "Noun", "FactType", "Role", "Subtype", "RefScheme",
        "Constraint", "DerivationRule", "EnumValues", "StateMachine",
        "State Machine", "Migration",
    ];
    let cell_rows = |state: &ast::Object, name: &str| -> usize {
        let c = ast::fetch_cell_seq(name, state);
        c.as_seq().map(|s| s.len())
            .or_else(|| c.as_map().map(|m| m.len()))
            .unwrap_or(0)
    };
    let mut merged = loaded;
    let schema_rows_before: Vec<usize> = LEAF_SCHEMA_CELLS.iter()
        .map(|n| cell_rows(&merged, n)).collect();
    // 987-A.3 increment 2 (canary-3: parse+gate was 17.6s for a
    // 14-LINE file): parse against the 3-cell resolution catalog
    // (Noun / FactType / Role — exactly what the full path's corpus
    // seed pass extracts, arc issues 14/14b), NOT the whole 171MB
    // state. Parsed outputs accumulate into the catalog so
    // cross-file references within one delta still resolve.
    let mut parse_ctx: ast::Object = {
        let mut m: hashbrown::HashMap<String, ast::Object> = hashbrown::HashMap::new();
        for c in ["Noun", "FactType", "Role"] {
            m.insert(c.to_string(), ast::fetch_cell_seq(c, &merged));
        }
        ast::Object::map(m)
    };
    let mut targets: hashbrown::HashSet<String> = hashbrown::HashSet::new();
    for (name, text) in readings.iter().filter(|(n, _)| changed.iter().any(|c| c == n)) {
        let this = match parse_forml2::parse_to_state_from_in_domain(
            text.as_str(), &parse_ctx, name.as_str()) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("[load] leaf-ingest declined: {}: {} (the full \
                           pipeline will report it properly)", name, e);
                return false;
            }
        };
        let this = ast::annotate_noun_domain(&this, name.as_str());
        let this = ast::merge_states(&this, &ast::stamp_file_domain(&this, name.as_str()));
        targets.extend(ast::cells_iter(&this).into_iter()
            .filter(|(_, c)| c.as_seq().map(|s| !s.is_empty()).unwrap_or(false)
                || c.as_map().map(|m| !m.is_empty()).unwrap_or(false))
            .map(|(n, _)| n.to_string()));
        parse_ctx = ast::merge_states(&parse_ctx, &this);
        merged = ast::merge_states(&merged, &this);
    }
    if targets.is_empty() {
        eprintln!("[load] leaf-ingest declined: the changed files parsed to \
                   no non-empty cells");
        return false;
    }
    // Structural schema gate, post-fold half: any new schema-cell row
    // means the changed files were NOT purely instance-level after all
    // (prose leak or a stray declaration) — full compile owns that.
    for (i, name) in LEAF_SCHEMA_CELLS.iter().enumerate() {
        let after = cell_rows(&merged, name);
        if after > schema_rows_before[i] {
            eprintln!("[load] leaf-ingest declined: the changed files emitted \
                       {} new row(s) into schema cell `{}` (prose leak or \
                       stray declaration) — full compile",
                after - schema_rows_before[i], name);
            return false;
        }
    }
    laps.push(("parse+gate", t_phase.elapsed())); t_phase = std::time::Instant::now();
    // UC upsert (984-B parity): corrected single-valued facts in the
    // changed files displace stale priors at the same key BEFORE the
    // chain reads the population — same order as the full path.
    let (d, reconciled_cells) = {
        let key_roles = crate::evaluate::read_cell_key_roles(&merged);
        let (next, displaced) = ast::reconcile_keyed_cells(&merged, &key_roles);
        for (cell, n) in &displaced {
            eprintln!("[load] UC upsert (leaf): {} — {} stale row(s) displaced \
                       by a later value at the same key", cell, n);
        }
        let names: Vec<String> = displaced.iter().map(|(c, _)| c.clone()).collect();
        (next, names)
    };
    laps.push(("reconcile", t_phase.elapsed())); t_phase = std::time::Instant::now();
    // compile-chain-before-reflect-lag, LEAF half: the full path now
    // reflects schema-as-facts BEFORE its chain; mirror that here or
    // the leaf chain cannot see membership/schema reflections for the
    // delta's OWN new instances. (Fixture v2 caught it: the
    // inherited-instance rows `<<Resource, j2>, <Noun, Resource>>`
    // for the delta's new resources existed in the full-recompile db
    // but lagged on the leaf db — the prior db's reflection cells
    // cover PRIOR resources only.) Reflection cells whose content
    // actually changed join the chain seed so the rules reading them
    // (inherited-instance, ns-domain) re-fire for the new resources.
    let d = {
        let mut map: hashbrown::HashMap<String, ast::Object> =
            ast::cells_iter(&d).into_iter()
                .map(|(name, contents)| (name.to_string(), contents.clone()))
                .collect();
        for (name, contents) in crate::compile::reflect_schema_cells(&d) {
            let changed = map.get(&name).map_or(true, |prior| *prior != contents);
            if changed { targets.insert(name.clone()); }
            map.insert(name, contents);
        }
        ast::Object::map(map)
    };
    laps.push(("reflect-pre", t_phase.elapsed())); t_phase = std::time::Instant::now();
    // Seeded chain — the apply path's exact rule pack: user rules +
    // the synthetic SM family, sidecars as stored. Seed = the cells
    // the changed files wrote PLUS the reflection cells the delta
    // moved; only rules (transitively) touching those fire. A
    // pure-percept delta read by no rule derives nothing and the
    // chain is effectively free.
    let collect = |prefix: &str, state: &ast::Object| -> Vec<(String, ast::Func)> {
        ast::cells_iter(state).into_iter()
            .filter(|(n, _)| n.starts_with(prefix))
            .map(|(n, contents)| (n.to_string(), ast::metacompose(contents, state)))
            .collect()
    };
    let mut rules = collect("derivation:rule_", &d);
    // 987-A.3 increment 2 (canary-3 phase split: chain = 299s of the
    // 511s wall, with only 20 facts derived): the SM fold family is
    // deliberately sidecar-less and re-folds EVERY resource of EVERY
    // SM noun each round (compile.rs sm-retire note). On the leaf
    // path that is provably wasted work for untouched nouns: their
    // event/primary cells are unchanged, so the deterministic fold
    // output is already in the state (AREST.tex Thm Derivability) —
    // no new facts are possible, and skipping cannot change the LFP.
    // Pack the SM family ONLY for nouns the delta touches: FT cell
    // ids prefix their subject noun (`Run_plays_Game` → noun `Run`),
    // and unary trigger cells share the prefix. The membership cell
    // changing does NOT touch other nouns: its new rows belong to
    // the delta's own (touched-noun) resources.
    const SM_FAMILIES: [&str; 4] = [
        "derivation:_sm_init_", "derivation:_sm_event_fold_",
        "derivation:_sm_for_resource_backfill_",
        "derivation:_sm_instance_of_def_backfill_",
    ];
    let touched_noun = |noun_suffix: &str| -> bool {
        let prefix = alloc::format!("{}_", noun_suffix.replace(' ', "_"));
        targets.iter().any(|t| t.starts_with(prefix.as_str()))
    };
    let mut n_sm_total = 0usize;
    for family in SM_FAMILIES {
        for (n, contents) in ast::cells_iter(&d).into_iter()
            .filter(|(n, _)| n.starts_with(family)) {
            n_sm_total += 1;
            if touched_noun(&n[family.len()..]) {
                rules.push((n.to_string(), ast::metacompose(contents, &d)));
            }
        }
    }
    let n_sm_packed = rules.len() - rules.iter()
        .filter(|(n, _)| n.starts_with("derivation:rule_")).count();
    if n_sm_total > n_sm_packed {
        eprintln!("[load] leaf-ingest: SM fold pack noun-scoped — {} of {} \
                   SM defs packed (untouched nouns' folds are deterministic \
                   no-ops, skipped)", n_sm_packed, n_sm_total);
    }
    let n_rules = rules.len();
    let (d, n_derived, chain_written) = if rules.is_empty() {
        (d, 0usize, hashbrown::HashSet::<String>::new())
    } else {
        let packed: Vec<(&str, &ast::Func, Option<Vec<String>>)> = rules.iter()
            .map(|(name, func)| {
                let id = name.split_once(':').map(|(_, id)| id).unwrap_or(name.as_str());
                (name.as_str(), func, crate::evaluate::read_derivation_reads(&d, id))
            })
            .collect();
        let refs: Vec<(&str, &ast::Func, Option<&[String]>)> = packed.iter()
            .map(|(name, func, reads)| (*name, *func, reads.as_deref()))
            .collect();
        // derivation-semi-naive-delta-joins stage 2: the seed cells'
        // NEW ROWS (current minus snapshot, identity by encoding) ride
        // along so AREST_DELTA_JOINS=1 can evaluate sidecar'd rules
        // over per-antecedent delta views. New cells short-circuit
        // (empty prior set → every row is delta).
        let seed_delta: hashbrown::HashMap<String, Vec<ast::Object>> = targets.iter()
            .map(|cell| {
                let prior: hashbrown::HashSet<String> =
                    ast::fetch_cell_seq(cell, &snapshot).as_seq()
                        .map(|s| s.iter().map(|f| f.to_string()).collect())
                        .unwrap_or_default();
                let rows: Vec<ast::Object> = ast::fetch_cell_seq(cell, &d).as_seq()
                    .map(|s| s.iter()
                        .filter(|f| prior.is_empty() || !prior.contains(&f.to_string()))
                        .cloned().collect())
                    .unwrap_or_default();
                (cell.clone(), rows)
            })
            .filter(|(_, rows)| !rows.is_empty())
            .collect();
        let (new_d, derived) = crate::evaluate::forward_chain_defs_state_seeded_with_delta(
            &refs, targets.iter().cloned().collect(), seed_delta, &d, 100);
        if crate::evaluate::take_chain_abort() {
            eprintln!("[load] leaf-ingest declined: the seeded chain hit its \
                       time budget — NOTHING was persisted; falling back to \
                       the full pipeline");
            return false;
        }
        let written: hashbrown::HashSet<String> =
            derived.iter().map(|f| f.fact_type_id.clone()).collect();
        (new_d, derived.len(), written)
    };
    laps.push(("chain", t_phase.elapsed())); t_phase = std::time::Instant::now();
    // Tail parity with the full pipeline, same order: converged-sig
    // stores → persist-dedup → schema reflection → DDL → persist. The
    // reflection re-runs because changed files can mint NEW instances
    // (the Resource_is_instance_of_Noun membership sweep must see
    // them); every reflection layer is set-replace idempotent.
    let d = ast::store("_CompileSig", ast::Object::atom(&compile_input_sig(readings)), &d);
    let d = ast::store("_FileSigs", ast::Object::atom(&encode_file_sigs(readings)), &d);
    // 987-A.3: SCOPED dedup — GC+dedup only the cells this ingest
    // touched. Increment 2 (canary-3: the diff computing this set
    // cost 16.2s of full-store compares): build it from what we
    // ALREADY KNOW wrote — parse targets (reflect-moved cells
    // included), chain-derived consequents, reconcile-displaced
    // cells — instead of diffing the whole store. One full diff per
    // ingest remains (the persist delta below); equivalence is
    // guarded by the A/B fixture.
    let touched: hashbrown::HashSet<String> = targets.iter().cloned()
        .chain(chain_written.iter().cloned())
        .chain(reconciled_cells.iter().cloned())
        .collect();
    let d = super::dedup_state_for_persist_scoped(&d, &touched);
    laps.push(("dedup-scoped+diff1", t_phase.elapsed())); t_phase = std::time::Instant::now();
    let d = {
        let mut map: hashbrown::HashMap<String, ast::Object> =
            ast::cells_iter(&d).into_iter()
                .map(|(name, contents)| (name.to_string(), contents.clone()))
                .collect();
        for (name, contents) in crate::compile::reflect_schema_cells(&d) {
            map.insert(name, contents);
        }
        ast::Object::map(map)
    };
    laps.push(("reflect-tail", t_phase.elapsed())); t_phase = std::time::Instant::now();
    // 987-A.3 delta persist: final diff vs the load snapshot — the
    // exact cell set this ingest changed (parse + chain + reconcile +
    // dedup + reflect inclusive; reflect layers that reproduced their
    // prior bytes drop out of the diff naturally). A vanished cell
    // would mean the additive contract broke somewhere — fall back to
    // the full persist, loudly.
    let final_delta = ast::diff_cells(&snapshot, &d);
    let changed_cells: hashbrown::HashSet<String> =
        ast::cells_iter(&final_delta).into_iter().map(|(n, _)| n.to_string()).collect();
    let final_names: hashbrown::HashSet<&str> =
        ast::cells_iter(&d).into_iter().map(|(n, _)| n).collect();
    let vanished: Vec<String> = ast::cells_iter(&snapshot).into_iter()
        .filter(|(n, _)| !final_names.contains(n))
        .map(|(n, _)| n.to_string())
        .collect();
    laps.push(("diff-final", t_phase.elapsed())); t_phase = std::time::Instant::now();
    if !vanished.is_empty() {
        eprintln!("[load] leaf-ingest: {} cell(s) vanished {:?} — additive \
                   contract anomaly; falling back to the FULL persist",
            vanished.len(), vanished);
        db::apply_ddl(conn, &d);
        db::persist_state(conn, &d);
    } else {
        // Schema unchanged by construction ⇒ tables/triggers exist:
        // skip apply_ddl (and its fossil sweep) entirely.
        let (n_cells, n_tables) = db::persist_state_delta(conn, &d, &changed_cells);
        eprintln!("[load] leaf-ingest delta-persist: {} cell(s) upserted, {} \
                   projection table(s) refreshed ({} cells total in store)",
            n_cells, n_tables, final_names.len());
    }
    laps.push(("persist", t_phase.elapsed())); t_phase = std::time::Instant::now();
    // 987-A.3: refresh the loadcache sidecar IN PLACE — the leaf path
    // holds the final tree, so the NEXT reader pays a sidecar decode
    // (~13s at 171MB) instead of a full re-parse (~98s measured).
    if let Some(path) = db_path {
        if let Some(key) = db_load_cache_key(path) {
            crate::loadcache::store(std::path::Path::new(path), key, &d);
            eprintln!("[load] leaf-ingest: loadcache sidecar refreshed in place");
        }
    }
    laps.push(("sidecar", t_phase.elapsed()));
    eprintln!("[load] leaf-ingest phases: load {:?}, {}",
        t_load,
        laps.iter().map(|(n, dur)| alloc::format!("{} {:?}", n, dur))
            .collect::<Vec<_>>().join(", "));
    eprintln!("[load] leaf-ingest EXECUTED: {} changed file(s) {:?} → {} target \
               cell(s); {} rule def(s) packed (seeded), {} fact(s) derived; \
               {} cell(s) in the persist delta; load {:?}, total {:?}. \
               Additive contract: a REMOVED instance line does not retract on \
               this path — the next full compile reconciles.",
        changed.len(), changed, targets.len(), n_rules, n_derived,
        changed_cells.len(), t_load, t0.elapsed());
    true
}

/// load-state-cache-or-warm-engine LEVER B — warm engine v1 (user
/// 6/30 deadline escalation, 2026-06-12). Hold the decoded state
/// RESIDENT and serve the single-SYSTEM verb protocol over TCP
/// localhost: the per-call spawn cost (process boot + 13-25s state
/// decode at arc scale) drops to zero for routed calls.
///
/// Protocol: one request per connection — a single JSON line
/// `{"key": "...", "input": "..."}` — answered with the verb's raw
/// output; connection closes. SINGLE-THREADED accept loop: writes
/// serialize by construction (closes R2, the snapshot race) and an
/// in-flight op always completes atomically even if the client
/// vanishes (closes R1 — committed-after-abandon is logged, not
/// lost).
///
/// Lifecycle: a port file `<db>.warm` (port / pid / binary hash)
/// advertises the process; callers route to it when live, else fall
/// back to per-call spawn — zero-config compat. The loop exits
/// CLEANLY (sidecar refreshed, port file removed) when the on-disk
/// binary changes (redeploy) or a FOREIGN process writes the db
/// (spawn-path recompile) — fallback serves until `serve` restarts.
///
/// Persistence: mutating verbs delta-persist immediately (changed
/// cells + scoped 3NF projection; full persist on the rare vanished
/// cell). The loadcache sidecar refreshes on EXIT, not per write —
/// a per-write refresh costs ~6-10s at arc scale and only benefits
/// fallback spawns; the tradeoff (one re-parse on the first fallback
/// call after unflushed writes) is recorded here deliberately.
#[cfg(feature = "local")]
fn serve_loop(conn: &rusqlite::Connection, db_path: &str) -> ! {
    use std::io::{BufRead, BufReader, Write as IoWrite};
    let exe_disk_sig = |p: &std::path::Path| -> u64 {
        let md = std::fs::metadata(p).ok();
        let len = md.as_ref().map(|m| m.len()).unwrap_or(0);
        let mtime = md.and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|dur| dur.as_nanos() as u64).unwrap_or(0);
        let mut h: u64 = 0xcbf29ce484222325;
        for v in [len, mtime] {
            for b in v.to_le_bytes() { h ^= b as u64; h = h.wrapping_mul(0x100000001b3); }
        }
        h
    };
    let exe_path = std::env::current_exe().ok();
    let my_exe_sig = exe_path.as_deref().map(exe_disk_sig);
    let t0 = std::time::Instant::now();
    let mut d = load_and_compile(conn, Some(db_path));
    eprintln!("[serve] state resident in {:?}", t0.elapsed());
    let listener = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap_or_else(|e| { eprintln!("serve: bind failed: {}", e); std::process::exit(1); });
    let port = listener.local_addr().map(|a| a.port()).unwrap_or(0);
    let warm_path = alloc::format!("{}.warm", db_path);
    let _ = std::fs::write(&warm_path, alloc::format!(
        "{}\n{}\n{:016x}\n", port, std::process::id(), binary_self_hash()));
    let db_mtime = || std::fs::metadata(db_path).ok().and_then(|m| m.modified().ok());
    let mut last_mtime = db_mtime();
    eprintln!("[serve] warm engine: 127.0.0.1:{} db={} pid={} (port file {})",
        port, db_path, std::process::id(), warm_path);
    let cleanup_exit = |state: &ast::Object, why: &str| -> ! {
        eprintln!("[serve] exiting: {} — sidecar refresh + port-file removal", why);
        if let Some(key) = db_load_cache_key(db_path) {
            crate::loadcache::store(std::path::Path::new(db_path), key, state);
        }
        let _ = std::fs::remove_file(&warm_path);
        std::process::exit(0);
    };
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        if let (Some(p), Some(sig)) = (exe_path.as_deref(), my_exe_sig) {
            if exe_disk_sig(p) != sig {
                cleanup_exit(&d, "binary changed on disk (redeploy)");
            }
        }
        if db_mtime() != last_mtime {
            cleanup_exit(&d, "db written by a foreign process");
        }
        let mut reader = BufReader::new(&stream);
        let mut line = String::new();
        if reader.read_line(&mut line).is_err() || line.trim().is_empty() { continue; }
        let (key, input) = match serde_json::from_str::<serde_json::Value>(line.trim()) {
            Ok(v) => (
                v.get("key").and_then(|k| k.as_str()).unwrap_or("").to_string(),
                v.get("input").and_then(|i| i.as_str()).unwrap_or("").to_string(),
            ),
            Err(e) => {
                let mut s = stream;
                let _ = writeln!(s, "{{\"error\":\"bad request: {}\"}}", e);
                continue;
            }
        };
        if key.is_empty() {
            let mut s = stream;
            let _ = writeln!(s, "{{\"error\":\"missing key\"}}");
            continue;
        }
        if key == "shutdown" { cleanup_exit(&d, "shutdown requested"); }
        let t_req = std::time::Instant::now();
        let (output, new_d) = system(&key, &input, &d);
        if new_d != d {
            // Delta-persist the mutation immediately (same machinery as
            // the leaf tail); the resident tree is the source of truth.
            let delta = ast::diff_cells(&d, &new_d);
            let changed: hashbrown::HashSet<String> = ast::cells_iter(&delta)
                .into_iter().map(|(n, _)| n.to_string()).collect();
            let vanished = {
                let new_names: hashbrown::HashSet<&str> =
                    ast::cells_iter(&new_d).into_iter().map(|(n, _)| n).collect();
                ast::cells_iter(&d).into_iter().any(|(n, _)| !new_names.contains(n))
            };
            if vanished {
                db::apply_ddl(conn, &new_d);
                db::persist_state(conn, &new_d);
            } else {
                let (_nc, _nt) = db::persist_state_delta(conn, &new_d, &changed);
            }
            d = new_d;
            last_mtime = db_mtime();
        }
        eprintln!("[serve] {} in {:?}", key, t_req.elapsed());
        let mut s = stream;
        // Best-effort response: a vanished client cannot un-commit the
        // op above (R1 — committed-after-abandon, logged not lost).
        if writeln!(s, "{}", output).is_err() {
            eprintln!("[serve] client vanished after {} — result committed, response dropped", key);
        }
    }
    cleanup_exit(&d, "listener closed");
}

/// load-state-cache lever A: the sidecar key — a function of the
/// engine identity (binary self-hash; a rebuilt engine must never
/// serve a stale tree) and the db file identity (length + mtime; any
/// SQLite commit moves them, so writes self-invalidate and the next
/// reader re-parses once and rewrites the sidecar).
#[cfg(feature = "local")]
fn db_load_cache_key(db_path: &str) -> Option<u64> {
    let md = std::fs::metadata(db_path).ok()?;
    let mtime = md.modified().ok()?
        .duration_since(std::time::UNIX_EPOCH).ok()?
        .as_nanos() as u64;
    let mut h: u64 = 0xcbf29ce484222325;
    for v in [binary_self_hash(), md.len(), mtime] {
        for b in v.to_le_bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
    }
    Some(h)
}

/// `db::load_state` behind the sidecar cache (task
/// load-state-cache-or-warm-engine, lever A): a HIT decodes the binary
/// tree (no tokenization — the ~2-minute `Object::parse` pass over a
/// 113 MB cells store drops to the file read + decode); a MISS parses
/// as before and writes the sidecar for every later spawn. Read-side
/// only by design: persist is NOT hooked, so the first reader after
/// any write pays one parse and the cache can never diverge from what
/// `load_state` would return. `AREST_LOAD_CACHE=0` bypasses entirely.
#[cfg(feature = "local")]
fn load_state_cached(conn: &rusqlite::Connection, db_path: Option<&str>) -> ast::Object {
    let cache_on = std::env::var("AREST_LOAD_CACHE").as_deref() != Ok("0");
    let key = cache_on
        .then(|| db_path)
        .flatten()
        .and_then(db_load_cache_key);
    if let Some(k) = key {
        if let Some(state) = crate::loadcache::load(
            std::path::Path::new(db_path.unwrap()), k) {
            eprintln!("[profile] load_state: sidecar cache HIT");
            return state;
        }
    }
    let loaded = db::load_state(conn);
    if let Some(k) = key {
        crate::loadcache::store(std::path::Path::new(db_path.unwrap()), k, &loaded);
        eprintln!("[load] load-state sidecar written (next spawn skips the parse)");
    }
    loaded
}

#[cfg(feature = "local")]
fn load_and_compile(conn: &rusqlite::Connection, db_path: Option<&str>) -> ast::Object {
    let t = std::time::Instant::now();
    let loaded = load_state_cached(conn, db_path);
    eprintln!("[profile] load_state: {:?}", t.elapsed());
    // Strip stale persisted compiled defs — keep population cells only.
    // Recompiled below; a stale orphan must not survive into the apply D.
    let population = population_only(&loaded);
    let t = std::time::Instant::now();
    let mut defs = compile::compile_to_defs_state(&population);
    defs.push(("compile".to_string(), ast::Func::Platform("compile".to_string())));
    defs.push(("apply".to_string(), ast::Func::Platform("apply_command".to_string())));
    defs.push(("verify_signature".to_string(), ast::Func::Platform("verify_signature".to_string())));
    defs.push(("audit".to_string(), ast::Func::Platform("audit".to_string())));
    defs.push(("induce".to_string(), ast::Func::Platform("induce".to_string())));
    let d = ast::defs_to_state(&defs, &population);
    eprintln!("[profile] compile: {:?} ({} defs)", t.elapsed(), defs.len());
    d
}

/// read-path-fast-path: keys whose CLI dispatch is a *pure read-only
/// projection* over the loaded cell graph and therefore does NOT need
/// the write-only compile work (`population_only` strip + recompile +
/// `defs_to_state`) that `load_and_compile` performs.
///
/// `sql` (sql.rs), `cells` (cells_introspect.rs) and `orient`
/// (orient.rs) are the three read-only intercepts in `system()` above
/// (entry.rs:221/231/240): each takes `&d`, returns `d.clone()`
/// unchanged, and reads only POPULATION cells plus — for `sql`'s
/// empty-stored view fact types — the persisted `view:` def cells. All
/// of those are loaded verbatim by `db::load_state` (the persisted
/// `defs` table is the compiler's own output cache: a tasks-scale DB
/// carries the `schema:`/`view:`/`derivation:` families pre-built — see
/// the 8.6k-row `defs` table), so a read can serve straight off the
/// loaded snapshot and skip the ~1.3s recompile entirely.
///
/// Write keys (`apply`, `compile`, `retract:`/`assert:` families, the
/// REPL's mixed stream) keep the full `load_and_compile`: they mutate D
/// and the recompile-from-population discipline is load-bearing there
/// (it strips a stale persisted def so it can't reach the apply forward
/// chain — cli-apply-large-tasksdb-nonterminating).
#[cfg(feature = "local")]
fn is_read_only_cli_key(key: &str) -> bool {
    // query-read-fastpath (2026-06-12): the `query:{fact_type}` key
    // family joins the fast path — the MCP query tool dispatches
    // `query:Task_has_Task_Priority`-shaped keys (server.ts
    // systemCall), each a pure read-only projection resolving the
    // PERSISTED `query:{ft}` def (the defs table carries them — the
    // persist_state test fixture's own example is `query:Ticket`).
    // Measured on arc-agi-3 (113 MB): ~8s of the 23s steady-state
    // canary was load_and_compile's recompile, which this skip
    // removes. `get` stays OFF the fast path: it is federation-aware
    // (external nouns absorb fetched rows + Citations — a write).
    matches!(key, "sql" | "cells" | "orient") || key.starts_with("query:")
}

/// read-path-fast-path: lighter load for the read-only verbs.
///
/// All three read keys skip the `population_only` strip,
/// `compile::compile_to_defs_state`, and `ast::defs_to_state` that only
/// the write path needs (the ~1.3s recompile win). `sql` additionally
/// prunes the def table to its reachable closure (read-path-defprune):
///
///   * `sql` — `db::load_state_closure` seeded with the `view:` defs
///     (the only defs `sql` resolves, via `ast::resolve_view` for an
///     empty-stored view FT) plus the colon-free platform singletons,
///     then transitively closed over any def those bodies reference.
///     On tasks.db the closure is ~14 defs vs the full 8.6k, so the
///     `load_state` `Object::parse` cost drops from ~0.5s to a blink.
///     Byte-identical: `sql` only ever reaches the loaded closure, and
///     the closure load brings in exactly that (verified on tasks.db).
///
///   * `cells` / `orient` — `db::load_state` (FULL). Both are
///     whole-snapshot consumers whose output is order- or
///     enumeration-sensitive to the LOADED set, so pruning the defs
///     would change the envelope even though it never changes
///     correctness:
///       - `cells list *` enumerates EVERY cell and `cells get <name>`
///         can fetch any persisted def (cells_introspect.rs:93/126), so
///         the whole def table is genuinely reachable.
///       - `orient`'s `recent_changes` truncates an iteration over the
///         loaded `Object::Map` to the first 10 (orient.rs:342); the
///         `hashbrown` map's iteration order is a function of how many
///         entries were inserted, so a pruned load reorders the sample.
///         The population it reads is identical either way — only the
///         arbitrary 10-of-N window would shift — but byte-identity is
///         the contract, so `orient` keeps the full load. (The compile
///         skip is the win it already had; the def-prune is sql-only.)
///
/// Escape hatch: set `AREST_READ_FASTPATH=0` to force the full
/// `load_and_compile` even for a read key (A/B timing, or a fallback if
/// a stale on-disk `defs` cache is ever suspected — a recompile then
/// regenerates the def families from the loaded population).
#[cfg(feature = "local")]
fn load_for_read(conn: &rusqlite::Connection, key: &str, db_path: Option<&str>) -> ast::Object {
    if std::env::var("AREST_READ_FASTPATH").as_deref() == Ok("0") {
        return load_and_compile(conn, db_path);
    }
    let t = std::time::Instant::now();
    let d = if key == "sql" {
        // `sql`: load only the defs reachable from the `view:` +
        // platform-singleton seed (transitively closed). The def-prune
        // already makes this load cheap relative to a sidecar decode of
        // the FULL state, and its byte-identity contract is scoped to
        // the closure — leave it off the sidecar path.
        db::load_state_closure(conn, |name| {
            name.starts_with("view:") || !name.contains(':')
        })
    } else {
        // `cells` / `orient`: whole-snapshot consumers — full load,
        // sidecar-cached (load-state-cache lever A).
        load_state_cached(conn, db_path)
    };
    eprintln!("[profile] load_for_read ({}, compile skipped): {:?}", key, t.elapsed());
    d
}

/// Extract `--db <path>` from `tokens`, returning the chosen path
/// (defaulting to `arest.db`) and the residual args. Mirrors the
/// inline `--db` parser in `main_entry()` but in a form the subcommand
/// dispatchers can call without re-implementing the same fold.
fn take_db_flag(tokens: &[String]) -> (String, Vec<String>) {
    let mut db = "arest.db".to_string();
    let mut rest: Vec<String> = Vec::new();
    let mut expect_db = false;
    for arg in tokens {
        if expect_db {
            db = arg.clone();
            expect_db = false;
            continue;
        }
        if arg == "--db" {
            expect_db = true;
            continue;
        }
        rest.push(arg.clone());
    }
    (db, rest)
}

/// Pure stack-size resolver: maps an optional `AREST_STACK_MB` value (in
/// megabytes) to a worker-thread stack size in bytes. A missing, empty,
/// non-numeric, or zero override falls back to the 512 MiB default. Split out
/// from `desired_stack_bytes` so the parse/default logic is unit-testable
/// without mutating the process environment.
pub fn stack_bytes_from_env(override_mb: Option<String>) -> usize {
    const DEFAULT_MB: usize = 512;
    let mb = override_mb
        .and_then(|s| s.trim().parse::<usize>().ok())
        .filter(|&mb| mb > 0)
        .unwrap_or(DEFAULT_MB);
    mb.saturating_mul(1024 * 1024)
}

/// Worker-thread stack size for the CLI entry, in bytes. The Windows MSVC main
/// thread reserves only 1 MiB — too small for the engine's deep forward-chain
/// recursion on large apps (support.auto.dev: 670 rules over 6461 fact types
/// overflowed mid-fixpoint). `main.rs` runs `main_entry` on a thread sized by
/// this. Override the default with `AREST_STACK_MB` (megabytes).
pub fn desired_stack_bytes() -> usize {
    stack_bytes_from_env(std::env::var("AREST_STACK_MB").ok())
}

/// CLI entry point. Called from src/main.rs's `fn main()` shim.
pub fn main_entry() {
    // Install host entropy source (#591 / #574) BEFORE any subcommand
    // dispatch. `csprng::random_bytes` panics with a "no entropy source
    // installed" message if a caller fires before this — `arest run`
    // and the readings-compile path don't currently consume randomness,
    // but the kernel-shaped `POST /arest/entity` direct-write fallback
    // (already on disk via #614/#615 even when running under the host
    // CLI) *does*, and any future verb that emits opaque entity ids
    // (`csprng::random_bytes` for #614's `k{counter}{fnv}` shape, or
    // a forthcoming UUIDv4 variant) would otherwise trip the lazy-seed
    // panic on first use. Adapter implements `EntropySource` over
    // `getrandom` (Linux/macOS/Windows getrandom(2) /
    // BCryptGenRandom). Calling `install` again would REPLACE the
    // source (entropy.rs:116) — production paths must avoid that;
    // tests swap in `DeterministicSource` via the same hook.
    crate::entropy::install(crate::cli::entropy_host::HostEntropySource::boxed());

    // Install the per-tenant master key (#663) BEFORE any subcommand
    // dispatch. On first run, generates 32 fresh CSPRNG bytes (which
    // is why this MUST follow the entropy::install above — csprng's
    // lazy-seed otherwise panics with "no entropy source installed")
    // and persists to `~/.arest/tenant_master.bin` with mode 0600.
    // On subsequent runs, reads the same file and installs the bytes
    // into the `arest::cell_aead` global slot. Once installed, every
    // cell_seal / cell_open path through the engine has the master
    // available via `cell_aead::current_tenant_master()`.
    //
    // `expect` is the right call here: a failure to read or write
    // `~/.arest/tenant_master.bin` means an unwriteable home directory
    // (read-only filesystem, missing $HOME, broken ACL) — none of
    // which we can recover from at runtime, and all of which the user
    // will recognise from the panic message.
    crate::cli::tenant_master_host::install()
        .expect("tenant master install (#663): \
                 could not read or generate ~/.arest/tenant_master.bin");

    // task-919 gap-4 production wiring: when AREST_APPS_DIR points at a
    // real directory and the local feature is on (rusqlite reach), register
    // the four arest-dev Rebuild SM Platform Functions
    // (rebuild_snapshot/verify/apply_bulk/init) so an arest-dev SM
    // transition can dispatch to them. The install is opt-in by env var so
    // default invocations don't require an apps_dir layout: the names stay
    // absent from PLATFORM_FALLBACK until the operator sets the env. The
    // four names are pre-approved in `ast::APPROVED_PLATFORM_FN_NAMES`
    // (sec-2 audit, _reports/sec-2-platform-audit-2026-04-21.md);
    // filesystem reach inside each handler is bounded to this `apps_dir`
    // via the closure capture in `rebuild::install_rebuild_fns`. The unit
    // pin `rebuild_install_fns_handlers_are_dispatchable_via_platform_apply`
    // covers the install side; this hook is the production caller.
    #[cfg(feature = "local")]
    if let Ok(apps_dir) = std::env::var("AREST_APPS_DIR") {
        let path = std::path::PathBuf::from(&apps_dir);
        if path.is_dir() {
            crate::rebuild::install_rebuild_fns(path);
        }
    }

    // pb-render-fn-contract (§5.2) production wiring: install the
    // reference HTML render function unconditionally — it is PURE
    // (operand in, markup Atom out; no filesystem/network/state reach),
    // pre-approved in `ast::APPROVED_PLATFORM_FN_NAMES`, and inert
    // until a `Render Target` population names `render:html`
    // (readings/ui/render-target.md declares the 'html' target under
    // the ui-readings gate).
    crate::platform::render_html::install();
    // pb-effect-fns-canonical (§5.2) production wiring: the canonical
    // effect bodies. Inert until a Verb's `Function has Name` fact (or a
    // direct apply) names them; both pre-approved (sec-2 audit). The
    // wasm32/uefi targets never reach this entry point, so the
    // http_fetch submodule's target gate matches.
    crate::platform::http_fetch::install();
    crate::platform::notify::install();

    let args: Vec<String> = std::env::args().skip(1).collect();

    // ── Subcommand dispatch ────────────────────────────────────────────
    // Subcommands are detected before flag parsing so they can have
    // their own argv conventions (a free-form app name with embedded
    // dashes / spaces would otherwise collide with --flags here).
    // Matched subcommands consume the rest of argv and return their
    // own exit code; unmatched first args fall through to the legacy
    // single-arg form (`arest <readings_dir>` etc.) below.
    if let Some(verb) = args.first() {
        if verb == "version" || verb == "--version" || verb == "-V" {
            // `arest-cli version` — emit the build provenance embedded by
            // build.rs so the MCP (or an operator) can tell WHICH engine
            // is actually live and whether it matches the repo HEAD. The
            // MCP pins this binary's path at startup and re-spawns it every
            // call, so a stale pin runs a stale engine undetected; this is
            // the running binary self-reporting to close that gap. Feature-
            // independent (no `local`/SQLite reach) and side-effect-free:
            // reads env!() compile-time constants and prints JSON. Shape:
            //   {"sha":"<git HEAD at build>","built":"<UTC>","pkg":"<ver>"}
            // `sha` is "unknown" when git was unavailable at build time.
            println!(
                "{{\"sha\":\"{}\",\"built\":\"{}\",\"pkg\":\"{}\"}}",
                env!("AREST_GIT_SHA"),
                env!("AREST_BUILD_TIME"),
                env!("CARGO_PKG_VERSION"),
            );
            std::process::exit(0);
        }
        if verb == "reload" {
            // `arest reload <file.md>` (#561 / DynRdg-T2) — runtime reading
            // load via SystemVerb::LoadReading. Reads the body off disk,
            // routes through `cli::reload::dispatch` (which opens the
            // configured DB, threads through `dispatch_with_state`, and
            // persists on success). Implemented under the `local` feature
            // because the persist path needs SQLite — without `--features
            // local`, the verb errors with the same "build with --features
            // local" message as the readings-compile flow.
            let (db_path, rest_args) = take_db_flag(&args[1..]);
            #[cfg(feature = "local")]
            {
                let mut stdout = std::io::stdout();
                let mut stderr = std::io::stderr();
                let code = crate::cli::reload::dispatch(
                    &rest_args, &db_path, &mut stdout, &mut stderr);
                std::process::exit(code);
            }
            #[cfg(not(feature = "local"))]
            {
                let _ = (rest_args, db_path);
                eprintln!("`arest reload` requires the `local` feature.");
                eprintln!("  cargo run --bin arest-cli --features local -- reload <file.md>");
                std::process::exit(2);
            }
        }
        if verb == "watch" {
            // `arest watch <dir>` (#561 followup / DynRdg-T2) — poll
            // a directory for `.md` changes and re-apply each via the
            // same `LoadReading` pipeline as `reload`. Same `--db` +
            // `local`-feature shape as `reload`; the call returns
            // only on initial-scan failure (the polling loop runs
            // until SIGTERM).
            let (db_path, rest_args) = take_db_flag(&args[1..]);
            #[cfg(feature = "local")]
            {
                let mut stdout = std::io::stdout();
                let mut stderr = std::io::stderr();
                let code = crate::cli::watch::dispatch(
                    &rest_args, &db_path, &mut stdout, &mut stderr);
                std::process::exit(code);
            }
            #[cfg(not(feature = "local"))]
            {
                let _ = (rest_args, db_path);
                eprintln!("`arest watch` requires the `local` feature.");
                eprintln!("  cargo run --bin arest-cli --features local -- watch <dir>");
                std::process::exit(2);
            }
        }
        if verb == "run" {
            // `arest run <app-name>` (#543) — resolve a Wine App name to
            // its (slug, prefix Directory) pair via wine_app_by_name.
            // Read-only; doesn't load --db, doesn't compile, doesn't
            // execve `wine`. Wine prefix bootstrap lands in #504.
            #[cfg(feature = "wine")]
            {
                let rest: Vec<String> = args.iter().skip(1).cloned().collect();
                // `metamodel_readings()` hands back &'static (&str, &str)
                // pointing into .rodata; flatten to owned (&str, &str)
                // pairs so dispatch's slice signature lines up with what
                // the unit tests pass too.
                let readings: Vec<(&str, &str)> = crate::metamodel_readings()
                    .into_iter()
                    .map(|(n, t)| (*n, *t))
                    .collect();
                let mut stdout = std::io::stdout();
                let mut stderr = std::io::stderr();
                let code = crate::cli::run::dispatch(&rest, &readings, &mut stdout, &mut stderr);
                std::process::exit(code);
            }
            #[cfg(not(feature = "wine"))]
            {
                eprintln!("`arest run` requires the `wine` feature.");
                eprintln!("  cargo run --bin arest-cli --features wine -- run \"App Name\"");
                std::process::exit(2);
            }
        }
    }

    // Parse flags.
    let no_validate = args.iter().any(|a| a == "--no-validate");
    // mcp-apply-stdin-payload: `--stdin-input` reads the <input>
    // argument from STDIN (to EOF) instead of argv. Windows caps a
    // spawned command line at ~32 KB, which capped MCP `apply` batches
    // at ~50 ops (task-930 advertises 4096 atomic ops) and forced bulk
    // loads into independently-committed chunks — forfeiting the
    // all-or-nothing contract. The MCP shim switches to this flag for
    // large payloads; argv stays the path for small ones.
    let stdin_input = args.iter().any(|a| a == "--stdin-input");
    let (db_path, mut rest, _) = args.iter()
        .filter(|a| !matches!(a.as_str(), "--no-validate" | "--strict" | "--stdin-input"))
        .fold(
        ("arest.db".to_string(), Vec::<String>::new(), false),
        |(db, mut rest, expect_db), arg| match (expect_db, arg.as_str()) {
            (true, _) => (arg.clone(), rest, false),
            (false, "--db") => (db, rest, true),
            (false, "--help" | "-h") => {
                println!("Usage: arest-cli [<readings_dir> ...] [--db <path>] [<key> <input>]");
                println!();
                println!("  <dir> [<dir2>]:    compile readings, persist to --db");
                println!("  <key> <input>:     single SYSTEM call against persisted state");
                println!("  (no args):         REPL — load state, interactive system calls");
                println!();
                println!("  --db <path>        SQLite database path (default: arest.db)");
                println!("  --no-validate      skip constraint validation during compile");
                println!("  --strict           reject undeclared nouns (no auto-creation)");
                println!("  --export-norma <f> compile, write NORMA .orm to <f>, exit (no persist)");
                println!("  --stdin-input      read <input> from stdin (avoids the Windows argv cap)");
                std::process::exit(0);
            }
            (false, _) => { rest.push(arg.clone()); (db, rest, false) }
        },
    );

    // task-951: `--export-norma <file>` — compile the readings (which builds
    // the `norma:model` cell when the app opts into the `norma` generator),
    // write that .orm to <file>, and exit WITHOUT persisting — export is
    // read-only w.r.t. the DB. Extracted after the fold so the flag and its
    // value aren't mistaken for readings directories.
    let export_norma_path: Option<String> = {
        let idx = rest.iter().position(|a| a == "--export-norma");
        idx.map(|i| {
            let v = rest.get(i + 1).cloned().unwrap_or_default();
            rest.remove(i);
            if i < rest.len() { rest.remove(i); }
            v
        }).filter(|v| !v.is_empty())
    };

    #[cfg(not(feature = "local"))]
    {
        let _ = &db_path; let _ = &rest; let _ = no_validate; let _ = &export_norma_path; // flags-only invocation
        eprintln!("Build with --features local for SQLite support.");
        eprintln!("  cargo run --bin arest-cli --features local -- <readings_dir>");
        std::process::exit(1);
    }

    #[cfg(feature = "local")]
    {
        // Determine mode from arguments.
        // - Directories → compile readings into DB via SYSTEM
        // - Two args (neither a dir) → single SYSTEM call
        // - No args → error (REPL not yet implemented)

        let dirs: Vec<String> = rest.iter()
            .filter(|a| std::path::Path::new(a).is_dir())
            .cloned().collect();
        let non_dirs: Vec<String> = rest.iter()
            .filter(|a| !std::path::Path::new(a).is_dir())
            .cloned().collect();

        let conn = db::open(&db_path);
        db::ensure_meta_tables(&conn);

        // load-state-cache-or-warm-engine LEVER B: `arest-cli serve
        // --db <path>` — the warm engine. Never returns (exits on
        // redeploy / foreign write / shutdown verb).
        #[cfg(feature = "local")]
        if non_dirs.first().map(|s| s.as_str()) == Some("serve") {
            serve_loop(&conn, &db_path);
        }

        match (dirs.is_empty(), non_dirs.len()) {
            // arest <dir1> [<dir2> ...] — compile readings via SYSTEM
            (false, _) => {
                let readings = read_readings(&dirs);
                readings.is_empty().then(|| {
                    eprintln!("No .md files found.");
                    std::process::exit(1);
                });

                // Extract generator opt-ins from raw reading text before parsing.
                // The parser doesn't yet handle dual-quoted instance facts like
                // "App 'X' uses Generator 'sqlite'" — extract via regex.
                //
                // Generators are App-scoped (`App 'X' uses Generator 'Y'.`):
                // we keep the (App, Generator) pair so downstream generators
                // can emit per-App cells. The set-of-generators view is
                // derived from the pairs for backward-compat paths (SQL
                // trigger emission still keys off generator names only).
                // 987-A: per-file signature delta against the prior
                // compile's `_FileSigs` registry (A.1). When the delta is
                // leaf-only (instance-fact files, no schema sections) AND
                // the operator opted in via AREST_LEAF_INGEST=1, A.2 runs
                // the changed files as one big seeded apply — skipping the
                // corpus re-parse, compile_to_defs_state, the #836 wipe
                // and the full chain — then persists and EXITS. Every
                // decline is loud and falls through to the full pipeline.
                match leaf_only_changed_files(&conn, &readings) {
                    Some(changed) => {
                        eprintln!("[load] leaf-ingest ELIGIBLE: {} instance-only \
                                   file(s) changed {:?}", changed.len(), changed);
                        if export_norma_path.is_some() {
                            eprintln!("[load] leaf-ingest declined: --export-norma \
                                       needs the full compile (Provenance rebuild)");
                        } else if !leaf_ingest_enabled() {
                            eprintln!("[load] leaf-ingest: DISABLED via \
                                       AREST_LEAF_INGEST=0 — full pipeline");
                        } else if try_leaf_ingest(&conn, Some(db_path.as_str()), &readings, &changed) {
                            eprintln!("Compiled {} readings into {} (leaf ingest)",
                                readings.len(), &db_path);
                            std::process::exit(0);
                        } else {
                            eprintln!("[load] leaf-ingest fell back to the full pipeline");
                        }
                    }
                    None => {}
                }

                let opt_in_re = regex::Regex::new(r"App '([^']+)' uses Generator '([^']+)'").unwrap();
                let opt_in_pairs: Vec<(String, String)> = readings.iter()
                    .flat_map(|(_, text)| opt_in_re.captures_iter(text)
                        .filter_map(|c| {
                            let app = c.get(1)?.as_str().to_string();
                            let gen = c.get(2)?.as_str().to_lowercase();
                            Some((app, gen))
                        })
                        .collect::<Vec<_>>())
                    .collect();
                let opted_generators: std::collections::HashSet<String> = opt_in_pairs.iter()
                    .map(|(_, g)| g.clone())
                    .collect();
                eprintln!("[load] opt-in (App, Generator) pairs: {:?}", opt_in_pairs);
                eprintln!("[load] generators (set view): {:?}", opted_generators);

                // Fold readings (metamodel + user) into Object state.
                //
                // Closure Under Self-Modification (AREST.tex Corollary 6
                // + Migration Remark, #831 / cor:closure): compile
                // preserves P. The split:
                //   - SCHEMA cells (Noun, FactType, Role, Constraint,
                //     DerivationRule, EnumValues, InstanceFact, Subtype,
                //     RefScheme, …) are PURE FUNCTIONS of the readings —
                //     parse-emitted, never apply-emitted. They get
                //     rebuilt every compile, so we drop prior copies
                //     before the fold.
                //   - User POPULATION cells (FT cells like
                //     Task_has_Task_Subject, SM cells like
                //     State_Machine_is_currently_in_Status) collect both
                //     apply-emitted entries and chain-derived entries.
                //     These survive recompile.
                //
                // Identification is structural: parse all readings into
                // a fresh state (no prior seed), then any cell name in
                // that parsed state IS readings-derived. Prior cells
                // matching those names get dropped from the preserve
                // set; prior cells NOT matching survive. The earlier
                // `READINGS_DERIVED_META_CELLS` hardcoded list (just
                // "DerivationRule") was a band-aid for this same
                // problem — leaving the rest of the schema cells
                // accumulating stale entries (e.g. the post-#931
                // Derivation Mode InstanceFact migration: prior
                // 'derived-and-stored' values stuck around alongside
                // the corrected 'fully-derived', and index_single's
                // first-wins picked the stale one).
                let all_readings: Vec<(&str, &str)> = crate::metamodel_readings().into_iter()
                    .map(|r| (r.0, r.1))
                    .chain(readings.iter().map(|(n, t)| (n.as_str(), t.as_str())))
                    .collect();
                // perf-metamodel-parse-cache (Step 1 — correctness): fold ONLY
                // the app readings onto the SEEDED, app-independent metamodel
                // parse (`metamodel_parsed_state_seeded`), rather than re-parsing
                // every metamodel slice each compile.
                //
                // The metamodel's circular deps (e.g. core.md uses `Transition`
                // from state.md) were resolved by the metamodel-NOUN seed when the
                // cache was folded — so the phantom that broke the earlier attempt
                // (which cached the UN-seeded fold) cannot arise. App slices are
                // still parsed against the GLOBAL noun catalog (cached metamodel
                // nouns + every app noun) so cross-app-file forward refs resolve
                // exactly as the old full-corpus seed allowed. Gated by the
                // 6254-fact / 838-completed / full-suite equivalence checks; if a
                // metamodel slice genuinely needed an APP noun, those catch it.
                //
                // Step 2 (cross-process): a cold `arest-cli` LOADS the seeded
                // parse from its content-addressed FILE cache (~1-2s) instead of
                // re-folding it (~15s); only a cache MISS (first compile per
                // binary) pays the fold and writes the cache. See the
                // perf-metamodel-parse-cache block above mod `system`.
                let mm_parsed_cached: ast::Object;
                let mm_parsed: &ast::Object = match load_metamodel_parse_cache() {
                    Some(cells) => {
                        eprintln!("[load] metamodel parse: FILE cache hit");
                        mm_parsed_cached = cells;
                        &mm_parsed_cached
                    }
                    None => {
                        let fresh = crate::metamodel_parsed_state_seeded();
                        store_metamodel_parse_cache(fresh);
                        eprintln!("[load] metamodel parse: cache miss, folded + stored");
                        fresh
                    }
                };
                let app_noun_seed: ast::Object = {
                    let corpus: String = readings.iter()
                        .map(|(_, t)| t.as_str()).collect::<Vec<_>>().join("\n\n");
                    if corpus.trim().is_empty() {
                        ast::Object::phi()
                    } else {
                        // arc-agi-3 engine-issue 14b: the pre-parse runs
                        // against the METAMODEL noun catalog, not φ. An app
                        // FT declaration referencing a metamodel noun
                        // (`Case observes Fact` — `Fact` is core
                        // vocabulary) failed to parse in the φ-context
                        // corpus pass, so the FactType seed lacked it and
                        // the per-file fold still mis-filed those instance
                        // facts (arc's round-8 residual: case-*.md
                        // warnings survived the 14a fix). The fold below
                        // already parses every file against mm_parsed +
                        // this seed; giving the seed pass the same noun
                        // context closes the gap. The seed extraction
                        // keeps only Noun/FactType/Role cells, and
                        // fold_base re-merges mm_parsed anyway, so the
                        // metamodel rows the context contributes dedupe
                        // by identity.
                        let mm_noun_ctx: ast::Object = {
                            let mut m: hashbrown::HashMap<String, ast::Object> =
                                hashbrown::HashMap::new();
                            m.insert("Noun".to_string(),
                                ast::fetch_cell_seq("Noun", mm_parsed));
                            ast::Object::map(m)
                        };
                        let full = parse_forml2::parse_to_state_from(&corpus, &mm_noun_ctx)
                            .unwrap_or_else(|e| { eprintln!("app corpus parse: {}", e); std::process::exit(1); });
                        let mut m: hashbrown::HashMap<String, ast::Object> = hashbrown::HashMap::new();
                        m.insert("Noun".to_string(), ast::fetch_cell_seq("Noun", &full));
                        // arc-agi-3 engine-issue 14: ALSO seed the app's
                        // FactType + Role catalogs. The per-file fold below
                        // parses each file against the ACCUMULATED context,
                        // so an instance fact in an alphabetically-early
                        // file could not resolve a fact type declared in a
                        // later file — it mis-filed under its raw verb and
                        // never reached the FT cell (silent data loss, one
                        // layer-1b warning per fact). The full-corpus parse
                        // above already pays for this; extracting the FT
                        // catalog into the seed makes instance-fact
                        // resolution file-order independent, exactly as the
                        // Noun seed already did for noun references.
                        m.insert("FactType".to_string(), ast::fetch_cell_seq("FactType", &full));
                        m.insert("Role".to_string(), ast::fetch_cell_seq("Role", &full));
                        ast::Object::map(m)
                    }
                };
                // Global Noun-only seed (cached metamodel nouns + app nouns) — the
                // shape `build_provenance_cell` expects.
                let global_noun_seed: ast::Object = {
                    let mut m: hashbrown::HashMap<String, ast::Object> = hashbrown::HashMap::new();
                    m.insert("Noun".to_string(), ast::fetch_cell_seq("Noun", mm_parsed));
                    ast::merge_states(&ast::Object::map(m), &app_noun_seed)
                };
                // task-951-b: source-file → ORM-element provenance map for the
                // NORMA exporter's per-file ORMDiagram tabs. Still over every file.
                let provenance_cell = build_provenance_cell(&all_readings, &global_noun_seed);
                // Fold base = cached (seeded) metamodel cells + the app noun
                // catalog; fold ONLY the app readings on top.
                let fold_base = ast::merge_states(mm_parsed, &app_noun_seed);
                let parsed_fresh = readings.iter().fold(
                    fold_base,
                    |merged, (name, text)| {
                        // ns-5: parse knowing this slice's local domain (ns-3
                        // file domain = reading name) so a bare reference to a
                        // locally-declared noun resolves locally (precedence 1).
                        let this = parse_forml2::parse_to_state_from_in_domain(text.as_str(), &merged, name.as_str())
                            .unwrap_or_else(|e| { eprintln!("{}: {}", name, e); std::process::exit(1); });
                        // ns-3: stamp declared Functions with their file domain.
                        // ns-4: tag Noun facts with homeDomain for keyed identity.
                        let this = ast::annotate_noun_domain(&this, name.as_str());
                        let this = ast::merge_states(&this, &ast::stamp_file_domain(&this, name.as_str()));
                        ast::merge_states(&merged, &this)
                    },
                );
                // Empty-cell guard: a cell present-but-empty in the
                // fresh parse means the parser tagged the name but
                // had nothing to emit. Don't drop prior content for
                // these (an empty-readings recompile should preserve
                // prior schema for re-validation, not wipe it).
                let mut parsed_cell_names: hashbrown::HashSet<String> =
                    ast::cells_iter(&parsed_fresh).into_iter()
                        .filter(|(_, c)| c.as_seq().map(|s| !s.is_empty()).unwrap_or(false)
                            || c.as_map().map(|m| !m.is_empty()).unwrap_or(false))
                        .map(|(name, _)| name.to_string())
                        .collect();
                // Exclude FT cells from the drop set: they're named
                // after FactType ids, and readings can pre-populate them
                // (instance facts like `Task '916' has Task Priority 'p1'`
                // fan out into Task_has_Task_Priority). Dropping the prior
                // would wipe apply-emitted entries. Schema cells (Noun,
                // FactType, Role, Constraint, InstanceFact, EnumValues, …)
                // are NOT FactType ids; they get dropped as intended.
                // merge_states is identity-aware (id/name/ruleId keys),
                // so when fresh-parse + prior overlap on an FT cell the
                // dedupe handles it without losing either side.
                let ft_ids: hashbrown::HashSet<String> = ast::fetch_cell_seq("FactType", &parsed_fresh)
                    .as_seq()
                    .map(|facts| facts.iter()
                        .filter_map(|f| ast::binding(f, "id").map(|s| s.to_string()))
                        .collect())
                    .unwrap_or_default();
                parsed_cell_names.retain(|name| !ft_ids.contains(name));
                // task-958: declared arity per FT id, so the subjectless-GC
                // below keys off the SCHEMA arity, not the malformed-inflated
                // modal arity — otherwise a unary FT cell holding both the
                // correct 1-binding apply/transition rows and 2-binding
                // bulk-reading relics infers arity 2 and drops the correct rows
                // (then those resources lose their event facts and SM-init
                // re-seeds them 'pending' on recompile).
                let ft_arity: hashbrown::HashMap<String, usize> =
                    ast::fetch_cell_seq("FactType", &parsed_fresh).as_seq()
                        .map(|facts| facts.iter()
                            .filter_map(|f| Some((
                                ast::binding(f, "id")?.to_string(),
                                ast::binding(f, "arity")?.parse::<usize>().ok()?)))
                            .collect())
                        .unwrap_or_default();
                let prior_population: ast::Object = {
                    let loaded = db::load_state(&conn);
                    // compile-gc-orphaned-derived-facts: cor:closure carries the prior
                    // DB population forward so runtime data survives a recompile; the
                    // preserve-and-GC logic (drop sidecar `:` / fresh-re-emitted cells +
                    // orphan relics whose FactType is no longer declared, then scrub
                    // subjectless rows) lives in ast::preserve_prior_population so it is
                    // unit-testable rather than buried in this binary-only path.
                    let (pop, gc_orphans) = ast::preserve_prior_population(
                        &loaded, &parsed_cell_names, &ft_ids, &ft_arity);
                    if !gc_orphans.is_empty() {
                        eprintln!("[load] cor:closure GC: dropped {} orphaned cell(s) whose \
                                   FactType is no longer declared: {:?}", gc_orphans.len(), gc_orphans);
                    }
                    pop
                };
                let prior_cell_count = ast::cells_iter(&prior_population).len();
                if prior_cell_count > 0 {
                    eprintln!("[load] preserving {} user-population cells from existing DB \
                              (Closure Under Self-Modification, cor:closure)",
                              prior_cell_count);
                }
                // Pass 2 (effective): the merged result is parsed_fresh
                // (clean schema) layered on prior_population (durable
                // user FT/SM cells). merge_states is identity-aware so
                // overlap on FT cells (parser emits some entries via
                // InstanceFact → chain-derive, but FT cells themselves
                // aren't parser-emitted) doesn't lose user data.
                let state = ast::merge_states(&prior_population, &parsed_fresh);
                // Tree-shake the UoD (tree-shake-app-uod-to-reachable-closure):
                // drop bundled-metamodel DOMAIN fact types (ui/os/templates/
                // compat) the app never reaches — and the relics cor:closure
                // carried forward — so an app's UoD is its own schema plus the
                // core substrate, not the whole shared library. Conservative:
                // ONLY domain-module FTs are eligible, and only when unreached
                // from the app's own (non-base) fact types; app and core FTs
                // are always kept. Runs while bootstrap mode is still on so the
                // domain-module re-parse ids match `parsed_fresh`.
                let state = {
                    let ft_ids = |st: &ast::Object| -> hashbrown::HashSet<String> {
                        ast::fetch_cell_seq("FactType", st).as_seq()
                            .map(|fs| fs.iter()
                                .filter_map(|f| ast::binding(f, "id").map(|s| s.to_string()))
                                .collect())
                            .unwrap_or_default()
                    };
                    let all_ft_ids = ft_ids(&state);
                    // perf-metamodel-parse-cache: enumerate the metamodel's FT
                    // ids and bundled-domain FT ids from the already-cached
                    // seeded parse (`mm_parsed`) instead of forcing the cold
                    // `metamodel_state()` fold+compile (~8-9s/process) — both of
                    // these previously triggered it just to read FT/Noun cells.
                    // The metamodel FactType/Noun cells are identical between the
                    // seeded parse and metamodel_state (FT enumeration is
                    // parse-determined), so `roots`/`keep` are unchanged — gated
                    // by the unchanged pruned-count + 6249/838.
                    let base_ft_ids = ft_ids(mm_parsed);
                    let domain_ft_ids = crate::compile::bundled_domain_fact_type_ids_from(mm_parsed);
                    // App roots = fact types the app itself declares (not base).
                    let roots: hashbrown::HashSet<String> =
                        all_ft_ids.difference(&base_ft_ids).cloned().collect();
                    let idx = crate::compile::cell_index_from_state(&state);
                    let reachable = crate::compile::reachable_fact_types(&idx, &roots);
                    let keep: hashbrown::HashSet<String> = all_ft_ids.iter()
                        .filter(|id| !domain_ft_ids.contains(*id) || reachable.contains(*id))
                        .cloned()
                        .collect();
                    let pruned = all_ft_ids.len().saturating_sub(keep.len());
                    if pruned > 0 {
                        eprintln!("[load] tree-shake: pruned {} unreached domain fact \
                                   types ({} of {} kept)", pruned, keep.len(), all_ft_ids.len());
                    }
                    crate::compile::prune_unreachable_fact_types(&state, &keep)
                };

                // Diagnostics: read cell sizes from the Object state.
                let cell_len = |name: &str| ast::fetch_cell_seq(name, &state)
                    .as_seq().map(|s| s.len()).unwrap_or(0);
                eprintln!("[load] {} nouns, {} fts, {} instance facts",
                    cell_len("Noun"), cell_len("FactType"), cell_len("InstanceFact"));
                let ft_cell = ast::fetch_cell_seq("FactType", &state);
                let generator_fts: Vec<String> = ft_cell.as_seq()
                    .map(|facts| facts.iter()
                        .filter_map(|f| ast::binding(f, "id").map(|s| s.to_string()))
                        .filter(|k| k.to_lowercase().contains("generator") || k.to_lowercase().contains("uses"))
                        .collect())
                    .unwrap_or_default();
                eprintln!("[load] Generator-related FTs: {:?}", generator_fts);
                let inst_cell = ast::fetch_cell_seq("InstanceFact", &state);
                let app_ifs: Vec<String> = inst_cell.as_seq()
                    .map(|facts| facts.iter()
                        .filter(|f| ast::binding(f, "subjectNoun") == Some("App")
                            || ast::binding(f, "objectValue").map(|v| v.to_lowercase().contains("sqlite")).unwrap_or(false))
                        .map(|f| format!("{}({}).{}={}({})",
                            ast::binding(f, "subjectNoun").unwrap_or(""),
                            ast::binding(f, "subjectValue").unwrap_or(""),
                            ast::binding(f, "fieldName").unwrap_or(""),
                            ast::binding(f, "objectNoun").unwrap_or(""),
                            ast::binding(f, "objectValue").unwrap_or("")))
                        .collect())
                    .unwrap_or_default();
                eprintln!("[load] App/sqlite instance facts: {:?}", app_ifs);
                let mut state = state;
                // `--no-validate` only matters for runtime `compile` SYSTEM
                // calls (#689). The dirs path parses + persists directly
                // without going through `platform_compile`, so the policy
                // cell would be dead code here — the install lives in the
                // single-SYSTEM and REPL branches below.
                // Store (App, Generator) opt-ins as a cell so compile can
                // emit per-App artifacts (openapi, eventually sqlite/etc.).
                // W2 (task-932): App_uses_Generator is a junction — the pair
                // (App, Generator) is unique, so write via cell_put_keyed
                // keyed by both roles. Idempotent re-registration of the same
                // pair is a set-semantic no-op; a fresh pair gets its own
                // Map entry. On the defensive KeyConflict path (same pair
                // with different non-key roles — structurally impossible for
                // this two-role fact) keep prior state.
                opt_in_pairs.iter().for_each(|(app, g)| {
                    let fact = ast::fact_from_pairs(&[("App", app.as_str()), ("Generator", g.as_str())]);
                    state = ast::cell_put_keyed("App_uses_Generator", &["App", "Generator"], fact, &state)
                        .unwrap_or_else(|_| state.clone());
                });
                // `sql:trigger:*` DDL is already emitted by
                // `compile::compile_to_defs_state` (see compile.rs:1363
                // — `Func::constant(Object::atom(ddl))`). An earlier
                // block here re-materialised the typed derivation-rule
                // + fact-type inputs from cells and called
                // `generate_derivation_triggers` again, but the
                // materialisation only copied three fields out of
                // `DerivationRuleDef` and left `antecedent_sources`
                // empty — the function bails on empty antecedents, so
                // this path always produced zero triggers and the
                // "[load] N SQL triggers generated" log was always
                // "0". Removed; retire four typed-IR materialisations
                // along the way (#325).

                let defs = vec![
                    ("compile".to_string(), ast::Func::Platform("compile".to_string())),
                    ("apply".to_string(), ast::Func::Platform("apply_command".to_string())),
                    ("verify_signature".to_string(), ast::Func::Platform("verify_signature".to_string())),
                    ("audit".to_string(), ast::Func::Platform("audit".to_string())),
                ];
                let d = ast::defs_to_state(&defs, &state);
                let compiled = readings.len();

                // Materialize compile defs (schemas, derivations,
                // sql:sqlite:* DDL, sql:trigger:* triggers, …) into the
                // state cells so apply_ddl + persist_state see them and
                // so user `derivation:rule_*` defs can be forward-chained
                // over the population. Without this step the compile
                // emits cells for instance facts but never fires literal-
                // iff rules, leaving consequent FT cells empty (#822 in
                // apps/tasks).
                // task-951-b: attach the source-file provenance cell so
                // `compile_to_defs_state`'s NORMA generator can emit one
                // ORMDiagram (a NORMA tab) per source file. Stored directly
                // (not via the fold/merge) so it bypasses the schema-cell
                // drop + tree-shake above — it's a fresh, whole-corpus map
                // rebuilt every compile, never persisted population.
                let state = ast::store("Provenance", provenance_cell, &state);
                let compile_defs = crate::compile::compile_to_defs_state(&state);
                let d = ast::defs_to_state(&compile_defs, &d);

                // task-984 part B (arc-agi-3 issue 10): enforce alethic
                // UCs on the LOAD path. The cor:closure merge dedupes
                // identical tuples only, so a single-valued fact
                // CORRECTED in readings coexisted with its stale
                // carried-forward prior. Rebuild each keyed cell via the
                // keyed upsert (Seq order — fresh parsed rows land after
                // priors, so corrected readings win) and report what was
                // displaced. _CellKeyRoles is in `d` as of the
                // defs_to_state above; runs BEFORE the #836 wipe + chain
                // so derivations read the reconciled population.
                let d = {
                    let key_roles = crate::evaluate::read_cell_key_roles(&d);
                    let (next, displaced) = ast::reconcile_keyed_cells(&d, &key_roles);
                    for (cell, n) in &displaced {
                        eprintln!("[load] UC upsert: {} — {} stale row(s) displaced \
                                   by a later value at the same key (alethic UC is \
                                   the policy; corrected readings beat \
                                   carried-forward priors)", cell, n);
                    }
                    next
                };

                // compile-chain-before-reflect-lag: reflect schema-as-facts
                // BEFORE the #836 wipe + forward chain, so metamodel rules
                // whose antecedents are reflection cells (Fact_Type_has_Role,
                // Noun_has_Object_Type, …) fire on the FIRST compile instead
                // of materializing one compile late. Observed class (987-A.2
                // equivalence fixture): the eager Format/Enum projections —
                // compile-1's chain found the reflection cells empty (reflect
                // ran post-chain), derived nothing; compile-2 loaded the
                // persisted reflection cells via cor:closure and fired —
                // i.e. compile(compile(x)) != compile(x), masked on no-change
                // recompiles by the delta-LFP skip. The post-chain reflect
                // below STAYS: it re-canonicalizes the set-replace layers
                // over whatever the chain added; both calls are idempotent
                // pure functions of (schema, population).
                let d = {
                    let mut map: hashbrown::HashMap<String, ast::Object> =
                        ast::cells_iter(&d).into_iter()
                            .map(|(name, contents)| (name.to_string(), contents.clone()))
                            .collect();
                    for (name, contents) in crate::compile::reflect_schema_cells(&d) {
                        map.insert(name, contents);
                    }
                    ast::Object::map(map)
                };

                // Surface deontic (non-blocking) structural findings on the
                // dirs-compile path. `platform_compile` (the runtime `compile`
                // SYSTEM verb / `apps.compile`) already emits these as
                // `[model warning] …` (ast.rs ~3310); the dirs path parses +
                // persists directly without routing through it, so without this
                // the warnings would be invisible from `arest-cli <dir> --db`.
                // Reuse the SAME classified validator and the SAME
                // `[model warning]` prefix — no parallel channel. Currently the
                // only deontic finding produced here is the range-unrestricted
                // derivation-rule warning (an `at most N` rule whose head var is
                // bound only by the count premise → suppressed to φ by
                // `compile_explicit_derivation`); making it loud is the point.
                crate::compile::validate_model_classified_from_state(&state)
                    .iter()
                    .filter(|v| !v.alethic)
                    .for_each(|v| eprintln!("[model warning] {}", v.message));

                // arc-agi-3 engine-issue 2: ALSO run the layered check
                // battery (check_readings_func, layers 1–8) here and print
                // the diagnostics the APP introduced. The dynamic
                // load_reading gate (`validate_loaded_state`) rejects on
                // errors but swallows warnings, and this dirs path never
                // ran the battery at all — so check-layer warnings (layer-7
                // unbound computed bindings, layer-8 widget drift, …) were
                // invisible from `apps.compile` even after the MCP
                // diagnostics channel landed. `[check …]` lines pass the
                // MCP stderr filter.
                //
                // DIFFERENTIAL, not raw: the battery over the merged state
                // also sees the bundled substrate readings, whose ~350
                // pre-existing layer-1 resolver warnings would flood the
                // MCP's 100-line diagnostics cap and bury the app's own
                // signal. Baseline = the battery over the cached seeded
                // metamodel parse (`mm_parsed`, no app); only diagnostics
                // NEW relative to that baseline print. Population-caused
                // findings (atom ids, widget drift) survive the diff — the
                // bare substrate has no app population to fire them.
                {
                    let diag_key = |d: &crate::check::ReadingDiagnostic| {
                        format!("{:?}|{:?}|{}|{}", d.level, d.source, d.reading, d.message)
                    };
                    let baseline: hashbrown::HashSet<String> =
                        crate::load_reading_core::check_state_diagnostics(mm_parsed)
                            .iter().map(&diag_key).collect();
                    let mut suppressed = 0usize;
                    for d in crate::load_reading_core::check_state_diagnostics(&state) {
                        if baseline.contains(&diag_key(&d)) {
                            suppressed += 1;
                            continue;
                        }
                        let loc = if d.line > 0 {
                            format!(" line {}", d.line)
                        } else {
                            String::new()
                        };
                        let fix = d.suggestion.as_deref()
                            .map(|s| format!(" (fix: {})", s))
                            .unwrap_or_default();
                        eprintln!("[check {:?} {:?}]{}: {}{}",
                            d.level, d.source, loc, d.message, fix);
                    }
                    if suppressed > 0 {
                        eprintln!("[check] {} substrate-baseline diagnostics suppressed \
                                   (they fire on the bundled readings alone — not \
                                   app-actionable)", suppressed);
                    }
                }

                // task-951: `--export-norma <file>` short-circuits here.
                // `compile_to_defs_state` has just built the `norma:model`
                // def cell (compile.rs:2912) from the `norma` generator; we
                // write it and exit before the forward-chain + persist, so
                // export never mutates the DB.
                if let Some(out_path) = &export_norma_path {
                    let orm = ast::apply(&ast::Func::Def("norma:model".to_string()),
                        &ast::Object::phi(), &d).as_atom().map(str::to_string).unwrap_or_default();
                    if orm.is_empty() {
                        eprintln!("export-norma: no norma:model cell — does the app opt in with \"App '<name>' uses Generator 'norma'.\"?");
                        std::process::exit(1);
                    }
                    match std::fs::write(out_path, &orm) {
                        Ok(()) => {
                            eprintln!("[export-norma] wrote {} bytes to {}", orm.len(), out_path);
                            std::process::exit(0);
                        }
                        Err(e) => {
                            eprintln!("export-norma: failed to write {}: {}", out_path, e);
                            std::process::exit(1);
                        }
                    }
                }

                // #836 — drop derived consequent cells before forward-
                // chain so the LFP recomputes from primary facts. Per
                // AREST.tex §4.3: "Derivation: forward chaining,
                // monotonic, evaluated to the least fixed point per
                // request. Derivation only adds facts over finite P;
                // the fixed point exists by Knaster-Tarski and is
                // reached in ≤ |P_max|-|P| steps." Without this step,
                // cor:closure preserves derived facts whose primary
                // support has changed, leaving stale conclusions in
                // the population (the #772 stuck-blocked symptom).
                let derived_cells: hashbrown::HashSet<String> = derived_wipe_set(&d);
                // delta-lfp-noop-skip: the derivation output is a deterministic
                // function of (readings, population, binary) — AREST.tex
                // cacheability / Thm. Derivability. The population's derived
                // facts are kept consistent by `apply` (it re-derives on every
                // mutation), so the persisted derived cells — carried into `d` by
                // the cor:closure merge — are ALREADY the valid LFP whenever the
                // READINGS and BINARY are unchanged: `#836`'s drop + re-derive
                // would reproduce them byte-for-byte. Hash both; if the sig
                // matches the one the last *converged* compile stored, skip the
                // drop + chain (the ~14s app SM-fold). Any readings/binary change
                // → full re-derive. Gated by warm-recompile == cold (838).
                // (sig computation extracted to `compile_input_sig` so the
                // leaf-ingest path stores the identical sig — see its doc.)
                let compile_sig: String = compile_input_sig(&readings);
                let prior_sig = ast::fetch_or_phi("_CompileSig", &d).as_atom().map(|s| s.to_string());
                // Only skip when prior derived state actually exists (a populated
                // consequent cell) — never on a first/empty compile.
                let has_derived = derived_cells.iter().any(|c|
                    ast::fetch_cell_seq(c, &d).as_seq().map_or(false, |s| !s.is_empty()));
                let inputs_unchanged = prior_sig.as_deref() == Some(compile_sig.as_str()) && has_derived;
                if inputs_unchanged {
                    eprintln!("[load] derivation skipped: readings+binary unchanged since last \
                               converged compile (delta-LFP no-op)");
                }
                let d = if derived_cells.is_empty() || inputs_unchanged { d } else {
                    eprintln!("[load] dropping {} derived cells before forward-chain (LFP per request, #836): {:?}",
                        derived_cells.len(), derived_cells);
                    let mut new_map: hashbrown::HashMap<String, ast::Object> = hashbrown::HashMap::new();
                    for (name, contents) in ast::cells_iter(&d).into_iter() {
                        if derived_cells.contains(name) {
                            new_map.insert(name.to_string(), ast::Object::phi());
                        } else {
                            new_map.insert(name.to_string(), contents.clone());
                        }
                    }
                    ast::Object::map(new_map)
                };

                // Forward-chain over user `derivation:rule_*` defs to
                // materialize derived FT cells (e.g.
                // `Task has Task Readiness 'ready' iff Task has Task
                // Status 'pending'.` populates Task_has_Task_Readiness
                // alongside the parsed Task_has_Task_Status cell).
                //
                // Negation-stratification retired: only the positive
                // `derivation:rule_*` stratum exists (no producer emits
                // `derivation_strat2:`), so this is one fixpoint over the
                // positive rules (see the single-pass call below).
                let collect_derivs = |prefix: &str, state: &ast::Object| -> Vec<(String, ast::Func)> {
                    ast::cells_iter(state).into_iter()
                        .filter(|(n, _)| n.starts_with(prefix))
                        .map(|(n, contents)| (n.to_string(), ast::metacompose(contents, state)))
                        .collect()
                };
                // #866 — joint fixpoint over stratum 1 + stratum 2.
                // The prior pattern ran each stratum to fixpoint
                // independently. That misses the case where a stratum-2
                // negation guard's denial enables a new stratum-1
                // positive rule that should re-fire (e.g. unary
                // derivation chains: `Task is parallelizable` depends
                // on `Task is not file-conflicting`, which is
                // stratum-2 over `Task is file-conflicting`, which is
                // stratum-1 over status). forward_chain_stratified
                // iterates the two strata together until no new facts
                // appear in an outer round.
                // #905 / task-740: include `_sm_init_<Noun>` and
                // `_sm_event_fold_<Noun>` synthetic derivations alongside
                // user-reading `rule_<hash>` rules. Pre-#905 the filter
                // was `derivation:rule_` which silently dropped every SM
                // synthetic derivation, so user-declared SMs never
                // materialized their currentlyInStatus cell. The broader
                // `derivation:` prefix would also pull in `_cwa_negation_*`
                // per-FT expansions (1120+ on a metamodel-scale
                // population) which can spike the fixpoint into
                // multi-minute runtime; keep those out unless they're
                // proven needed.
                let mut stratum1 = collect_derivs("derivation:rule_", &d);
                stratum1.extend(collect_derivs("derivation:_sm_init_", &d));
                stratum1.extend(collect_derivs("derivation:_sm_event_fold_", &d));
                // task-922-sm-init-projection: backfill for_Resource
                // for entities whose currently_in_Status row was
                // written without a companion for_Resource row
                // (apply transition / direct cell_push from
                // command.rs::transition_via_defs paths). Runs in
                // stratum 1 alongside init + event-fold so the bridge
                // derivation in stratum 2 sees the complete pair.
                stratum1.extend(collect_derivs("derivation:_sm_for_resource_backfill_", &d));
                // rmap-3nf-tables (iii): instance-of-definition backfill
                // (compile.rs::compile_sm_instance_of_definition_backfill_for)
                // — populates the mandatory State_Machine_is_instance_of_
                // State_Machine_Definition FT the 3NF state_machine table's
                // NOT NULL definition column projects from.
                stratum1.extend(collect_derivs("derivation:_sm_instance_of_def_backfill_", &d));
                // negation-strat-reroute: the negation-stratification
                // subsystem is dead — `uses_negation` is never set true
                // (every CompiledDerivation hardcodes `false`), so the
                // `derivation_strat2:` stratum is provably always empty
                // and `forward_chain_stratified_n(positive, [], …)` reduces
                // to a single `forward_chain_defs_state` over the positive
                // rules (see evaluate.rs:683-684). Call that directly.
                let (d, chain_converged) = if stratum1.is_empty() || inputs_unchanged {
                    // delta-lfp-noop-skip: nothing to derive (no rules) OR inputs
                    // unchanged → the kept derived cells are the valid LFP. Treat
                    // as converged so the input sig is (re)persisted below.
                    (d, true)
                } else {
                    // perf-chain-seminaive: run the full-compile chain through
                    // the SEMI-NAIVE chainer (the apply path already does this —
                    // command.rs). Each rule is packed with its
                    // `derivation_reads:<id>` sidecar (compile.rs emits one per
                    // rule, incl. the SM event-fold), so a round only re-runs
                    // rules whose antecedent cells were written last round. A
                    // rule with NO sidecar maps to `None` and runs EVERY round —
                    // identical to the prior naive chainer — so the change can
                    // only ever SHRINK work, never alter the fixpoint. Round 0
                    // runs every rule (`dirty_cells` starts `None`), so a full
                    // compile still materializes the complete LFP; the
                    // self-referential SM event-fold (reads + writes the status
                    // cell) re-fires each round it advances and converges as
                    // before. Net: the tasks-app chain drops from ~19s (89 rules
                    // × every round) to a few seconds.
                    let packed: Vec<(&str, &ast::Func, Option<Vec<String>>)> = stratum1.iter()
                        .map(|(name, func)| {
                            let id = name.split_once(':').map(|(_, id)| id).unwrap_or(name.as_str());
                            (name.as_str(), func, crate::evaluate::read_derivation_reads(&d, id))
                        })
                        .collect();
                    let sn_refs: Vec<(&str, &ast::Func, Option<&[String]>)> = packed.iter()
                        .map(|(name, func, reads)| (*name, *func, reads.as_deref()))
                        .collect();
                    let (new_d, derived) =
                        crate::evaluate::forward_chain_defs_state_semi_naive(&sn_refs, &d, 100);
                    // cli-apply-large-tasksdb-nonterminating: consume the
                    // chain-abort flag so it can't leak past this compile.
                    // The chain already logged a traced ⊥ naming the
                    // churning rule/cell; surface a clear compile-time
                    // note too. Partial state is persisted (the chain's
                    // pre-existing cap-hit behavior) rather than hanging.
                    let aborted = crate::evaluate::take_chain_abort();
                    if aborted {
                        eprintln!("[load] WARNING: forward-chain did NOT converge \
                            (aborted on its time budget — see the ⊥ trace above); \
                            persisting a PARTIAL fixpoint. A derivation rule is \
                            likely non-terminating (e.g. alethic-UC re-fire).");
                    }
                    eprintln!("[load] forward-chain fixpoint: {} rules, {} facts derived",
                        stratum1.len(), derived.len());
                    (new_d, !aborted)
                };
                // delta-lfp-noop-skip: persist the input sig so a FUTURE no-op
                // recompile can skip — but ONLY after a COMPLETE LFP (converged),
                // never after a partial/aborted chain (so the next compile
                // re-derives). On the skip path `d` already carries a matching
                // sig (from the prior compile, via the cor:closure merge); we
                // re-store it harmlessly.
                let d = if chain_converged {
                    ast::store("_CompileSig", ast::Object::atom(&compile_sig), &d)
                } else { d };
                // 987-A: persist the per-file signature registry so the
                // NEXT compile can compute the leaf-only delta. Stored on
                // every converged compile (full or, later, leaf).
                let d = if chain_converged {
                    ast::store("_FileSigs",
                        ast::Object::atom(&encode_file_sigs(&readings)), &d)
                } else { d };

                // Final subjectless-GC: extend cor:closure sanitation to the
                // compiled output. The preserve-time GC (above) only cleans
                // the prior population; this also drops subjectless /
                // arity-deficient relics RE-PRODUCED by the parse or
                // forward-chain — empty `{}` event facts and the
                // ⟨State Machine=∅, Status⟩ orphan — which lives in the
                // SYNTHETIC SM-output cell `State_Machine_is_currently_in_Status`
                // (NOT a declared FactType, so not in `ft_ids`; that's why the
                // ft_ids-scoped GC missed it). Declared FT data cells (uniform
                // arity) get the full arity+subject GC; other data cells
                // (synthetic SM outputs etc.) get the arity-free empty-subject
                // drop, safe without a uniformity assumption; ':' view / meta
                // cells are left untouched (they regenerate from data cells).
                // compile-gc-orphaned-derived-facts: dedup identity-equal
                // facts before persist so asserted cells (Task_is_epic et al.)
                // don't accrue one copy per recompile. Helper lives in
                // cli/mod.rs and is shared with the reload/watch sibling
                // persist paths -- single source of truth (task-958 schema-
                // arity GC + ':' meta/view pass-through + identity dedup).
                let d = super::dedup_state_for_persist(&d);

                // compile-reflect-schema-as-facts, LOAD-PATH parity (arc
                // issue 9): regenerate the schema-as-facts population
                // (Fact_Type_has_Role / Noun_plays_Role /
                // Noun_has_Object_Type / Noun_has_Conceptual_Data_Type)
                // here too. platform_compile (ast.rs) reflects on live
                // compiles, but apps.compile'd dbs lacked
                // Noun_has_Conceptual_Data_Type entirely — so any
                // FULL-scope validate (the assertFact path) tripped the
                // Format→CDT subset constraint on nouns whose CDT lives
                // only in the absorbed Noun-row field (csdp's
                // `The data type of Design Note is text.`), rejecting
                // every assertFact in every app. Set-replace, idempotent.
                let d = {
                    let mut map: hashbrown::HashMap<String, ast::Object> =
                        ast::cells_iter(&d).into_iter()
                            .map(|(name, contents)| (name.to_string(), contents.clone()))
                            .collect();
                    for (name, contents) in crate::compile::reflect_schema_cells(&d) {
                        map.insert(name, contents);
                    }
                    ast::Object::map(map)
                };

                // Persist state to SQLite (tables + triggers).
                db::apply_ddl(&conn, &d);
                db::persist_state(&conn, &d);

                eprintln!("Compiled {} readings into {}", compiled, &db_path);
            }

            // arest <key> <input> — single SYSTEM call. With
            // `--stdin-input` the input rides STDIN and argv carries
            // only the key (mcp-apply-stdin-payload — Windows argv cap).
            (true, n) if n >= 2 || (stdin_input && n >= 1) => {
                let key = &non_dirs[0];
                let stdin_owned;
                let input: &String = if stdin_input {
                    use std::io::Read;
                    let mut buf = String::new();
                    if let Err(e) = std::io::stdin().read_to_string(&mut buf) {
                        eprintln!("--stdin-input: failed to read stdin: {}", e);
                        std::process::exit(2);
                    }
                    // Pipes append a trailing newline; the payload is
                    // JSON / an id, where trailing whitespace is noise.
                    stdin_owned = buf.trim_end().to_string();
                    &stdin_owned
                } else {
                    &non_dirs[1]
                };
                // read-path-fast-path: `sql`/`cells`/`orient` are pure
                // read-only projections (system() returns D unchanged) and
                // serve straight off the loaded snapshot — skip the
                // write-only recompile via the lighter `load_for_read`.
                let d = if is_read_only_cli_key(key) {
                    load_for_read(&conn, key, Some(&db_path))
                } else {
                    load_and_compile(&conn, Some(&db_path))
                };
                // `--no-validate` only affects the compile path of a write
                // key (a read never validates), so it's a no-op on the
                // fast-load branch — apply it only when we actually compiled.
                let d = if no_validate && !is_read_only_cli_key(key) {
                    ast::install_skip_validate(&d)
                } else {
                    d
                };
                let t = std::time::Instant::now();
                let (output, new_d) = system(key, input, &d);
                eprintln!("[{:?}]", t.elapsed());
                println!("{}", output);
                (new_d != d).then(|| db::persist_state(&conn, &new_d));
            }

            // arest --db <path> — REPL mode
            _ => {
                let mut d = load_and_compile(&conn, Some(&db_path));
                if no_validate { d = ast::install_skip_validate(&d); }

                eprintln!("AREST REPL — SYSTEM is the only function.");
                eprintln!("  <key> <input>    call system(key, input)");
                eprintln!("  :quit            exit");
                eprintln!();

                let stdin = std::io::stdin();
                let mut line = String::new();
                loop {
                    eprint!("arest> ");
                    line.clear();
                    match stdin.read_line(&mut line) {
                        Ok(0) => break, // EOF
                        Err(e) => { eprintln!("Read error: {}", e); break; }
                        _ => {}
                    }
                    let trimmed = line.trim();
                    match trimmed {
                        "" => continue,
                        ":quit" | ":q" | ":exit" => break,
                        _ => {
                            // Split on first whitespace: key + rest
                            let (key, input) = trimmed.split_once(char::is_whitespace)
                                .map(|(k, i)| (k, i.trim()))
                                .unwrap_or((trimmed, ""));
                            let t = std::time::Instant::now();
                            let (output, new_d) = system(key, input, &d);
                            eprintln!("[{:?}]", t.elapsed());
                            println!("{}", output);
                            // Update in-memory state if changed; persist periodically
                            (new_d != d).then(|| {
                                d = new_d;
                                db::persist_state(&conn, &d);
                            });
                        }
                    }
                }
            }
        }
    }
}

// ── cli-apply-large-tasksdb-nonterminating regression guards ───────────
#[cfg(all(test, feature = "local"))]
mod stale_def_tests {
    use super::*;

    /// Bug A guard. `population_only` must DROP every persisted compiled-def
    /// cell (any `:`-named cell, plus the platform singletons) and KEEP
    /// population cells, so a stale orphan compiled def in the loaded
    /// snapshot can never survive the recompile and reach the apply chain.
    ///
    /// This is the exact shape that hung the live tasks.db: a 101 KB
    /// `derivation:_subtype_inheritance` orphan (no longer emitted by the
    /// current compiler) lingered in the persisted `defs` and, with no
    /// reads sidecar, ran every forward-chain round over the whole
    /// population (~20 s / round). Dropping it on load is the fix.
    #[test]
    fn population_only_drops_stale_compiled_defs_keeps_population() {
        // A loaded snapshot mixing population cells, a STALE orphan
        // derivation def, a few other compiled-def families, and the
        // platform singletons — exactly what `load_state` returns.
        let mut map: hashbrown::HashMap<String, ast::Object> = hashbrown::HashMap::new();
        // Population cells (colon-free) — must survive.
        map.insert("Task".to_string(), ast::Object::atom("task-pop"));
        map.insert("Task_has_Task_Description".to_string(), ast::Object::atom("desc-pop"));
        map.insert("DerivationRule".to_string(), ast::Object::atom("rules-pop"));
        // Stale / compiled defs (colon-named) — must be dropped.
        map.insert("derivation:_subtype_inheritance".to_string(),
            ast::Object::atom("STALE-101kb-orphan"));
        map.insert("derivation:rule_abc".to_string(), ast::Object::atom("d"));
        map.insert("schema:Task_has_Task_Description".to_string(), ast::Object::atom("s"));
        map.insert("validate:Task".to_string(), ast::Object::atom("v"));
        map.insert("derivation_reads:rule_abc".to_string(), ast::Object::atom("r"));
        // Platform singletons (colon-free but still defs) — must be dropped.
        map.insert("compile".to_string(), ast::Object::atom("p"));
        map.insert("apply".to_string(), ast::Object::atom("p"));
        map.insert("validate".to_string(), ast::Object::atom("p"));
        let loaded = ast::Object::map(map);

        let pop = population_only(&loaded);

        // The stale orphan and every other compiled def are gone.
        assert_eq!(ast::fetch("derivation:_subtype_inheritance", &pop), ast::Object::Bottom,
            "stale orphan derivation def must NOT survive into the recompile input");
        assert_eq!(ast::fetch("derivation:rule_abc", &pop), ast::Object::Bottom);
        assert_eq!(ast::fetch("schema:Task_has_Task_Description", &pop), ast::Object::Bottom);
        assert_eq!(ast::fetch("validate:Task", &pop), ast::Object::Bottom);
        assert_eq!(ast::fetch("derivation_reads:rule_abc", &pop), ast::Object::Bottom);
        assert_eq!(ast::fetch("compile", &pop), ast::Object::Bottom);
        assert_eq!(ast::fetch("apply", &pop), ast::Object::Bottom);
        assert_eq!(ast::fetch("validate", &pop), ast::Object::Bottom);
        // Population cells survive untouched.
        assert_eq!(ast::fetch("Task", &pop), ast::Object::atom("task-pop"));
        assert_eq!(ast::fetch("Task_has_Task_Description", &pop), ast::Object::atom("desc-pop"));
        assert_eq!(ast::fetch("DerivationRule", &pop), ast::Object::atom("rules-pop"));
    }

    /// read-path-fast-path guard. The single-SYSTEM dispatch routes
    /// the read-only verbs through the lighter `load_for_read` (skip
    /// the write-only recompile) and EVERYTHING else through the full
    /// `load_and_compile`. `sql`/`cells`/`orient` are the read-only
    /// intercepts in `system()`; `query` joined 2026-06-12
    /// (query-read-fastpath) — it resolves the persisted `schema:{id}`
    /// defs exactly as `sql` resolves persisted `view:` defs, the same
    /// staleness trade. `get` stays off (federation absorb writes).
    /// Write keys (`apply`, `compile`, the `retract:`/`assert:`
    /// families) and any unknown key MUST fall through to the full
    /// compile so the recompile-from-population (stale-def strip)
    /// discipline still runs.
    #[test]
    fn is_read_only_cli_key_selects_only_pure_read_verbs() {
        // The read-only projection verbs.
        assert!(is_read_only_cli_key("sql"));
        assert!(is_read_only_cli_key("cells"));
        assert!(is_read_only_cli_key("orient"));
        assert!(is_read_only_cli_key("query:Task_has_Task_Priority"));
        // Write / mutating / federation-aware keys keep load_and_compile.
        assert!(!is_read_only_cli_key("apply"));
        assert!(!is_read_only_cli_key("compile"));
        assert!(!is_read_only_cli_key("get"));
        assert!(!is_read_only_cli_key("retract:Task_blocks_Task"));
        assert!(!is_read_only_cli_key("assert:Task_is_epic"));
        // Unknown keys default to the safe (full-compile) path.
        assert!(!is_read_only_cli_key(""));
        assert!(!is_read_only_cli_key("sqlx"));
        // Bare `query` (no fact type) is not a real dispatch — safe path.
        assert!(!is_read_only_cli_key("query"));
    }

    /// ring-ffi-bottom — the headline repro+fix. The CLI `system()` is the
    /// path the MCP shim shells out to (`arest-cli --db <db> assert:<ft>
    /// "<<…>>"`). Asserting a SAME-NOUN ring fact (`Task blocks Task`, the
    /// noun `Task` filling BOTH roles) must LAND the fact — NOT bottom with
    /// `⊥ origin: … in rule `assert:Task_blocks_Task``.
    ///
    /// PRE-FIX: `system()` had a `retract:` intercept but NO `assert:`
    /// intercept, so the key fell through to `apply(Func::Def("assert:\
    /// Task_blocks_Task"), …)`; no such Def exists, so it returned ⊥ and the
    /// armed bottom-trace stamped the rule name from the key — the exact
    /// reported failure, and D was left unchanged (the fact never landed).
    /// POST-FIX: the new `assert:` intercept dispatches `Command::AssertFact`
    /// and merges the delta.
    #[test]
    fn assert_cli_same_noun_ring_lands_in_cell_not_bottom() {
        // Compile a minimal ring model (Task blocks Task + the cross-noun
        // bridge derivation + the irreflexive/asymmetric ring constraints)
        // into a def-state — exactly the shape `load_and_compile` hands to
        // `system()` for a write key.
        let readings = "\
Task(.id) is an entity type.
Task Readiness is a value type.
Task blocks Task.
Task has Task Readiness.
Task blocks Task is irreflexive.
Task blocks Task is asymmetric.
* Task2 has Task Readiness 'blocked' iff Task1 blocks Task2.
";
        let state = crate::parse_forml2_stage2::parse_to_state_via_stage12(readings)
            .expect("ring readings must parse");
        let defs = crate::compile::compile_to_defs_state(&state);
        let d = ast::defs_to_state(&defs, &state);

        // The live CLI verb shape: ordered (role, value) pairs, the role
        // name `Task` repeated for both ends of the ring.
        let (out, d1) = system(
            "assert:Task_blocks_Task",
            "<<Task, task-A>, <Task, task-B>>",
            &d,
        );
        // PRE-FIX this was `⊥ origin: … in rule `assert:Task_blocks_Task``.
        assert!(!out.starts_with('\u{22a5}'),
            "assert:<ft> CLI verb must NOT bottom on a same-noun ring fact; got: {out}");
        assert_eq!(out, "ok",
            "assert:<ft> CLI verb must return ok after landing the ring fact; got: {out}");
        // State must have advanced (the fact was committed, NOT D'=D).
        assert_ne!(d1, d,
            "a successful assert must change D so the caller persists it");

        // The fact must ACTUALLY be in the cell — exact ordered tuple, two
        // DISTINCT same-noun values (anti-collapse proof).
        let cell = ast::fetch_cell_seq("Task_blocks_Task", &d1);
        let tuples: Vec<Vec<(String, String)>> = cell.as_seq()
            .map(|facts| facts.iter().filter_map(|f| {
                let pairs = f.as_seq()?;
                Some(pairs.iter().filter_map(|p| {
                    let kv = p.as_seq()?;
                    Some((kv.first()?.as_atom()?.to_string(),
                          kv.get(1)?.as_atom()?.to_string()))
                }).collect::<Vec<(String, String)>>())
            }).collect())
            .unwrap_or_default();
        assert_eq!(tuples.len(), 1,
            "exactly one ring fact must be present after the assert; got {tuples:?}");
        assert_eq!(tuples[0],
            vec![("Task".to_string(), "task-A".to_string()),
                 ("Task".to_string(), "task-B".to_string())],
            "the materialized tuple must be the EXACT ordered <<Task,task-A>,\
             <Task,task-B>> (no same-noun collapse); got {:?}", tuples[0]);

        // The cross-noun bridge derivation must have fired: task-B blocked.
        let readiness = ast::fetch_cell_seq("Task_has_Task_Readiness", &d1);
        let b_blocked = readiness.as_seq().map(|fs| fs.iter().any(|f|
            ast::binding(f, "Task") == Some("task-B")
            && ast::binding(f, "Task Readiness") == Some("blocked"))).unwrap_or(false);
        assert!(b_blocked,
            "CLI assert must drive the derivation — task-B must be 'blocked'; \
             readiness={readiness:?}");

        // A SECOND, independent ring pair must coexist (no clobber) — this
        // is the folded-Map append path (after the first commit the cell may
        // fold), the live-tasks.db data-loss guard.
        let (out2, d2) = system(
            "assert:Task_blocks_Task",
            "<<Task, task-B>, <Task, task-C>>",
            &d1,
        );
        assert_eq!(out2, "ok",
            "the second distinct ring pair must also land; got: {out2}");
        let cell2 = ast::fetch_cell_seq("Task_blocks_Task", &d2);
        let tuples2: Vec<Vec<(String, String)>> = cell2.as_seq()
            .map(|facts| facts.iter().filter_map(|f| {
                let pairs = f.as_seq()?;
                Some(pairs.iter().filter_map(|p| {
                    let kv = p.as_seq()?;
                    Some((kv.first()?.as_atom()?.to_string(),
                          kv.get(1)?.as_atom()?.to_string()))
                }).collect::<Vec<(String, String)>>())
            }).collect())
            .unwrap_or_default();
        let has_ab = tuples2.iter().any(|t| t ==
            &vec![("Task".to_string(), "task-A".to_string()),
                  ("Task".to_string(), "task-B".to_string())]);
        let has_bc = tuples2.iter().any(|t| t ==
            &vec![("Task".to_string(), "task-B".to_string()),
                  ("Task".to_string(), "task-C".to_string())]);
        assert!(has_ab && has_bc,
            "BOTH ring tuples must coexist after the second assert (the first \
             must NOT be clobbered); got {tuples2:?}");

        // A self-loop on the same noun is still REJECTED by the irreflexive
        // ring constraint: the verb returns ⊥ and D is left unchanged.
        let (loop_out, d3) = system(
            "assert:Task_blocks_Task",
            "<<Task, task-X>, <Task, task-X>>",
            &d2,
        );
        assert!(loop_out.starts_with('\u{22a5}'),
            "a same-noun self-loop must be rejected (⊥) by the irreflexive \
             ring constraint; got: {loop_out}");
        assert_eq!(d3, d2,
            "a rejected self-loop must leave D unchanged (D'=D)");
    }

    /// task-951-b. `build_provenance_cell` is the foundational deliverable:
    /// from the per-file readings fold it must produce a `Provenance` cell
    /// attributing each ORM element (Noun / FactType / Constraint) to the
    /// readings file that FIRST declared it. This is exactly the map the
    /// NORMA exporter consumes to split the model into per-file diagram tabs.
    #[test]
    fn build_provenance_cell_attributes_each_element_to_its_source_file() {
        // Two domains, each in its own "file": orders.md declares Order +
        // a binary fact; customers.md declares Customer. The nouns/fact
        // types each file introduces must be credited to THAT file.
        let orders = "\
            Order(.id) is an entity type.\n\
            Customer(.id) is an entity type.\n\
            \n\
            ## Fact Types\n\
            Order is placed by Customer.\n";
        let customers = "\
            Customer(.id) is an entity type.\n\
            Region(.id) is an entity type.\n\
            \n\
            ## Fact Types\n\
            Customer lives in Region.\n";
        let all_readings: Vec<(&str, &str)> =
            vec![("orders.md", orders), ("customers.md", customers)];

        let cell = build_provenance_cell(&all_readings, &ast::Object::phi());
        let facts = cell.as_seq().expect("Provenance is a Seq cell");
        assert!(!facts.is_empty(), "provenance map must not be empty");

        // Helper: the sourceFile attributed to (kind, element), if any.
        let file_of = |kind: &str, element: &str| -> Option<String> {
            facts.iter().find_map(|f| {
                (ast::binding(f, "kind") == Some(kind)
                    && ast::binding(f, "element") == Some(element))
                    .then(|| ast::binding(f, "sourceFile").unwrap_or("").to_string())
            })
        };

        // Order is declared only in orders.md.
        assert_eq!(file_of("Noun", "Order").as_deref(), Some("orders.md"),
            "Order must be attributed to orders.md; facts: {:?}", facts);
        // Region is declared only in customers.md.
        assert_eq!(file_of("Noun", "Region").as_deref(), Some("customers.md"),
            "Region must be attributed to customers.md; facts: {:?}", facts);
        // Customer is referenced in both but FIRST declared in orders.md —
        // first-declarer wins (mirrors merge_states identity dedup).
        assert_eq!(file_of("Noun", "Customer").as_deref(), Some("orders.md"),
            "Customer (first-declared in orders.md) must be attributed there; \
             facts: {:?}", facts);

        // Each fact type is attributed to the file whose `## Fact Types`
        // section introduced it. Look them up by reading→kind FactType.
        let ft_files: std::collections::BTreeSet<String> = facts.iter()
            .filter(|f| ast::binding(f, "kind") == Some("FactType"))
            .filter_map(|f| ast::binding(f, "sourceFile").map(str::to_string))
            .collect();
        assert!(ft_files.contains("orders.md") && ft_files.contains("customers.md"),
            "fact types from both files must be attributed; got {:?}", ft_files);

        // No element is attributed to more than one file (first-declarer wins
        // ⇒ exactly one provenance fact per (kind, element)).
        let mut keys: Vec<(&str, &str)> = facts.iter()
            .filter_map(|f| Some((ast::binding(f, "kind")?, ast::binding(f, "element")?)))
            .collect();
        let n = keys.len();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), n, "each (kind, element) must appear exactly once");

        // Non-empty seed regression (the whole-corpus noun catalog the real
        // loader passes): nouns MUST still be attributed. A prior version
        // pre-seeded `seen` from this catalog, which marked every noun
        // already-seen and left ALL ObjectTypeShapes unattributed in the
        // NORMA export. With the catalog as `seed`, Order/Region must still
        // map to their files.
        let catalog_corpus = format!("{}\n\n{}", orders, customers);
        let catalog = parse_forml2::parse_to_state_from(&catalog_corpus, &ast::Object::phi())
            .expect("catalog parse");
        let mut m: hashbrown::HashMap<String, ast::Object> = hashbrown::HashMap::new();
        m.insert("Noun".to_string(), ast::fetch_cell_seq("Noun", &catalog));
        let seed = ast::Object::map(m);
        let cell2 = build_provenance_cell(&all_readings, &seed);
        let facts2 = cell2.as_seq().expect("Provenance is a Seq cell");
        let file_of2 = |kind: &str, element: &str| -> Option<String> {
            facts2.iter().find_map(|f| {
                (ast::binding(f, "kind") == Some(kind)
                    && ast::binding(f, "element") == Some(element))
                    .then(|| ast::binding(f, "sourceFile").unwrap_or("").to_string())
            })
        };
        assert_eq!(file_of2("Noun", "Order").as_deref(), Some("orders.md"),
            "with the whole-corpus catalog as seed, Order must STILL be \
             attributed (no pre-seed suppression); facts: {:?}", facts2);
        assert_eq!(file_of2("Noun", "Region").as_deref(), Some("customers.md"),
            "with the catalog seed, Region must STILL be attributed; \
             facts: {:?}", facts2);
    }

    /// read-path-defprune. `db::load_state_closure` must load the
    /// transitive closure of defs reachable from the seed and PRUNE the
    /// rest, while keeping ALL population cells. The reachability edge is
    /// a body atom that names another def (how `metacompose_atom`
    /// resolves a reference, ast.rs:6009) — so a `view:` def that fetches
    /// a populated cell pulls that nothing extra, but a `view:` def whose
    /// body names ANOTHER def must transitively pull that def in.
    #[test]
    fn load_state_closure_loads_reachable_defs_and_prunes_the_rest() {
        let conn = rusqlite::Connection::open_in_memory().expect("in-memory sqlite");
        db::ensure_meta_tables(&conn);

        // Population cells — every one must survive (reads may touch any).
        conn.execute("INSERT INTO cells (name, contents) VALUES (?1, ?2)",
            rusqlite::params!["Task_has_Task_Status", "<<<status, open>>>"]).unwrap();
        conn.execute("INSERT INTO cells (name, contents) VALUES (?1, ?2)",
            rusqlite::params!["FactType", "<<<id, Task_has_Task_Status>>>"]).unwrap();

        // Seed def `view:A` references population cell `Task_has_Task_Status`
        // (already loaded) AND another def `derivation:shared` by name — the
        // closure walk must follow that edge.
        conn.execute("INSERT INTO defs (name, func) VALUES (?1, ?2)",
            rusqlite::params!["view:A",
                "<., ^?, <[, <', Task_has_Task_Status>, <', derivation:shared>>>"]).unwrap();
        // `derivation:shared` is reachable ONLY transitively from view:A. Its
        // own body references a further def `resolve:deep` — closure again.
        conn.execute("INSERT INTO defs (name, func) VALUES (?1, ?2)",
            rusqlite::params!["derivation:shared", "<', resolve:deep>"]).unwrap();
        conn.execute("INSERT INTO defs (name, func) VALUES (?1, ?2)",
            rusqlite::params!["resolve:deep", "id"]).unwrap();
        // Platform singleton (colon-free) is part of the seed predicate.
        conn.execute("INSERT INTO defs (name, func) VALUES (?1, ?2)",
            rusqlite::params!["compile", "platform:compile"]).unwrap();
        // UNREACHABLE generator def — referenced by NOTHING in the seed
        // closure. Must be pruned (this is the 8.6k-row bulk on tasks.db).
        conn.execute("INSERT INTO defs (name, func) VALUES (?1, ?2)",
            rusqlite::params!["query:Unrelated", "<', Task_has_Task_Status>"]).unwrap();

        let d = db::load_state_closure(&conn, |name| {
            name.starts_with("view:") || !name.contains(':')
        });

        // Population cells: always present.
        assert_ne!(ast::fetch("Task_has_Task_Status", &d), ast::Object::Bottom,
            "population cells must always load");
        assert_ne!(ast::fetch("FactType", &d), ast::Object::Bottom);
        // Seed + transitively-reached defs: present.
        assert_ne!(ast::fetch("view:A", &d), ast::Object::Bottom, "seed view def must load");
        assert_ne!(ast::fetch("derivation:shared", &d), ast::Object::Bottom,
            "def referenced by a loaded view body must be pulled in (closure)");
        assert_ne!(ast::fetch("resolve:deep", &d), ast::Object::Bottom,
            "transitively-referenced def must be pulled in (closure depth > 1)");
        assert_ne!(ast::fetch("compile", &d), ast::Object::Bottom,
            "colon-free platform singleton is in the seed");
        // Unreachable bulk: pruned.
        assert_eq!(ast::fetch("query:Unrelated", &d), ast::Object::Bottom,
            "a def reachable from nothing in the seed closure must be pruned");
    }
}

// build.rs / `version`-subcommand embedding smoke check. Feature-
// independent (the `version` verb itself is) so it runs on the default
// `cargo test --lib` without --features local. Asserts the build script
// actually populated the provenance env vars that the subcommand prints
// via env!(): a non-empty git SHA (real 40-hex when git is on PATH, or
// the literal "unknown" graceful-degrade fallback) and a build time.
// This is the Rust-side guard that the MCP's engine_version verb gets a
// parseable, non-empty payload.
#[cfg(test)]
mod version_embedding_tests {
    #[test]
    fn build_rs_embeds_a_nonempty_git_sha() {
        let sha = env!("AREST_GIT_SHA");
        assert!(!sha.is_empty(), "AREST_GIT_SHA must be embedded by build.rs");
        // Either a 40-char lowercase hex commit, or the documented
        // graceful-degrade sentinel when git was unavailable at build.
        let is_hex_sha = sha.len() == 40 && sha.bytes().all(|b| b.is_ascii_hexdigit());
        assert!(
            is_hex_sha || sha == "unknown",
            "AREST_GIT_SHA should be a 40-hex commit or \"unknown\", got {sha:?}"
        );
    }

    #[test]
    fn build_rs_embeds_a_build_time() {
        let built = env!("AREST_BUILD_TIME");
        assert!(!built.is_empty(), "AREST_BUILD_TIME must be embedded by build.rs");
        // Format is YYYY-MM-DDTHH:MM:SSZ (or "unknown" if SystemTime failed).
        assert!(
            built == "unknown"
                || (built.len() == 20 && built.ends_with('Z') && built.as_bytes()[10] == b'T'),
            "AREST_BUILD_TIME should be RFC3339-ish UTC or \"unknown\", got {built:?}"
        );
    }

    #[test]
    fn pkg_version_is_present() {
        // The subcommand reads CARGO_PKG_VERSION, set by cargo for every
        // build — guard it so the printed JSON's `pkg` field is never empty.
        assert!(!env!("CARGO_PKG_VERSION").is_empty());
    }
}

#[cfg(test)]
mod stack_size_tests {
    use super::stack_bytes_from_env;

    const MB: usize = 1024 * 1024;

    #[test]
    fn defaults_to_512_mib_when_unset() {
        assert_eq!(stack_bytes_from_env(None), 512 * MB);
    }

    #[test]
    fn honors_a_valid_megabyte_override() {
        assert_eq!(stack_bytes_from_env(Some("256".to_string())), 256 * MB);
        assert_eq!(stack_bytes_from_env(Some("1024".to_string())), 1024 * MB);
    }

    #[test]
    fn trims_surrounding_whitespace() {
        assert_eq!(stack_bytes_from_env(Some("  768  ".to_string())), 768 * MB);
    }

    #[test]
    fn falls_back_to_default_on_zero_or_garbage() {
        // 0 would mean "no stack" — nonsensical; fall back rather than honor it.
        assert_eq!(stack_bytes_from_env(Some("0".to_string())), 512 * MB);
        assert_eq!(stack_bytes_from_env(Some("abc".to_string())), 512 * MB);
        assert_eq!(stack_bytes_from_env(Some("".to_string())), 512 * MB);
    }
}

// AREST CLI — SYSTEM is the only function.
//
// Usage:
//   arest <readings_dir> [<readings_dir2> ...] [--db <path>]
//
// Reads .md files from each directory, feeds them through
// system(h, 'compile', text), then persists state to SQLite.
// Subsequent system calls load state from the database.
//
// Interactive mode (no directories):
//   arest --db <path> <key> <input>
//
// Everything goes through SYSTEM. No separate bootstrap, synthesize,
// or forward-chain commands. Per AREST paper: SYSTEM:x = ⟨o, D'⟩.

#[allow(dead_code)]
mod ast;
#[allow(dead_code)]
mod types;
#[allow(dead_code)]
mod compile;
#[allow(dead_code)]
mod evaluate;
#[allow(dead_code)]
mod query;
#[allow(dead_code)]
mod rmap;
#[allow(dead_code)]
mod naming;
#[allow(dead_code)]
mod conceptual_query;
#[allow(dead_code)]
mod parse_rule;
#[allow(dead_code)]
mod parse_forml2;
#[allow(dead_code)]
mod verbalize;
#[allow(dead_code)]
mod arest;
#[allow(dead_code)]
mod validate;
#[allow(dead_code)]
mod induce;
#[allow(dead_code)]
mod crypto;
#[allow(dead_code)]
mod generators;

// =========================================================================
// SQLite persistence (feature = "local")
// =========================================================================

#[cfg(feature = "local")]
mod db {
    use rusqlite::{Connection, params};
    use crate::ast;
    use std::collections::HashMap;

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
    pub fn apply_ddl(conn: &Connection, d: &ast::Object) {
        ast::cells_iter(d).into_iter()
            .filter(|(name, _)| name.starts_with("sql:sqlite:"))
            .filter_map(|(_, contents)| contents.as_atom().map(|s| s.to_string()))
            .for_each(|ddl| {
                conn.execute_batch(&ddl).unwrap_or_else(|e| {
                    eprintln!("Warning: DDL failed: {}", e);
                });
            });
    }

    /// Persist the full state D to SQLite.
    pub fn persist_state(conn: &Connection, d: &ast::Object) {
        let tx = conn.unchecked_transaction()
            .unwrap_or_else(|e| { eprintln!("Transaction failed: {}", e); std::process::exit(1); });

        // Store each cell as a JSON blob keyed by cell name.
        ast::cells_iter(d).into_iter()
            .filter(|(name, _)| !name.contains(':'))  // skip def cells
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

        tx.commit()
            .unwrap_or_else(|e| { eprintln!("Commit failed: {}", e); std::process::exit(1); });
    }

    /// Load state D from SQLite.
    pub fn load_state(conn: &Connection) -> ast::Object {
        let mut state = ast::Object::phi();

        // Load cells (population facts).
        let mut stmt = match conn.prepare("SELECT name, contents FROM cells") {
            Ok(s) => s,
            Err(_) => return state,
        };
        state = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        }).unwrap_or_else(|e| { eprintln!("Failed to read cells: {}", e); std::process::exit(1); })
        .filter_map(|r| r.ok())
        .fold(state, |acc, (name, contents)| {
            let obj = ast::Object::parse(&contents);
            ast::store(&name, obj, &acc)
        });

        // Load defs.
        let mut stmt = match conn.prepare("SELECT name, func FROM defs") {
            Ok(s) => s,
            Err(_) => return state,
        };
        state = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        }).unwrap_or_else(|e| { eprintln!("Failed to read defs: {}", e); std::process::exit(1); })
        .filter_map(|r| r.ok())
        .fold(state, |acc, (name, contents)| {
            let obj = ast::Object::parse(&contents);
            ast::store(&name, obj, &acc)
        });

        state
    }
}

// =========================================================================
// SYSTEM is the only function
// =========================================================================

/// system(key, input, D) → (output, D')
/// Pure ρ-dispatch. Same as lib.rs system_impl but operates on an
/// owned state instead of a global handle registry.
fn system(key: &str, input: &str, d: &ast::Object) -> (String, ast::Object) {
    let obj = ast::Object::parse(input);
    let result = ast::apply(&ast::Func::Def(key.to_string()), &obj, d);

    // State transition: if result contains cells (Noun, GraphSchema, etc.)
    // it's a new D. Otherwise it's a display-only output.
    let is_new_d = result.as_seq().is_some()
        && ast::fetch("Noun", &result) != ast::Object::Bottom;

    let new_d = match is_new_d {
        true => result.clone(),
        false => d.clone(),
    };

    (result.to_string(), new_d)
}

/// Read .md files from directories, sorted alphabetically, app.md first.
fn read_readings(dirs: &[String]) -> Vec<(String, String)> {
    let (readings, app_md) = dirs.iter().flat_map(|dir| {
        let dir_path = std::path::Path::new(dir);
        (!dir_path.is_dir()).then(|| {
            eprintln!("Not a directory: {}", dir);
            std::process::exit(1);
        });
        let mut entries: Vec<std::path::PathBuf> = std::fs::read_dir(dir_path)
            .unwrap_or_else(|e| { eprintln!("Failed to read {}: {}", dir, e); std::process::exit(1); })
            .filter_map(|e| e.ok()).map(|e| e.path())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("md"))
            .collect();
        entries.sort();
        entries.into_iter().map(|path| {
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| { eprintln!("Failed to read {}: {}", path.display(), e); std::process::exit(1); });
            (name, text)
        }).collect::<Vec<_>>()
    }).fold((Vec::new(), None::<(String, String)>), |(mut readings, app), (name, text)| {
        match name.as_str() {
            "app.md" => (readings, Some((name, text))),
            _ => { readings.push((name, text)); (readings, app) }
        }
    });

    app_md.into_iter().chain(readings).collect()
}

/// Bundled metamodel readings — same as lib.rs METAMODEL_READINGS.
const METAMODEL_READINGS: &[(&str, &str)] = &[
    ("core",          include_str!("../../../readings/core.md")),
    ("state",         include_str!("../../../readings/state.md")),
    ("instances",     include_str!("../../../readings/instances.md")),
    ("outcomes",      include_str!("../../../readings/outcomes.md")),
    ("validation",    include_str!("../../../readings/validation.md")),
    ("evolution",     include_str!("../../../readings/evolution.md")),
    ("organizations", include_str!("../../../readings/organizations.md")),
    ("agents",        include_str!("../../../readings/agents.md")),
    ("ui",            include_str!("../../../readings/ui.md")),
];

/// Create D with bundled metamodel cells + platform primitives.
/// No compile_to_defs_state here — that happens lazily on first
/// system(h, 'compile', text) via platform_compile. Same strategy
/// as lib.rs metamodel_state().
fn create() -> ast::Object {
    parse_forml2::set_bootstrap_mode(true);
    let merged = METAMODEL_READINGS.iter().fold(ast::Object::phi(), |acc, (name, text)| {
        let parsed = parse_forml2::parse_to_state_from(text, &acc)
            .unwrap_or_else(|e| { eprintln!("metamodel {}.md: {}", name, e); std::process::exit(1); });
        ast::merge_states(&acc, &parsed)
    });
    parse_forml2::set_bootstrap_mode(false);

    let defs = vec![
        ("compile".to_string(), ast::Func::Platform("compile".to_string())),
        ("apply".to_string(), ast::Func::Platform("apply_command".to_string())),
        ("verify_signature".to_string(), ast::Func::Platform("verify_signature".to_string())),
    ];
    ast::defs_to_state(&defs, &merged)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    // Parse flags.
    let no_validate = args.iter().any(|a| a == "--no-validate");
    let (db_path, rest, _) = args.iter()
        .filter(|a| a.as_str() != "--no-validate")
        .fold(
        ("arest.db".to_string(), Vec::<String>::new(), false),
        |(db, mut rest, expect_db), arg| match (expect_db, arg.as_str()) {
            (true, _) => (arg.clone(), rest, false),
            (false, "--db") => (db, rest, true),
            (false, "--help" | "-h") => {
                println!("Usage: arest [<readings_dir> ...] [--db <path>] [<key> <input>]");
                println!();
                println!("  No args:           load state from --db, start REPL (not yet implemented)");
                println!("  <dir> [<dir2>]:    compile readings via SYSTEM, persist to --db");
                println!("  <key> <input>:     single SYSTEM call against persisted state");
                println!();
                println!("  --db <path>        SQLite database path (default: arest.db)");
                println!("  --no-validate      skip constraint validation during compile");
                std::process::exit(0);
            }
            (false, _) => { rest.push(arg.clone()); (db, rest, false) }
        },
    );

    #[cfg(not(feature = "local"))]
    {
        eprintln!("Build with --features local for SQLite support.");
        eprintln!("  cargo run --bin arest --features local -- <readings_dir>");
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

        match (dirs.is_empty(), non_dirs.len()) {
            // arest <dir1> [<dir2> ...] — compile readings via SYSTEM
            (false, _) => {
                let readings = read_readings(&dirs);
                readings.is_empty().then(|| {
                    eprintln!("No .md files found.");
                    std::process::exit(1);
                });

                // Fast path: fold all readings (metamodel + user) into a
                // single Domain IR, then convert to Object state ONCE.
                // No merge_states loop — O(n) in total content.
                parse_forml2::set_bootstrap_mode(true);
                let all_readings: Vec<(&str, &str)> = METAMODEL_READINGS.iter()
                    .map(|(n, t)| (*n, *t))
                    .chain(readings.iter().map(|(n, t)| (n.as_str(), t.as_str())))
                    .collect();
                let domain = all_readings.iter().fold(
                    types::Domain::default(),
                    |mut merged, (name, text)| {
                        let ir = match merged.nouns.is_empty() {
                            true => parse_forml2::parse_markdown(text),
                            false => parse_forml2::parse_markdown_with_nouns(text, &merged.nouns),
                        }.unwrap_or_else(|e| { eprintln!("{}: {}", name, e); std::process::exit(1); });
                        merged.nouns.extend(ir.nouns);
                        merged.fact_types.extend(ir.fact_types);
                        merged.constraints.extend(ir.constraints);
                        merged.state_machines.extend(ir.state_machines);
                        merged.derivation_rules.extend(ir.derivation_rules);
                        merged.general_instance_facts.extend(ir.general_instance_facts);
                        merged.subtypes.extend(ir.subtypes);
                        merged.enum_values.extend(ir.enum_values);
                        merged.ref_schemes.extend(ir.ref_schemes);
                        merged.objectifications.extend(ir.objectifications);
                        merged.named_spans.extend(ir.named_spans);
                        merged.autofill_spans.extend(ir.autofill_spans);
                        merged
                    },
                );
                parse_forml2::set_bootstrap_mode(false);
                let state = parse_forml2::domain_to_state(&domain);
                let defs = vec![
                    ("compile".to_string(), ast::Func::Platform("compile".to_string())),
                    ("apply".to_string(), ast::Func::Platform("apply_command".to_string())),
                    ("verify_signature".to_string(), ast::Func::Platform("verify_signature".to_string())),
                ];
                let d = ast::defs_to_state(&defs, &state);
                let compiled = readings.len();

                // Persist state to SQLite.
                db::apply_ddl(&conn, &d);
                db::persist_state(&conn, &d);

                eprintln!("Compiled {} readings into {}", compiled, &db_path);
            }

            // arest <key> <input> — single SYSTEM call
            // Lazy compile: if defs aren't in state, compile them now.
            (true, n) if n >= 2 => {
                let loaded = db::load_state(&conn);
                let d = match ast::fetch("validate", &loaded) {
                    ast::Object::Bottom => {
                        // No compiled defs yet — compile now from stored cells.
                        eprintln!("Compiling defs from stored state...");
                        let mut defs = compile::compile_to_defs_state(&loaded);
                        defs.push(("compile".to_string(), ast::Func::Platform("compile".to_string())));
                        defs.push(("apply".to_string(), ast::Func::Platform("apply_command".to_string())));
                        defs.push(("verify_signature".to_string(), ast::Func::Platform("verify_signature".to_string())));
                        let compiled = ast::defs_to_state(&defs, &loaded);
                        db::persist_state(&conn, &compiled);
                        compiled
                    }
                    _ => loaded,
                };
                let key = &non_dirs[0];
                let input = &non_dirs[1];
                let (output, new_d) = system(key, input, &d);
                println!("{}", output);

                // Persist if state changed.
                (new_d != d).then(|| db::persist_state(&conn, &new_d));
            }

            // No args or single non-dir arg
            _ => {
                eprintln!("Usage: arest <readings_dir> [--db <path>]");
                eprintln!("       arest <key> <input> [--db <path>]");
                std::process::exit(1);
            }
        }
    }
}

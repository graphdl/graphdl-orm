// AREST CLI — SYSTEM is the only function.
//
// Usage:
//   arest-cli <readings_dir> [<readings_dir2> ...] [--db <path>]
//
// Reads .md files from each directory, feeds them through
// system(h, 'compile', text), then persists state to SQLite.
// Subsequent system calls load state from the database.
//
// Interactive mode (no directories):
//   arest-cli --db <path> <key> <input>
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
        // CREATE TABLE from sql:sqlite:* cells
        ast::cells_iter(d).into_iter()
            .filter(|(name, _)| name.starts_with("sql:sqlite:"))
            .filter_map(|(_, contents)| contents.as_atom().map(|s| s.to_string()))
            .for_each(|ddl| {
                conn.execute_batch(&ddl).unwrap_or_else(|e| {
                    eprintln!("Warning: DDL failed: {}", e);
                });
            });
        // CREATE TRIGGER from sql:trigger:* cells
        ast::cells_iter(d).into_iter()
            .filter(|(name, _)| name.starts_with("sql:trigger:"))
            .filter_map(|(_, contents)| contents.as_atom().map(|s| s.to_string()))
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

        state.to_store()
    }
}

// =========================================================================
// SYSTEM is the only function
// =========================================================================

/// system(key, input, D) → (output, D')
/// Pure ρ-dispatch. Same as lib.rs system_impl but operates on an
/// owned state instead of a global handle registry.
#[cfg(feature = "local")]
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

/// Bundled metamodel readings — same as lib.rs METAMODEL_READINGS.
#[cfg(feature = "local")]
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

/// Load population from SQLite, compile defs in memory.
/// Defs are never persisted — population cells only on disk.
/// Compile takes ~500ms and produces the full D for SYSTEM calls.
#[cfg(feature = "local")]
fn load_and_compile(conn: &rusqlite::Connection) -> ast::Object {
    let t = std::time::Instant::now();
    let loaded = db::load_state(conn);
    eprintln!("[profile] load_state: {:?}", t.elapsed());
    let t = std::time::Instant::now();
    let mut defs = compile::compile_to_defs_state(&loaded);
    defs.push(("compile".to_string(), ast::Func::Platform("compile".to_string())));
    defs.push(("apply".to_string(), ast::Func::Platform("apply_command".to_string())));
    defs.push(("verify_signature".to_string(), ast::Func::Platform("verify_signature".to_string())));
    let d = ast::defs_to_state(&defs, &loaded);
    eprintln!("[profile] compile: {:?} ({} defs)", t.elapsed(), defs.len());
    d
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    // Parse flags.
    let no_validate = args.iter().any(|a| a == "--no-validate");
    let strict = args.iter().any(|a| a == "--strict");
    let (db_path, rest, _) = args.iter()
        .filter(|a| !matches!(a.as_str(), "--no-validate" | "--strict"))
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
                std::process::exit(0);
            }
            (false, _) => { rest.push(arg.clone()); (db, rest, false) }
        },
    );

    // Wire parsed flags to their engine-level thread_local toggles.
    if no_validate { ast::set_skip_validate(true); }
    if strict { parse_forml2::set_strict_mode(true); }

    #[cfg(not(feature = "local"))]
    {
        let _ = &db_path; let _ = &rest; // flags-only invocation
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

        match (dirs.is_empty(), non_dirs.len()) {
            // arest <dir1> [<dir2> ...] — compile readings via SYSTEM
            (false, _) => {
                let readings = read_readings(&dirs);
                readings.is_empty().then(|| {
                    eprintln!("No .md files found.");
                    std::process::exit(1);
                });

                // Extract generator opt-ins from raw reading text before parsing.
                // The parser doesn't handle dual-quoted instance facts like
                // "App 'X' uses Generator 'sqlite'" — extract via regex.
                let generator_re = regex::Regex::new(r"uses Generator '([^']+)'").unwrap();
                let opted_generators: std::collections::HashSet<String> = readings.iter()
                    .flat_map(|(_, text)| generator_re.captures_iter(text)
                        .filter_map(|c| c.get(1).map(|m| m.as_str().to_lowercase()))
                        .collect::<Vec<_>>())
                    .collect();
                eprintln!("[load] generators from readings: {:?}", opted_generators);

                // Fast path: fold all readings (metamodel + user) into a
                // single Domain IR, then convert to Object state ONCE.
                // No merge_states loop — O(n) in total content.
                parse_forml2::set_bootstrap_mode(true);
                parse_forml2::set_strict_mode(strict);
                let all_readings: Vec<(&str, &str)> = METAMODEL_READINGS.iter()
                    .map(|(n, t)| (*n, *t))
                    .chain(readings.iter().map(|(n, t)| (n.as_str(), t.as_str())))
                    .collect();
                let domain = all_readings.iter().fold(
                    types::Domain::default(),
                    |mut merged, (name, text)| {
                        let ir = match merged.nouns.is_empty() {
                            true => parse_forml2::parse_markdown(text),
                            false => parse_forml2::parse_markdown_with_context(text, &merged.nouns, &merged.fact_types),
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
                parse_forml2::set_strict_mode(false);
                eprintln!("[load] {} nouns, {} fts, {} instance facts",
                    domain.nouns.len(), domain.fact_types.len(), domain.general_instance_facts.len());
                let generator_fts: Vec<_> = domain.fact_types.keys()
                    .filter(|k| k.to_lowercase().contains("generator") || k.to_lowercase().contains("uses"))
                    .collect();
                eprintln!("[load] Generator-related FTs: {:?}", generator_fts);
                let app_ifs: Vec<_> = domain.general_instance_facts.iter()
                    .filter(|f| f.subject_noun == "App" || f.object_value.to_lowercase().contains("sqlite"))
                    .map(|f| format!("{}({}).{}={}({})", f.subject_noun, f.subject_value, f.field_name, f.object_noun, f.object_value))
                    .collect();
                eprintln!("[load] App/sqlite instance facts: {:?}", app_ifs);
                no_validate.then(|| ast::set_skip_validate(true));
                let mut state = parse_forml2::domain_to_state(&domain);
                // Store generator opt-ins as a cell so the query path can find them.
                opted_generators.iter().for_each(|g| {
                    state = ast::cell_push("App_uses_Generator",
                        ast::fact_from_pairs(&[("Generator", g.as_str())]), &state);
                });
                // Generate SQL triggers for derivation rules.
                if opted_generators.iter().any(|g| ["sqlite","postgresql","mysql"].contains(&g.as_str())) {
                    let sql_tables = crate::rmap::rmap(&domain);
                    let table_names: std::collections::HashSet<String> = sql_tables.iter()
                        .map(|t| t.name.clone()).collect();
                    let triggers = compile::generate_derivation_triggers(&domain, &sql_tables, &table_names);
                    triggers.iter().for_each(|(name, ddl)| {
                        state = ast::cell_push(&format!("sql:trigger:{}", name),
                            ast::Object::atom(ddl), &state);
                    });
                    eprintln!("[load] {} SQL triggers generated", triggers.len());
                }

                let defs = vec![
                    ("compile".to_string(), ast::Func::Platform("compile".to_string())),
                    ("apply".to_string(), ast::Func::Platform("apply_command".to_string())),
                    ("verify_signature".to_string(), ast::Func::Platform("verify_signature".to_string())),
                ];
                let d = ast::defs_to_state(&defs, &state);
                let compiled = readings.len();

                // Persist state to SQLite (tables + triggers).
                db::apply_ddl(&conn, &d);
                db::persist_state(&conn, &d);

                eprintln!("Compiled {} readings into {}", compiled, &db_path);
            }

            // arest <key> <input> — single SYSTEM call
            (true, n) if n >= 2 => {
                let d = load_and_compile(&conn);
                let key = &non_dirs[0];
                let input = &non_dirs[1];
                let t = std::time::Instant::now();
                let (output, new_d) = system(key, input, &d);
                eprintln!("[{:?}]", t.elapsed());
                println!("{}", output);
                (new_d != d).then(|| db::persist_state(&conn, &new_d));
            }

            // arest --db <path> — REPL mode
            _ => {
                let mut d = load_and_compile(&conn);

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

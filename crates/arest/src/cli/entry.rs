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

use crate::{ast, compile, parse_forml2};

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

    // State transition: if result contains cells (Noun, FactType, etc.)
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
    defs.push(("audit".to_string(), ast::Func::Platform("audit".to_string())));
    let d = ast::defs_to_state(&defs, &loaded);
    eprintln!("[profile] compile: {:?} ({} defs)", t.elapsed(), defs.len());
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

    let args: Vec<String> = std::env::args().skip(1).collect();

    // ── Subcommand dispatch ────────────────────────────────────────────
    // Subcommands are detected before flag parsing so they can have
    // their own argv conventions (a free-form app name with embedded
    // dashes / spaces would otherwise collide with --flags here).
    // Matched subcommands consume the rest of argv and return their
    // own exit code; unmatched first args fall through to the legacy
    // single-arg form (`arest <readings_dir>` etc.) below.
    if let Some(verb) = args.first() {
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
            #[cfg(feature = "compat-readings")]
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
            #[cfg(not(feature = "compat-readings"))]
            {
                eprintln!("`arest run` requires the `compat-readings` feature.");
                eprintln!("  cargo run --bin arest-cli --features compat-readings -- run \"App Name\"");
                std::process::exit(2);
            }
        }
    }

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
                // The parser doesn't yet handle dual-quoted instance facts like
                // "App 'X' uses Generator 'sqlite'" — extract via regex.
                //
                // Generators are App-scoped (`App 'X' uses Generator 'Y'.`):
                // we keep the (App, Generator) pair so downstream generators
                // can emit per-App cells. The set-of-generators view is
                // derived from the pairs for backward-compat paths (SQL
                // trigger emission still keys off generator names only).
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

                // Fold all readings (metamodel + user) into Object state.
                // Each reading parses to its own state; consecutive states
                // merge via cell concatenation. No Domain struct.
                parse_forml2::set_bootstrap_mode(true);
                parse_forml2::set_strict_mode(strict);
                let all_readings: Vec<(&str, &str)> = crate::metamodel_readings().into_iter()
                    .map(|r| (r.0, r.1))
                    .chain(readings.iter().map(|(n, t)| (n.as_str(), t.as_str())))
                    .collect();
                let state = all_readings.iter().fold(
                    ast::Object::phi(),
                    |merged, (name, text)| {
                        let this = parse_forml2::parse_to_state_from(text, &merged)
                            .unwrap_or_else(|e| { eprintln!("{}: {}", name, e); std::process::exit(1); });
                        ast::merge_states(&merged, &this)
                    },
                );
                parse_forml2::set_bootstrap_mode(false);
                parse_forml2::set_strict_mode(false);

                // Diagnostics: read cell sizes from the Object state.
                let cell_len = |name: &str| ast::fetch_or_phi(name, &state)
                    .as_seq().map(|s| s.len()).unwrap_or(0);
                eprintln!("[load] {} nouns, {} fts, {} instance facts",
                    cell_len("Noun"), cell_len("FactType"), cell_len("InstanceFact"));
                let ft_cell = ast::fetch_or_phi("FactType", &state);
                let generator_fts: Vec<String> = ft_cell.as_seq()
                    .map(|facts| facts.iter()
                        .filter_map(|f| ast::binding(f, "id").map(|s| s.to_string()))
                        .filter(|k| k.to_lowercase().contains("generator") || k.to_lowercase().contains("uses"))
                        .collect())
                    .unwrap_or_default();
                eprintln!("[load] Generator-related FTs: {:?}", generator_fts);
                let inst_cell = ast::fetch_or_phi("InstanceFact", &state);
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
                no_validate.then(|| ast::set_skip_validate(true));
                let mut state = state;
                // Store (App, Generator) opt-ins as a cell so compile can
                // emit per-App artifacts (openapi, eventually sqlite/etc.).
                opt_in_pairs.iter().for_each(|(app, g)| {
                    state = ast::cell_push("App_uses_Generator",
                        ast::fact_from_pairs(&[("App", app.as_str()), ("Generator", g.as_str())]),
                        &state);
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

// CLI for the FOL engine -- first-order logic reasoning over GraphDL domain models.
//
// Modes (legacy --ir):
//   evaluate       Check text/response against compiled constraint predicates
//   synthesize     Collect all knowledge about a noun (fact types, constraints, related nouns)
//   forward-chain  Derive new facts from a population until fixed point
//
// Modes (local SQLite):
//   bootstrap      Parse readings, compile, store in SQLite
//   system         Look up a def key and apply to input (includes constraint eval)
//   synthesize     Collect knowledge about a noun
//   forward-chain  Derive new facts from population
//
// The constraint IR is compiled once at load time. All evaluation is pure
// function application -- no dispatch, no branching on kind, no mutable state.
// Implements Backus's FP algebra (1977).

#[allow(dead_code)] // Functions used by WASM lib.rs, not by this binary
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

use types::Domain;

// =========================================================================
// SQLite-backed local runtime (feature = "local")
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

    pub fn create_tables(conn: &Connection, sql_defs: &[(String, ast::Func)]) {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS cells (name TEXT PRIMARY KEY, contents TEXT);
             CREATE TABLE IF NOT EXISTS defs (name TEXT PRIMARY KEY, func TEXT);"
        ).unwrap_or_else(|e| { eprintln!("Failed to create tables: {}", e); std::process::exit(1); });
        let ddl_errors = sql_defs.iter()
            .filter(|(name, _)| name.starts_with("sql:sqlite:"))
            .filter_map(|(name, func)| match func {
                ast::Func::Constant(ref obj) => obj.as_atom().map(|ddl| (name, ddl.to_string())),
                _ => None,
            })
            .fold(0usize, |errors, (name, ddl)| match conn.execute_batch(&ddl) {
                Err(e) => { eprintln!("Warning: DDL for {} failed: {}", name, e); errors + 1 }
                Ok(_) => errors,
            });
        if ddl_errors > 0 {
            eprintln!("{} DDL statements had errors (duplicate columns from RMAP)", ddl_errors);
        }
    }

    pub fn store_defs(conn: &Connection, defs: &[(String, ast::Func)]) {
        let tx = conn.unchecked_transaction()
            .unwrap_or_else(|e| { eprintln!("Failed to begin transaction: {}", e); std::process::exit(1); });
        tx.execute("DELETE FROM defs", [])
            .unwrap_or_else(|e| { eprintln!("Failed to clear defs: {}", e); std::process::exit(1); });
        {
            let mut stmt = tx.prepare("INSERT OR REPLACE INTO defs (name, func) VALUES (?1, ?2)")
                .unwrap_or_else(|e| { eprintln!("Failed to prepare insert: {}", e); std::process::exit(1); });
            defs.iter().for_each(|(name, func)| {
                let text = ast::func_to_object(func).to_string();
                stmt.execute(params![name, text])
                    .unwrap_or_else(|e| { eprintln!("Failed to insert def {}: {}", name, e); std::process::exit(1); });
            });
        }
        tx.commit()
            .unwrap_or_else(|e| { eprintln!("Failed to commit defs: {}", e); std::process::exit(1); });
    }

    pub fn store_facts(conn: &Connection, state: &ast::Object, tables: &[crate::rmap::TableDef]) {
        let tx = conn.unchecked_transaction()
            .unwrap_or_else(|e| { eprintln!("Failed to begin transaction: {}", e); std::process::exit(1); });

        let table_by_snake: HashMap<String, &crate::rmap::TableDef> = tables.iter()
            .map(|table| (table.name.clone(), table)).collect();

        let mut domain_rows: usize = 0;
        let mut meta_rows: usize = 0;

        // InstanceFacts are domain data.
        let inst_cell = ast::fetch_or_phi("InstanceFact", state);
        if let Some(instance_facts) = inst_cell.as_seq() {
            let rows: HashMap<(String, String), Vec<(String, String)>> = instance_facts.iter()
                .fold(HashMap::new(), |mut acc, inst| {
                    let get = |key: &str| ast::binding(inst, key).unwrap_or("").to_string();
                    let key = (crate::rmap::to_snake(&get("subjectNoun")), get("subjectValue"));
                    acc.entry(key).or_default().push((crate::rmap::to_snake(&get("fieldName")), get("objectValue")));
                    acc
                });

            rows.iter().filter_map(|((table_name, subject_id), columns)| {
                let table = table_by_snake.get(table_name.as_str())?;
                let pk_col = table.primary_key.first().map(|s| s.as_str()).unwrap_or("id");
                let table_col_names: Vec<&str> = table.columns.iter().map(|c| c.name.as_str()).collect();
                let (extra_cols, extra_vals): (Vec<_>, Vec<_>) = columns.iter()
                    .filter(|(col, _)| table_col_names.contains(&col.as_str()) && col != pk_col)
                    .map(|(col, val)| (col.clone(), val.clone()))
                    .unzip();
                let col_list: Vec<String> = std::iter::once(pk_col.to_string()).chain(extra_cols).collect();
                let val_list: Vec<String> = std::iter::once(subject_id.clone()).chain(extra_vals).collect();
                Some((table_name, col_list, val_list))
            }).for_each(|(table_name, col_list, val_list)| {
                let quoted_cols: Vec<String> = col_list.iter().map(|c| format!("\"{}\"", c)).collect();
                let placeholders: Vec<String> = (1..=col_list.len()).map(|i| format!("?{}", i)).collect();
                let sql = format!("INSERT OR REPLACE INTO \"{}\" ({}) VALUES ({})",
                    table_name, quoted_cols.join(", "), placeholders.join(", "));
                let params: Vec<&dyn rusqlite::ToSql> = val_list.iter().map(|v| v as &dyn rusqlite::ToSql).collect();
                match tx.execute(&sql, params.as_slice()) {
                    Ok(_) => domain_rows += 1,
                    Err(e) => eprintln!("Warning: INSERT into {} failed: {}", table_name, e),
                }
            });
        }

        // All other fact types are metamodel facts. Store in cells.
        ast::cells_iter(state).into_iter()
            .filter(|(ft_id, _)| *ft_id != "InstanceFact")
            .flat_map(|(ft_id, contents)| contents.as_seq()
                .map(|facts| facts.iter().map(move |fact| (ft_id, fact)).collect::<Vec<_>>())
                .unwrap_or_default())
            .for_each(|(ft_id, fact)| {
                let bindings: Vec<(String, String)> = fact.as_seq().map(|pairs|
                    pairs.iter().filter_map(|pair| {
                        let items = pair.as_seq()?;
                        Some((items.get(0)?.as_atom()?.to_string(), items.get(1)?.as_atom()?.to_string()))
                    }).collect()
                ).unwrap_or_default();
                let bindings_json = serde_json::to_string(&bindings)
                    .unwrap_or_else(|e| { eprintln!("Failed to serialize bindings: {}", e); std::process::exit(1); });
                tx.execute(
                    "INSERT OR REPLACE INTO cells (name, contents) VALUES (?1, ?2)",
                    params![format!("fact:{}:{}", ft_id, bindings_json), bindings_json],
                ).unwrap_or_else(|e| { eprintln!("Failed to store fact in cells: {}", e); std::process::exit(1); });
                meta_rows += 1;
            });

        tx.commit()
            .unwrap_or_else(|e| { eprintln!("Failed to commit facts: {}", e); std::process::exit(1); });
        eprintln!("  {} domain rows, {} metamodel rows", domain_rows, meta_rows);
    }

    pub fn load_defs(conn: &Connection) -> Vec<(String, ast::Func)> {
        let mut stmt = conn.prepare("SELECT name, func FROM defs")
            .unwrap_or_else(|e| { eprintln!("Failed to query defs: {}", e); std::process::exit(1); });
        let rows = stmt.query_map([], |row| {
            let name: String = row.get(0)?;
            let text: String = row.get(1)?;
            Ok((name, text))
        }).unwrap_or_else(|e| { eprintln!("Failed to read defs: {}", e); std::process::exit(1); });

        let empty_d = ast::Object::phi();
        let defs: Vec<(String, ast::Func)> = rows.filter_map(|row| {
            let (name, text) = row.ok()?;
            Some((name, ast::metacompose(&ast::Object::parse(&text), &empty_d)))
        }).collect();
        defs
    }

    pub fn load_state(conn: &Connection, tables: &[crate::rmap::TableDef]) -> ast::Object {
        let mut state = ast::Object::phi();

        // foldl(read_table, state, tables) — each table's rows fold into state
        let state = tables.iter().fold(state, |acc, table| {
            let col_names: Vec<&str> = table.columns.iter().map(|c| c.name.as_str()).collect();
            let quoted_cols: Vec<String> = col_names.iter().map(|c| format!("\"{}\"", c)).collect();
            let sql = format!("SELECT {} FROM \"{}\"", quoted_cols.join(", "), table.name);
            let mut stmt = match conn.prepare(&sql) { Ok(s) => s, Err(_) => return acc };
            let col_count = col_names.len();
            let rows = stmt.query_map([], |row| {
                Ok((0..col_count).filter_map(|i|
                    row.get::<_, String>(i).ok().map(|v| (col_names[i].to_string(), v))
                ).collect::<Vec<_>>())
            }).unwrap_or_else(|e| { eprintln!("Failed to read {}: {}", table.name, e); std::process::exit(1); });
            rows.filter_map(|r| r.ok()).filter(|pairs| !pairs.is_empty())
                .fold(acc, |s, pairs| {
                    let refs: Vec<(&str, &str)> = pairs.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
                    ast::cell_push(&table.name, ast::fact_from_pairs(&refs), &s)
                })
        });

        // foldl(read_cell, state, overflow_rows)
        let mut stmt = match conn.prepare("SELECT name, contents FROM cells WHERE name LIKE 'fact:%'") {
            Ok(s) => s, Err(_) => return state,
        };
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        }).unwrap_or_else(|e| { eprintln!("Failed to read cells: {}", e); std::process::exit(1); });
        rows.filter_map(|r| r.ok()).fold(state, |acc, (name, contents)| {
            let parts: Vec<&str> = name.splitn(3, ':').collect();
            (parts.len() >= 2).then(|| parts[1]).and_then(|ft_id|
                serde_json::from_str::<Vec<(String, String)>>(&contents).ok().map(|bindings| {
                    let refs: Vec<(&str, &str)> = bindings.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
                    ast::cell_push(ft_id, ast::fact_from_pairs(&refs), &acc)
                })
            ).unwrap_or(acc)
        })
    }

    pub fn store_table_meta(conn: &Connection, tables: &[crate::rmap::TableDef]) {
        let json = serde_json::to_string(tables)
            .unwrap_or_else(|e| { eprintln!("Failed to serialize table metadata: {}", e); std::process::exit(1); });
        conn.execute(
            "INSERT OR REPLACE INTO cells (name, contents) VALUES (?1, ?2)",
            params!["rmap:tables", json],
        ).unwrap_or_else(|e| { eprintln!("Failed to store table metadata: {}", e); std::process::exit(1); });
    }

    pub fn load_table_meta(conn: &Connection) -> Vec<crate::rmap::TableDef> {
        let mut stmt = match conn.prepare("SELECT contents FROM cells WHERE name = 'rmap:tables'") {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        let result: Option<String> = stmt.query_row([], |row| row.get(0)).ok();
        match result {
            Some(json) => serde_json::from_str(&json).unwrap_or_default(),
            None => Vec::new(),
        }
    }
}

// =========================================================================
// Local CLI (feature = "local") -- subcommand-based interface
// =========================================================================

#[cfg(feature = "local")]
fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 || args[1] == "--help" || args[1] == "-h" {
        print_local_help();
        std::process::exit(0);
    }

    match args[1].as_str() {
        "bootstrap" => cmd_bootstrap(&args[2..]),
        "system" => cmd_system(&args[2..]),
        "synthesize" => cmd_synthesize(&args[2..]),
        "forward-chain" => cmd_forward_chain(&args[2..]),
        other => {
            eprintln!("Unknown subcommand: {}", other);
            eprintln!("Run with --help for usage.");
            std::process::exit(1);
        }
    }
}

#[cfg(feature = "local")]
fn parse_db_flag(args: &[String]) -> (String, Vec<String>) {
    let mut db_path = String::from("./app.db");
    let mut rest = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--db" {
            i += 1;
            if let Some(p) = args.get(i) {
                db_path = p.clone();
            }
        } else {
            rest.push(args[i].clone());
        }
        i += 1;
    }
    (db_path, rest)
}

#[cfg(feature = "local")]
fn cmd_bootstrap(args: &[String]) {
    let (db_path, rest) = parse_db_flag(args);

    if rest.is_empty() {
        eprintln!("Usage: fol bootstrap <readings_dir> [<readings_dir2> ...] [--db <path>]");
        std::process::exit(1);
    }

    // Collect all .md files from each directory, sorted alphabetically.
    // If app.md exists in any directory, read it first.
    let mut readings: Vec<(String, String)> = Vec::new();
    let mut app_md: Option<(String, String)> = None;

    let (readings, app_md) = rest.iter().flat_map(|dir| {
        let dir_path = std::path::Path::new(dir);
        if !dir_path.is_dir() { eprintln!("Not a directory: {}", dir); std::process::exit(1); }
        let mut entries: Vec<std::path::PathBuf> = std::fs::read_dir(dir_path)
            .unwrap_or_else(|e| { eprintln!("Failed to read directory {}: {}", dir, e); std::process::exit(1); })
            .filter_map(|e| e.ok()).map(|e| e.path())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("md")).collect();
        entries.sort();
        entries.into_iter().map(|path| {
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| { eprintln!("Failed to read {}: {}", path.display(), e); std::process::exit(1); });
            (name, text)
        }).collect::<Vec<_>>()
    }).fold((Vec::new(), None::<(String, String)>), |(mut readings, mut app), (name, text)| {
        if name == "app.md" { app = Some((name, text)); } else { readings.push((name, text)); }
        (readings, app)
    });

    // app.md first (for domain ordering), then the rest alphabetically.
    let mut ordered: Vec<(String, String)> = Vec::new();
    if let Some(app) = app_md {
        ordered.push(app);
    }
    ordered.extend(readings);

    if ordered.is_empty() {
        eprintln!("No .md files found in specified directories.");
        std::process::exit(1);
    }

    // Parse readings -- same merge strategy as parse_and_compile_impl in lib.rs.
    let merged = ordered.iter().fold(Domain::default(), |mut merged, (name, text)| {
        let ir = if merged.nouns.is_empty() {
            parse_forml2::parse_markdown(text)
        } else {
            parse_forml2::parse_markdown_with_nouns(text, &merged.nouns)
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
    });

    // Convert to Object state and compile.
    let state = parse_forml2::domain_to_state(&merged);
    let defs = compile::compile_to_defs_state(&state);
    let tables = rmap::rmap(&merged);

    // Count categories.
    let noun_count = merged.nouns.len();
    let fact_type_count = merged.fact_types.len();
    let constraint_count = merged.constraints.len();
    let derivation_count = merged.derivation_rules.len();
    let state_machine_count = merged.state_machines.len();
    let table_count = tables.len();

    // Store in SQLite using generated DDL from sql:{table} defs.
    let conn = db::open(&db_path);
    db::create_tables(&conn, &defs);
    db::store_facts(&conn, &state, &tables);
    db::store_defs(&conn, &defs);
    db::store_table_meta(&conn, &tables);

    // Summary.
    println!("Bootstrapped {} into {}", rest.join(", "), db_path);
    println!("  {} files parsed", ordered.len());
    println!("  {} nouns", noun_count);
    println!("  {} fact types", fact_type_count);
    println!("  {} constraints", constraint_count);
    println!("  {} derivation rules", derivation_count);
    println!("  {} state machines", state_machine_count);
    println!("  {} tables generated", table_count);
    println!("  {} defs compiled", defs.len());
}

#[cfg(feature = "local")]
fn cmd_system(args: &[String]) {
    let (db_path, rest) = parse_db_flag(args);

    if rest.len() < 2 {
        eprintln!("Usage: fol system <key> <input> [--db <path>]");
        std::process::exit(1);
    }

    let key = &rest[0];
    let input = &rest[1];

    let conn = db::open(&db_path);
    let defs = db::load_defs(&conn);

    // Backus 14.4.2: D contains FILE (population) + DEFS.
    let tables = db::load_table_meta(&conn);
    let state = db::load_state(&conn, &tables);
    let d = ast::defs_to_state(&defs, &state);
    let input_obj = ast::Object::parse(input);
    let obj = ast::Object::seq(vec![input_obj, ast::Object::phi(), d.clone()]);

    let def_obj = ast::fetch_or_phi(key.as_str(), &d);
    match &def_obj {
        ast::Object::Bottom => {
            eprintln!("Key not found in defs: {}", key);
            std::process::exit(1);
        }
        _ => {
            let result = ast::apply(&ast::metacompose(&def_obj, &d), &obj, &d);
            println!("{}", result);
        }
    }
}

#[cfg(feature = "local")]
fn cmd_synthesize(args: &[String]) {
    let (db_path, rest) = parse_db_flag(args);

    // Parse --depth flag from rest.
    let mut noun: Option<String> = None;
    let mut depth: usize = 1;
    let mut i = 0;
    while i < rest.len() {
        if rest[i] == "--depth" {
            i += 1;
            depth = rest.get(i).and_then(|s| s.parse().ok()).unwrap_or(1);
        } else if noun.is_none() {
            noun = Some(rest[i].clone());
        }
        i += 1;
    }

    let noun = match noun {
        Some(n) => n,
        None => {
            eprintln!("Usage: fol synthesize <noun> [--depth <n>] [--db <path>]");
            std::process::exit(1);
        }
    };

    let conn = db::open(&db_path);
    let tables = db::load_table_meta(&conn);
    let state = db::load_state(&conn, &tables);
    let result = evaluate::synthesize_from_state(&state, &noun, depth);
    println!("{}", serde_json::to_string_pretty(&result).unwrap());
}

#[cfg(feature = "local")]
fn cmd_forward_chain(args: &[String]) {
    let (db_path, _rest) = parse_db_flag(args);

    let conn = db::open(&db_path);
    let defs = db::load_defs(&conn);
    let tables = db::load_table_meta(&conn);
    let state = db::load_state(&conn, &tables);

    let derivation_defs: Vec<(&str, &ast::Func)> = defs.iter()
        .filter(|(n, _)| n.starts_with("derivation:"))
        .map(|(n, f)| (n.as_str(), f))
        .collect();
    let (new_state, derived) = evaluate::forward_chain_defs_state(&derivation_defs, &state);

    if derived.is_empty() {
        println!("No new facts derived");
    } else {
        // Store derived facts back.
        db::store_facts(&conn, &new_state, &tables);
        println!("{}", serde_json::to_string_pretty(&derived).unwrap());
    }
}

#[cfg(feature = "local")]
fn print_local_help() {
    eprintln!("fol -- first-order logic reasoning engine for GraphDL domain models");
    eprintln!();
    eprintln!("SUBCOMMANDS:");
    eprintln!();
    eprintln!("  bootstrap <readings_dir> [<dir2> ...] [--db <path>]");
    eprintln!("    Parse .md readings, compile to defs, store in SQLite.");
    eprintln!("    Default --db: ./app.db");
    eprintln!();
    eprintln!("  system <key> <input> [--db <path>]");
    eprintln!("    Look up key in defs, apply to input, print result.");
    eprintln!("    Evaluate a constraint: fol system constraint:<id> <text>");
    eprintln!();
    eprintln!("  synthesize <noun> [--depth <n>] [--db <path>]");
    eprintln!("    Collect all knowledge about a noun.");
    eprintln!();
    eprintln!("  forward-chain [--db <path>]");
    eprintln!("    Derive new facts from population until fixed point.");
    eprintln!();
    eprintln!("OPTIONS:");
    eprintln!("  --db <path>   SQLite database path (default: ./app.db)");
    eprintln!("  --help, -h    Show this help");
}

// =========================================================================
// Legacy CLI (no "local" feature) -- --ir based interface
// =========================================================================

#[cfg(not(feature = "local"))]
fn main() {
    let args: Vec<String> = std::env::args().collect();

    let mut ir_path: Option<String> = None;
    let mut response_path: Option<String> = None;
    let mut text: Option<String> = None;
    let mut population_path: Option<String> = None;
    let mut synthesize_noun: Option<String> = None;
    let mut synthesize_depth: usize = 1;
    let mut do_forward_chain = false;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--ir" => { i += 1; ir_path = args.get(i).cloned(); }
            "--response" => { i += 1; response_path = args.get(i).cloned(); }
            "--text" => { i += 1; text = args.get(i).cloned(); }
            "--population" => { i += 1; population_path = args.get(i).cloned(); }
            "--synthesize" => { i += 1; synthesize_noun = args.get(i).cloned(); }
            "--depth" => {
                i += 1;
                synthesize_depth = args.get(i)
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(1);
            }
            "--forward-chain" => { do_forward_chain = true; }
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            _ => {
                eprintln!("Unknown argument: {}", args[i]);
                std::process::exit(1);
            }
        }
        i += 1;
    }

    // Load IR
    let ir_json = match ir_path {
        Some(p) => std::fs::read_to_string(&p)
            .unwrap_or_else(|e| { eprintln!("Failed to read IR file: {}", e); std::process::exit(1); }),
        None => { eprintln!("--ir is required. Run with --help for usage."); std::process::exit(1); }
    };

    let ir: Domain = serde_json::from_str(&ir_json)
        .unwrap_or_else(|e| { eprintln!("Failed to parse IR: {}", e); std::process::exit(1); });
    let ir_state = parse_forml2::domain_to_state(&ir);
    let defs = compile::compile_to_defs_state(&ir_state);
    let d = ast::defs_to_state(&defs, &ir_state);

    // -- Synthesize mode --
    if let Some(noun_name) = synthesize_noun {
        let result = evaluate::synthesize_from_state(&ir_state, &noun_name, synthesize_depth);
        println!("{}", serde_json::to_string_pretty(&result).unwrap());
        std::process::exit(0);
    }

    // -- Forward chain mode --
    if do_forward_chain {
        let pop_state = load_state(population_path, true);
        let derivation_defs: Vec<(&str, &ast::Func)> = defs.iter()
            .filter(|(n, _)| n.starts_with("derivation:"))
            .map(|(n, f)| (n.as_str(), f))
            .collect();
        let (_new_state, derived) = evaluate::forward_chain_defs_state(&derivation_defs, &pop_state);
        if derived.is_empty() {
            println!("No new facts derived");
        } else {
            println!("{}", serde_json::to_string_pretty(&derived).unwrap());
        }
        std::process::exit(0);
    }

    // -- Evaluate mode (default) --
    let response_text: String = if let Some(t) = text {
        t
    } else if let Some(p) = response_path {
        std::fs::read_to_string(&p)
            .unwrap_or_else(|e| { eprintln!("Failed to read response file: {}", e); std::process::exit(1); })
    } else {
        let mut input = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut input)
            .unwrap_or_else(|e| { eprintln!("Failed to read stdin: {}", e); std::process::exit(1); });
        input.trim().to_string()
    };

    let pop_state = load_state(population_path, false);
    let ctx_obj = ast::encode_eval_context_state(&response_text, None, &pop_state);
    let violations: Vec<types::Violation> = defs.iter()
        .filter(|(n, _)| n.starts_with("constraint:"))
        .flat_map(|(name, func)| {
            let result = ast::apply(func, &ctx_obj, &d);
            let is_deontic = name.contains("obligatory") || name.contains("forbidden");
            ast::decode_violations(&result).into_iter().map(move |mut v| {
                v.alethic = !is_deontic;
                v
            })
        })
        .collect();

    if violations.is_empty() {
        println!("OK -- no violations");
        std::process::exit(0);
    } else {
        println!("{}", serde_json::to_string_pretty(&violations).unwrap());
        std::process::exit(1);
    }
}

#[cfg(not(feature = "local"))]
fn load_state(path: Option<String>, required: bool) -> ast::Object {
    match path {
        Some(p) => {
            let json = std::fs::read_to_string(&p)
                .unwrap_or_else(|e| { eprintln!("Failed to read state file: {}", e); std::process::exit(1); });
            ast::Object::parse(&json)
        }
        None if required => {
            eprintln!("--population <path> is required for this mode");
            std::process::exit(1);
        }
        None => ast::Object::phi(),
    }
}

#[cfg(not(feature = "local"))]
fn print_help() {
    eprintln!("fol -- first-order logic reasoning engine for GraphDL domain models");
    eprintln!();
    eprintln!("Implements Backus FP algebra: constraints compile to pure functions,");
    eprintln!("evaluation is function application over whole structures.");
    eprintln!();
    eprintln!("MODES:");
    eprintln!();
    eprintln!("  Evaluate (default) -- check text against constraint predicates");
    eprintln!("    fol --ir <ir.json> --text <text to verify>");
    eprintln!("    fol --ir <ir.json> --response <response.json>");
    eprintln!();
    eprintln!("  Synthesize -- collect all knowledge about a noun");
    eprintln!("    fol --ir <ir.json> --synthesize <noun> [--depth <n>]");
    eprintln!("    Returns: fact types, constraints, state machines, related nouns");
    eprintln!();
    eprintln!("  Forward Chain -- derive new facts from a population until fixed point");
    eprintln!("    fol --ir <ir.json> --forward-chain --population <pop.json>");
    eprintln!("    Derivation rules: subtype inheritance, modus ponens, transitivity,");
    eprintln!("    closed-world negation. Returns all derived facts with proof chains.");
    eprintln!();
    eprintln!("OPTIONS:");
    eprintln!("  --ir <path>            Constraint IR JSON file (required)");
    eprintln!("  --text <string>        Text to evaluate against constraints");
    eprintln!("  --response <path>      Response JSON file");
    eprintln!("  --population <path>    Population JSON file");
    eprintln!("  --synthesize <noun>    Synthesize knowledge about a noun");
    eprintln!("  --depth <n>            Synthesis depth for related nouns (default: 1)");
    eprintln!("  --forward-chain        Run forward inference on population");
    eprintln!();
    eprintln!("EXIT CODES:");
    eprintln!("  0  Clean -- no violations / successful operation");
    eprintln!("  1  Violations found");
}

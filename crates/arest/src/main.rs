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
#[cfg(not(feature = "local"))]
use types::Population;

// =========================================================================
// SQLite-backed local runtime (feature = "local")
// =========================================================================

#[cfg(feature = "local")]
mod db {
    use rusqlite::{Connection, params};
    use crate::types::{Population, FactInstance};
    use crate::ast;
    use std::collections::HashMap;

    pub fn open(path: &str) -> Connection {
        Connection::open(path)
            .unwrap_or_else(|e| { eprintln!("Failed to open database {}: {}", path, e); std::process::exit(1); })
    }

    pub fn create_tables(conn: &Connection) {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS cells (name TEXT PRIMARY KEY, contents TEXT);
             CREATE TABLE IF NOT EXISTS defs (name TEXT PRIMARY KEY, func TEXT);
             CREATE TABLE IF NOT EXISTS facts (fact_type_id TEXT, bindings TEXT);"
        ).unwrap_or_else(|e| { eprintln!("Failed to create tables: {}", e); std::process::exit(1); });
    }

    pub fn store_defs(conn: &Connection, defs: &[(String, ast::Func)]) {
        let tx = conn.unchecked_transaction()
            .unwrap_or_else(|e| { eprintln!("Failed to begin transaction: {}", e); std::process::exit(1); });
        tx.execute("DELETE FROM defs", [])
            .unwrap_or_else(|e| { eprintln!("Failed to clear defs: {}", e); std::process::exit(1); });
        {
            let mut stmt = tx.prepare("INSERT OR REPLACE INTO defs (name, func) VALUES (?1, ?2)")
                .unwrap_or_else(|e| { eprintln!("Failed to prepare insert: {}", e); std::process::exit(1); });
            for (name, func) in defs {
                let obj = ast::func_to_object(func);
                let text = obj.to_string();
                stmt.execute(params![name, text])
                    .unwrap_or_else(|e| { eprintln!("Failed to insert def {}: {}", name, e); std::process::exit(1); });
            }
        }
        tx.commit()
            .unwrap_or_else(|e| { eprintln!("Failed to commit defs: {}", e); std::process::exit(1); });
    }

    pub fn store_facts(conn: &Connection, pop: &Population) {
        let tx = conn.unchecked_transaction()
            .unwrap_or_else(|e| { eprintln!("Failed to begin transaction: {}", e); std::process::exit(1); });
        tx.execute("DELETE FROM facts", [])
            .unwrap_or_else(|e| { eprintln!("Failed to clear facts: {}", e); std::process::exit(1); });
        {
            let mut stmt = tx.prepare("INSERT OR REPLACE INTO facts (fact_type_id, bindings) VALUES (?1, ?2)")
                .unwrap_or_else(|e| { eprintln!("Failed to prepare insert: {}", e); std::process::exit(1); });
            for (ft_id, instances) in &pop.facts {
                for inst in instances {
                    let bindings_json = serde_json::to_string(&inst.bindings)
                        .unwrap_or_else(|e| { eprintln!("Failed to serialize bindings: {}", e); std::process::exit(1); });
                    stmt.execute(params![ft_id, bindings_json])
                        .unwrap_or_else(|e| { eprintln!("Failed to insert fact: {}", e); std::process::exit(1); });
                }
            }
        }
        tx.commit()
            .unwrap_or_else(|e| { eprintln!("Failed to commit facts: {}", e); std::process::exit(1); });
    }

    pub fn load_defs(conn: &Connection) -> Vec<(String, ast::Func)> {
        let mut stmt = conn.prepare("SELECT name, func FROM defs")
            .unwrap_or_else(|e| { eprintln!("Failed to query defs: {}", e); std::process::exit(1); });
        let rows = stmt.query_map([], |row| {
            let name: String = row.get(0)?;
            let text: String = row.get(1)?;
            Ok((name, text))
        }).unwrap_or_else(|e| { eprintln!("Failed to read defs: {}", e); std::process::exit(1); });

        let mut defs = Vec::new();
        let empty_defs = HashMap::new();
        for row in rows {
            let (name, text) = row.unwrap_or_else(|e| { eprintln!("Failed to read def row: {}", e); std::process::exit(1); });
            let obj = ast::Object::parse(&text);
            let func = ast::metacompose(&obj, &empty_defs);
            defs.push((name, func));
        }
        defs
    }

    pub fn load_population(conn: &Connection) -> Population {
        let mut stmt = conn.prepare("SELECT fact_type_id, bindings FROM facts")
            .unwrap_or_else(|e| { eprintln!("Failed to query facts: {}", e); std::process::exit(1); });
        let rows = stmt.query_map([], |row| {
            let ft_id: String = row.get(0)?;
            let bindings_json: String = row.get(1)?;
            Ok((ft_id, bindings_json))
        }).unwrap_or_else(|e| { eprintln!("Failed to read facts: {}", e); std::process::exit(1); });

        let mut facts: HashMap<String, Vec<FactInstance>> = HashMap::new();
        for row in rows {
            let (ft_id, bindings_json) = row.unwrap_or_else(|e| { eprintln!("Failed to read fact row: {}", e); std::process::exit(1); });
            let bindings: Vec<(String, String)> = serde_json::from_str(&bindings_json)
                .unwrap_or_else(|e| { eprintln!("Failed to parse bindings: {}", e); std::process::exit(1); });
            facts.entry(ft_id.clone()).or_default().push(FactInstance {
                fact_type_id: ft_id,
                bindings,
            });
        }
        Population { facts }
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

    for dir in &rest {
        let dir_path = std::path::Path::new(dir);
        if !dir_path.is_dir() {
            eprintln!("Not a directory: {}", dir);
            std::process::exit(1);
        }
        let mut entries: Vec<std::path::PathBuf> = std::fs::read_dir(dir_path)
            .unwrap_or_else(|e| { eprintln!("Failed to read directory {}: {}", dir, e); std::process::exit(1); })
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("md"))
            .collect();
        entries.sort();

        for path in entries {
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| { eprintln!("Failed to read {}: {}", path.display(), e); std::process::exit(1); });
            if name == "app.md" {
                app_md = Some((name, text));
            } else {
                readings.push((name, text));
            }
        }
    }

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
    let mut merged = Domain::default();
    for (name, text) in &ordered {
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
    }

    // Convert to population and compile.
    let pop = parse_forml2::domain_to_population(&merged);
    let defs = compile::compile_to_defs(&pop);

    // Count categories.
    let noun_count = merged.nouns.len();
    let fact_type_count = merged.fact_types.len();
    let constraint_count = merged.constraints.len();
    let derivation_count = merged.derivation_rules.len();
    let state_machine_count = merged.state_machines.len();

    // Store in SQLite.
    let conn = db::open(&db_path);
    db::create_tables(&conn);
    db::store_facts(&conn, &pop);
    db::store_defs(&conn, &defs);

    // Summary.
    println!("Bootstrapped {} into {}", rest.join(", "), db_path);
    println!("  {} files parsed", ordered.len());
    println!("  {} nouns", noun_count);
    println!("  {} fact types", fact_type_count);
    println!("  {} constraints", constraint_count);
    println!("  {} derivation rules", derivation_count);
    println!("  {} state machines", state_machine_count);
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
    let def_map: std::collections::HashMap<String, ast::Func> =
        defs.iter().map(|(n, f)| (n.clone(), f.clone())).collect();

    // Backus 14.4.2: the operand includes input and FILE (population).
    // Every def receives the same operand. No branching on key.
    let pop = db::load_population(&conn);
    let input_obj = ast::Object::parse(input);
    let pop_obj = ast::encode_population(&pop);
    let obj = ast::Object::seq(vec![input_obj, ast::Object::phi(), pop_obj]);

    if let Some(func) = def_map.get(key.as_str()) {
        let result = ast::apply(func, &obj, &def_map);
        println!("{}", result);
    } else {
        eprintln!("Key not found in defs: {}", key);
        std::process::exit(1);
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
    let pop = db::load_population(&conn);
    let result = evaluate::synthesize_from_pop(&pop, &noun, depth);
    println!("{}", serde_json::to_string_pretty(&result).unwrap());
}

#[cfg(feature = "local")]
fn cmd_forward_chain(args: &[String]) {
    let (db_path, _rest) = parse_db_flag(args);

    let conn = db::open(&db_path);
    let defs = db::load_defs(&conn);
    let mut pop = db::load_population(&conn);
    let def_map: std::collections::HashMap<String, ast::Func> =
        defs.iter().map(|(n, f)| (n.clone(), f.clone())).collect();
    let _ = &def_map; // defs used by name matching below

    let derivation_defs: Vec<(&str, &ast::Func)> = defs.iter()
        .filter(|(n, _)| n.starts_with("derivation:"))
        .map(|(n, f)| (n.as_str(), f))
        .collect();
    let derived = evaluate::forward_chain_defs(&derivation_defs, &mut pop);

    if derived.is_empty() {
        println!("No new facts derived");
    } else {
        // Store derived facts back.
        db::store_facts(&conn, &pop);
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
    let pop = parse_forml2::domain_to_population(&ir);
    let defs = compile::compile_to_defs(&pop);
    let def_map: std::collections::HashMap<String, ast::Func> =
        defs.iter().map(|(n, f)| (n.clone(), f.clone())).collect();

    // -- Synthesize mode --
    if let Some(noun_name) = synthesize_noun {
        let result = evaluate::synthesize_from_pop(&pop, &noun_name, synthesize_depth);
        println!("{}", serde_json::to_string_pretty(&result).unwrap());
        std::process::exit(0);
    }

    // -- Forward chain mode --
    if do_forward_chain {
        let mut population = load_population(population_path, true);
        let derivation_defs: Vec<(&str, &ast::Func)> = defs.iter()
            .filter(|(n, _)| n.starts_with("derivation:"))
            .map(|(n, f)| (n.as_str(), f))
            .collect();
        let derived = evaluate::forward_chain_defs(&derivation_defs, &mut population);
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

    let population = load_population(population_path, false);
    let ctx_obj = ast::encode_eval_context(&response_text, None, &population);
    let violations: Vec<types::Violation> = defs.iter()
        .filter(|(n, _)| n.starts_with("constraint:"))
        .flat_map(|(name, func)| {
            let result = ast::apply(func, &ctx_obj, &def_map);
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
fn load_population(path: Option<String>, required: bool) -> Population {
    match path {
        Some(p) => {
            let json = std::fs::read_to_string(&p)
                .unwrap_or_else(|e| { eprintln!("Failed to read population file: {}", e); std::process::exit(1); });
            serde_json::from_str(&json)
                .unwrap_or_else(|e| { eprintln!("Failed to parse population: {}", e); std::process::exit(1); })
        }
        None if required => {
            eprintln!("--population <path> is required for this mode");
            std::process::exit(1);
        }
        None => Population { facts: std::collections::HashMap::new() },
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

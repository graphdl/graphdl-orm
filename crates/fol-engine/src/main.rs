// CLI for the FOL engine — first-order logic reasoning over GraphDL domain models.
//
// Modes:
//   evaluate       Check text/response against compiled constraint predicates
//   synthesize     Collect all knowledge about a noun (fact types, constraints, related nouns)
//   forward-chain  Derive new facts from a population until fixed point
//   query          Filter a population by a predicate, return matching entities
//
// The constraint IR is compiled once at load time. All evaluation is pure
// function application — no dispatch, no branching on kind, no mutable state.
// Implements Backus's FP algebra (1977).

mod ast;
mod types;
mod compile;
mod evaluate;
mod query;
mod rmap;
mod naming;
mod conceptual_query;
mod parse_rule;
mod arest;
mod validate;

use types::{ConstraintIR, ResponseContext, Population};
use query::QueryPredicate;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    let mut ir_path: Option<String> = None;
    let mut response_path: Option<String> = None;
    let mut text: Option<String> = None;
    let mut population_path: Option<String> = None;
    let mut synthesize_noun: Option<String> = None;
    let mut synthesize_depth: usize = 1;
    let mut do_forward_chain = false;
    let mut query_path: Option<String> = None;

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
            "--query" => { i += 1; query_path = args.get(i).cloned(); }
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

    let ir: ConstraintIR = serde_json::from_str(&ir_json)
        .unwrap_or_else(|e| { eprintln!("Failed to parse IR: {}", e); std::process::exit(1); });
    let model = compile::compile(&ir);

    // ── Synthesize mode ──────────────────────────────────────────────
    if let Some(noun_name) = synthesize_noun {
        let result = evaluate::synthesize(&model, &ir, &noun_name, synthesize_depth);
        println!("{}", serde_json::to_string_pretty(&result).unwrap());
        std::process::exit(0);
    }

    // ── Forward chain mode ───────────────────────────────────────────
    if do_forward_chain {
        let mut population = load_population(population_path, true);
        let derived = evaluate::forward_chain_ast(&model, &mut population);
        if derived.is_empty() {
            println!("No new facts derived");
        } else {
            println!("{}", serde_json::to_string_pretty(&derived).unwrap());
        }
        std::process::exit(0);
    }

    // ── Query mode ───────────────────────────────────────────────────
    if let Some(qp) = query_path {
        let population = load_population(population_path, true);
        let query_json = std::fs::read_to_string(&qp)
            .unwrap_or_else(|e| { eprintln!("Failed to read query file: {}", e); std::process::exit(1); });
        let predicate: QueryPredicate = serde_json::from_str(&query_json)
            .unwrap_or_else(|e| { eprintln!("Failed to parse query: {}", e); std::process::exit(1); });
        let result = query::query_population(&population, &predicate);
        println!("{}", serde_json::to_string_pretty(&result).unwrap());
        std::process::exit(0);
    }

    // ── Evaluate mode (default) ──────────────────────────────────────
    let response: ResponseContext = if let Some(t) = text {
        ResponseContext { text: t, sender_identity: None, fields: None }
    } else if let Some(p) = response_path {
        let json = std::fs::read_to_string(&p)
            .unwrap_or_else(|e| { eprintln!("Failed to read response file: {}", e); std::process::exit(1); });
        serde_json::from_str(&json)
            .unwrap_or_else(|e| { eprintln!("Failed to parse response: {}", e); std::process::exit(1); })
    } else {
        // Read from stdin
        let mut input = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut input)
            .unwrap_or_else(|e| { eprintln!("Failed to read stdin: {}", e); std::process::exit(1); });
        if input.trim().starts_with('{') {
            serde_json::from_str(&input)
                .unwrap_or_else(|e| { eprintln!("Failed to parse stdin JSON: {}", e); std::process::exit(1); })
        } else {
            ResponseContext { text: input.trim().to_string(), sender_identity: None, fields: None }
        }
    };

    let population = load_population(population_path, false);
    let violations = evaluate::evaluate_via_ast(&model, &response, &population);

    if violations.is_empty() {
        println!("OK — no violations");
        std::process::exit(0);
    } else {
        println!("{}", serde_json::to_string_pretty(&violations).unwrap());
        std::process::exit(1);
    }
}

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

fn print_help() {
    eprintln!("fol — first-order logic reasoning engine for GraphDL domain models");
    eprintln!();
    eprintln!("Implements Backus's FP algebra: constraints compile to pure functions,");
    eprintln!("evaluation is function application over whole structures.");
    eprintln!();
    eprintln!("MODES:");
    eprintln!();
    eprintln!("  Evaluate (default) — check text against constraint predicates");
    eprintln!("    fol --ir <ir.json> --text \"text to verify\"");
    eprintln!("    fol --ir <ir.json> --response <response.json>");
    eprintln!("    echo '{{\"text\":\"...\"}}' | fol --ir <ir.json>");
    eprintln!();
    eprintln!("  Synthesize — collect all knowledge about a noun");
    eprintln!("    fol --ir <ir.json> --synthesize <noun> [--depth <n>]");
    eprintln!("    Returns: fact types, constraints, state machines, related nouns");
    eprintln!();
    eprintln!("  Forward Chain — derive new facts from a population until fixed point");
    eprintln!("    fol --ir <ir.json> --forward-chain --population <pop.json>");
    eprintln!("    Derivation rules: subtype inheritance, modus ponens, transitivity,");
    eprintln!("    closed-world negation. Returns all derived facts with proof chains.");
    eprintln!();
    eprintln!("  Query — filter a population by a predicate");
    eprintln!("    fol --ir <ir.json> --query <predicate.json> --population <pop.json>");
    eprintln!("    Predicate: {{\"factTypeId\":\"..\",\"targetNoun\":\"..\",\"filterBindings\":[[k,v]]}}");
    eprintln!("    Returns matching entity references.");
    eprintln!();
    eprintln!("OPTIONS:");
    eprintln!("  --ir <path>            Constraint IR JSON file (required)");
    eprintln!("  --text <string>        Text to evaluate against constraints");
    eprintln!("  --response <path>      Response JSON ({{\"text\":\"...\",\"senderIdentity\":\"...\"}})");
    eprintln!("  --population <path>    Population JSON ({{\"facts\":{{id:[{{factTypeId,bindings}}]}}}})");
    eprintln!("  --synthesize <noun>    Synthesize knowledge about a noun");
    eprintln!("  --depth <n>            Synthesis depth for related nouns (default: 1)");
    eprintln!("  --forward-chain        Run forward inference on population");
    eprintln!("  --query <path>         Query predicate JSON file");
    eprintln!();
    eprintln!("EXIT CODES:");
    eprintln!("  0  Clean — no violations / successful operation");
    eprintln!("  1  Violations found / query returned results");
}

// Generate Solidity from a directory of FORML 2 readings.
//
// Usage:
//   cargo run --example gen_solidity -- <readings_dir>
//
// Reads every .md file in the given directory, compiles the merged
// domain, and writes the Solidity contract source to stdout.
//
// The Foundry project in /contracts consumes this by redirecting
// stdout into contracts/src/Generated.sol.

use std::env;
use std::fs;
use std::path::PathBuf;

use arest::parse_forml2;
use arest::ast;
use arest::generators::solidity;

// The bundled metamodel readings. User readings parse on top of these
// so SM wiring like "State Machine Definition 'Order' is for Noun 'Order'"
// resolves with the correct role nouns (State Machine Definition, Noun)
// rather than collapsing to two Order roles.
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

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: gen_solidity <readings_dir>");
        std::process::exit(1);
    }
    let dir = PathBuf::from(&args[1]);

    // Recursively collect .md files in the directory, depth-first.
    let user_readings = collect_readings(&dir);
    if user_readings.is_empty() {
        eprintln!("no .md files found under {}", dir.display());
        std::process::exit(1);
    }

    // Bootstrap mode bypasses the metamodel-namespace guard so the
    // metamodel can redeclare its own nouns across files. The guard is
    // only needed for user-domain compiles.
    parse_forml2::set_bootstrap_mode(true);
    let meta_state = METAMODEL_READINGS.iter().fold(ast::Object::phi(), |acc, (name, text)| {
        match parse_forml2::parse_to_state_from(text, &acc) {
            Ok(parsed) => ast::merge_states(&acc, &parsed),
            Err(e) => {
                eprintln!("metamodel parse failed in readings/{}.md: {}", name, e);
                std::process::exit(1);
            }
        }
    });
    parse_forml2::set_bootstrap_mode(false);

    // Capture the metamodel's entity noun names so we can exclude them
    // from the generated output: users want contracts for their own
    // domain only, not for State Machine Definition, Frequency Constraint,
    // and the rest of the metamodel.
    let meta_nouns = entity_nouns(&meta_state);

    // User readings parse on top of the metamodel. Now SM wiring
    // instance facts resolve to the right role nouns.
    let state = user_readings.iter().fold(meta_state, |acc, (name, text)| {
        match parse_forml2::parse_to_state_from(text, &acc) {
            Ok(parsed) => ast::merge_states(&acc, &parsed),
            Err(e) => {
                eprintln!("parse failed in {}: {}", name, e);
                std::process::exit(1);
            }
        }
    });

    // User nouns = full entity nouns minus metamodel entity nouns.
    let all_nouns = entity_nouns(&state);
    let user_nouns: Vec<&str> = all_nouns.iter()
        .filter(|n| !meta_nouns.contains(n.as_str()))
        .map(|s| s.as_str())
        .collect();

    print!("{}", solidity::compile_to_solidity_for_nouns(&state, &user_nouns));
}

/// Extract every entity noun name from a state. Value types and
/// abstract supertypes are skipped.
fn entity_nouns(state: &ast::Object) -> std::collections::HashSet<String> {
    let nouns = ast::fetch_or_phi("Noun", state);
    nouns.as_seq().map(|seq| seq.iter().filter_map(|n| {
        let name = ast::binding(n, "name")?.to_string();
        let obj_type = ast::binding(n, "objectType")?;
        (obj_type == "entity").then_some(name)
    }).collect()).unwrap_or_default()
}

fn collect_readings(dir: &PathBuf) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let entries = match fs::read_dir(dir) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("failed to read {}: {}", dir.display(), e);
            std::process::exit(1);
        }
    };
    let mut sorted: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
    sorted.sort();
    for path in sorted {
        if path.is_dir() {
            out.extend(collect_readings(&path));
        } else if path.extension().map_or(false, |e| e == "md") {
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            match fs::read_to_string(&path) {
                Ok(text) => out.push((name, text)),
                Err(e) => {
                    eprintln!("failed to read {}: {}", path.display(), e);
                    std::process::exit(1);
                }
            }
        }
    }
    out
}

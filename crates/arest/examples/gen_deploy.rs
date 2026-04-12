// Generate a Foundry deploy script from FORML 2 readings.
//
// Usage:
//   cargo run --example gen_deploy -- <readings_dir>
//
// Walks the user's entity nouns, imports each contract from the
// generated Solidity, and emits a Deploy.s.sol that deploys every
// one and logs its address. The generated script assumes the
// contracts live at ../src/Generated.sol relative to script/.

use std::env;
use std::fs;
use std::path::PathBuf;

use arest::parse_forml2;
use arest::ast;

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
        eprintln!("usage: gen_deploy <readings_dir>");
        std::process::exit(1);
    }
    let dir = PathBuf::from(&args[1]);

    let user_readings = collect_readings(&dir);
    if user_readings.is_empty() {
        eprintln!("no .md files found under {}", dir.display());
        std::process::exit(1);
    }

    parse_forml2::set_bootstrap_mode(true);
    let meta_state = METAMODEL_READINGS.iter().fold(ast::Object::phi(), |acc, (name, text)| {
        parse_forml2::parse_to_state_from(text, &acc)
            .map(|p| ast::merge_states(&acc, &p))
            .unwrap_or_else(|e| {
                eprintln!("metamodel parse failed at readings/{}.md: {}", name, e);
                std::process::exit(1);
            })
    });
    parse_forml2::set_bootstrap_mode(false);
    let meta_nouns = entity_nouns(&meta_state);

    let state = user_readings.iter().fold(meta_state, |acc, (name, text)| {
        parse_forml2::parse_to_state_from(text, &acc)
            .map(|p| ast::merge_states(&acc, &p))
            .unwrap_or_else(|e| {
                eprintln!("parse failed in {}: {}", name, e);
                std::process::exit(1);
            })
    });

    let all_nouns = entity_nouns(&state);
    let mut user_nouns: Vec<String> = all_nouns.iter()
        .filter(|n| !meta_nouns.contains(n.as_str()))
        .cloned()
        .collect();
    user_nouns.sort();

    // Contract names in Solidity are the sanitized entity noun names.
    let contract_names: Vec<(String, String)> = user_nouns.iter()
        .map(|noun| (noun.clone(), sanitize_name(noun)))
        .collect();

    println!("{}", emit_deploy_script(&contract_names));
}

fn emit_deploy_script(contracts: &[(String, String)]) -> String {
    let imports = contracts.iter()
        .map(|(_, name)| name.clone())
        .collect::<Vec<_>>()
        .join(", ");

    let deploys: Vec<String> = contracts.iter().map(|(_, name)| {
        format!(
"        {} {} = new {}();\n        console.log(\"{} deployed at:\", address({}));",
            name, lowercase_first(name), name, name, lowercase_first(name)
        )
    }).collect();

    let mut s = String::new();
    s.push_str("// SPDX-License-Identifier: MIT\n");
    s.push_str("// Generated from FORML2 readings by AREST\n");
    s.push_str("pragma solidity ^0.8.20;\n\n");
    s.push_str("import {Script, console} from \"forge-std/Script.sol\";\n");
    s.push_str(&format!("import {{{}}} from \"../src/Generated.sol\";\n\n", imports));
    s.push_str("/// Deploy every user contract declared in readings/.\n");
    s.push_str("/// Invoke: forge script script/Deploy.s.sol --rpc-url $RPC --private-key $KEY --broadcast\n");
    s.push_str("contract Deploy is Script {\n");
    s.push_str("    function run() external {\n");
    s.push_str("        vm.startBroadcast();\n");
    s.push_str(&deploys.join("\n"));
    s.push_str("\n        vm.stopBroadcast();\n");
    s.push_str("    }\n");
    s.push_str("}\n");
    s
}

fn entity_nouns(state: &ast::Object) -> std::collections::HashSet<String> {
    let nouns = ast::fetch_or_phi("Noun", state);
    nouns.as_seq().map(|seq| seq.iter().filter_map(|n| {
        let name = ast::binding(n, "name")?.to_string();
        let obj_type = ast::binding(n, "objectType")?;
        (obj_type == "entity").then_some(name)
    }).collect()).unwrap_or_default()
}

fn sanitize_name(name: &str) -> String {
    name.chars().fold((String::new(), true), |(mut acc, cap), c| {
        match c {
            ' ' | '_' | '-' => (acc, true),
            c if c.is_alphanumeric() => {
                acc.push(if cap { c.to_ascii_uppercase() } else { c });
                (acc, false)
            }
            _ => (acc, cap),
        }
    }).0
}

fn lowercase_first(name: &str) -> String {
    name.char_indices().map(|(i, c)| {
        if i == 0 { c.to_ascii_lowercase() } else { c }
    }).collect()
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
            let text = fs::read_to_string(&path)
                .unwrap_or_else(|e| { eprintln!("read {}: {}", path.display(), e); std::process::exit(1); });
            out.push((name, text));
        }
    }
    out
}

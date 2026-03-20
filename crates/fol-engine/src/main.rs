// CLI for the FOL engine.
//
// Usage:
//   fol --ir <ir.json> --response <response.json>
//   fol --ir <ir.json> --text "response text to check"
//   fol --ir <ir.json> --synthesize <noun_name> [--depth <n>]
//   fol --ir <ir.json> --forward-chain --population <population.json>
//   echo '{"text":"..."}' | fol --ir <ir.json>

mod types;
mod compile;
mod evaluate;

use types::{ConstraintIR, ResponseContext, Population};
use compile::EvalContext;

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
                eprintln!("fol — evaluate text against constraint IR");
                eprintln!();
                eprintln!("Usage:");
                eprintln!("  fol --ir <ir.json> --text \"response text\"");
                eprintln!("  fol --ir <ir.json> --response <response.json>");
                eprintln!("  fol --ir <ir.json> --synthesize <noun> [--depth <n>]");
                eprintln!("  fol --ir <ir.json> --forward-chain --population <pop.json>");
                eprintln!("  echo '{{\"text\":\"...\"}}' | fol --ir <ir.json>");
                eprintln!();
                eprintln!("Options:");
                eprintln!("  --ir <path>            Constraint IR JSON file (required)");
                eprintln!("  --text <string>        Text to evaluate");
                eprintln!("  --response <path>      Response JSON file ({{\"text\":\"...\"}})");
                eprintln!("  --population <path>    Population JSON file (optional)");
                eprintln!("  --synthesize <noun>    Synthesize all knowledge about a noun");
                eprintln!("  --depth <n>            Synthesis depth for related nouns (default: 1)");
                eprintln!("  --forward-chain        Run forward inference on population");
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
        None => { eprintln!("--ir is required"); std::process::exit(1); }
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
        let mut population: Population = match population_path {
            Some(p) => {
                let json = std::fs::read_to_string(&p)
                    .unwrap_or_else(|e| { eprintln!("Failed to read population file: {}", e); std::process::exit(1); });
                serde_json::from_str(&json)
                    .unwrap_or_else(|e| { eprintln!("Failed to parse population: {}", e); std::process::exit(1); })
            }
            None => {
                eprintln!("--forward-chain requires --population <path>");
                std::process::exit(1);
            }
        };

        let response = ResponseContext {
            text: String::new(),
            sender_identity: None,
            fields: None,
        };

        let derived = evaluate::forward_chain(&model, &response, &mut population);

        if derived.is_empty() {
            println!("No new facts derived");
        } else {
            println!("{}", serde_json::to_string_pretty(&derived).unwrap());
        }
        std::process::exit(0);
    }

    // ── Evaluate mode (default) ──────────────────────────────────────
    // Build response context
    let response: ResponseContext = if let Some(t) = text {
        ResponseContext {
            text: t,
            sender_identity: None,
            fields: None,
        }
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
            ResponseContext {
                text: input.trim().to_string(),
                sender_identity: None,
                fields: None,
            }
        }
    };

    // Load population (optional)
    let population: Population = match population_path {
        Some(p) => {
            let json = std::fs::read_to_string(&p)
                .unwrap_or_else(|e| { eprintln!("Failed to read population file: {}", e); std::process::exit(1); });
            serde_json::from_str(&json)
                .unwrap_or_else(|e| { eprintln!("Failed to parse population: {}", e); std::process::exit(1); })
        }
        None => Population { facts: std::collections::HashMap::new() },
    };

    let ctx = EvalContext {
        response: &response,
        population: &population,
    };

    let violations = evaluate::evaluate(&model, &ctx);

    if violations.is_empty() {
        println!("OK — no violations");
        std::process::exit(0);
    } else {
        println!("{}", serde_json::to_string_pretty(&violations).unwrap());
        std::process::exit(1);
    }
}

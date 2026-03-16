// CLI for the constraint evaluator.
//
// Usage:
//   graphdl-rules --ir <ir.json> --response <response.json>
//   graphdl-rules --ir <ir.json> --text "response text to check"
//   echo '{"text":"..."}' | graphdl-rules --ir <ir.json>

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

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--ir" => { i += 1; ir_path = args.get(i).cloned(); }
            "--response" => { i += 1; response_path = args.get(i).cloned(); }
            "--text" => { i += 1; text = args.get(i).cloned(); }
            "--population" => { i += 1; population_path = args.get(i).cloned(); }
            "--help" | "-h" => {
                eprintln!("graphdl-rules — evaluate text against constraint IR");
                eprintln!();
                eprintln!("Usage:");
                eprintln!("  graphdl-rules --ir <ir.json> --text \"response text\"");
                eprintln!("  graphdl-rules --ir <ir.json> --response <response.json>");
                eprintln!("  echo '{{\"text\":\"...\"}}' | graphdl-rules --ir <ir.json>");
                eprintln!();
                eprintln!("Options:");
                eprintln!("  --ir <path>          Constraint IR JSON file (required)");
                eprintln!("  --text <string>      Text to evaluate");
                eprintln!("  --response <path>    Response JSON file ({{\"text\":\"...\"}})");
                eprintln!("  --population <path>  Population JSON file (optional)");
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

// CLI driver — requires std (env, fs, process).
// crates/arest/src/bin/check-cli.rs
//
// Standalone invoker for arest::check::check_readings. Reads every
// *.md file in each directory passed on the command line (including
// a sibling app.md of a readings/ subdir, matching arest-cli's
// convention), concatenates the text, and runs the three-layer
// readings checker (parse / resolve / deontic) against the merged
// corpus. Prints diagnostics to stdout; exits 1 if any ERROR
// diagnostics are reported.
//
// Usage:
//   check-cli <readings_dir> [<readings_dir> ...]

use arest::check::{check_readings, Level, Source};

fn main() {
    let dirs: Vec<String> = std::env::args().skip(1).collect();
    if dirs.is_empty() {
        eprintln!("Usage: check-cli <readings_dir> [<readings_dir> ...]");
        std::process::exit(2);
    }

    let mut text = String::new();
    for dir in &dirs {
        let path = std::path::Path::new(dir);
        if !path.is_dir() {
            eprintln!("Not a directory: {}", dir);
            std::process::exit(2);
        }
        if let Some(parent) = path.parent() {
            let app_md = parent.join("app.md");
            if app_md.exists() {
                text.push_str(&std::fs::read_to_string(&app_md).expect("read app.md"));
                text.push_str("\n\n");
            }
        }
        let mut entries: Vec<_> = std::fs::read_dir(path)
            .expect("readdir")
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("md"))
            .collect();
        entries.sort();
        for p in entries {
            text.push_str(&std::fs::read_to_string(&p).expect("read reading"));
            text.push_str("\n\n");
        }
    }

    let diags = check_readings(&text);
    let mut err_count = 0usize;
    let mut warn_count = 0usize;
    let mut hint_count = 0usize;
    for d in &diags {
        let lvl = match d.level {
            Level::Error => { err_count += 1; "ERROR" }
            Level::Warning => { warn_count += 1; "WARN" }
            Level::Hint => { hint_count += 1; "HINT" }
        };
        let src = match d.source {
            Source::Parse => "parse",
            Source::Resolve => "resolve",
            Source::Deontic => "deontic",
        };
        let reading_preview = if d.reading.is_empty() {
            "(no reading)".to_string()
        } else if d.reading.chars().count() > 140 {
            format!("{}...", d.reading.chars().take(140).collect::<String>())
        } else {
            d.reading.clone()
        };
        let suggestion = d.suggestion.as_deref()
            .map(|s| format!("\n    (suggestion: {s})"))
            .unwrap_or_default();
        println!("[{lvl} {src}] {reading_preview}: {}{suggestion}", d.message);
    }
    println!("\n{} diagnostics ({} errors, {} warnings, {} hints)",
        diags.len(), err_count, warn_count, hint_count);
    if err_count > 0 { std::process::exit(1); }
}

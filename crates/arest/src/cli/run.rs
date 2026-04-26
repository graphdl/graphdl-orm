// `arest run "App Name"` — CLI subcommand dispatcher (#543).
//
// Reads the bundled metamodel readings (`crate::metamodel_readings()`),
// folds them into a `crate::ast::Object` state, and resolves the
// user-supplied app name via `crate::command::wine_app_by_name`.
//
// On hit: prints the resolved `(slug, prefix Directory id)` pair to
// stdout, returns exit code 0.
// On miss: prints a "did you mean…?" suggestion based on Levenshtein
// distance over the slug + display-title set, returns exit code 1.
// On missing argument: prints usage, returns exit code 2.
//
// Read-only against state — no Wine prefix bootstrap, no winetricks
// invocation, no actual `wine` execve. Those land in #504 (Wine
// runtime layer); this module only resolves the name → (slug, prefix)
// mapping the runtime layer needs.
//
// The actual `wine_app_by_name` lookup lives in `command.rs` (#503,
// `637c333`). This module supplies the CLI layer: argv parsing, state
// loading, output formatting, and the near-name suggestion fallback.

use crate::ast;
use crate::command;

/// Top-level entry point used by `main.rs`. Takes the residual argv
/// (everything after `run`), the readings sources to fold into state,
/// and a writer for both stdout and stderr.
///
/// Returns the process exit code. Writers receive the formatted output
/// so unit tests can assert on the printed text without spawning a
/// child process.
pub fn dispatch<O: std::io::Write, E: std::io::Write>(
    args: &[String],
    readings: &[(&str, &str)],
    out: &mut O,
    err: &mut E,
) -> i32 {
    let name = match args.first() {
        Some(n) => n,
        None => {
            let _ = writeln!(err, "{}", usage_text());
            return 2;
        }
    };

    let state = build_state(readings);

    // Two-stage resolve. `wine_app_by_name` (command.rs #503) handles the
    // canonical lookup paths: exact slug match, plus the legacy
    // mis-bucketed `has display- Title 'X'` cell scan that the parser
    // emits when no canonical title FT is recognised. Under the full
    // bundled metamodel (compat-readings + os-readings + ui-readings +
    // templates + core all loaded), the parser DOES recognise the
    // `Wine App has display- Title.` FT and emits a clean
    // `Wine_App_has_display-_Title` cell instead — which the legacy
    // path never reads. The local `resolve_by_clean_title_cell` covers
    // that case so the CLI works in both partial-metamodel (the
    // command.rs unit-test fixture) and full-metamodel (the bundled
    // arest-cli runtime) state shapes.
    let resolved = command::wine_app_by_name(&state, name)
        .or_else(|| resolve_by_clean_title_cell(&state, name));

    match resolved {
        Some((slug, prefix_dir)) => {
            let _ = writeln!(out, "{}", format_resolution(&slug, &prefix_dir));
            0
        }
        None => {
            let suggestions = near_name_suggestions(&state, name, 3);
            let _ = writeln!(err, "{}", format_miss(name, &suggestions));
            1
        }
    }
}

/// Fallback resolver for the canonical `Wine_App_has_display-_Title`
/// cell. Pairs with `command::wine_app_by_name`'s legacy-path scan:
/// where that helper reads the parser's mis-bucketed
/// `has display- Title 'X'` cells, this one reads the clean cell the
/// parser emits when the `Wine App has display- Title.` fact-type
/// declaration is in scope (which it is under the full bundled
/// metamodel).
///
/// Returns `(slug, prefix Directory id)` on hit, `None` on miss.
/// Defers prefix lookup to `command::wine_prefix_for` so a slug
/// without a `Wine App has prefix Directory` binding still misses
/// (the runtime layer in #504 needs the prefix; surfacing a
/// title-only resolution would just push the failure downstream).
fn resolve_by_clean_title_cell(state: &ast::Object, name: &str) -> Option<(String, String)> {
    let cell = ast::fetch_or_phi("Wine_App_has_display-_Title", state);
    let seq = cell.as_seq()?;
    for fact in seq.iter() {
        let title = ast::binding(fact, "Title")?;
        if title == name {
            let slug = ast::binding(fact, "Wine App")?.to_string();
            let prefix = command::wine_prefix_for(state, &slug)?;
            return Some((slug, prefix));
        }
    }
    None
}

/// Plain-text usage string. Kept terse — full `--help` output lives
/// in main.rs's flag parser; this is the "you forgot the app name"
/// fallback.
pub fn usage_text() -> &'static str {
    "Usage: arest-cli run <app-name>\n\
     \n\
     Resolve a Wine App name (slug or display title) to its prefix\n\
     Directory id. Reads from the bundled compat-readings metamodel.\n\
     \n\
     Examples:\n\
       arest-cli run \"Notepad++\"           # display title\n\
       arest-cli run notepad-plus-plus     # slug\n\
     \n\
     Build with --features compat-readings to include the Wine App\n\
     catalogue."
}

/// Format the success line for a resolved Wine App. Single-line
/// output keyed by labels so downstream scripts can grep for
/// `slug=` / `prefix=` without parsing structure.
///
/// Exact shape:
/// ```text
/// slug=<slug> prefix=<prefix dir id>
/// ```
pub fn format_resolution(slug: &str, prefix_dir: &str) -> String {
    format!("slug={} prefix={}", slug, prefix_dir)
}

/// Format the miss line for an unresolved name plus optional
/// near-name suggestions. Always single-line on the first row;
/// suggestions follow as bullet rows.
pub fn format_miss(name: &str, suggestions: &[String]) -> String {
    if suggestions.is_empty() {
        return format!("No Wine App matches '{}'.", name);
    }
    let mut out = format!("No Wine App matches '{}'. Did you mean:", name);
    for s in suggestions {
        out.push_str("\n  ");
        out.push_str(s);
    }
    out
}

/// Build the metamodel state by folding every reading into an
/// `Object` (cells only — no def compilation needed for name lookup).
///
/// Bootstrap mode is enabled for the duration of the parse so the
/// metamodel's own self-reference (Noun is a Noun, …) doesn't trip
/// the strict-mode "undeclared noun" check the way a fresh user
/// corpus would.
pub fn build_state(readings: &[(&str, &str)]) -> ast::Object {
    crate::parse_forml2::set_bootstrap_mode(true);
    let state = readings.iter().fold(ast::Object::phi(), |acc, (name, text)| {
        match crate::parse_forml2::parse_to_state_from(text, &acc) {
            Ok(this) => ast::merge_states(&acc, &this),
            Err(e) => {
                eprintln!("[arest run] reading {} failed to parse: {}", name, e);
                acc
            }
        }
    });
    crate::parse_forml2::set_bootstrap_mode(false);
    state
}

/// Surface up to `limit` near-name suggestions for a missed lookup.
///
/// The candidate set is the union of every Wine App slug
/// (`wine_app_ids`) and every display title — sourced both from
/// `command::wine_app_display_title` (legacy mis-bucketed cells) and
/// from the canonical `Wine_App_has_display-_Title` cell the parser
/// emits when the FT declaration is in scope. Dedupes on label so
/// the same title doesn't appear twice when both paths populate.
///
/// Ranks by Levenshtein distance and returns the top `limit` entries
/// with distance ≤ a threshold proportional to the input length (so a
/// one-character typo on "Notepad++" still surfaces, but a wholly
/// unrelated string doesn't dump the whole catalogue).
pub fn near_name_suggestions(state: &ast::Object, name: &str, limit: usize) -> Vec<String> {
    let slugs = command::wine_app_ids(state);
    let mut candidates: Vec<String> = Vec::new();
    for slug in &slugs {
        candidates.push(slug.clone());
        if let Some(title) = command::wine_app_display_title(state, slug) {
            if !candidates.iter().any(|c| c == &title) {
                candidates.push(title);
            }
        }
    }
    // Also harvest titles from the clean `Wine_App_has_display-_Title`
    // cell — present under the full metamodel where the FT is parsed.
    let cell = ast::fetch_or_phi("Wine_App_has_display-_Title", state);
    if let Some(seq) = cell.as_seq() {
        for fact in seq.iter() {
            if let Some(title) = ast::binding(fact, "Title") {
                let t = title.to_string();
                if !candidates.iter().any(|c| c == &t) {
                    candidates.push(t);
                }
            }
        }
    }

    rank_near_names(name, &candidates, limit)
}

/// Distance threshold for near-name suggestion: ½ the longer of the
/// two strings, capped at 5. Prevents wholly-unrelated names from
/// surfacing the entire catalogue while still tolerating multi-char
/// typos in long display titles.
fn distance_threshold(a: &str, b: &str) -> usize {
    let longer = a.chars().count().max(b.chars().count());
    (longer / 2).min(5).max(1)
}

/// Rank `candidates` by Levenshtein distance to `query`, returning
/// the top `limit` whose distance is at-or-below the per-pair
/// threshold (`distance_threshold`).
///
/// Public for the unit tests; the production caller is
/// `near_name_suggestions`.
pub fn rank_near_names(query: &str, candidates: &[String], limit: usize) -> Vec<String> {
    let mut scored: Vec<(usize, &String)> = candidates.iter()
        .map(|c| (levenshtein(query, c), c))
        .filter(|(d, c)| *d <= distance_threshold(query, c))
        .collect();
    scored.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(b.1)));
    scored.into_iter().take(limit).map(|(_, c)| c.clone()).collect()
}

/// Levenshtein edit distance — classical O(|a| × |b|) DP, two-row
/// rolling buffer. Hand-rolled rather than pulling a `strsim`-style
/// dep for one CLI suggestion path.
///
/// Public for direct unit testing; production callers go through
/// `rank_near_names`.
pub fn levenshtein(a: &str, b: &str) -> usize {
    if a == b {
        return 0;
    }
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    if a_chars.is_empty() {
        return b_chars.len();
    }
    if b_chars.is_empty() {
        return a_chars.len();
    }

    let mut prev: Vec<usize> = (0..=b_chars.len()).collect();
    let mut curr: Vec<usize> = vec![0; b_chars.len() + 1];

    for (i, &ac) in a_chars.iter().enumerate() {
        curr[0] = i + 1;
        for (j, &bc) in b_chars.iter().enumerate() {
            let cost = if ac == bc { 0 } else { 1 };
            curr[j + 1] = (curr[j] + 1)              // insertion
                .min(prev[j + 1] + 1)                // deletion
                .min(prev[j] + cost);                // substitution
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[b_chars.len()]
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levenshtein_zero_for_equal_strings() {
        assert_eq!(levenshtein("hello", "hello"), 0);
        assert_eq!(levenshtein("", ""), 0);
    }

    #[test]
    fn levenshtein_counts_single_char_edits() {
        // One substitution: Notpad++ → Notepad++
        assert_eq!(levenshtein("Notpad++", "Notepad++"), 1);
        // One insertion: foo → foob
        assert_eq!(levenshtein("foo", "foob"), 1);
        // One deletion: foob → foo
        assert_eq!(levenshtein("foob", "foo"), 1);
    }

    #[test]
    fn levenshtein_handles_empty_inputs() {
        assert_eq!(levenshtein("", "abc"), 3);
        assert_eq!(levenshtein("abc", ""), 3);
    }

    #[test]
    fn levenshtein_unicode_safe() {
        // Unicode chars must count as one edit each (not byte-counted).
        // 'é' is 2 bytes in UTF-8 but one char.
        assert_eq!(levenshtein("café", "cafe"), 1);
    }

    #[test]
    fn rank_near_names_returns_closest_first() {
        let candidates: Vec<String> = vec![
            "notepad-plus-plus".into(),
            "Notepad++".into(),
            "vscode".into(),
            "spotify".into(),
        ];
        // "Notpad++" (typo on "Notepad++") — distance 1 to "Notepad++",
        // higher to others; "Notepad++" comes first.
        let out = rank_near_names("Notpad++", &candidates, 3);
        assert!(!out.is_empty(), "expected at least one suggestion");
        assert_eq!(out[0], "Notepad++");
    }

    #[test]
    fn rank_near_names_filters_unrelated() {
        let candidates: Vec<String> = vec![
            "notepad-plus-plus".into(),
            "Notepad++".into(),
        ];
        // Wholly unrelated query — distance >> threshold, no
        // suggestion surfaces.
        let out = rank_near_names("xxxxxxxx", &candidates, 3);
        assert!(out.is_empty(), "expected no suggestion for unrelated query, got {:?}", out);
    }

    #[test]
    fn rank_near_names_respects_limit() {
        // Five close-typo candidates; limit of 2 returns 2.
        let candidates: Vec<String> = vec![
            "abcd".into(), "abce".into(), "abcf".into(), "abcg".into(), "abch".into(),
        ];
        let out = rank_near_names("abcd", &candidates, 2);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn format_resolution_single_line() {
        let s = format_resolution("notepad-plus-plus", "notepad-plus-plus-prefix");
        assert_eq!(s, "slug=notepad-plus-plus prefix=notepad-plus-plus-prefix");
        assert!(!s.contains('\n'), "format_resolution must be single-line");
    }

    #[test]
    fn format_miss_no_suggestions() {
        let s = format_miss("nope", &[]);
        assert_eq!(s, "No Wine App matches 'nope'.");
    }

    #[test]
    fn format_miss_with_suggestions() {
        let s = format_miss("Notpad++", &vec!["Notepad++".to_string(), "notepad-plus-plus".to_string()]);
        assert!(s.starts_with("No Wine App matches 'Notpad++'. Did you mean:"));
        assert!(s.contains("\n  Notepad++"));
        assert!(s.contains("\n  notepad-plus-plus"));
    }

    #[test]
    fn dispatch_missing_arg_returns_usage() {
        let mut out: Vec<u8> = Vec::new();
        let mut err: Vec<u8> = Vec::new();
        let code = dispatch(&[], &[], &mut out, &mut err);
        assert_eq!(code, 2);
        let err_text = String::from_utf8(err).unwrap();
        assert!(err_text.contains("Usage: arest-cli run <app-name>"));
    }

    /// End-to-end test against the real bundled wine.md corpus.
    /// Gated on `compat-readings` because the slice is conditionally
    /// included via `crate::COMPAT_READINGS`.
    #[cfg(feature = "compat-readings")]
    #[test]
    fn dispatch_resolves_known_display_title() {
        let readings: Vec<(&str, &str)> = crate::metamodel_readings()
            .into_iter()
            .map(|(n, t)| (*n, *t))
            .collect();
        let mut out: Vec<u8> = Vec::new();
        let mut err: Vec<u8> = Vec::new();
        let code = dispatch(&["Notepad++".to_string()], &readings, &mut out, &mut err);
        assert_eq!(code, 0, "stderr was: {}", String::from_utf8_lossy(&err));
        let out_text = String::from_utf8(out).unwrap();
        assert!(out_text.contains("slug=notepad-plus-plus"), "got: {}", out_text);
        assert!(out_text.contains("prefix=notepad-plus-plus-prefix"), "got: {}", out_text);
    }

    #[cfg(feature = "compat-readings")]
    #[test]
    fn dispatch_resolves_known_slug() {
        let readings: Vec<(&str, &str)> = crate::metamodel_readings()
            .into_iter()
            .map(|(n, t)| (*n, *t))
            .collect();
        let mut out: Vec<u8> = Vec::new();
        let mut err: Vec<u8> = Vec::new();
        let code = dispatch(&["notepad-plus-plus".to_string()], &readings, &mut out, &mut err);
        assert_eq!(code, 0);
        let out_text = String::from_utf8(out).unwrap();
        assert!(out_text.contains("slug=notepad-plus-plus"));
    }

    #[cfg(feature = "compat-readings")]
    #[test]
    fn dispatch_typo_returns_suggestion() {
        let readings: Vec<(&str, &str)> = crate::metamodel_readings()
            .into_iter()
            .map(|(n, t)| (*n, *t))
            .collect();
        let mut out: Vec<u8> = Vec::new();
        let mut err: Vec<u8> = Vec::new();
        let code = dispatch(&["Notpad++".to_string()], &readings, &mut out, &mut err);
        assert_eq!(code, 1);
        let err_text = String::from_utf8(err).unwrap();
        assert!(err_text.contains("Did you mean:"), "got stderr: {}", err_text);
        assert!(err_text.contains("Notepad++"), "got stderr: {}", err_text);
    }

    #[cfg(feature = "compat-readings")]
    #[test]
    fn dispatch_unrelated_name_returns_no_suggestion() {
        let readings: Vec<(&str, &str)> = crate::metamodel_readings()
            .into_iter()
            .map(|(n, t)| (*n, *t))
            .collect();
        let mut out: Vec<u8> = Vec::new();
        let mut err: Vec<u8> = Vec::new();
        let code = dispatch(&["xxxxxxxxxxxx".to_string()], &readings, &mut out, &mut err);
        assert_eq!(code, 1);
        let err_text = String::from_utf8(err).unwrap();
        assert!(err_text.contains("No Wine App matches 'xxxxxxxxxxxx'."));
        // No suggestion line.
        assert!(!err_text.contains("Did you mean"), "expected no suggestion, got: {}", err_text);
    }
}

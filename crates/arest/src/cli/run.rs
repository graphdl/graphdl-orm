// `arest run "App Name"` — CLI subcommand dispatcher (#543, #504).
//
// Reads the bundled metamodel readings (`crate::metamodel_readings()`),
// folds them into a `crate::ast::Object` state, and resolves the
// user-supplied app name via `crate::command::wine_app_by_name`.
//
// On hit: prints the resolved `(slug, prefix Directory id)` pair,
// then invokes `crate::cli::wine_bootstrap::bootstrap_prefix` to
// apply winetricks recipes / DLL overrides / registry keys. Returns
// exit code 0 if every fact-driven mutation succeeded; 3 if at least
// one winetricks recipe failed.
// On miss: prints a "did you mean…?" suggestion based on Levenshtein
// distance over the slug + display-title set, returns exit code 1.
// On missing argument: prints usage, returns exit code 2.
//
// The bootstrap step writes into `<root>/<prefix-dir-id>/` where
// `<root>` is the AREST data root (defaults to a per-process scratch
// dir under `std::env::temp_dir()` — production wiring to
// `/var/wine/` lands when the runtime layer (#506) launches the app).
// The current dispatcher prints the prefix path it bootstrapped so
// downstream callers can locate the artefacts.
//
// The actual `wine_app_by_name` lookup lives in `command.rs` (#503,
// `637c333`). This module supplies the CLI layer: argv parsing, state
// loading, output formatting, the near-name suggestion fallback, and
// the bootstrap dispatch. Actual installer-binary execution and the
// per-app launch wrapper land in #505 / #506.

use crate::ast;
use crate::cli::wine_bootstrap;
use crate::command;

/// Top-level entry point used by `main.rs`. Takes the residual argv
/// (everything after `run`), the readings sources to fold into state,
/// and a writer for both stdout and stderr. Reads the prefix root
/// from the `AREST_WINE_PREFIX_ROOT` env var (default:
/// `<temp>/arest/wine-prefixes/`).
///
/// Returns the process exit code. Writers receive the formatted output
/// so unit tests can assert on the printed text without spawning a
/// child process.
///
/// Exit codes:
///   * 0 — resolved + bootstrap succeeded (or no facts to apply).
///   * 1 — name not found in the readings.
///   * 2 — usage error (no app name supplied).
///   * 3 — bootstrap surfaced one or more recipe failures.
pub fn dispatch<O: std::io::Write, E: std::io::Write>(
    args: &[String],
    readings: &[(&str, &str)],
    out: &mut O,
    err: &mut E,
) -> i32 {
    let prefix_root = wine_prefix_root_from_env();
    dispatch_with_prefix_root(args, readings, &prefix_root, out, err)
}

/// Variant of `dispatch` that takes an explicit `prefix_root` instead
/// of reading the `AREST_WINE_PREFIX_ROOT` env var. Used by unit
/// tests so they can isolate per-test scratch directories without
/// stomping on the process-global env (which is racy across threads).
/// The production main.rs path goes through `dispatch`.
pub fn dispatch_with_prefix_root<O: std::io::Write, E: std::io::Write>(
    args: &[String],
    readings: &[(&str, &str)],
    prefix_root: &std::path::Path,
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
        Some((slug, prefix_dir_id)) => {
            let _ = writeln!(out, "{}", format_resolution(&slug, &prefix_dir_id));

            // #504: walk the FORML compat facts and apply them to the
            // physical prefix on disk. The prefix dir lives under
            // `<prefix_root>/<prefix_dir_id>/` — the kernel-side
            // mount under `/var/wine/` arrives with #506 (launch +
            // monitor). Print the absolute path so downstream tooling
            // (and the user) can locate it.
            let prefix_path = prefix_root.join(&prefix_dir_id);
            let raw_wine_md = compat_wine_md_text(readings);
            let bootstrap = match wine_bootstrap::bootstrap_prefix(
                &state, raw_wine_md, &slug, &prefix_path, None,
            ) {
                Ok(r) => r,
                Err(e) => {
                    let _ = writeln!(err, "Bootstrap failed: {}", e);
                    return 3;
                }
            };
            let _ = writeln!(out, "  prefix path: {}", prefix_path.display());
            let _ = write!(out, "{}", wine_bootstrap::format_report(&bootstrap, &slug));
            if !bootstrap.all_succeeded() { 3 } else { 0 }
        }
        None => {
            let suggestions = near_name_suggestions(&state, name, 3);
            let _ = writeln!(err, "{}", format_miss(name, &suggestions));
            1
        }
    }
}

/// Per-host root under which Wine prefixes are materialised. Defaults
/// to `<temp>/arest/wine-prefixes/` so unit tests + a fresh user
/// install never accidentally touch `/var/wine`. The kernel layer
/// (#506) overrides this via the `AREST_WINE_PREFIX_ROOT` env var.
pub fn wine_prefix_root_from_env() -> std::path::PathBuf {
    if let Some(root) = std::env::var_os("AREST_WINE_PREFIX_ROOT") {
        return std::path::PathBuf::from(root);
    }
    std::env::temp_dir().join("arest").join("wine-prefixes")
}

/// Locate the raw `wine.md` text inside the loaded readings, falling
/// back to `""` if the slice isn't present. The bootstrap module needs
/// the raw text to recover the third role of the ternary `requires
/// DLL Override / requires Registry Key` facts (the parser drops the
/// `with X 'Y'` tail; see comments in `cli::wine_overrides`).
pub fn compat_wine_md_text<'a>(readings: &'a [(&str, &str)]) -> &'a str {
    readings.iter()
        .find(|(name, _)| *name == "wine")
        .map(|(_, text)| *text)
        .unwrap_or("")
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
    /// included via `crate::COMPAT_READINGS`. Uses
    /// `dispatch_with_prefix_root` so each test owns a distinct
    /// scratch directory without manipulating the process-global
    /// `AREST_WINE_PREFIX_ROOT` env var (which would race across
    /// libtest's parallel test threads).
    #[cfg(feature = "compat-readings")]
    #[test]
    fn dispatch_resolves_known_display_title() {
        let readings: Vec<(&str, &str)> = crate::metamodel_readings()
            .into_iter()
            .map(|(n, t)| (*n, *t))
            .collect();
        let root = test_prefix_root("dispatch-display-title");
        let mut out: Vec<u8> = Vec::new();
        let mut err: Vec<u8> = Vec::new();
        let code = dispatch_with_prefix_root(
            &["Notepad++".to_string()], &readings, &root, &mut out, &mut err,
        );
        assert_eq!(code, 0, "stderr was: {}", String::from_utf8_lossy(&err));
        let out_text = String::from_utf8(out).unwrap();
        assert!(out_text.contains("slug=notepad-plus-plus"), "got: {}", out_text);
        assert!(out_text.contains("prefix=notepad-plus-plus-prefix"), "got: {}", out_text);
        // Bootstrap progress block is also printed.
        assert!(out_text.contains("Bootstrapping Wine prefix"), "got: {}", out_text);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(feature = "compat-readings")]
    #[test]
    fn dispatch_resolves_known_slug() {
        let readings: Vec<(&str, &str)> = crate::metamodel_readings()
            .into_iter()
            .map(|(n, t)| (*n, *t))
            .collect();
        let root = test_prefix_root("dispatch-slug");
        let mut out: Vec<u8> = Vec::new();
        let mut err: Vec<u8> = Vec::new();
        let code = dispatch_with_prefix_root(
            &["notepad-plus-plus".to_string()], &readings, &root, &mut out, &mut err,
        );
        assert_eq!(code, 0);
        let out_text = String::from_utf8(out).unwrap();
        assert!(out_text.contains("slug=notepad-plus-plus"));
        assert!(out_text.contains("Bootstrapping Wine prefix"));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Bootstrap a non-trivial app (Steam Windows: Required Components +
    /// DLL Overrides) and verify the resulting `system.reg` contains
    /// the expected entries. Confirms the run-dispatch → bootstrap →
    /// wine_overrides chain end-to-end.
    #[cfg(feature = "compat-readings")]
    #[test]
    fn dispatch_bootstraps_steam_windows_prefix() {
        let readings: Vec<(&str, &str)> = crate::metamodel_readings()
            .into_iter()
            .map(|(n, t)| (*n, *t))
            .collect();
        let root = test_prefix_root("dispatch-steam-bootstrap");
        let mut out: Vec<u8> = Vec::new();
        let mut err: Vec<u8> = Vec::new();
        let code = dispatch_with_prefix_root(
            &["steam-windows".to_string()], &readings, &root, &mut out, &mut err,
        );
        assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&err));
        let out_text = String::from_utf8(out).unwrap();
        assert!(out_text.contains("DLL overrides: 3 written"), "got: {}", out_text);
        let reg_path = root.join("steam-windows-prefix").join("system.reg");
        let body = std::fs::read_to_string(&reg_path)
            .unwrap_or_else(|e| panic!("system.reg at {:?} must exist: {}", reg_path, e));
        assert!(body.contains("\"dwrite\"=\"\""), "got: {}", body);
        assert!(body.contains("\"msxml3\"=\"native\""), "got: {}", body);
        assert!(body.contains("\"msxml6\"=\"native\""), "got: {}", body);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Spotify writes an HKCU registry key. Confirms registry keys
    /// at HKCU land in `user.reg`, not `system.reg`.
    #[cfg(feature = "compat-readings")]
    #[test]
    fn dispatch_bootstraps_spotify_registry_key() {
        let readings: Vec<(&str, &str)> = crate::metamodel_readings()
            .into_iter()
            .map(|(n, t)| (*n, *t))
            .collect();
        let root = test_prefix_root("dispatch-spotify-registry");
        let mut out: Vec<u8> = Vec::new();
        let mut err: Vec<u8> = Vec::new();
        let code = dispatch_with_prefix_root(
            &["spotify".to_string()], &readings, &root, &mut out, &mut err,
        );
        assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&err));
        let user_reg = std::fs::read_to_string(
            root.join("spotify-prefix").join("user.reg")
        ).expect("user.reg must exist");
        assert!(user_reg.contains("[HKCU\\\\Software\\\\Spotify\\\\CrashReporter]"),
                "got user.reg: {}", user_reg);
        assert!(user_reg.contains("@=\"disabled\""));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Re-running `arest run` against the same app must produce a
    /// byte-identical prefix state — the idempotency invariant the
    /// bootstrap module promises.
    #[cfg(feature = "compat-readings")]
    #[test]
    fn dispatch_idempotent_bootstrap() {
        let readings: Vec<(&str, &str)> = crate::metamodel_readings()
            .into_iter()
            .map(|(n, t)| (*n, *t))
            .collect();
        let root = test_prefix_root("dispatch-idempotent");
        let mut out1: Vec<u8> = Vec::new();
        let mut err1: Vec<u8> = Vec::new();
        let _ = dispatch_with_prefix_root(
            &["office-2016-word".to_string()], &readings, &root, &mut out1, &mut err1,
        );
        let body1 = std::fs::read_to_string(
            root.join("office-2016-word-prefix").join("system.reg")
        ).unwrap();
        let mut out2: Vec<u8> = Vec::new();
        let mut err2: Vec<u8> = Vec::new();
        let _ = dispatch_with_prefix_root(
            &["office-2016-word".to_string()], &readings, &root, &mut out2, &mut err2,
        );
        let body2 = std::fs::read_to_string(
            root.join("office-2016-word-prefix").join("system.reg")
        ).unwrap();
        assert_eq!(body1, body2, "second run must produce byte-identical system.reg");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// `compat_wine_md_text` returns the wine.md slice when present,
    /// empty string otherwise. Smoke tests for the helper.
    #[test]
    fn compat_wine_md_text_returns_slice_when_present() {
        let readings: &[(&str, &str)] = &[
            ("core", "core body"),
            ("wine", "wine body"),
            ("ui",   "ui body"),
        ];
        assert_eq!(compat_wine_md_text(readings), "wine body");
    }

    #[test]
    fn compat_wine_md_text_returns_empty_when_absent() {
        let readings: &[(&str, &str)] = &[("core", "core body")];
        assert_eq!(compat_wine_md_text(readings), "");
    }

    /// Carve out a fresh per-test scratch directory under the system
    /// tempdir. The label keeps directories distinct so the parallel
    /// test runner doesn't have two tests fighting over the same
    /// path. Each test should remove the dir on exit.
    fn test_prefix_root(label: &str) -> std::path::PathBuf {
        let pid = std::process::id();
        let path = std::env::temp_dir().join(format!("arest-run-test-{}-{}", pid, label));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("temp prefix root create");
        path
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

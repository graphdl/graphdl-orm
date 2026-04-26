// Wine prefix bootstrap orchestrator (#504).
//
// Walks the FORML facts in the compat readings (`readings/compat/wine.md`)
// for a given Wine App and applies them to a target prefix Directory:
//
//   1. `Wine App requires Required Component <C>` →
//      `winetricks --no-isolate --unattended <C>` via the
//      `winetricks` sibling module. Each recipe is idempotent through
//      winetricks.log inspection.
//
//   2. `Wine App requires DLL Override of DLL Name 'D' with DLL
//      Behavior 'B'` → registry entry under
//      `[Software\\Wine\\DllOverrides]` in `<prefix>/system.reg`,
//      written via the `wine_overrides` sibling module.
//
//   3. `Wine App requires Registry Key at Registry Path 'P' with
//      Registry Value 'V'` → `[<root>\\<sub>] @="<value>"` in
//      `<prefix>/user.reg` (HKCU) or `<prefix>/system.reg` (HKLM /
//      HKCR / HKU), also via `wine_overrides`.
//
// As of #553 every read above goes through the canonical per-FT
// cell (`Wine_App_requires_Required_Component`,
// `Wine_App_requires_dll_override_of_DLL_Name_with_DLL_Behavior`,
// `Wine_App_requires_registry_key_at_Registry_Path_with_Registry_Value`,
// `Required_Component_Anchor_has_recipe_of_Required_Component`,
// `Required_Component_Anchor_has_win64-_recipe_of_Required_Component`).
// The legacy raw-text recovery path is gone — the parser now
// preserves the third role of each ternary instance fact.
//
// **Architecture transitivity**: when the app's Prefix Architecture
// is `'win64'` and the Required Component anchor declares a
// `win64- Recipe`, the win64 variant is substituted automatically
// (so `vcrun2019` becomes `vcrun2019_x64` on a 64-bit prefix). This
// mirrors the FORML derivation rule in wine.md without requiring the
// derivation engine — simpler and avoids a full forward-chain pass at
// CLI time.
//
// Returns a `BootstrapReport` summarising what was applied so
// `cli::run` can print human-readable progress without the bootstrap
// module owning stdout.

use std::path::Path;

use crate::ast;
use crate::cli::wine_overrides;
use crate::cli::winetricks;

/// Aggregate summary of a single `bootstrap_prefix` invocation.
/// Returned to the caller (typically `cli::run::dispatch`) so it can
/// pretty-print progress + a final outcome line. All fields are
/// counters; the bootstrap module does not own stdout.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct BootstrapReport {
    /// Number of `Required Component` recipes the prefix asked for.
    pub recipes_total: usize,
    /// Recipes already in `winetricks.log` (no-op, no subprocess).
    pub recipes_already_applied: usize,
    /// Recipes freshly applied this invocation.
    pub recipes_applied: usize,
    /// Recipes skipped because winetricks isn't on $PATH.
    pub recipes_skipped_no_winetricks: usize,
    /// Number of DLL Override entries written to `system.reg`.
    pub dll_overrides_written: usize,
    /// Number of registry-key entries written.
    pub registry_keys_written: usize,
    /// Recipes that failed (non-zero exit, spawn error, …). Their
    /// names accompany the count so the caller can render
    /// per-recipe diagnostics.
    pub recipe_failures: Vec<String>,
}

impl BootstrapReport {
    /// True iff every fact-driven mutation succeeded (or was a no-op).
    /// `recipes_skipped_no_winetricks` is treated as success because
    /// the caller may legitimately run on a host without winetricks
    /// (registry / DLL writes still proceed).
    pub fn all_succeeded(&self) -> bool {
        self.recipe_failures.is_empty()
    }

    /// Total number of fact-driven mutations attempted, regardless of
    /// outcome. Useful for the "no-op bootstrap" detection in the
    /// caller (zero attempts → no progress to print).
    pub fn total_attempts(&self) -> usize {
        self.recipes_total
            + self.dll_overrides_written
            + self.registry_keys_written
    }
}

/// Walk the facts for `app_id` in `state` (parsed) and apply them to
/// the prefix at `prefix_dir`. Returns a `BootstrapReport`
/// summarising the work done.
///
/// `winetricks_path` is forwarded to `winetricks::apply_recipe`; pass
/// `None` to use the host's PATH-resolved binary.
///
/// `_wine_md_text` is unused as of #553 — every fact source now
/// resolves through the canonical per-FT cells. Kept as a parameter
/// so callers (`cli::run::dispatch`) don't need a coordinated change
/// in the same release; remove on the follow-up pass after the
/// signature settles in downstream consumers.
///
/// Idempotent: re-running with the same fact set produces the same
/// final state (no double-installs, no duplicated registry blocks).
pub fn bootstrap_prefix(
    state: &ast::Object,
    _wine_md_text: &str,
    app_id: &str,
    prefix_dir: &Path,
    winetricks_path: Option<&Path>,
) -> std::io::Result<BootstrapReport> {
    std::fs::create_dir_all(prefix_dir)?;

    let mut report = BootstrapReport::default();

    // 1. Required Components → winetricks recipes.
    let arch = wine_app_prefix_architecture(state, app_id);
    let recipes = required_components_for(state, app_id);
    let recipes = expand_arch_variants(state, &recipes, arch.as_deref());
    report.recipes_total = recipes.len();
    for recipe in &recipes {
        match winetricks::apply_recipe(prefix_dir, recipe, winetricks_path) {
            Ok(winetricks::RecipeOutcome::Applied) => {
                report.recipes_applied += 1;
            }
            Ok(winetricks::RecipeOutcome::AlreadyApplied) => {
                report.recipes_already_applied += 1;
            }
            Ok(winetricks::RecipeOutcome::WinetricksUnavailable) => {
                report.recipes_skipped_no_winetricks += 1;
            }
            Err(e) => {
                report.recipe_failures.push(format!("{}: {}", recipe, e));
            }
        }
    }

    // 2. DLL Overrides → system.reg.
    let dll_count = wine_overrides::apply_dll_overrides(state, app_id, prefix_dir)?;
    report.dll_overrides_written = dll_count;

    // 3. Registry Keys → user.reg / system.reg.
    let reg_count = wine_overrides::apply_registry_keys(state, app_id, prefix_dir)?;
    report.registry_keys_written = reg_count;

    Ok(report)
}

/// Returns the Required Component recipes declared for `app_id` in
/// `state`. Pulls from `Wine_App_requires_Required_Component`. Order
/// matches the cell's facts; deduplication is left to the caller
/// (winetricks itself is idempotent on the same recipe).
pub fn required_components_for(state: &ast::Object, app_id: &str) -> Vec<String> {
    let cell = ast::fetch_or_phi("Wine_App_requires_Required_Component", state);
    let mut out: Vec<String> = Vec::new();
    let Some(seq) = cell.as_seq() else { return out };
    for fact in seq.iter() {
        if ast::binding(fact, "Wine App") != Some(app_id) {
            continue;
        }
        if let Some(component) = ast::binding(fact, "Required Component") {
            out.push(component.to_string());
        }
    }
    out
}

/// Lookup the prefix architecture for `app_id` from
/// `Wine_App_has_Prefix_Architecture`. Returns `None` if the cell
/// has no fact for the app (which is a constraint violation in the
/// readings — every Wine App must have Prefix Architecture — but we
/// don't enforce it here; bootstrap proceeds with the win32 default).
pub fn wine_app_prefix_architecture(state: &ast::Object, app_id: &str) -> Option<String> {
    let cell = ast::fetch_or_phi("Wine_App_has_Prefix_Architecture", state);
    let seq = cell.as_seq()?;
    for fact in seq.iter() {
        if ast::binding(fact, "Wine App") == Some(app_id) {
            return ast::binding(fact, "Prefix Architecture").map(|s| s.to_string());
        }
    }
    None
}

/// For each recipe in `base`, substitute the `win64- Recipe` variant
/// declared on its `Required Component Anchor` if `arch == Some("win64")`.
/// Recipes without a declared `win64- Recipe` (or apps on win32) are
/// returned unchanged. Mirrors the wine.md "architecture transitivity"
/// derivation rule.
pub fn expand_arch_variants(
    state: &ast::Object,
    base: &[String],
    arch: Option<&str>,
) -> Vec<String> {
    if arch != Some("win64") {
        return base.to_vec();
    }
    base.iter().map(|recipe| {
        win64_variant_for(state, recipe).unwrap_or_else(|| recipe.clone())
    }).collect()
}

/// Lookup the `win64- Recipe` variant declared on the Required
/// Component Anchor whose `Recipe` is `recipe`. The recipe ↔ anchor
/// mapping is via the `has Recipe '<R>'` and `has win64- Recipe '<R64>'`
/// legacy-style cells (the parser hasn't yet promoted these to clean
/// per-FT cells; they live as flat string-keyed cells).
///
/// Returns `Some("vcrun2019_x64")` for `recipe = "vcrun2019"`,
/// `None` for recipes without an explicit win64 variant
/// (the caller falls back to the bare recipe name).
pub fn win64_variant_for(state: &ast::Object, recipe: &str) -> Option<String> {
    // Find the anchor whose `has Recipe '<R>'` cell contains `recipe`.
    // The anchor id is the `Required Component Anchor` binding on the
    // matching fact.
    let anchor_id = {
        let recipe_cell_name = format!("has Recipe '{}'", recipe);
        let cell = ast::fetch_or_phi(&recipe_cell_name, state);
        let seq = cell.as_seq()?;
        let fact = seq.iter().next()?;
        ast::binding(fact, "Required Component Anchor")?.to_string()
    };
    // Walk every `has win64- Recipe '<R64>'` cell looking for one
    // whose anchor binding matches.
    for (name, contents) in ast::cells_iter(state) {
        let Some(rest) = name.strip_prefix("has win64- Recipe '") else { continue };
        let Some(r64) = rest.strip_suffix('\'') else { continue };
        if let Some(seq) = contents.as_seq() {
            for fact in seq.iter() {
                if ast::binding(fact, "Required Component Anchor") == Some(&anchor_id) {
                    return Some(r64.to_string());
                }
            }
        }
    }
    None
}

/// Format `report` as a human-readable progress block for the CLI to
/// print. Multi-line; one section per fact category. Stable formatting
/// so downstream scripts can grep without parsing.
pub fn format_report(report: &BootstrapReport, app_id: &str) -> String {
    let mut out = String::new();
    out.push_str(&format!("Bootstrapping Wine prefix for '{}':\n", app_id));
    if report.recipes_total > 0 {
        out.push_str(&format!(
            "  winetricks recipes: {} requested ({} applied, {} already applied, {} skipped)\n",
            report.recipes_total,
            report.recipes_applied,
            report.recipes_already_applied,
            report.recipes_skipped_no_winetricks,
        ));
    } else {
        out.push_str("  winetricks recipes: 0 (no Required Components declared)\n");
    }
    if report.dll_overrides_written > 0 {
        out.push_str(&format!(
            "  DLL overrides: {} written to system.reg\n",
            report.dll_overrides_written,
        ));
    }
    if report.registry_keys_written > 0 {
        out.push_str(&format!(
            "  registry keys: {} written\n",
            report.registry_keys_written,
        ));
    }
    if !report.recipe_failures.is_empty() {
        out.push_str("  failures:\n");
        for f in &report.recipe_failures {
            out.push_str(&format!("    - {}\n", f));
        }
    }
    if report.total_attempts() == 0 {
        out.push_str("  (no fact-driven mutations needed; prefix is bootstrapped from FORML defaults only)\n");
    }
    out
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// Hand-build a minimal state with the cells the bootstrap walks.
    /// Keeps tests independent of the full readings parse.
    fn seeded_state() -> ast::Object {
        let mut s = ast::Object::phi();
        // Required Components.
        s = ast::cell_push(
            "Wine_App_requires_Required_Component",
            ast::fact_from_pairs(&[("Wine App", "office-2016-word"),
                                   ("Required Component", "corefonts")]),
            &s,
        );
        s = ast::cell_push(
            "Wine_App_requires_Required_Component",
            ast::fact_from_pairs(&[("Wine App", "office-2016-word"),
                                   ("Required Component", "gdiplus")]),
            &s,
        );
        s = ast::cell_push(
            "Wine_App_requires_Required_Component",
            ast::fact_from_pairs(&[("Wine App", "steam-windows"),
                                   ("Required Component", "vcrun2019")]),
            &s,
        );
        // Architectures.
        s = ast::cell_push(
            "Wine_App_has_Prefix_Architecture",
            ast::fact_from_pairs(&[("Wine App", "office-2016-word"),
                                   ("Prefix Architecture", "win64")]),
            &s,
        );
        s = ast::cell_push(
            "Wine_App_has_Prefix_Architecture",
            ast::fact_from_pairs(&[("Wine App", "steam-windows"),
                                   ("Prefix Architecture", "win64")]),
            &s,
        );
        s = ast::cell_push(
            "Wine_App_has_Prefix_Architecture",
            ast::fact_from_pairs(&[("Wine App", "notepad-plus-plus"),
                                   ("Prefix Architecture", "win32")]),
            &s,
        );
        // Anchor mapping for vcrun2019 ↔ vcrun2019_x64.
        s = ast::cell_push(
            "has Recipe 'vcrun2019'",
            ast::fact_from_pairs(&[("Required Component Anchor", "vcrun2019")]),
            &s,
        );
        s = ast::cell_push(
            "has win64- Recipe 'vcrun2019_x64'",
            ast::fact_from_pairs(&[("Required Component Anchor", "vcrun2019")]),
            &s,
        );
        // Anchor for corefonts (no win64 variant).
        s = ast::cell_push(
            "has Recipe 'corefonts'",
            ast::fact_from_pairs(&[("Required Component Anchor", "corefonts")]),
            &s,
        );
        s
    }

    #[test]
    fn required_components_for_returns_empty_for_unknown_app() {
        let state = seeded_state();
        assert!(required_components_for(&state, "no-such-app").is_empty());
    }

    #[test]
    fn required_components_for_returns_declared_recipes() {
        let state = seeded_state();
        let recipes = required_components_for(&state, "office-2016-word");
        assert_eq!(recipes, vec!["corefonts".to_string(), "gdiplus".to_string()]);
    }

    #[test]
    fn wine_app_prefix_architecture_returns_declared_arch() {
        let state = seeded_state();
        assert_eq!(
            wine_app_prefix_architecture(&state, "office-2016-word").as_deref(),
            Some("win64")
        );
        assert_eq!(
            wine_app_prefix_architecture(&state, "notepad-plus-plus").as_deref(),
            Some("win32")
        );
    }

    #[test]
    fn wine_app_prefix_architecture_returns_none_for_unknown_app() {
        let state = seeded_state();
        assert!(wine_app_prefix_architecture(&state, "nope").is_none());
    }

    #[test]
    fn win64_variant_for_returns_x64_recipe_when_declared() {
        let state = seeded_state();
        assert_eq!(win64_variant_for(&state, "vcrun2019").as_deref(), Some("vcrun2019_x64"));
    }

    #[test]
    fn win64_variant_for_returns_none_when_recipe_has_no_variant() {
        let state = seeded_state();
        assert!(win64_variant_for(&state, "corefonts").is_none());
    }

    #[test]
    fn expand_arch_variants_substitutes_win64_recipe() {
        let state = seeded_state();
        let base = vec!["vcrun2019".to_string(), "corefonts".to_string()];
        let out = expand_arch_variants(&state, &base, Some("win64"));
        assert_eq!(out, vec!["vcrun2019_x64".to_string(), "corefonts".to_string()]);
    }

    #[test]
    fn expand_arch_variants_passes_through_on_win32() {
        let state = seeded_state();
        let base = vec!["vcrun2019".to_string(), "corefonts".to_string()];
        let out = expand_arch_variants(&state, &base, Some("win32"));
        assert_eq!(out, base, "win32 must NOT substitute the _x64 variant");
    }

    #[test]
    fn expand_arch_variants_passes_through_when_arch_unknown() {
        let state = seeded_state();
        let base = vec!["vcrun2019".to_string()];
        let out = expand_arch_variants(&state, &base, None);
        assert_eq!(out, base);
    }

    #[test]
    fn bootstrap_report_default_is_all_zeros() {
        let r = BootstrapReport::default();
        assert!(r.all_succeeded());
        assert_eq!(r.total_attempts(), 0);
    }

    #[test]
    fn bootstrap_report_all_succeeded_false_when_failure_present() {
        let r = BootstrapReport {
            recipe_failures: vec!["dotnet48: spawn failed".to_string()],
            ..Default::default()
        };
        assert!(!r.all_succeeded());
    }

    #[test]
    fn format_report_prints_recipes_and_overrides() {
        let r = BootstrapReport {
            recipes_total: 2,
            recipes_applied: 1,
            recipes_already_applied: 1,
            dll_overrides_written: 3,
            ..Default::default()
        };
        let s = format_report(&r, "office-2016-word");
        assert!(s.contains("Bootstrapping Wine prefix for 'office-2016-word'"));
        assert!(s.contains("winetricks recipes: 2 requested"));
        assert!(s.contains("DLL overrides: 3 written"));
    }

    #[test]
    fn format_report_prints_zero_attempts_message() {
        let r = BootstrapReport::default();
        let s = format_report(&r, "notepad-plus-plus");
        assert!(s.contains("no fact-driven mutations needed"),
                "expected the no-op message; got: {}", s);
    }

    #[test]
    fn format_report_prints_failures() {
        let r = BootstrapReport {
            recipes_total: 1,
            recipe_failures: vec!["dotnet48: exit 2".to_string()],
            ..Default::default()
        };
        let s = format_report(&r, "test");
        assert!(s.contains("failures:"));
        assert!(s.contains("- dotnet48: exit 2"));
    }

    #[test]
    fn bootstrap_prefix_handles_app_with_no_facts() {
        // Notepad++ in the seeded state has only an Architecture decl,
        // no Required Components, no DLL Overrides, no Registry Keys.
        let state = seeded_state();
        let tmp = tempdir();
        let report = bootstrap_prefix(&state, "", "notepad-plus-plus", &tmp, None)
            .expect("bootstrap must succeed even with empty facts");
        assert_eq!(report.recipes_total, 0);
        assert_eq!(report.dll_overrides_written, 0);
        assert_eq!(report.registry_keys_written, 0);
        assert!(report.all_succeeded());
    }

    /// Push a `(Wine App, DLL Name, DLL Behavior)` ternary fact onto the
    /// canonical #553 cell. Helper for the bootstrap fixture tests
    /// since the raw-text recovery path is gone.
    fn push_dll_override(s: ast::Object, app: &str, dll: &str, behavior: &str) -> ast::Object {
        ast::cell_push(
            "Wine_App_requires_dll_override_of_DLL_Name_with_DLL_Behavior",
            ast::fact_from_pairs(&[
                ("Wine App",     app),
                ("DLL Name",     dll),
                ("DLL Behavior", behavior),
            ]),
            &s,
        )
    }

    /// Push a `(Wine App, Registry Path, Registry Value)` ternary onto
    /// the canonical #553 registry-key cell. Path string preserves
    /// the markdown-source double backslashes — `parse_registry_keys_from_state`
    /// collapses them on read.
    fn push_registry_key(s: ast::Object, app: &str, path: &str, value: &str) -> ast::Object {
        ast::cell_push(
            "Wine_App_requires_registry_key_at_Registry_Path_with_Registry_Value",
            ast::fact_from_pairs(&[
                ("Wine App",       app),
                ("Registry Path",  path),
                ("Registry Value", value),
            ]),
            &s,
        )
    }

    #[test]
    fn bootstrap_prefix_writes_dll_overrides_from_canonical_cell() {
        let state = push_dll_override(
            seeded_state(), "office-2016-word", "riched20.dll", "native");
        let tmp = tempdir();
        let report = bootstrap_prefix(&state, "", "office-2016-word", &tmp, None)
            .expect("bootstrap must succeed");
        assert_eq!(report.dll_overrides_written, 1);
        let body = std::fs::read_to_string(tmp.join("system.reg")).expect("system.reg written");
        assert!(body.contains("\"riched20\"=\"native\""));
    }

    #[test]
    fn bootstrap_prefix_writes_registry_keys_from_canonical_cell() {
        let state = push_registry_key(
            seeded_state(), "spotify",
            r"HKCU\\Software\\Spotify\\CrashReporter", "disabled");
        let tmp = tempdir();
        let report = bootstrap_prefix(&state, "", "spotify", &tmp, None)
            .expect("bootstrap must succeed");
        assert_eq!(report.registry_keys_written, 1);
        let body = std::fs::read_to_string(tmp.join("user.reg")).expect("user.reg written");
        assert!(body.contains("[HKCU\\\\Software\\\\Spotify\\\\CrashReporter]"));
        assert!(body.contains("@=\"disabled\""));
    }

    #[test]
    fn bootstrap_prefix_idempotent_against_filesystem() {
        let state = push_dll_override(
            seeded_state(), "office-2016-word", "riched20.dll", "native");
        let tmp = tempdir();
        bootstrap_prefix(&state, "", "office-2016-word", &tmp, None).expect("first run");
        let body1 = std::fs::read_to_string(tmp.join("system.reg")).unwrap();
        bootstrap_prefix(&state, "", "office-2016-word", &tmp, None).expect("second run");
        let body2 = std::fs::read_to_string(tmp.join("system.reg")).unwrap();
        assert_eq!(body1, body2, "second bootstrap must produce byte-identical state");
    }

    #[test]
    fn bootstrap_prefix_substitutes_win64_recipes() {
        // steam-windows is win64 + requires vcrun2019; the bootstrap
        // should resolve to vcrun2019_x64 before invoking winetricks.
        // We can't actually invoke winetricks in unit tests, so we
        // just confirm the resolved recipe list via expand_arch_variants
        // (the path bootstrap_prefix takes internally).
        let state = seeded_state();
        let recipes = required_components_for(&state, "steam-windows");
        let arch = wine_app_prefix_architecture(&state, "steam-windows");
        let expanded = expand_arch_variants(&state, &recipes, arch.as_deref());
        assert_eq!(expanded, vec!["vcrun2019_x64".to_string()]);
    }

    /// End-to-end with the full bundled wine.md corpus. Confirms that
    /// the bootstrap walks succeed against the real cell shapes the
    /// parser emits (not just our hand-built fixture). The test
    /// uses None as the winetricks_path so winetricks recipes are
    /// either short-circuited (logged) or marked WinetricksUnavailable
    /// — never actually run.
    #[cfg(feature = "compat-readings")]
    #[test]
    fn bootstrap_prefix_walks_real_wine_md_for_office() {
        let filesystem_md = include_str!("../../../../readings/os/filesystem.md");
        let wine_md = include_str!("../../../../readings/compat/wine.md");
        let fs_state = crate::parse_forml2::parse_to_state(filesystem_md)
            .expect("filesystem.md must parse cleanly");
        let state = crate::parse_forml2::parse_to_state_from(wine_md, &fs_state)
            .expect("wine.md must parse cleanly with filesystem.md preloaded");

        let tmp = tempdir();
        let report = bootstrap_prefix(&state, wine_md, "office-2016-word", &tmp, None)
            .expect("bootstrap against real readings must succeed");
        // Office 2016 Word: requires corefonts + gdiplus (both win64-
        // identical, no _x64 variant) and a riched20 DLL override.
        assert_eq!(report.recipes_total, 2,
            "office-2016-word declares 2 Required Components");
        assert_eq!(report.dll_overrides_written, 1,
            "office-2016-word declares 1 DLL Override (riched20.dll = native)");
        let body = std::fs::read_to_string(tmp.join("system.reg")).unwrap();
        assert!(body.contains("\"riched20\"=\"native\""),
                "system.reg must include the riched20 override; got: {}", body);
    }

    #[cfg(feature = "compat-readings")]
    #[test]
    fn bootstrap_prefix_walks_real_wine_md_for_steam_windows() {
        let filesystem_md = include_str!("../../../../readings/os/filesystem.md");
        let wine_md = include_str!("../../../../readings/compat/wine.md");
        let fs_state = crate::parse_forml2::parse_to_state(filesystem_md)
            .expect("filesystem.md must parse cleanly");
        let state = crate::parse_forml2::parse_to_state_from(wine_md, &fs_state)
            .expect("wine.md must parse cleanly with filesystem.md preloaded");

        let tmp = tempdir();
        let report = bootstrap_prefix(&state, wine_md, "steam-windows", &tmp, None)
            .expect("bootstrap against real readings must succeed");
        // steam-windows: 2 Required Components (vcrun2019, corefonts) +
        // 3 DLL Overrides (dwrite, msxml3, msxml6).
        assert_eq!(report.recipes_total, 2);
        assert_eq!(report.dll_overrides_written, 3);
        let body = std::fs::read_to_string(tmp.join("system.reg")).unwrap();
        assert!(body.contains("\"dwrite\"=\"\""),     "got: {}", body);
        assert!(body.contains("\"msxml3\"=\"native\""), "got: {}", body);
        assert!(body.contains("\"msxml6\"=\"native\""), "got: {}", body);
    }

    fn tempdir() -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let pid = std::process::id();
        let path = std::env::temp_dir().join(format!("arest-bootstrap-test-{}-{}", pid, n));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("tempdir create");
        path
    }
}

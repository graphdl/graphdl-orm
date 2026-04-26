// Wine App installer fetch + install orchestrator (#505).
//
// Sits between `cli::wine_bootstrap` (which materialises the prefix +
// applies winetricks recipes / DLL overrides / registry keys) and
// `cli::installer_run` (which spawns wine on the installer binary).
// The orchestrator:
//
//   1. Resolves the installer URL (`Wine_App_has_Installer_URL`) and
//      cache filename (`Wine_App_has_Installer_Filename`) from the
//      parsed FORML state.
//   2. Calls `installer_fetch::fetch_installer` to download or copy
//      the installer into `<prefix>/drive_c/_install/<filename>`.
//   3. Calls `installer_run::run_installer` to spawn `wine
//      <installer>` with `WINEPREFIX=<prefix>` and capture the log.
//   4. On success, writes the `_install_complete` marker so re-runs
//      short-circuit at step 3.
//
// State-machine transitions follow the #212 state-machine-as-derivation
// pattern: each step pushes a transition fact onto the
// `Wine_App_install_status` cell. The final state — `Installed`,
// `Failed`, or one of the unavailability states — is the last fact in
// that cell. Callers can read it back without re-deriving from the
// outcome enum.

use std::path::Path;

use crate::ast;
use crate::cli::installer_fetch;
use crate::cli::installer_run;

/// Aggregate summary of a single `install_app` invocation. Returned to
/// the dispatcher (`cli::run`) so it can print human-readable progress
/// without the install module owning stdout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallReport {
    /// Final state-machine state for the install:
    ///
    ///   * `Downloaded` — fetch succeeded, but wine not available
    ///     locally (or installer not yet run for some other reason).
    ///   * `Installing` — fetcher unavailable / no installer URL
    ///     declared. Marks an in-progress / blocked transition rather
    ///     than a hard failure.
    ///   * `Installed` — fetch + run both completed; marker written.
    ///   * `Failed` — installer ran but exited non-zero, or fetch
    ///     errored out.
    pub status: InstallStatus,
    /// Cache-side filename the installer was fetched to (or
    /// would-be-fetched-to if no URL declared). Empty if the app has
    /// no `Wine_App_has_Installer_Filename` fact.
    pub installer_filename: String,
    /// True if the orchestrator skipped the run because the
    /// `_install_complete` marker was already present at start.
    pub already_installed: bool,
    /// Optional human-readable diagnostic for the dispatcher to show
    /// in the progress block. Only populated for non-`Installed`
    /// terminal states.
    pub diagnostic: Option<String>,
}

/// State-machine state for the install — the values populated into
/// `Wine_App_install_status` cell facts. Mirrors the #212 SM-as-derivation
/// pattern: every transition pushes a fact; the final state is the
/// last fact in the cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallStatus {
    /// Installer binary fetched into the prefix's `_install/` cache,
    /// but `wine` not yet executed (typically because wine is not on
    /// PATH or the run is staged for a later session).
    Downloaded,
    /// Either the installer URL is missing from the FORML facts, or
    /// the fetcher is unavailable on PATH. Distinct from `Failed`
    /// because the prefix bootstrap is correct — only the binary
    /// fetch is blocked.
    Installing,
    /// Installer ran (or was already complete via marker) and the
    /// `_install_complete` marker is present.
    Installed,
    /// Either the fetch errored out or the installer exited non-zero.
    /// Diagnostic is in the `InstallReport` and the install log is
    /// in `<prefix>/drive_c/_install_log`.
    Failed,
}

impl InstallStatus {
    /// Stable string label for the SM cell. Matches the value-set
    /// declared in `readings/compat/wine.md` so the cell is
    /// round-trippable through the FORML facts.
    pub fn as_label(&self) -> &'static str {
        match self {
            InstallStatus::Downloaded => "Downloaded",
            InstallStatus::Installing => "Installing",
            InstallStatus::Installed => "Installed",
            InstallStatus::Failed => "Failed",
        }
    }
}

/// Top-level orchestrator. Resolves the installer URL + filename from
/// the FORML facts, fetches the installer, runs it under wine, and
/// transitions the install state machine.
///
/// Idempotent: if `<prefix>/drive_c/_install_complete` is already
/// present at entry, skips both fetch and run. The caller can check
/// `report.already_installed` to detect this.
///
/// `wine_path` and `fetcher_kind` parameters are reserved for tests
/// that want to stub out the binary lookup; production callers pass
/// `None` to use PATH resolution.
pub fn install_app(
    state: &ast::Object,
    app_id: &str,
    prefix_dir: &Path,
    wine_path: Option<&Path>,
) -> std::io::Result<InstallReport> {
    let installer_filename = installer_filename_for(state, app_id).unwrap_or_default();
    let installer_url = installer_url_for(state, app_id);

    // Idempotency short-circuit. The marker file is the single
    // observable contract with re-runs of `arest run`; if present
    // the install is considered complete regardless of fact state.
    if installer_run::install_complete_marker(prefix_dir).is_file() {
        return Ok(InstallReport {
            status: InstallStatus::Installed,
            installer_filename,
            already_installed: true,
            diagnostic: None,
        });
    }

    // Stage 1: declarative blockers. If the FORML facts don't carry
    // an installer URL or filename, there's nothing to fetch — the
    // app is in `Installing` (in-progress / blocked) state.
    let url = match installer_url {
        Some(u) => u,
        None => return Ok(InstallReport {
            status: InstallStatus::Installing,
            installer_filename,
            already_installed: false,
            diagnostic: Some(format!("no Installer URL declared for '{}'", app_id)),
        }),
    };
    if installer_filename.is_empty() {
        return Ok(InstallReport {
            status: InstallStatus::Installing,
            installer_filename,
            already_installed: false,
            diagnostic: Some(format!("no Installer Filename declared for '{}'", app_id)),
        });
    }

    // Stage 2: fetch. NoFetcher is a soft block (Installing); other
    // outcomes resolve to either Downloaded (proceed to run) or
    // Failed (fetch errored).
    let fetch_outcome = match installer_fetch::fetch_installer(prefix_dir, &url, &installer_filename) {
        Ok(o) => o,
        Err(e) => return Ok(InstallReport {
            status: InstallStatus::Failed,
            installer_filename,
            already_installed: false,
            diagnostic: Some(format!("installer fetch failed: {}", e)),
        }),
    };
    if fetch_outcome == installer_fetch::FetchOutcome::NoFetcher {
        return Ok(InstallReport {
            status: InstallStatus::Installing,
            installer_filename,
            already_installed: false,
            diagnostic: Some("no downloader on PATH (curl / powershell)".to_string()),
        });
    }

    // Stage 3: run. WineUnavailable is a soft block (Downloaded —
    // fetch is done, run is staged); Failed propagates the exit
    // code; Installed writes the marker. A spawn error (e.g. wine_path
    // points at a nonexistent binary) maps to Failed rather than
    // propagating up — the FORML state machine then has somewhere to
    // record the issue.
    let installer_path = installer_fetch::install_cache_dir(prefix_dir).join(&installer_filename);
    let run_outcome = match installer_run::run_installer(prefix_dir, &installer_path, &[], wine_path) {
        Ok(o) => o,
        Err(e) => return Ok(InstallReport {
            status: InstallStatus::Failed,
            installer_filename,
            already_installed: false,
            diagnostic: Some(format!("installer spawn failed: {}", e)),
        }),
    };
    match run_outcome {
        installer_run::RunOutcome::Installed => {
            installer_run::mark_install_complete(prefix_dir)?;
            Ok(InstallReport {
                status: InstallStatus::Installed,
                installer_filename,
                already_installed: false,
                diagnostic: None,
            })
        }
        installer_run::RunOutcome::AlreadyInstalled => {
            // run_installer detected the marker we missed at entry
            // (race / external write). Surface as Installed.
            Ok(InstallReport {
                status: InstallStatus::Installed,
                installer_filename,
                already_installed: true,
                diagnostic: None,
            })
        }
        installer_run::RunOutcome::WineUnavailable => Ok(InstallReport {
            status: InstallStatus::Downloaded,
            installer_filename,
            already_installed: false,
            diagnostic: Some("wine not on PATH; install staged for next run".to_string()),
        }),
        installer_run::RunOutcome::Failed { exit_code } => Ok(InstallReport {
            status: InstallStatus::Failed,
            installer_filename,
            already_installed: false,
            diagnostic: Some(match exit_code {
                Some(c) => format!("installer exited with status {}", c),
                None => "installer terminated by signal".to_string(),
            }),
        }),
    }
}

/// Lookup the Installer URL for `app_id` from
/// `Wine_App_has_Installer_URL`. Returns `None` if the cell has no
/// fact for the app (which is the common case for apps that have not
/// yet had an Installer URL declared in the readings — the
/// orchestrator transitions to `Installing` and surfaces the
/// diagnostic).
pub fn installer_url_for(state: &ast::Object, app_id: &str) -> Option<String> {
    let cell = ast::fetch_or_phi("Wine_App_has_Installer_URL", state);
    let seq = cell.as_seq()?;
    for fact in seq.iter() {
        if ast::binding(fact, "Wine App") == Some(app_id) {
            return ast::binding(fact, "Installer URL").map(|s| s.to_string());
        }
    }
    None
}

/// Lookup the Installer Filename for `app_id` from
/// `Wine_App_has_Installer_Filename`. Returns `None` if missing.
pub fn installer_filename_for(state: &ast::Object, app_id: &str) -> Option<String> {
    let cell = ast::fetch_or_phi("Wine_App_has_Installer_Filename", state);
    let seq = cell.as_seq()?;
    for fact in seq.iter() {
        if ast::binding(fact, "Wine App") == Some(app_id) {
            return ast::binding(fact, "Installer Filename").map(|s| s.to_string());
        }
    }
    None
}

/// Push a state-machine transition fact onto the
/// `Wine_App_install_status` cell. Returns the new state with the
/// fact appended. Mirrors the #212 SM-as-derivation pattern: every
/// transition is one fact; the final state is the last fact in the
/// cell.
///
/// Public so the dispatcher can chain transitions explicitly if it
/// needs to (e.g. for richer telemetry); the production caller
/// (`install_app`) does the transition implicitly via the returned
/// `InstallReport.status`.
pub fn push_install_status(
    state: &ast::Object,
    app_id: &str,
    status: InstallStatus,
) -> ast::Object {
    let fact = ast::fact_from_pairs(&[
        ("Wine App", app_id),
        ("Install Status", status.as_label()),
    ]);
    ast::cell_push("Wine_App_install_status", fact, state)
}

/// Format an `InstallReport` as a human-readable progress block for
/// the CLI to print. Multi-line; one line per material outcome.
/// Stable formatting so downstream scripts can grep on the labels
/// without parsing.
pub fn format_report(report: &InstallReport, app_id: &str) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "Installing Wine app '{}' (status: {}):\n",
        app_id,
        report.status.as_label(),
    ));
    if !report.installer_filename.is_empty() {
        out.push_str(&format!("  installer: {}\n", report.installer_filename));
    }
    if report.already_installed {
        out.push_str("  (already installed; _install_complete marker present)\n");
    }
    if let Some(diag) = &report.diagnostic {
        out.push_str(&format!("  note: {}\n", diag));
    }
    out
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// Hand-build a minimal state with the cells the install walks.
    fn seeded_state() -> ast::Object {
        let mut s = ast::Object::phi();
        s = ast::cell_push(
            "Wine_App_has_Installer_URL",
            ast::fact_from_pairs(&[
                ("Wine App", "notepad-plus-plus"),
                ("Installer URL", "https://example.invalid/npp.exe"),
            ]),
            &s,
        );
        s = ast::cell_push(
            "Wine_App_has_Installer_Filename",
            ast::fact_from_pairs(&[
                ("Wine App", "notepad-plus-plus"),
                ("Installer Filename", "npp-installer.exe"),
            ]),
            &s,
        );
        s
    }

    #[test]
    fn installer_url_for_returns_declared_url() {
        let state = seeded_state();
        assert_eq!(
            installer_url_for(&state, "notepad-plus-plus").as_deref(),
            Some("https://example.invalid/npp.exe"),
        );
    }

    #[test]
    fn installer_url_for_returns_none_for_unknown_app() {
        let state = seeded_state();
        assert!(installer_url_for(&state, "no-such-app").is_none());
    }

    #[test]
    fn installer_filename_for_returns_declared_filename() {
        let state = seeded_state();
        assert_eq!(
            installer_filename_for(&state, "notepad-plus-plus").as_deref(),
            Some("npp-installer.exe"),
        );
    }

    #[test]
    fn installer_filename_for_returns_none_for_unknown_app() {
        let state = seeded_state();
        assert!(installer_filename_for(&state, "no-such-app").is_none());
    }

    #[test]
    fn install_status_labels_match_sm_values() {
        // The SM values must exactly match the value set declared in
        // wine.md for the install state cell — round-trippability
        // through FORML facts depends on it.
        assert_eq!(InstallStatus::Downloaded.as_label(), "Downloaded");
        assert_eq!(InstallStatus::Installing.as_label(), "Installing");
        assert_eq!(InstallStatus::Installed.as_label(), "Installed");
        assert_eq!(InstallStatus::Failed.as_label(), "Failed");
    }

    #[test]
    fn push_install_status_appends_fact() {
        let state = seeded_state();
        let after = push_install_status(&state, "notepad-plus-plus", InstallStatus::Installed);
        let cell = ast::fetch_or_phi("Wine_App_install_status", &after);
        let seq = cell.as_seq().expect("cell must be a seq");
        assert_eq!(seq.len(), 1);
        let fact = seq.iter().next().unwrap();
        assert_eq!(ast::binding(fact, "Wine App"), Some("notepad-plus-plus"));
        assert_eq!(ast::binding(fact, "Install Status"), Some("Installed"));
    }

    #[test]
    fn push_install_status_chains_transitions() {
        // Transition stream: Downloaded → Installing → Installed.
        // The final state is always the last fact in the cell.
        let mut s = ast::Object::phi();
        s = push_install_status(&s, "x", InstallStatus::Downloaded);
        s = push_install_status(&s, "x", InstallStatus::Installing);
        s = push_install_status(&s, "x", InstallStatus::Installed);
        let cell = ast::fetch_or_phi("Wine_App_install_status", &s);
        let seq = cell.as_seq().unwrap();
        let labels: Vec<&str> = seq.iter()
            .map(|f| ast::binding(f, "Install Status").unwrap_or("?"))
            .collect();
        assert_eq!(labels, vec!["Downloaded", "Installing", "Installed"]);
    }

    #[test]
    fn install_app_short_circuits_when_marker_present() {
        let state = seeded_state();
        let tmp = tempdir();
        // Simulate a prior successful install by pre-writing the marker.
        installer_run::mark_install_complete(&tmp).expect("seed marker");
        let report = install_app(&state, "notepad-plus-plus", &tmp, None)
            .expect("install must short-circuit");
        assert_eq!(report.status, InstallStatus::Installed);
        assert!(report.already_installed,
                "marker presence must surface as already_installed");
    }

    #[test]
    fn install_app_marks_installing_when_no_url_declared() {
        let state = ast::Object::phi();   // no facts at all
        let tmp = tempdir();
        let report = install_app(&state, "no-such-app", &tmp, None)
            .expect("missing URL must not error");
        assert_eq!(report.status, InstallStatus::Installing);
        assert!(report.diagnostic.is_some());
        assert!(report.diagnostic.as_ref().unwrap().contains("no Installer URL"));
    }

    #[test]
    fn install_app_marks_installing_when_no_filename_declared() {
        // URL declared but filename missing.
        let mut s = ast::Object::phi();
        s = ast::cell_push(
            "Wine_App_has_Installer_URL",
            ast::fact_from_pairs(&[
                ("Wine App", "x"),
                ("Installer URL", "https://example.invalid/x.exe"),
            ]),
            &s,
        );
        let tmp = tempdir();
        let report = install_app(&s, "x", &tmp, None)
            .expect("missing filename must not error");
        assert_eq!(report.status, InstallStatus::Installing);
        assert!(report.diagnostic.as_ref().unwrap().contains("no Installer Filename"));
    }

    #[test]
    fn install_app_uses_local_path_for_non_url_source() {
        // A local-path fact source: copy is the fast path that
        // doesn't touch network. The wine_path is bogus so the
        // run-stage will return WineUnavailable (or — if wine is
        // installed — fail because the "installer" is text).
        let tmp = tempdir();
        let src_path = tmp.join("local-installer.exe");
        std::fs::write(&src_path, b"text-not-real-pe").unwrap();
        let mut s = ast::Object::phi();
        s = ast::cell_push(
            "Wine_App_has_Installer_URL",
            ast::fact_from_pairs(&[
                ("Wine App", "local-app"),
                ("Installer URL", src_path.to_str().unwrap()),
            ]),
            &s,
        );
        s = ast::cell_push(
            "Wine_App_has_Installer_Filename",
            ast::fact_from_pairs(&[
                ("Wine App", "local-app"),
                ("Installer Filename", "setup.exe"),
            ]),
            &s,
        );
        let bad_wine = std::path::PathBuf::from("/this/path/does/not/exist/wine");
        let report = install_app(&s, "local-app", &tmp, Some(&bad_wine))
            .expect("local-path install must not error before wine spawn");
        // With a fake wine binary we may get a Failed (spawn error
        // wrapped to io::Error — propagated up) — but the more likely
        // outcome on the typical host is the io spawn error
        // surfaces as an Err. We accept either: the test confirms
        // the local-copy fast path does run before any wine logic.
        // Inspect the cache dir directly.
        let cache_file = installer_fetch::install_cache_dir(&tmp).join("setup.exe");
        assert!(cache_file.is_file(),
                "local installer must be copied into cache, got: {:?}", cache_file);
        // Status is one of {Failed, Downloaded, Installed} — never
        // Installing because both URL + filename are declared and
        // copy succeeded.
        assert_ne!(report.status, InstallStatus::Installing,
                   "local copy succeeded, must not be in Installing state");
    }

    #[test]
    fn format_report_prints_status_and_filename() {
        let r = InstallReport {
            status: InstallStatus::Installed,
            installer_filename: "npp-installer.exe".to_string(),
            already_installed: false,
            diagnostic: None,
        };
        let s = format_report(&r, "notepad-plus-plus");
        assert!(s.contains("Installing Wine app 'notepad-plus-plus' (status: Installed)"));
        assert!(s.contains("installer: npp-installer.exe"));
    }

    #[test]
    fn format_report_prints_diagnostic_for_failed() {
        let r = InstallReport {
            status: InstallStatus::Failed,
            installer_filename: "x.exe".to_string(),
            already_installed: false,
            diagnostic: Some("installer exited with status 2".to_string()),
        };
        let s = format_report(&r, "x");
        assert!(s.contains("status: Failed"));
        assert!(s.contains("note: installer exited with status 2"));
    }

    #[test]
    fn format_report_prints_already_installed_marker_message() {
        let r = InstallReport {
            status: InstallStatus::Installed,
            installer_filename: "x.exe".to_string(),
            already_installed: true,
            diagnostic: None,
        };
        let s = format_report(&r, "x");
        assert!(s.contains("(already installed; _install_complete marker present)"));
    }

    /// End-to-end with the bundled wine.md corpus. Confirms the
    /// installer-URL + filename cells are populated for at least one
    /// Wine App after the parser walks the readings (per the new
    /// instance facts added in this commit).
    #[cfg(feature = "compat-readings")]
    #[test]
    fn installer_url_resolves_for_notepad_plus_plus_in_real_corpus() {
        let filesystem_md = include_str!("../../../../readings/os/filesystem.md");
        let wine_md = include_str!("../../../../readings/compat/wine.md");
        let fs_state = crate::parse_forml2::parse_to_state(filesystem_md)
            .expect("filesystem.md must parse cleanly");
        let state = crate::parse_forml2::parse_to_state_from(wine_md, &fs_state)
            .expect("wine.md must parse cleanly with filesystem.md preloaded");
        let url = installer_url_for(&state, "notepad-plus-plus");
        assert!(url.is_some(),
                "notepad-plus-plus must declare an Installer URL in wine.md; got {:?}", url);
        let filename = installer_filename_for(&state, "notepad-plus-plus");
        assert!(filename.is_some(),
                "notepad-plus-plus must declare an Installer Filename in wine.md; got {:?}", filename);
    }

    fn tempdir() -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let pid = std::process::id();
        let path = std::env::temp_dir().join(format!("arest-wine-install-test-{}-{}", pid, n));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("tempdir create");
        path
    }
}

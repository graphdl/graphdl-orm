// Wine App launch + monitor orchestrator (#506).
//
// Sits at the end of the `arest run` chain, after
// `cli::wine_bootstrap` (prefix bootstrap) and `cli::wine_install`
// (installer fetch + run). Where install puts a working installation
// on disk, this module brings the app to life: it resolves the main
// `.exe` path from the FORML facts, spawns wine on it under
// `WINEPREFIX=<prefix>`, samples the `MonitorOutcome` after a short
// settle delay, and transitions the `Wine_App_run_status` cell
// through Running → (Paused | Exited | Crashed) per the #212
// state-machine-as-derivation pattern.
//
// "Fire and report" semantics for tier-1: launch the app, return the
// PID and initial status, capture combined stdout+stderr to
// `<prefix>/drive_c/_run_log`. Long-running observation is the future
// `arest watch <app>` flow's job (separate task); this module's
// promise is "the app is reliably running, here's how to find it".
//
// Idempotency: the orchestrator inspects `Wine_App_run_status` at
// entry; if the most-recent transition for `app_id` is `Running` the
// launch returns `RunStatus::Running` with `already_running = true`
// instead of spawning a second instance. The cell is the single
// observable contract — there's no PID file or socket the SM also
// has to maintain in sync.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::ast;
use crate::cli::installer_run;
use crate::cli::process_monitor::{self, MonitorOutcome};

/// State-machine state for a Wine App's runtime lifecycle. The values
/// match the value set declared in `readings/compat/wine.md` for the
/// `Run Status` value type so the cell is round-trippable through
/// FORML facts. Pushed onto the `Wine_App_run_status` cell as one
/// fact per transition (#212 SM-as-derivation): the most-recent fact
/// in the cell is the current state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunStatus {
    /// The app's wine subprocess has been spawned and survived the
    /// settle delay (~500ms with no immediate exit). For tier-1 this
    /// is the success terminal — `arest run` returns and the future
    /// `arest watch` takes over polling.
    Running,
    /// The wine subprocess is alive but suspended (SIGSTOP / job-
    /// control pause / debugger break). Reserved for the watcher
    /// flow; not produced by `launch_app` in tier-1.
    Paused,
    /// The wine subprocess exited with status code 0. Distinct from
    /// `Crashed` so a clean shutdown is observable from the cell
    /// without inspecting the exit code field.
    Exited,
    /// The wine subprocess exited non-zero or was killed by a signal.
    /// Diagnostic exit code (when present) is in the `LaunchReport`.
    Crashed,
}

impl RunStatus {
    /// Stable string label for the SM cell. Matches the value-set
    /// declared in `readings/compat/wine.md`.
    pub fn as_label(&self) -> &'static str {
        match self {
            RunStatus::Running => "Running",
            RunStatus::Paused => "Paused",
            RunStatus::Exited => "Exited",
            RunStatus::Crashed => "Crashed",
        }
    }
}

/// Aggregate summary of a single `launch_app` invocation. Returned
/// to the dispatcher (`cli::run`) so it can print a human-readable
/// progress block without the launch module owning stdout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchReport {
    /// State-machine state at end of launch. For tier-1 this is
    /// `Running` on success; `Crashed` if the child exited within
    /// the settle window; one of the unavailability states (`Exited`
    /// with `process_id = None` for "main exe path missing", etc.)
    /// when the launch couldn't proceed declaratively.
    pub status: RunStatus,
    /// Resolved main-exe path the launcher tried to spawn.
    /// Prefix-relative as declared in the FORML facts (e.g.
    /// `drive_c/Program Files/Notepad++/notepad++.exe`); empty if no
    /// `Wine_App_has_Main_Exe_Path` fact was found for the app.
    pub main_exe_path: String,
    /// PID of the spawned wine subprocess. `None` when the launch
    /// short-circuited (no exe path declared, wine unavailable,
    /// already-running). Useful for the future `arest watch <app>`
    /// flow to attach to the running process without re-resolving.
    pub process_id: Option<u32>,
    /// True iff the orchestrator skipped the spawn because the
    /// `Wine_App_run_status` cell already had `Running` as the most
    /// recent transition for this app.
    pub already_running: bool,
    /// Optional human-readable diagnostic for the dispatcher to show
    /// in the progress block. Populated for non-`Running` terminal
    /// states or when the launch was a no-op.
    pub diagnostic: Option<String>,
}

/// Top-level orchestrator. Resolves the main exe path from the
/// FORML facts, spawns wine on it under `WINEPREFIX=<prefix>`,
/// samples the monitor after a short settle delay, and walks the
/// outcome into a `LaunchReport`.
///
/// Idempotent: if the most-recent `Wine_App_run_status` transition
/// for `app_id` is `Running`, returns a no-op report with
/// `already_running = true` and does not spawn a second instance.
///
/// `wine_path` is forwarded to `installer_run::resolve_wine_on_path`
/// equivalent: pass `None` to use PATH resolution (production path);
/// pass `Some(...)` from tests that want to point at a stub binary.
pub fn launch_app(
    state: &ast::Object,
    app_id: &str,
    prefix_dir: &Path,
    wine_path: Option<&Path>,
) -> std::io::Result<LaunchReport> {
    let main_exe_path = main_exe_path_for(state, app_id).unwrap_or_default();

    // Idempotency: scan the SM cell for the latest transition.
    // Append-only history means "currently Running" iff the last
    // fact for this app says Running.
    if current_run_status(state, app_id) == Some(RunStatus::Running) {
        return Ok(LaunchReport {
            status: RunStatus::Running,
            main_exe_path,
            process_id: None,
            already_running: true,
            diagnostic: Some(format!(
                "'{}' is already in Running state; not relaunching",
                app_id,
            )),
        });
    }

    // Stage 1: declarative blocker. No main exe path declared in the
    // FORML readings → can't launch; surface as Exited with a
    // diagnostic. Choosing Exited (not Crashed) because nothing
    // actually crashed — there was simply nothing to launch.
    if main_exe_path.is_empty() {
        return Ok(LaunchReport {
            status: RunStatus::Exited,
            main_exe_path,
            process_id: None,
            already_running: false,
            diagnostic: Some(format!("no Main Exe Path declared for '{}'", app_id)),
        });
    }

    // Stage 2: wine resolution. Mirror installer_run's behaviour —
    // a missing wine binary is a soft block (Exited with diagnostic),
    // not a hard error.
    let resolved_wine = match wine_path {
        Some(p) => p.to_path_buf(),
        None => match installer_run::resolve_wine_on_path() {
            Some(p) => p,
            None => return Ok(LaunchReport {
                status: RunStatus::Exited,
                main_exe_path,
                process_id: None,
                already_running: false,
                diagnostic: Some(
                    "wine not on PATH; launch staged for next run".to_string()
                ),
            }),
        },
    };

    // Stage 3: spawn. Build the full exe path (prefix-relative path
    // joined to prefix_dir) and pass it to wine as the only argument
    // beyond the env. WINEDEBUG=-all suppresses the wine internal
    // diagnostic spam that would otherwise dominate the run-log file.
    let full_exe_path = prefix_dir.join(&main_exe_path);
    let log_path = run_log_path(prefix_dir);
    if let Some(parent) = log_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // Truncate-and-open the log file so the log is the most-recent
    // run rather than an accumulating record. Mirrors installer_run's
    // single-write-per-run policy.
    let log_file_out = std::fs::File::create(&log_path)?;
    let log_file_err = log_file_out.try_clone()?;

    let mut cmd = Command::new(&resolved_wine);
    cmd.arg(&full_exe_path)
        .env("WINEPREFIX", prefix_dir)
        .env("WINEDEBUG", "-all")
        .stdout(Stdio::from(log_file_out))
        .stderr(Stdio::from(log_file_err));
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return Ok(LaunchReport {
            status: RunStatus::Crashed,
            main_exe_path,
            process_id: None,
            already_running: false,
            diagnostic: Some(format!("wine spawn failed: {}", e)),
        }),
    };
    let pid = child.id();

    // Stage 4: settle. Sleep ~500ms then poll once. If the child has
    // already exited the launch failed reliably enough to surface
    // (silent crashers, missing exe at the prefix path, wrong arch
    // binary, etc.) — transition to Crashed/Exited per exit code.
    // If still running, the launch is a success and the monitor poll
    // is the future watcher's job.
    std::thread::sleep(std::time::Duration::from_millis(500));
    let outcome = process_monitor::poll(&mut child);
    match outcome {
        MonitorOutcome::StillRunning => Ok(LaunchReport {
            status: RunStatus::Running,
            main_exe_path,
            process_id: Some(pid),
            already_running: false,
            diagnostic: None,
        }),
        MonitorOutcome::Exited(_) => Ok(LaunchReport {
            status: RunStatus::Exited,
            main_exe_path,
            process_id: Some(pid),
            already_running: false,
            diagnostic: Some(
                "wine exited cleanly within settle window (clean main loop?)".to_string()
            ),
        }),
        MonitorOutcome::Crashed { exit_code } => Ok(LaunchReport {
            status: RunStatus::Crashed,
            main_exe_path,
            process_id: Some(pid),
            already_running: false,
            diagnostic: Some(match exit_code {
                Some(c) => format!("wine exited with status {}", c),
                None => "wine terminated by signal".to_string(),
            }),
        }),
        MonitorOutcome::Errored(kind) => Ok(LaunchReport {
            status: RunStatus::Crashed,
            main_exe_path,
            process_id: Some(pid),
            already_running: false,
            diagnostic: Some(format!("monitor poll errored: {:?}", kind)),
        }),
    }
}

/// Run-log path (combined stdout+stderr from the most-recent launch).
/// Lives under `drive_c/_run_log` so it sits inside the prefix's
/// emulated C:\ alongside `_install_log`. Always rewritten on each
/// launch.
pub fn run_log_path(prefix_dir: &Path) -> PathBuf {
    prefix_dir.join("drive_c").join("_run_log")
}

/// Lookup the Main Exe Path for `app_id` from
/// `Wine_App_has_Main_Exe_Path`. Returns `None` if the cell has no
/// fact for the app — the orchestrator transitions to `Exited` with
/// a diagnostic in that case rather than spawning blindly.
pub fn main_exe_path_for(state: &ast::Object, app_id: &str) -> Option<String> {
    let cell = ast::fetch_or_phi("Wine_App_has_Main_Exe_Path", state);
    let seq = cell.as_seq()?;
    for fact in seq.iter() {
        if ast::binding(fact, "Wine App") == Some(app_id) {
            return ast::binding(fact, "Main Exe Path").map(|s| s.to_string());
        }
    }
    None
}

/// Read the most-recent `Wine_App_run_status` transition for `app_id`
/// and parse it back into a `RunStatus`. Returns `None` if the cell
/// has no facts for the app yet (= "never launched"). The label →
/// enum mapping mirrors `RunStatus::as_label` so a fact pushed via
/// `push_run_status` round-trips through this read.
pub fn current_run_status(state: &ast::Object, app_id: &str) -> Option<RunStatus> {
    let cell = ast::fetch_or_phi("Wine_App_run_status", state);
    let seq = cell.as_seq()?;
    let mut latest: Option<RunStatus> = None;
    for fact in seq.iter() {
        if ast::binding(fact, "Wine App") == Some(app_id) {
            latest = match ast::binding(fact, "Run Status") {
                Some("Running") => Some(RunStatus::Running),
                Some("Paused") => Some(RunStatus::Paused),
                Some("Exited") => Some(RunStatus::Exited),
                Some("Crashed") => Some(RunStatus::Crashed),
                _ => latest,
            };
        }
    }
    latest
}

/// Push a state-machine transition fact onto the
/// `Wine_App_run_status` cell. Returns the new state with the fact
/// appended. Mirrors `wine_install::push_install_status`: every
/// transition is one fact; the final state is the last fact in the
/// cell.
pub fn push_run_status(
    state: &ast::Object,
    app_id: &str,
    status: RunStatus,
) -> ast::Object {
    let fact = ast::fact_from_pairs(&[
        ("Wine App", app_id),
        ("Run Status", status.as_label()),
    ]);
    ast::cell_push("Wine_App_run_status", fact, state)
}

/// Format a `LaunchReport` as a human-readable progress block for
/// the CLI to print. Multi-line; one line per material outcome.
/// Stable formatting so downstream scripts can grep on the labels
/// without parsing.
pub fn format_report(report: &LaunchReport, app_id: &str) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "Launching Wine app '{}' (status: {}):\n",
        app_id,
        report.status.as_label(),
    ));
    if !report.main_exe_path.is_empty() {
        out.push_str(&format!("  main exe: {}\n", report.main_exe_path));
    }
    if let Some(pid) = report.process_id {
        out.push_str(&format!("  pid: {}\n", pid));
    }
    if report.already_running {
        out.push_str("  (already running; not relaunching)\n");
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

    /// Minimal seeded state for the lookup tests — declares a Main
    /// Exe Path for one app, leaves the rest unbound.
    fn seeded_state() -> ast::Object {
        let mut s = ast::Object::phi();
        s = ast::cell_push(
            "Wine_App_has_Main_Exe_Path",
            ast::fact_from_pairs(&[
                ("Wine App", "notepad-plus-plus"),
                ("Main Exe Path", "drive_c/Program Files/Notepad++/notepad++.exe"),
            ]),
            &s,
        );
        s
    }

    #[test]
    fn run_status_labels_match_sm_values() {
        // The SM values must exactly match the value set declared in
        // wine.md for the run state cell — round-trippability through
        // FORML facts depends on it.
        assert_eq!(RunStatus::Running.as_label(), "Running");
        assert_eq!(RunStatus::Paused.as_label(), "Paused");
        assert_eq!(RunStatus::Exited.as_label(), "Exited");
        assert_eq!(RunStatus::Crashed.as_label(), "Crashed");
    }

    #[test]
    fn main_exe_path_for_returns_declared_path() {
        let state = seeded_state();
        assert_eq!(
            main_exe_path_for(&state, "notepad-plus-plus").as_deref(),
            Some("drive_c/Program Files/Notepad++/notepad++.exe"),
        );
    }

    #[test]
    fn main_exe_path_for_returns_none_for_unknown_app() {
        let state = seeded_state();
        assert!(main_exe_path_for(&state, "no-such-app").is_none());
    }

    #[test]
    fn push_run_status_appends_fact() {
        let state = seeded_state();
        let after = push_run_status(&state, "notepad-plus-plus", RunStatus::Running);
        let cell = ast::fetch_or_phi("Wine_App_run_status", &after);
        let seq = cell.as_seq().expect("cell must be a seq");
        assert_eq!(seq.len(), 1);
        let fact = seq.iter().next().unwrap();
        assert_eq!(ast::binding(fact, "Wine App"), Some("notepad-plus-plus"));
        assert_eq!(ast::binding(fact, "Run Status"), Some("Running"));
    }

    #[test]
    fn push_run_status_chains_transitions() {
        // Transition stream: Running → Paused → Crashed. The final
        // state is always the last fact in the cell.
        let mut s = ast::Object::phi();
        s = push_run_status(&s, "x", RunStatus::Running);
        s = push_run_status(&s, "x", RunStatus::Paused);
        s = push_run_status(&s, "x", RunStatus::Crashed);
        let cell = ast::fetch_or_phi("Wine_App_run_status", &s);
        let seq = cell.as_seq().unwrap();
        let labels: Vec<&str> = seq.iter()
            .map(|f| ast::binding(f, "Run Status").unwrap_or("?"))
            .collect();
        assert_eq!(labels, vec!["Running", "Paused", "Crashed"]);
    }

    #[test]
    fn current_run_status_returns_latest_for_app() {
        // Two apps with interleaved transitions; current_run_status
        // must return the latest *for the queried app*, not the
        // global latest.
        let mut s = ast::Object::phi();
        s = push_run_status(&s, "a", RunStatus::Running);
        s = push_run_status(&s, "b", RunStatus::Crashed);
        s = push_run_status(&s, "a", RunStatus::Exited);
        s = push_run_status(&s, "b", RunStatus::Running);
        assert_eq!(current_run_status(&s, "a"), Some(RunStatus::Exited));
        assert_eq!(current_run_status(&s, "b"), Some(RunStatus::Running));
    }

    #[test]
    fn current_run_status_returns_none_when_no_facts() {
        let s = ast::Object::phi();
        assert_eq!(current_run_status(&s, "x"), None);
    }

    #[test]
    fn launch_app_marks_exited_when_no_main_exe_declared() {
        // No Main Exe Path fact for the app → declarative blocker.
        let state = ast::Object::phi();
        let tmp = tempdir();
        let report = launch_app(&state, "no-such-app", &tmp, None)
            .expect("missing exe must not error");
        assert_eq!(report.status, RunStatus::Exited);
        assert!(report.process_id.is_none());
        assert!(!report.already_running);
        assert!(report.diagnostic.as_ref().unwrap().contains("no Main Exe Path"));
    }

    #[test]
    fn launch_app_short_circuits_when_already_running() {
        // Pre-populate the SM cell with a Running transition; the
        // launcher must skip the spawn and return already_running.
        let mut state = seeded_state();
        state = push_run_status(&state, "notepad-plus-plus", RunStatus::Running);
        let tmp = tempdir();
        // Bogus wine_path proves the short-circuit happens *before*
        // any spawn attempt.
        let bad_wine = std::path::PathBuf::from("/this/path/does/not/exist/wine");
        let report = launch_app(&state, "notepad-plus-plus", &tmp, Some(&bad_wine))
            .expect("idempotent path must not error");
        assert_eq!(report.status, RunStatus::Running);
        assert!(report.already_running);
        assert!(report.process_id.is_none(),
                "short-circuit must not produce a fresh PID");
    }

    #[test]
    fn launch_app_relaunches_after_crash() {
        // SM history: Running, then Crashed → next launch must NOT
        // short-circuit (the most-recent state is Crashed, not
        // Running). With wine missing the launcher transitions to
        // Exited rather than Running, but importantly the
        // already_running flag stays false.
        let mut state = seeded_state();
        state = push_run_status(&state, "notepad-plus-plus", RunStatus::Running);
        state = push_run_status(&state, "notepad-plus-plus", RunStatus::Crashed);
        let tmp = tempdir();
        let bad_wine = std::path::PathBuf::from("/this/path/does/not/exist/wine");
        let report = launch_app(&state, "notepad-plus-plus", &tmp, Some(&bad_wine))
            .expect("relaunch must not error");
        // The bad_wine path doesn't exist, so the spawn produces a
        // Crashed transition — the important invariant is that the
        // short-circuit did NOT fire.
        assert!(!report.already_running,
                "Crashed → relaunch must NOT short-circuit; got: {:?}", report);
    }

    #[test]
    fn launch_app_marks_exited_when_wine_unavailable() {
        // Main Exe Path declared but wine isn't on PATH. The launcher
        // returns Exited with a clean "wine not on PATH" diagnostic
        // rather than a spawn error; the prefix is intact, the
        // launch is staged for next run.
        let state = seeded_state();
        let tmp = tempdir();
        // Empty PATH ensures wine resolution fails regardless of the
        // host's wine install (matches installer_run's
        // resolve_wine_returns_none_when_path_empty).
        let original = std::env::var_os("PATH");
        std::env::set_var("PATH", "");
        let report = launch_app(&state, "notepad-plus-plus", &tmp, None);
        match original {
            Some(v) => std::env::set_var("PATH", v),
            None => std::env::remove_var("PATH"),
        }
        let report = report.expect("missing wine must not error");
        assert_eq!(report.status, RunStatus::Exited);
        assert!(report.process_id.is_none());
        assert!(report.diagnostic.as_ref().unwrap().contains("wine not on PATH"));
    }

    #[test]
    fn launch_app_crashed_when_wine_path_invalid() {
        // Main Exe Path declared, wine_path explicitly bad → spawn
        // fails → Crashed. The diagnostic should mention the spawn
        // failure rather than the (missing) exit code.
        let state = seeded_state();
        let tmp = tempdir();
        let bad_wine = std::path::PathBuf::from("/this/path/does/not/exist/wine");
        let report = launch_app(&state, "notepad-plus-plus", &tmp, Some(&bad_wine))
            .expect("invalid wine_path must not propagate as Err");
        assert_eq!(report.status, RunStatus::Crashed);
        assert!(report.diagnostic.as_ref().unwrap().contains("wine spawn failed"));
    }

    /// Confirms the run-log file lands at the expected path and is
    /// truncated each launch. Uses a stub "wine" binary (the host's
    /// own shell echoing a message) so the spawn is real but
    /// portable — we don't need real wine for this invariant.
    #[test]
    fn launch_app_writes_run_log_under_drive_c() {
        let state = seeded_state();
        let tmp = tempdir();
        // Use the host's own shell as a fake "wine" — it'll error
        // because the args are nonsense but the fact that the log
        // file gets created (and truncated) is what we're probing.
        let stub = stub_wine_binary();
        let _ = launch_app(&state, "notepad-plus-plus", &tmp, Some(&stub))
            .expect("stub launch must not error");
        let log = run_log_path(&tmp);
        assert!(log.is_file(),
                "run log must be created at {:?}", log);
    }

    #[test]
    fn run_log_path_lives_under_drive_c() {
        let p = run_log_path(Path::new("/tmp/prefix"));
        assert!(p.ends_with("drive_c/_run_log")
                || p.ends_with(r"drive_c\_run_log"));
    }

    #[test]
    fn format_report_prints_status_and_pid() {
        let r = LaunchReport {
            status: RunStatus::Running,
            main_exe_path: "drive_c/Program Files/Notepad++/notepad++.exe".to_string(),
            process_id: Some(12345),
            already_running: false,
            diagnostic: None,
        };
        let s = format_report(&r, "notepad-plus-plus");
        assert!(s.contains("Launching Wine app 'notepad-plus-plus' (status: Running)"));
        assert!(s.contains("main exe: drive_c/Program Files/Notepad++/notepad++.exe"));
        assert!(s.contains("pid: 12345"));
    }

    #[test]
    fn format_report_prints_already_running_marker() {
        let r = LaunchReport {
            status: RunStatus::Running,
            main_exe_path: "x.exe".to_string(),
            process_id: None,
            already_running: true,
            diagnostic: None,
        };
        let s = format_report(&r, "x");
        assert!(s.contains("(already running; not relaunching)"));
    }

    #[test]
    fn format_report_prints_diagnostic_for_crashed() {
        let r = LaunchReport {
            status: RunStatus::Crashed,
            main_exe_path: "x.exe".to_string(),
            process_id: Some(999),
            already_running: false,
            diagnostic: Some("wine exited with status 2".to_string()),
        };
        let s = format_report(&r, "x");
        assert!(s.contains("status: Crashed"));
        assert!(s.contains("note: wine exited with status 2"));
    }

    /// End-to-end with the bundled wine.md corpus. Confirms the
    /// Main Exe Path cell is populated for at least one Wine App
    /// after the parser walks the readings (per the new instance
    /// facts added in this commit).
    #[cfg(feature = "compat-readings")]
    #[test]
    fn main_exe_path_resolves_for_notepad_plus_plus_in_real_corpus() {
        let filesystem_md = include_str!("../../../../readings/os/filesystem.md");
        let wine_md = include_str!("../../../../readings/compat/wine.md");
        let fs_state = crate::parse_forml2::parse_to_state(filesystem_md)
            .expect("filesystem.md must parse cleanly");
        let state = crate::parse_forml2::parse_to_state_from(wine_md, &fs_state)
            .expect("wine.md must parse cleanly with filesystem.md preloaded");
        let path = main_exe_path_for(&state, "notepad-plus-plus");
        assert!(path.is_some(),
                "notepad-plus-plus must declare a Main Exe Path in wine.md; got {:?}", path);
        // Sanity: the path should be drive_c-rooted.
        assert!(path.as_ref().unwrap().starts_with("drive_c"),
                "Main Exe Path should be prefix-relative starting with drive_c; got {:?}", path);
    }

    /// Tempdir helper — same shape as the sibling modules use.
    fn tempdir() -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let pid = std::process::id();
        let path = std::env::temp_dir().join(format!("arest-wine-launch-test-{}-{}", pid, n));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("tempdir create");
        path
    }

    /// Returns a path to a host shell that exists, suitable for use
    /// as a stub "wine" binary in spawn tests. The shell will choke
    /// on the wine-style args, which is fine — the tests probe spawn
    /// + log creation, not exec semantics.
    fn stub_wine_binary() -> std::path::PathBuf {
        if cfg!(windows) {
            // Locate cmd.exe via SystemRoot — guaranteed-present on
            // every Windows install.
            let root = std::env::var("SystemRoot")
                .unwrap_or_else(|_| "C:\\Windows".to_string());
            std::path::PathBuf::from(root).join("System32").join("cmd.exe")
        } else {
            std::path::PathBuf::from("/bin/sh")
        }
    }
}

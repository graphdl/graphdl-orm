// `wine <installer>` subprocess wrapper for `arest run` (#505).
//
// Sibling of `cli::installer_fetch`. Where the fetcher writes the
// installer binary into `<prefix>/drive_c/_install/<filename>`, this
// module spawns `wine <installer>` under `WINEPREFIX=<prefix>` and
// waits for completion. stdout + stderr are captured into
// `<prefix>/drive_c/_install_log` so debugging a silent installer
// crash doesn't require wrapping the cli call in `tee`.
//
// Idempotency is signalled by the `<prefix>/drive_c/_install_complete`
// marker file: when present, this module's public entrypoint
// (`run_installer`) returns `RunOutcome::AlreadyInstalled` without
// spawning wine. Caller (`cli::wine_install`) is responsible for
// writing the marker after a successful run, so the idempotency
// boundary is observable from disk by external scripts as well.
//
// `wine` resolution mirrors `winetricks::resolve_winetricks_on_path`:
// PATH walk for `wine` (or `wine.exe` on Windows hosts), no
// subprocess spawn until found. If `wine` is missing, returns
// `RunOutcome::WineUnavailable` rather than a spawn error so the
// dispatcher can render a clean `wine not installed` message.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Outcome of a `run_installer` call. Distinguishes the four cases
/// the orchestrator (`cli::wine_install`) cares about so the install
/// state machine can choose the right transition:
///
///   * `Installed` — wine ran and the installer exited 0; marker
///     file written by the caller.
///   * `AlreadyInstalled` — `_install_complete` marker present;
///     subprocess skipped.
///   * `WineUnavailable` — `wine` binary not on PATH; no subprocess
///     spawned, no error returned.
///   * `Failed { exit_code }` — wine ran but the installer exited
///     non-zero. Caller should transition to `Failed` state and
///     inspect `_install_log` for diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunOutcome {
    Installed,
    AlreadyInstalled,
    WineUnavailable,
    Failed { exit_code: Option<i32> },
}

/// Path of the install-complete marker file (writes after a
/// successful installer run). Lives under `drive_c/` so it sits
/// inside the prefix's emulated C:\ alongside the installer cache.
pub fn install_complete_marker(prefix_dir: &Path) -> PathBuf {
    prefix_dir.join("drive_c").join("_install_complete")
}

/// Path of the install-log file (combined stdout + stderr from the
/// last installer run). Always rewritten on each run so debugging
/// looks at the most-recent failure rather than an accumulating
/// historical record.
pub fn install_log_path(prefix_dir: &Path) -> PathBuf {
    prefix_dir.join("drive_c").join("_install_log")
}

/// Spawn `wine <installer>` under `WINEPREFIX=prefix_dir` and wait
/// for completion. The combined stdout + stderr are written to
/// `<prefix>/drive_c/_install_log`. The success-marker is NOT
/// written by this function — the caller (`cli::wine_install`) is
/// responsible for writing `_install_complete` once it has decided
/// the install qualifies as successful (some installers exit 0 but
/// silently no-op; the orchestrator may want to verify additional
/// invariants before marking).
///
/// `wine_path` is the resolved path to the wine binary. Pass
/// `Some(...)` to use an explicit path (e.g. for tests with a mock
/// script); pass `None` to look up `wine` on PATH via the host's
/// normal resolution.
///
/// `extra_args` is appended verbatim after the installer path.
/// Useful for installers that take an `/S` (silent) or
/// `/quiet` switch — the orchestrator passes these in based on
/// per-app facts. May be empty.
pub fn run_installer(
    prefix_dir: &Path,
    installer_path: &Path,
    extra_args: &[String],
    wine_path: Option<&Path>,
) -> std::io::Result<RunOutcome> {
    if install_complete_marker(prefix_dir).is_file() {
        return Ok(RunOutcome::AlreadyInstalled);
    }
    let resolved = match wine_path {
        Some(p) => p.to_path_buf(),
        None => match resolve_wine_on_path() {
            Some(p) => p,
            None => return Ok(RunOutcome::WineUnavailable),
        },
    };
    // Make sure the log dir exists before we open it.
    let log_path = install_log_path(prefix_dir);
    if let Some(parent) = log_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut cmd = Command::new(&resolved);
    cmd.arg(installer_path)
        .args(extra_args)
        .env("WINEPREFIX", prefix_dir);
    let output = cmd.output()?;
    // Concatenate stdout + stderr into a single log file with a
    // small header so debugging knows which stream is which.
    let mut combined = Vec::new();
    combined.extend_from_slice(b"--- stdout ---\n");
    combined.extend_from_slice(&output.stdout);
    combined.extend_from_slice(b"\n--- stderr ---\n");
    combined.extend_from_slice(&output.stderr);
    std::fs::write(&log_path, &combined)?;
    if output.status.success() {
        Ok(RunOutcome::Installed)
    } else {
        Ok(RunOutcome::Failed { exit_code: output.status.code() })
    }
}

/// Returns true iff `wine` resolves on the host's PATH. Mirrors the
/// `winetricks_available` probe — lightweight, no subprocess.
pub fn wine_available() -> bool {
    resolve_wine_on_path().is_some()
}

/// Walk PATH for `wine` (or `wine.exe` on Windows); return the first
/// hit. None if not found.
pub fn resolve_wine_on_path() -> Option<PathBuf> {
    let path_env = std::env::var_os("PATH")?;
    let candidates: &[&str] = if cfg!(windows) {
        &["wine.exe", "wine"]
    } else {
        &["wine"]
    };
    for dir in std::env::split_paths(&path_env) {
        for cand in candidates {
            let p = dir.join(cand);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

/// Mark the prefix as installed by writing the `_install_complete`
/// marker. Public so the orchestrator can call it after deciding the
/// install qualifies as successful; the run_installer function
/// itself does NOT write the marker.
pub fn mark_install_complete(prefix_dir: &Path) -> std::io::Result<()> {
    let marker = install_complete_marker(prefix_dir);
    if let Some(parent) = marker.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&marker, b"")?;
    Ok(())
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_complete_marker_lives_under_drive_c() {
        let p = install_complete_marker(Path::new("/tmp/prefix"));
        assert!(p.ends_with("drive_c/_install_complete")
                || p.ends_with(r"drive_c\_install_complete"));
    }

    #[test]
    fn install_log_path_lives_under_drive_c() {
        let p = install_log_path(Path::new("/tmp/prefix"));
        assert!(p.ends_with("drive_c/_install_log")
                || p.ends_with(r"drive_c\_install_log"));
    }

    #[test]
    fn run_installer_short_circuits_on_marker() {
        let tmp = tempdir();
        let drive_c = tmp.join("drive_c");
        std::fs::create_dir_all(&drive_c).unwrap();
        std::fs::write(install_complete_marker(&tmp), b"").unwrap();
        // Pass an obviously-bad wine_path to prove the short-circuit
        // happens BEFORE resolve_wine_on_path runs.
        let bad_wine = std::path::PathBuf::from("/this/path/does/not/exist/wine");
        let installer = tmp.join("setup.exe");
        let outcome = run_installer(&tmp, &installer, &[], Some(&bad_wine))
            .expect("must short-circuit");
        assert_eq!(outcome, RunOutcome::AlreadyInstalled);
    }

    #[test]
    fn run_installer_returns_unavailable_when_wine_absent() {
        let tmp = tempdir();
        let installer = tmp.join("setup.exe");
        // Skip when wine IS available — calling run_installer with
        // wine present would actually spawn it (and likely fail
        // because setup.exe doesn't exist), which is not what we're
        // probing. The None-path / no-wine scenario is the
        // observable case for hosts without a wine install.
        if !wine_available() {
            let outcome = run_installer(&tmp, &installer, &[], None)
                .expect("None branch must not error");
            assert_eq!(outcome, RunOutcome::WineUnavailable);
        }
    }

    #[test]
    fn mark_install_complete_creates_marker() {
        let tmp = tempdir();
        assert!(!install_complete_marker(&tmp).exists());
        mark_install_complete(&tmp).expect("mark must succeed");
        assert!(install_complete_marker(&tmp).is_file(),
                "_install_complete marker must be created");
    }

    #[test]
    fn mark_install_complete_idempotent() {
        let tmp = tempdir();
        mark_install_complete(&tmp).expect("first mark");
        mark_install_complete(&tmp).expect("second mark must not error");
        assert!(install_complete_marker(&tmp).is_file());
    }

    #[test]
    fn wine_available_does_not_spawn() {
        // Probe is just a PATH walk; safe to call on every host.
        let _ = wine_available();
    }

    #[test]
    fn resolve_wine_returns_none_when_path_empty() {
        let original = std::env::var_os("PATH");
        std::env::set_var("PATH", "");
        let got = resolve_wine_on_path();
        match original {
            Some(v) => std::env::set_var("PATH", v),
            None => std::env::remove_var("PATH"),
        }
        assert!(got.is_none(), "empty PATH must yield no wine; got {:?}", got);
    }

    /// Tempdir helper — same shape as the sibling modules use.
    fn tempdir() -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let pid = std::process::id();
        let path = std::env::temp_dir().join(format!("arest-installer-run-test-{}-{}", pid, n));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("tempdir create");
        path
    }
}

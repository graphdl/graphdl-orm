// Non-blocking subprocess status poller for `cli::wine_launch` (#506).
//
// Sibling of `cli::installer_run` — but where the install path's
// subprocess wrapper synchronously waits for completion, this monitor
// is a small wrapper around `std::process::Child::try_wait` that
// returns the current status without blocking. The launcher uses it
// twice: once shortly after spawn (to detect immediate-exit failures
// such as a missing main exe path) and again on each REPL status
// query.
//
// The launch path is "fire and report" for tier-1: spawn the child,
// poll for ~500ms to confirm it didn't immediately crash, then return
// the PID + initial status to the caller. The cell-based watcher
// (separate task, `arest watch <app>`) does the long-running poll.
// Keeping this module narrow lets both consumers share the same
// `MonitorOutcome` shape without paying for a runtime / async story.
//
// Distinguishing Crashed from Exited: per #506, exit code 0 maps to
// `Exited` and any other exit (non-zero code OR signal termination)
// maps to `Crashed`. The orchestrator (`cli::wine_launch`) walks the
// `MonitorOutcome` into a `Wine_App_run_status` cell transition.

use std::process::Child;

/// Snapshot of a child process's lifecycle state at the moment the
/// monitor was polled. Mirrors the four observable cases the launch
/// state machine needs to distinguish:
///
///   * `StillRunning` — `try_wait` returned `Ok(None)`. The child has
///     not exited; the launcher reports `Running` and returns.
///   * `Exited(code)` — child exited with status code 0. Anything else
///     comes back as `Crashed` so the SM can emit a distinct
///     transition. The retained `i32` is always 0 here; the field is
///     kept for symmetry with `Crashed { exit_code: Some(_) }` and to
///     leave room for future "graceful exit with non-zero code"
///     refinement without a struct shape change.
///   * `Crashed { exit_code }` — child exited non-zero or was killed
///     by a signal. `exit_code` is `Some(c)` for a non-zero exit and
///     `None` when terminated by a signal (Unix-only path; on Windows
///     the OS surfaces the termination reason as a status code so the
///     `None` branch is unreachable but kept total for portability).
///   * `Errored(io::ErrorKind)` — `try_wait` itself returned an
///     `io::Error`; the launcher logs and treats as `Crashed` for
///     SM purposes. The `ErrorKind` is preserved so callers can
///     differentiate "no such process" (the OS reaped the child
///     between two poll calls) from a real syscall failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MonitorOutcome {
    StillRunning,
    Exited(i32),
    Crashed { exit_code: Option<i32> },
    Errored(std::io::ErrorKind),
}

impl MonitorOutcome {
    /// True iff the child is still alive at the moment of poll.
    /// Sugar for `matches!(self, MonitorOutcome::StillRunning)`;
    /// callers reading the SM tend to phrase the check this way so
    /// the helper avoids re-spelling the variant pattern.
    pub fn is_running(&self) -> bool {
        matches!(self, MonitorOutcome::StillRunning)
    }

    /// True iff the child has reached a terminal state (exited or
    /// crashed). `Errored` counts as terminal because the launcher
    /// cannot recover the `Child` handle once `try_wait` errors —
    /// the wrapping `Wine_App_run_status` cell transitions to
    /// `Crashed` on this path.
    pub fn is_terminal(&self) -> bool {
        !self.is_running()
    }
}

/// Non-blocking status poll. Wraps `Child::try_wait` and translates
/// the result into a `MonitorOutcome`. Does not consume the child
/// handle — caller retains ownership so they can continue polling
/// across multiple invocations (the launcher polls once after the
/// settle delay; future `arest watch` polls on a timer).
///
/// The exit-code → variant mapping:
///   * `try_wait` returns `Ok(Some(status))` with `code() == Some(0)` →
///     `Exited(0)`.
///   * `try_wait` returns `Ok(Some(status))` with `code() == Some(c)`
///     for any non-zero `c` → `Crashed { exit_code: Some(c) }`.
///   * `try_wait` returns `Ok(Some(status))` with `code() == None`
///     (signal termination on Unix) → `Crashed { exit_code: None }`.
///   * `try_wait` returns `Ok(None)` → `StillRunning`.
///   * `try_wait` returns `Err(_)` → `Errored(kind)`.
pub fn poll(child: &mut Child) -> MonitorOutcome {
    match child.try_wait() {
        Ok(Some(status)) => {
            let code = status.code();
            match code {
                Some(0) => MonitorOutcome::Exited(0),
                Some(c) => MonitorOutcome::Crashed { exit_code: Some(c) },
                None => MonitorOutcome::Crashed { exit_code: None },
            }
        }
        Ok(None) => MonitorOutcome::StillRunning,
        Err(e) => MonitorOutcome::Errored(e.kind()),
    }
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    /// Spawn a process that exits successfully with code 0 and confirm
    /// the monitor surfaces it as `Exited(0)`. We use the host's own
    /// shell to spawn an immediate `exit 0` so the test is portable
    /// across hosts without needing a side binary.
    #[test]
    fn poll_returns_exited_for_clean_exit() {
        let mut child = spawn_immediate_exit(0).expect("spawn must succeed");
        // Give the child a moment to actually exit. try_wait is
        // non-blocking; we briefly spin until it reports terminal so
        // the test is deterministic without arbitrary sleeps.
        let outcome = wait_until_terminal(&mut child);
        assert_eq!(outcome, MonitorOutcome::Exited(0),
                   "exit 0 must map to Exited(0); got {:?}", outcome);
    }

    /// Spawn a process that exits with a non-zero code. Confirms the
    /// monitor distinguishes Crashed from Exited.
    #[test]
    fn poll_returns_crashed_for_nonzero_exit() {
        let mut child = spawn_immediate_exit(7).expect("spawn must succeed");
        let outcome = wait_until_terminal(&mut child);
        assert_eq!(outcome, MonitorOutcome::Crashed { exit_code: Some(7) },
                   "exit 7 must map to Crashed{{Some(7)}}; got {:?}", outcome);
    }

    /// A long-running process must report StillRunning when polled
    /// immediately. We spawn a sleep that lasts long enough for the
    /// monitor's first poll to land before the child exits, then kill
    /// the child to clean up.
    #[test]
    fn poll_returns_still_running_for_live_process() {
        let mut child = spawn_long_sleep().expect("spawn must succeed");
        let outcome = poll(&mut child);
        // Always kill before asserting — leaving an orphaned sleep
        // around even on assertion failure would noise up the test
        // host.
        let _ = child.kill();
        let _ = child.wait();
        assert_eq!(outcome, MonitorOutcome::StillRunning,
                   "live child must report StillRunning; got {:?}", outcome);
    }

    #[test]
    fn is_running_recognises_still_running() {
        assert!(MonitorOutcome::StillRunning.is_running());
        assert!(!MonitorOutcome::Exited(0).is_running());
        assert!(!MonitorOutcome::Crashed { exit_code: Some(1) }.is_running());
        assert!(!MonitorOutcome::Crashed { exit_code: None }.is_running());
        assert!(!MonitorOutcome::Errored(std::io::ErrorKind::Other).is_running());
    }

    #[test]
    fn is_terminal_recognises_terminal_states() {
        assert!(!MonitorOutcome::StillRunning.is_terminal());
        assert!(MonitorOutcome::Exited(0).is_terminal());
        assert!(MonitorOutcome::Crashed { exit_code: Some(2) }.is_terminal());
        assert!(MonitorOutcome::Crashed { exit_code: None }.is_terminal());
        // Errored is treated as terminal because the launcher cannot
        // recover the Child handle after a try_wait error.
        assert!(MonitorOutcome::Errored(std::io::ErrorKind::Other).is_terminal());
    }

    /// Spawn the host's shell with an immediate `exit <code>` body.
    /// On Windows uses cmd.exe; elsewhere uses /bin/sh.
    fn spawn_immediate_exit(code: i32) -> std::io::Result<std::process::Child> {
        if cfg!(windows) {
            Command::new("cmd")
                .args(["/C", &format!("exit {}", code)])
                .spawn()
        } else {
            Command::new("sh")
                .args(["-c", &format!("exit {}", code)])
                .spawn()
        }
    }

    /// Spawn a process that lives for ~10s. Used to probe the
    /// StillRunning branch; the test kills the child immediately
    /// after the poll so the wall time stays small.
    fn spawn_long_sleep() -> std::io::Result<std::process::Child> {
        if cfg!(windows) {
            // `timeout` would exit early on stdin; use ping with a
            // long count to force a sustained spawn that won't race
            // the poll. -n 100 with -w 100 on localhost takes ~10s.
            Command::new("cmd")
                .args(["/C", "ping -n 100 -w 100 127.0.0.1 > NUL"])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
        } else {
            Command::new("sh")
                .args(["-c", "sleep 10"])
                .spawn()
        }
    }

    /// Spin on `poll` until the child reaches a terminal state. Used
    /// by exit-code tests to avoid coupling assertion timing to the
    /// host's process-table latency. Loop is bounded — if a clean
    /// exit hasn't surfaced after 5 seconds something is broken in
    /// the test infra and we panic to signal that.
    fn wait_until_terminal(child: &mut std::process::Child) -> MonitorOutcome {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let outcome = poll(child);
            if outcome.is_terminal() {
                return outcome;
            }
            if std::time::Instant::now() > deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("child failed to reach terminal state within 5s");
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
}

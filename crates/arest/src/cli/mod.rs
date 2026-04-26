// CLI subcommand handlers — std-only.
//
// Submodules implement the `arest <verb> <args…>` family of CLI
// subcommands. main.rs (and the bin target) dispatches to them after
// argv parsing; each submodule owns its own help text + exit codes.
//
// Currently:
//   * `run` — `arest run "App Name"` dispatches to
//             `crate::command::wine_app_by_name` to resolve the slug +
//             prefix, then calls `wine_bootstrap::bootstrap_prefix`
//             to apply winetricks recipes / DLL overrides / registry
//             keys derived from the FORML facts in
//             `readings/compat/wine.md`, then calls
//             `wine_install::install_app` to fetch + run the
//             installer binary under wine.
//   * `wine_bootstrap` — orchestrates the prefix bootstrap by walking
//             `Wine_App_requires_Required_Component` cells (winetricks
//             recipes), `requires DLL Override of` legacy cells (DLL
//             overrides) and `requires Registry Key at` legacy cells
//             (registry keys) for a given app id.
//   * `winetricks` — wraps the `winetricks` shell script as a
//             subprocess; reads the prefix's `winetricks.log` to
//             short-circuit already-applied recipes for idempotency.
//   * `wine_overrides` — DLL override + registry-key writers; emits
//             `[Software\\Wine\\DllOverrides]` blocks into the
//             prefix's `system.reg` and `@="<value>"` keys into
//             `system.reg` / `user.reg` per the registry root.
//   * `wine_install` (#505) — installer fetch + install orchestrator.
//             Resolves Installer URL / Filename from the FORML facts,
//             fetches the binary into `<prefix>/drive_c/_install/`,
//             runs it under wine, transitions the install state
//             machine. Idempotent via `_install_complete` marker.
//   * `installer_fetch` (#505) — subprocess wrapper around curl /
//             PowerShell `Invoke-WebRequest` for the binary download;
//             also handles local-path copies for pre-staged
//             installers.
//   * `installer_run` (#505) — subprocess wrapper for `wine
//             <installer>`; captures stdout + stderr to
//             `<prefix>/drive_c/_install_log` for debugging.
//   * `wine_launch` (#506) — main app launch + monitor. Resolves the
//             Main Exe Path from FORML facts, spawns wine on it under
//             `WINEPREFIX=<prefix>` with `WINEDEBUG=-all`, samples
//             the monitor after a short settle delay, and walks the
//             outcome through the `Wine_App_run_status` SM cell
//             (Running → Paused | Exited | Crashed). Captures
//             stdout+stderr to `<prefix>/drive_c/_run_log`.
//             Idempotent: refuses to relaunch when the cell's
//             most-recent transition for the app is `Running`.
//   * `process_monitor` (#506) — non-blocking `Child::try_wait`
//             wrapper translating into a `MonitorOutcome` enum
//             (`StillRunning`, `Exited(i32)`, `Crashed { exit_code }`,
//             `Errored`). Used by `wine_launch` for the post-spawn
//             settle poll and by the future `arest watch` flow for
//             ongoing observation.
//
// Future verbs (`arest install`, `arest exec`, …) plug in here so
// main.rs doesn't grow another giant `match` arm per subcommand.

#[cfg(not(feature = "no_std"))]
pub mod run;
#[cfg(not(feature = "no_std"))]
pub mod wine_bootstrap;
#[cfg(not(feature = "no_std"))]
pub mod wine_overrides;
#[cfg(not(feature = "no_std"))]
pub mod winetricks;
#[cfg(not(feature = "no_std"))]
pub mod wine_install;
#[cfg(not(feature = "no_std"))]
pub mod installer_fetch;
#[cfg(not(feature = "no_std"))]
pub mod installer_run;
#[cfg(not(feature = "no_std"))]
pub mod process_monitor;
#[cfg(not(feature = "no_std"))]
pub mod wine_launch;

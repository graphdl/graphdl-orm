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
// `entropy_host` (#574) — host-OS `EntropySource` adapter delegating to
// `getrandom` (Linux/FreeBSD getrandom(2), macOS arc4random_buf, Windows
// BCryptGenRandom). Installed by callers that need RNG before any
// `csprng::random_*` path fires; the CLI itself doesn't auto-install
// today (per-target adapter job, see #575/#578).
#[cfg(not(feature = "no_std"))]
pub mod entropy_host;
// `tenant_master_host` (#663) — host-CLI tenant master installer.
// Generates 32 random bytes on first run, persists to
// `~/.arest/tenant_master.bin` (mode 0600 on Unix, restricted ACL on
// Windows), reads on subsequent runs. Wires into the cell_aead global
// slot via `arest::cell_aead::install_tenant_master`. Boot order:
// `entropy_host::install` first (csprng needs it for the seed), then
// `tenant_master_host::install` (uses csprng to generate the master
// on first run).
#[cfg(not(feature = "no_std"))]
pub mod tenant_master_host;
// `reload` (#561) — `arest reload <file.md>` runtime reading load.
// Routes through `crate::load_reading_core::load_reading` with
// `LoadReadingPolicy::AllowAll` and persists the merged state to the
// configured `--db`. Companion `arest watch <dir>` shares the same
// `dispatch_with_state` core.
#[cfg(not(feature = "no_std"))]
pub mod reload;
// `watch` (#561 followup / DynRdg-T2) — `arest watch <dir>` polls a
// directory for `.md` changes and re-applies each modified file via
// the same `LoadReading` pipeline as `arest reload`. Pure scan core
// (`scan_once_with_state`) is testable without DB; the DB-backed
// `dispatch` enters an infinite poll loop with per-reload persist.
#[cfg(not(feature = "no_std"))]
pub mod watch;
// `entry` (#684/#650b) — main CLI dispatcher extracted from src/main.rs.
// Pre-extract, src/main.rs declared `mod ast; mod compile; ...` for
// every lib module independently of lib.rs, forcing cargo to recompile
// the entire crate twice (once for the lib's rlib, once for the bin's
// compilation unit). Profile (cargo-timing 2026-05-01) showed ~120s of
// duplicate cumulative compile across `arest-cli "bin"` and
// `arest-cli "bin" (test)`. Now `cli::entry::main_entry` carries the
// dispatcher inside the lib's compilation, src/main.rs is a 6-line
// shim, and each source file compiles exactly once.
#[cfg(not(feature = "no_std"))]
pub mod entry;

// compile-gc-orphaned-derived-facts (asserted-cell dup-fact bloat): the
// pattern matches `cli/entry.rs:1144-1160` — the bake-time compile path
// applies an identity-aware dedup pass over the final state before
// persisting, so cells like `Task_is_epic` that accrue one extra
// identity-equal copy per recompile (312 bindings for 8 distinct tasks
// observed live) are scrubbed before the row hits SQLite.
//
// `arest reload <file.md>` and `arest watch <dir>` bypass that site —
// they thread through `load_reading_core::load_reading`, which merges
// the new reading into the prior state via `ast::merge_states` /
// `ast::concat_dedup`. `concat_dedup` dedups the INCOMING side against
// the accumulator but never the accumulator's OWN internal duplicates
// (documented at `ast::dedup_cell_facts`), so a bloated prior cell
// loaded from disk stays bloated through the merge and re-persists at
// the same size on every reload. This helper applies the same dedup
// pattern to the runtime-load paths so their persisted result self-
// heals identically to the dirs-compile path.
//
// Layout mirrors entry.rs:1144-1160:
//   * declared-FT data cells (in `FactType.id`) get the full
//     arity+subject GC plus identity dedup;
//   * non-`:` non-meta cells get the arity-free empty-subject drop plus
//     identity dedup (safe without a uniformity assumption for
//     synthetic SM outputs etc.);
//   * `:` view / meta cells pass through (they regenerate from data).
//
// TODO(arest#TBD): extract the matching block in `cli/entry.rs:1144-
// 1160` to call this helper too, so the three sites stop drifting.
#[cfg(all(not(feature = "no_std"), feature = "local"))]
pub(crate) fn dedup_state_for_persist(d: &crate::ast::Object) -> crate::ast::Object {
    use crate::ast;
    let ft_ids: hashbrown::HashSet<String> =
        ast::fetch_cell_seq("FactType", d).as_seq()
            .map(|facts| facts.iter()
                .filter_map(|f| ast::binding(f, "id").map(|s| s.to_string()))
                .collect())
            .unwrap_or_default();
    let ft_arity: hashbrown::HashMap<String, usize> =
        ast::fetch_cell_seq("FactType", d).as_seq()
            .map(|facts| facts.iter()
                .filter_map(|f| Some((
                    ast::binding(f, "id")?.to_string(),
                    ast::binding(f, "arity")?.parse::<usize>().ok()?)))
            .collect())
            .unwrap_or_default();
    let map: hashbrown::HashMap<String, ast::Object> =
        ast::cells_iter(d).into_iter()
            .map(|(name, contents)| if ft_ids.contains(name) {
                (name.to_string(), ast::dedup_cell_facts(
                    &ast::drop_subjectless_facts_with_arity(contents, ft_arity.get(name).copied())))
            } else if !name.contains(':') {
                (name.to_string(), ast::dedup_cell_facts(&ast::drop_empty_subject_facts(contents)))
            } else {
                (name.to_string(), contents.clone())
            })
            .collect();
    ast::Object::map(map)
}

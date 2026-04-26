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
//             `readings/compat/wine.md`.
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

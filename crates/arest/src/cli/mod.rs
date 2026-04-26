// CLI subcommand handlers — std-only.
//
// Submodules implement the `arest <verb> <args…>` family of CLI
// subcommands. main.rs (and the bin target) dispatches to them after
// argv parsing; each submodule owns its own help text + exit codes.
//
// Currently:
//   * `run` — `arest run "App Name"` dispatches to
//             `crate::command::wine_app_by_name` and prints the
//             resolved `(slug, prefix Directory id)` pair.
//             On miss, prints near-name suggestions via Levenshtein
//             distance over the slug + display-title set.
//
// Future verbs (`arest install`, `arest exec`, …) plug in here so
// main.rs doesn't grow another giant `match` arm per subcommand.

#[cfg(not(feature = "no_std"))]
pub mod run;

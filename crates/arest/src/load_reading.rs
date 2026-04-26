// crates/arest/src/load_reading.rs
//
// SystemVerb::LoadReading (#555 / DynRdg-1) — runtime parse + validate +
// register a FORML 2 reading body into the live cell graph.
//
// The pure-FORML core (types + the actual `load_reading` function +
// the FORML cell-walker helpers) was extracted to
// `crate::load_reading_core` in #586 (mirroring JJJJJ's
// `select_component_core` extraction in #565 part 2). The extraction
// is the architectural prerequisite for kernel reach: once
// `parse_forml2` + `check` land in no_std, the gate inside
// `load_reading_core::load_reading` lifts to a single-line edit and
// PPPPP-2's `load_reading_persist::replay_loaded_readings` closure
// caller can be updated to pass `arest::load_reading_core::load_reading`
// directly.
//
// This module is now a thin re-export shim. The historical
// `arest::load_reading::*` paths (the public API FFFFF shipped in
// #555 and the in-crate consumers — `command::load_reading_handler`,
// `command::system_impl`, the integration tests in `tests/`) keep
// resolving exactly the same as before. Any host-only sugar (JSON
// envelope encoding, std::process glue) would land in this file
// rather than `load_reading_core` to keep the core no_std-clean;
// today none of that exists for this verb (the JSON envelope sits
// in `command.rs` next to `system_impl`'s other dispatch arms).

pub use crate::load_reading_core::*;

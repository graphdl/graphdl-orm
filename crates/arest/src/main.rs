// arest-cli — thin bin shim. The CLI dispatcher lives in
// `arest::cli::entry::main_entry` so it compiles inside the lib's
// rlib once, not once per (lib, bin) compilation pair. See
// crates/arest/src/cli/entry.rs for the full pre-extract context.
//
// Pre-extract (#684/#650b), src/main.rs declared `mod ast; mod
// compile; …` for all 31 lib modules independently of lib.rs's
// `pub mod ast; …`, forcing cargo to recompile each source file
// twice. cargo-timing 2026-05-01 measured ~120s of duplicate
// cumulative compile across `arest-cli "bin"` and
// `arest-cli "bin" (test)`. Now this shim is the entire bin's source.

fn main() {
    arest::cli::entry::main_entry()
}

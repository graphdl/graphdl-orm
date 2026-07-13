// Cargo does not track files pulled in via `include!()` for change detection,
// so editing the shared canon would otherwise leave a STALE binary: the DEFs
// embedded by `include!("../../shared/*.canon")` in main.rs are frozen at the
// last time main.rs itself was recompiled. (This bit us 2026-07-12: a canon
// edit had no effect until `touch main.rs` forced a rebuild.) Declaring the
// included sources as build dependencies makes a canon edit force a rebuild.
//
// Paths are relative to the package root (engine/rust/), one level up from the
// `../../shared` main.rs uses relative to engine/rust/src/.
fn main() {
    println!("cargo:rerun-if-changed=../shared/arest.canon");
    println!("cargo:rerun-if-changed=../shared/scenarios.canon");
}

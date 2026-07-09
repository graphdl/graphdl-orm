// Canon at boot: stage the store the OS bakes into its image. The dev
// layout keeps the app fleet as a SIBLING of the arest repo
// (Repos/apps beside Repos/arest); when the support store is there,
// the image carries the real thing. Anywhere else (CI, a fresh
// clone), a minimal empty-store fallback keeps the build green — the
// banner then reports the load receipt honestly.
use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let dst = out.join("store.json");
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let candidate = manifest
        .join("../../../apps/support.auto.dev/support.auto.dev.store.json");
    if candidate.exists() {
        fs::copy(&candidate, &dst).unwrap();
        println!("cargo:rerun-if-changed={}", candidate.display());
    } else {
        fs::write(&dst, "{\"d\":[]}").unwrap();
    }
    println!("cargo:rerun-if-changed=build.rs");
}

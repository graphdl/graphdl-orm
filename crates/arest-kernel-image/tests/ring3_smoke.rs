// crates/arest-kernel-image/tests/ring3_smoke.rs
//
// Integration test that drives the ring-3 smoke harness.
// Shells out to scripts/test-ring3.ps1 (PowerShell, Windows-only),
// asserts exit code 0. On non-Windows hosts the test is ignored.

#![cfg(feature = "ring3-smoke")]

use std::path::PathBuf;
use std::process::Command;

#[test]
#[cfg_attr(not(windows), ignore = "harness is PowerShell-only (Windows host required)")]
fn ring3_smoke_passes() {
    // Resolve repo root — CARGO_MANIFEST_DIR is the image crate's
    // directory; the script lives two levels up.
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let script = manifest
        .parent().expect("crates/")
        .parent().expect("repo root")
        .join("scripts/test-ring3.ps1");
    assert!(script.is_file(), "missing harness: {}", script.display());

    let status = Command::new("powershell")
        .args([
            "-NoProfile",
            "-ExecutionPolicy", "Bypass",
            "-File", script.to_str().expect("utf8 path"),
        ])
        .status()
        .expect("failed to invoke PowerShell");

    assert!(
        status.success(),
        "ring3 smoke harness exited {:?}; see target/test-serial.log if present",
        status.code()
    );
}

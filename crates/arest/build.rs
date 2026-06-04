// build.rs — embed build provenance so the running binary can
// self-report which engine it actually is.
//
// WHY: the MCP pins its arest-cli at startup and re-spawns that exact
// path every call. When a stale binary gets pinned (a different build
// profile than the one being rebuilt), the server keeps running a stale
// engine undetected — mtime-only staleness checks are blind to it. The
// fix is the binary itself reporting the git SHA + build time it was
// compiled from. Captured here at build time and read back via env!()
// in the `version` subcommand (cli/entry.rs).
//
// Emits two rustc-env vars consumed by env!():
//   AREST_GIT_SHA   — `git rev-parse HEAD`, or "unknown" if git fails.
//   AREST_BUILD_TIME— RFC3339-ish UTC build timestamp, or "unknown".
//
// Degrades gracefully: a build with no git on PATH, or outside a repo,
// still compiles — the subcommand just reports sha "unknown".

use std::process::Command;

fn main() {
    let sha = git_head_sha().unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=AREST_GIT_SHA={}", sha);
    println!("cargo:rustc-env=AREST_BUILD_TIME={}", build_time_utc());

    // Rebuild the provenance whenever HEAD moves so a fresh `cargo build`
    // after a commit re-embeds the new SHA instead of reusing a cached
    // build-script output. These files are cheap to stat and only change
    // on commit / branch switch. Guarded by existence so a non-git build
    // (tarball, vendored) doesn't spuriously force rebuilds or warn.
    for p in ["../../.git/HEAD", "../../.git/refs/heads"] {
        if std::path::Path::new(p).exists() {
            println!("cargo:rerun-if-changed={}", p);
        }
    }
    // Also re-run if the build script itself changes.
    println!("cargo:rerun-if-changed=build.rs");
}

fn git_head_sha() -> Option<String> {
    let out = Command::new("git").args(["rev-parse", "HEAD"]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let sha = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if sha.is_empty() {
        None
    } else {
        Some(sha)
    }
}

// Best-effort UTC timestamp without pulling in a date crate: seconds
// since the Unix epoch rendered as an ISO-8601 UTC string. Pure
// arithmetic over the civil-time algorithm (Howard Hinnant's
// days_from_civil inverse) so it has no dependencies and never panics.
fn build_time_utc() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_secs() as i64,
        Err(_) => return "unknown".to_string(),
    };
    format_epoch_utc(secs)
}

// Render epoch seconds as `YYYY-MM-DDTHH:MM:SSZ` (UTC). Extracted as a
// free fn so the conversion is exercisable; the days→civil split is the
// standard algorithm, valid for any year in range.
fn format_epoch_utc(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (hour, min, sec) = (rem / 3600, (rem % 3600) / 60, rem % 60);

    // days since 1970-01-01 -> civil (y, m, d). Hinnant, "chrono-Compatible
    // Low-Level Date Algorithms".
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let year = if m <= 2 { y + 1 } else { y };

    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        year, m, d, hour, min, sec
    )
}

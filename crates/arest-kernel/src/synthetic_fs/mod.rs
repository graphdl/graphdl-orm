// crates/arest-kernel/src/synthetic_fs/mod.rs
//
// Synthetic-file table (#534, #475a — first slice of the synthetic-fs
// epic). Maps well-known POSIX paths (`/proc/cpuinfo`, `/proc/meminfo`
// today; `/sys/class`, `/dev/null`, per-pid `/proc/<n>` in follow-up
// tracks #535-#537) to byte renderers backed by AREST cells. The
// renderers compute their bytes on demand at read time so the surface
// reflects the latest cell state without any caching layer.
//
// Why a separate module rather than folding into `file_serve`
// -----------------------------------------------------------
// `crate::file_serve` resolves `/file/{id}/content` URLs against the
// `File_has_*` cell graph — a single fact-driven lookup. Synthetic
// files are a different shape: their content comes from arbitrary
// kernel state (CPU topology, memory map, process table) rather than
// stored bytes, and their paths are POSIX-style rather than `/file/{id}`-
// style. Keeping the two concerns in sibling modules means a future
// `openat` syscall (#498) can route `/proc/cpuinfo` through this table
// without first detouring through the HTTP file-serve plumbing, while
// the HTTP path (file_serve's fallback chain) stays a one-line
// delegation.
//
// Per-subtree submodule layout
// ----------------------------
// Each future subtree (`/proc`, `/sys`, `/dev`) gets its own
// dispatcher submodule. The top-level `resolve` matches on the leading
// path component and hands off. The current implementation only
// exposes `/proc/*`; `proc.rs` owns the dispatch within that subtree.
//
// Wire-up surface
// ---------------
// Two public entry points:
//
//   * `resolve(path: &str) -> Option<Vec<u8>>` — the generic resolver.
//     Callers (file_serve fallback today; #498 openat tomorrow) should
//     use this rather than reaching into `proc::render_proc_file`
//     directly so a future `/sys/*` track lights up automatically.
//
//   * `proc` / `cpuinfo` / `meminfo` submodules — exposed `pub` so
//     callers that want to render a fixture state (the verification
//     recipe, the boot banner's potential future "proc preview" line)
//     can construct snapshots and call the renderers directly without
//     going through the path table.

use alloc::vec::Vec;

pub mod cpuinfo;
pub mod meminfo;
pub mod proc;
pub mod proc_pid;

/// Top-level resolver. Returns `Some(bytes)` for any path the
/// synthetic-fs table recognises, `None` otherwise. The caller is
/// responsible for falling back to the regular File-cell lookup when
/// this returns `None` (see `crate::file_serve::try_serve_synthetic`
/// for the wired-up fallback in the HTTP path).
///
/// Path matching is leading-slash absolute: callers must hand in the
/// full POSIX-style path including the leading `/`. No tilde expansion,
/// no relative-path normalisation — this surface assumes the caller has
/// already canonicalised whatever it received from userspace.
pub fn resolve(path: &str) -> Option<Vec<u8>> {
    if path.starts_with("/proc/") {
        return proc::render_proc_file(path);
    }
    None
}

// Inline tests are gated on `cfg(target_os = "linux")` for the same
// reason `composer` / `slint_backend` / `doom` gate theirs: the
// `arest-kernel` bin sets `test = false` in Cargo.toml, so the only
// way to run these tests is via a host-target `cargo test --bin
// arest-kernel --target x86_64-unknown-linux-gnu` invocation. On a
// Windows / Darwin host the `no_std` + UEFI dep chain refuses to
// link a test binary, so the gate keeps the build cross-host clean.
#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn resolve_proc_cpuinfo_dispatches() {
        let bytes = resolve("/proc/cpuinfo").expect("cpuinfo");
        let s = core::str::from_utf8(&bytes).unwrap();
        assert!(s.contains("processor\t: 0\n"));
    }

    #[test]
    fn resolve_proc_meminfo_dispatches() {
        let bytes = resolve("/proc/meminfo").expect("meminfo");
        let s = core::str::from_utf8(&bytes).unwrap();
        assert!(s.starts_with("MemTotal:"));
    }

    #[test]
    fn resolve_unknown_path_returns_none() {
        assert!(resolve("/etc/passwd").is_none());
        assert!(resolve("/sys/class/net").is_none());
        assert!(resolve("/dev/null").is_none());
        // Bare `/proc` (no slash) does not match the prefix arm —
        // future readdir-shaped lookup will handle the directory case.
        assert!(resolve("/proc").is_none());
    }

    #[test]
    fn resolve_unknown_proc_subpath_returns_none() {
        // The `/proc/` arm hands off to proc::render_proc_file which
        // returns None for unmodelled subpaths — the top-level
        // resolver mirrors that without claiming the path.
        assert!(resolve("/proc/uptime").is_none());
        assert!(resolve("/proc/12345/status").is_none());
    }

    #[test]
    fn resolve_empty_path_returns_none() {
        assert!(resolve("").is_none());
    }
}

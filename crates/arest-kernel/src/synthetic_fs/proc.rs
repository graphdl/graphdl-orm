// crates/arest-kernel/src/synthetic_fs/proc.rs
//
// `/proc/*` path resolver (#534, #475a). Single entry point —
// `render_proc_file(path)` — that matches a leading-slash path against
// the synthetic-file table and dispatches to the appropriate renderer.
// Returns `None` for any path that is not a recognised `/proc/*` entry,
// letting the caller fall back to the regular File-cell lookup chain.
//
// Why a separate dispatcher rather than dispatching inline in `mod.rs`
// --------------------------------------------------------------------
// Future tracks (#535 per-pid `/proc/<n>/`, #536 `/sys/class`, #537
// `/dev/null` etc) will add their own dispatchers, each handling its
// own subtree. Keeping the per-subtree dispatch out of `mod.rs` means
// the top-level resolver can stay a single match-on-prefix that hands
// off to the right submodule. This file owns the `/proc/` subtree only.
//
// What the dispatcher matches today
// ---------------------------------
//   * `/proc/cpuinfo` → `cpuinfo::render_cpuinfo()`
//   * `/proc/meminfo` → `meminfo::render_meminfo()`
//
// Anything else under `/proc/` (e.g. `/proc/uptime`, `/proc/stat`,
// `/proc/<pid>/*`) returns `None` until its track lands. The fallback
// path in the caller treats `None` as "not a synthetic file" and routes
// to the regular File-cell lookup.

use alloc::vec::Vec;

use super::{cpuinfo, meminfo};

/// Render the bytes of a synthetic `/proc/*` file. Returns `Some(bytes)`
/// when `path` matches one of the modelled entries, `None` otherwise.
///
/// Expected to be called by the file-resolver fallback in
/// `crate::file_serve` (HTTP read path) and, eventually, by the
/// `openat` syscall handler (#498). Both call sites treat `None` as
/// "fall through to regular File-cell lookup", so this module never
/// emits an error byte stream — a wrong path is just `None`.
pub fn render_proc_file(path: &str) -> Option<Vec<u8>> {
    match path {
        "/proc/cpuinfo" => Some(cpuinfo::render_cpuinfo()),
        "/proc/meminfo" => Some(meminfo::render_meminfo()),
        _ => None,
    }
}

/// Stable list of synthetic `/proc/*` paths this module recognises.
/// Useful for the top-level `synthetic_fs::resolve` to enumerate the
/// table in tests + for a future `readdir` over `/proc`.
pub const PATHS: &[&str] = &["/proc/cpuinfo", "/proc/meminfo"];

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
    fn cpuinfo_path_returns_some_bytes() {
        let bytes = render_proc_file("/proc/cpuinfo").expect("cpuinfo");
        let s = core::str::from_utf8(&bytes).unwrap();
        assert!(s.starts_with("processor\t: 0\n"));
    }

    #[test]
    fn meminfo_path_returns_some_bytes() {
        let bytes = render_proc_file("/proc/meminfo").expect("meminfo");
        let s = core::str::from_utf8(&bytes).unwrap();
        // MemTotal is always the first line.
        assert!(s.starts_with("MemTotal:"));
    }

    #[test]
    fn unknown_proc_path_returns_none() {
        // /proc/uptime is a real Linux file but we don't model it yet —
        // resolver should pass through so the File-cell lookup gets
        // a chance.
        assert!(render_proc_file("/proc/uptime").is_none());
        assert!(render_proc_file("/proc/stat").is_none());
        // Per-pid paths are #535's territory — pass through.
        assert!(render_proc_file("/proc/1/cmdline").is_none());
    }

    #[test]
    fn non_proc_path_returns_none() {
        // Paths that do not begin with `/proc/` should never be matched
        // by this dispatcher — its scope is the `/proc/` subtree only.
        assert!(render_proc_file("/").is_none());
        assert!(render_proc_file("/etc/passwd").is_none());
        assert!(render_proc_file("/proc").is_none());        // sans trailing slash
        assert!(render_proc_file("/procx/cpuinfo").is_none()); // typo guard
    }

    #[test]
    fn empty_path_returns_none() {
        assert!(render_proc_file("").is_none());
    }

    #[test]
    fn paths_table_lists_modelled_entries() {
        // Sanity check that the public table includes everything the
        // dispatcher matches — and nothing else. Future tracks update
        // both PATHS and the match arm; this keeps them in sync.
        assert_eq!(PATHS.len(), 2);
        for p in PATHS {
            assert!(
                render_proc_file(p).is_some(),
                "PATHS lists `{}` but dispatcher returns None",
                p,
            );
        }
    }
}

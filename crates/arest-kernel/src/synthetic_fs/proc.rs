// crates/arest-kernel/src/synthetic_fs/proc.rs
//
// `/proc/*` path resolver (#534, #475a; extended by #535, #475b).
// Single entry point — `render_proc_file(path)` — that matches a
// leading-slash path against the synthetic-file table and dispatches
// to the appropriate renderer. Returns `None` for any path that is not
// a recognised `/proc/*` entry, letting the caller fall back to the
// regular File-cell lookup chain.
//
// Why a separate dispatcher rather than dispatching inline in `mod.rs`
// --------------------------------------------------------------------
// Future tracks (#536 `/sys/class`, #537 `/dev/null` etc) will add
// their own dispatchers, each handling its own subtree. Keeping the
// per-subtree dispatch out of `mod.rs` means the top-level resolver
// can stay a single match-on-prefix that hands off to the right
// submodule. This file owns the `/proc/` subtree only.
//
// What the dispatcher matches today
// ---------------------------------
//   * `/proc/cpuinfo`               → `cpuinfo::render_cpuinfo()`
//   * `/proc/meminfo`               → `meminfo::render_meminfo()`
//   * `/proc/<pid>/<entry>`         → `proc_pid::render(pid, entry)`
//                                     where pid is decimal-numeric
//   * `/proc/self/<entry>`          → `proc_pid::render(self_pid,
//                                     entry)` where self_pid =
//                                     `process::current_process_id()`.
//
// The `<entry>` covers `comm`, `stat`, `status`, `cmdline`, `maps`, and
// per-fd `fd/<n>` projections — see `proc_pid::render_with_snapshot`
// for the dispatch table.
//
// Anything else under `/proc/` (e.g. `/proc/uptime`, `/proc/stat`)
// returns `None` until its track lands. The fallback path in the
// caller treats `None` as "not a synthetic file" and routes to the
// regular File-cell lookup.

use alloc::vec::Vec;

use super::{cpuinfo, meminfo, proc_pid};
// Reach the per-process accessor through its full path rather than the
// re-export at `crate::process::current_process_id` because the
// re-export sits in `crate::process::mod.rs` which other tracks
// (#544 futex) are concurrently editing — racing on the same `pub
// use process::{...}` line would re-introduce the race that CLAUDE.md
// flags. The full path bypasses the shared re-export entirely.
use crate::process::process::current_process_id;

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
        "/proc/cpuinfo" => return Some(cpuinfo::render_cpuinfo()),
        "/proc/meminfo" => return Some(meminfo::render_meminfo()),
        _ => {}
    }
    // Per-pid projection — `/proc/<pid>/<entry>` or `/proc/self/<entry>`.
    // Strip the `/proc/` prefix, then split into (`<pid-or-self>`,
    // `<entry>`) on the first `/`. Anything that doesn't have two
    // components after the prefix is not a per-pid projection.
    let rest = path.strip_prefix("/proc/")?;
    let (pid_part, entry) = rest.split_once('/')?;
    let pid = resolve_pid(pid_part)?;
    proc_pid::render(pid, entry)
}

/// Resolve the `<pid-or-self>` path component to a numeric pid. Returns
/// `None` when the component is neither `self` nor a valid decimal
/// integer — the caller falls through to the regular File-cell lookup.
///
/// Linux's `/proc/self` symlink resolves to the calling thread's pid;
/// we mirror by looking up the kernel's currently-installed Process.
/// When no process is installed (kernel boot before any spawn), `self`
/// resolution returns `None` and the resolver passes through.
fn resolve_pid(part: &str) -> Option<u32> {
    if part == "self" {
        return current_process_id();
    }
    part.parse::<u32>().ok()
}

/// Stable list of synthetic `/proc/*` paths this module recognises
/// without per-pid dynamic content. Useful for the top-level
/// `synthetic_fs::resolve` to enumerate the table in tests + for a
/// future `readdir` over `/proc`. The per-pid entries are not enumerated
/// here because their pid component is dynamic — a future readdir lookup
/// over `/proc` will walk the live process table.
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
    use alloc::vec;
    use crate::process::address_space::AddressSpace;
    use crate::process::process::{current_process_install, current_process_uninstall, Process};

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
        // Sanity check that the public table includes the kernel-wide
        // entries the dispatcher matches without dynamic content. The
        // per-pid entries are not in the table because their pid
        // component is dynamic.
        assert_eq!(PATHS.len(), 2);
        for p in PATHS {
            assert!(
                render_proc_file(p).is_some(),
                "PATHS lists `{}` but dispatcher returns None",
                p,
            );
        }
    }

    /// `/proc/<pid>/comm` resolves against the currently-installed
    /// Process. Install a process with a known argv, render the comm
    /// path, verify the bytes are the basename of argv[0].
    #[test]
    fn per_pid_comm_resolves_against_current_process() {
        let address_space = AddressSpace::new(0x40_1000);
        let mut proc = Process::new(7, address_space);
        proc.argv = vec![b"/bin/sh".to_vec()];
        current_process_install(proc);

        let bytes =
            render_proc_file("/proc/7/comm").expect("per-pid comm should resolve");
        assert_eq!(bytes, b"sh\n");

        current_process_uninstall();
    }

    /// `/proc/self/comm` resolves the `self` symlink to the kernel's
    /// currently-installed Process pid, then projects the comm.
    #[test]
    fn per_pid_self_resolves_to_current_process() {
        let address_space = AddressSpace::new(0x40_1000);
        let mut proc = Process::new(13, address_space);
        proc.argv = vec![b"/bin/true".to_vec()];
        current_process_install(proc);

        let bytes =
            render_proc_file("/proc/self/comm").expect("self/comm should resolve");
        assert_eq!(bytes, b"true\n");

        current_process_uninstall();
    }

    /// `/proc/<pid>/cmdline` joins the argv with NUL bytes per Linux
    /// convention and ends with a trailing NUL after the last arg.
    #[test]
    fn per_pid_cmdline_joins_argv_with_nul() {
        let address_space = AddressSpace::new(0x40_1000);
        let mut proc = Process::new(42, address_space);
        proc.argv = vec![b"/bin/sh".to_vec(), b"-c".to_vec(), b"echo".to_vec()];
        current_process_install(proc);

        let bytes =
            render_proc_file("/proc/42/cmdline").expect("cmdline should resolve");
        assert_eq!(bytes, b"/bin/sh\0-c\0echo\0");

        current_process_uninstall();
    }

    /// `/proc/<pid>/fd/0` projects the seeded stdin slot to
    /// `/dev/console` (the kernel's serial console) per Linux's early-
    /// init convention.
    #[test]
    fn per_pid_fd_zero_projects_to_dev_console() {
        let address_space = AddressSpace::new(0x40_1000);
        let proc = Process::new(99, address_space);
        current_process_install(proc);

        let bytes =
            render_proc_file("/proc/99/fd/0").expect("fd/0 should resolve");
        assert_eq!(bytes, b"/dev/console");

        current_process_uninstall();
    }

    /// `/proc/<pid>/<unknown>` returns `None` — the resolver passes
    /// through to the File-cell lookup so an unmodelled per-pid entry
    /// doesn't shadow a real file.
    #[test]
    fn per_pid_unknown_entry_returns_none() {
        let address_space = AddressSpace::new(0x40_1000);
        let proc = Process::new(99, address_space);
        current_process_install(proc);

        assert!(render_proc_file("/proc/99/limits").is_none());
        assert!(render_proc_file("/proc/99/io").is_none());

        current_process_uninstall();
    }

    /// `/proc/<unknown_pid>/comm` returns `None` — the pid doesn't
    /// match the currently-installed process.
    #[test]
    fn per_pid_mismatched_pid_returns_none() {
        let address_space = AddressSpace::new(0x40_1000);
        let proc = Process::new(7, address_space);
        current_process_install(proc);

        // Asking for pid 99 when only pid 7 is installed → None.
        assert!(render_proc_file("/proc/99/comm").is_none());

        current_process_uninstall();
    }

    /// `/proc/self/comm` with no process installed returns `None`. The
    /// `self` resolution looks up `current_process_id()` which is
    /// `None` when no process is live.
    #[test]
    fn per_pid_self_no_process_installed_returns_none() {
        // Make sure no process is installed (other tests may have left
        // one — defensive uninstall).
        current_process_uninstall();
        assert!(render_proc_file("/proc/self/comm").is_none());
    }
}

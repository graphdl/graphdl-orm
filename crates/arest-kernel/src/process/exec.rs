// crates/arest-kernel/src/process/exec.rs
//
// The exec glue (#525/#526 → #527): path → File fact → loaded image →
// Process → spawn. This is the "eventual `arest run <binary>` path"
// the #522 loader docs promised — the thin orchestration layer that
// connects four already-landed surfaces without owning any policy of
// its own:
//
//   1. `interp_resolve::resolve_interp_bytes_in` — path → bytes via
//      the File cell graph (despite the name it resolves ANY path the
//      same way `openat` does; the interpreter case was just its
//      first caller).
//   2. `elf::parse` + `elf::load_program` — bytes → `LoadedImage`,
//      with the interpreter resolver closing over the same state so a
//      PT_INTERP program (none of tier-1's static binaries, but the
//      surface is uniform) resolves through the same File facts.
//   3. `Process::new` / `Process::from_dynamic_image` — the two
//      `LoadedImage` arms, exactly as documented on the enum.
//   4. `Process::spawn` — argv/envp/auxv stack build + ring-3
//      trampoline (diverges on UEFI success; `SpawnFailed` with a
//      preserved, inspectable stack on hosts).
//
// Split shape (mirrors openat/interp_resolve): `prepare_process_in`
// is the pure-state half — fully host-testable, returns the built
// `Process` BEFORE any spawn. `exec_path` is the live-SYSTEM wrapper
// that prepares and immediately spawns.

use alloc::vec::Vec;
use arest::ast::Object;

use super::elf::{self, LoadOrParseError, LoadedImage};
use super::interp_resolve::resolve_interp_bytes_in;
use super::process::{Process, SpawnError};

/// Why an exec attempt failed before (or at) the spawn step.
#[derive(Debug, PartialEq, Eq)]
pub enum ExecError {
    /// The path names no readable File fact (no `File_has_Name` match,
    /// or its `File_has_ContentRef` did not decode).
    FileNotFound,
    /// The bytes are not a loadable ELF (parse or segment placement).
    Load(LoadOrParseError),
    /// The image loaded but the spawn failed (stack build, or — on
    /// host targets — the trampoline's structural `NotYetImplemented`).
    Spawn(SpawnError),
}

impl From<LoadOrParseError> for ExecError {
    fn from(e: LoadOrParseError) -> Self {
        ExecError::Load(e)
    }
}

impl From<elf::ElfError> for ExecError {
    fn from(e: elf::ElfError) -> Self {
        ExecError::Load(LoadOrParseError::from(e))
    }
}

/// Pure-state exec preparation: resolve `path` against `state`'s File
/// cell graph, parse + load the ELF (resolving any PT_INTERP through
/// the same state), and build the `Process` with `pid`.
///
/// No spawn happens — the returned `Process` is in `Created` state
/// with its address space populated; the caller decides when (and
/// whether) to `spawn`. This is the host-testable half.
pub fn prepare_process_in(
    path: &str,
    pid: u32,
    state: &Object,
) -> Result<Process, ExecError> {
    let program_bytes =
        resolve_interp_bytes_in(path.as_bytes(), state).ok_or(ExecError::FileNotFound)?;
    let parsed = elf::parse(&program_bytes)?;
    let image = elf::load_program(&parsed, &program_bytes, |interp_path| {
        resolve_interp_bytes_in(interp_path, state)
    })?;
    Ok(match image {
        LoadedImage::Static(address_space) => Process::new(pid, address_space),
        LoadedImage::Dynamic(dynamic) => Process::from_dynamic_image(pid, dynamic),
    })
}

/// Resolve `path` in the LIVE SYSTEM state, build the Process, and
/// spawn it with `argv`/`envp`.
///
/// On UEFI x86_64 a successful spawn DIVERGES into ring 3 (this
/// function never returns); every reachable return is therefore an
/// error: resolution/load failures before the spawn, or the spawn
/// error itself (on hosts, structurally `NotYetImplemented`). The
/// failed `Process` — preserved argv, populated initial stack — is
/// returned alongside the error so callers (REPL, tests) can inspect
/// or report.
pub fn exec_path(
    path: &str,
    pid: u32,
    argv: &[&[u8]],
    envp: &[&[u8]],
) -> Result<(), (ExecError, Option<Process>)> {
    let prepared = crate::system::with_state(|state| prepare_process_in(path, pid, state));
    let mut process = match prepared {
        Some(Ok(p)) => p,
        Some(Err(e)) => return Err((e, None)),
        None => return Err((ExecError::FileNotFound, None)),
    };
    match process.spawn(argv, envp) {
        // Structurally impossible on UEFI (spawn diverges); kept for
        // signature honesty on hosts.
        Ok(()) => Ok(()),
        Err(e) => Err((ExecError::Spawn(e), Some(process))),
    }
}

/// argv helper for multi-call binaries: busybox dispatches on argv[0],
/// so "run `ls /` from /bin/busybox" is argv = ["ls", "/"] — the
/// busybox path appears nowhere in argv. Convenience for REPL callers
/// building byte-slice argv out of a whitespace-split command line.
pub fn argv_from_words(words: &[&str]) -> Vec<Vec<u8>> {
    words.iter().map(|w| w.as_bytes().to_vec()).collect()
}

/// The REPL `run` command (#527): parse the words after `run`, exec,
/// and render the outcome as a printable report.
///
/// Path rule:
///   * `run /path/bin args…` — leading `/` is an explicit File path;
///     argv is the words verbatim (argv[0] = the path, whose basename
///     busybox's dispatcher inspects).
///   * `run ls /` / `run busybox ls /` — no leading `/` means the
///     busybox multi-call form: path is `/bin/busybox`, argv is the
///     words verbatim so argv[0] names the applet (or `busybox`, whose
///     dispatcher then reads argv[1]).
///
/// On UEFI a successful exec DIVERGES into ring 3 — the report string
/// is only ever produced for failures (and on host targets, where the
/// trampoline structurally refuses). Tier-1 has no scheduler: a guest
/// that exits halts the machine (`syscall::exit`), so "return to the
/// prompt after the program ran" is #530 follow-on work, not this arm.
pub fn run_command(words: &[&str]) -> alloc::string::String {
    use alloc::format;
    if words.is_empty() {
        return "usage: run <applet|/path> [args…]\n\
                e.g.  run ls /        (busybox applet form)\n\
                      run /bin/busybox sh"
            .into();
    }
    let path: alloc::string::String = if words[0].starts_with('/') {
        words[0].into()
    } else {
        "/bin/busybox".into()
    };
    let argv_owned = argv_from_words(words);
    let argv: Vec<&[u8]> = argv_owned.iter().map(|v| v.as_slice()).collect();
    // Minimal environment: a PATH so shell builtins that re-exec
    // (`command -p`, scripts) resolve applets back through /bin.
    let envp: &[&[u8]] = &[b"PATH=/bin", b"HOME=/"];
    match exec_path(&path, 1, &argv, envp) {
        Ok(()) => format!("exec {path}: returned to kernel (unexpected)"),
        Err((e, _process)) => format!("exec {path} failed: {e:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arest::ast::Object;

    /// A path with no File fact behind it must error `FileNotFound`,
    /// not panic or misreport a load error.
    #[test]
    fn exec_missing_path_is_file_not_found() {
        // (`let Err(..) = .. else` rather than `expect_err`: `Process`
        // deliberately has no Debug impl.)
        let Err(err) = prepare_process_in("/bin/no-such-binary", 1, &Object::phi()) else {
            panic!("empty state must not resolve any path");
        };
        assert_eq!(err, ExecError::FileNotFound);
    }

    /// Garbage bytes behind a File fact must surface as a Load error.
    #[test]
    fn exec_non_elf_bytes_is_load_error() {
        use arest::ast::{cell_push, fact_from_pairs};
        let cref = crate::assets::encode_inline_hex(b"definitely not an ELF");
        let mut state = cell_push(
            "File_has_Name",
            fact_from_pairs(&[("File", "junk-1"), ("Name", "/bin/junk")]),
            &Object::phi(),
        );
        state = cell_push(
            "File_has_ContentRef",
            fact_from_pairs(&[("File", "junk-1"), ("ContentRef", &cref)]),
            &state,
        );
        let Err(err) = prepare_process_in("/bin/junk", 1, &state) else {
            panic!("non-ELF bytes must not load");
        };
        assert!(matches!(err, ExecError::Load(_)), "got: {err:?}");
    }

    /// The real product test: the baked busybox ELF, seeded exactly the
    /// way boot seeds it, loads through the full parse → load_program →
    /// Process chain. busybox is statically linked, so the image is the
    /// `Static` arm — no interpreter, no load bias, entry point set.
    #[cfg(busybox_built)]
    #[test]
    fn prepare_busybox_from_seeded_state_builds_static_process() {
        use super::super::process::ProcessState;
        let state = crate::system::seed_busybox_file_cells(Object::phi());
        let process = prepare_process_in("/bin/busybox", 1, &state)
            .expect("seeded busybox must prepare");
        assert_eq!(process.pid, 1);
        assert_eq!(process.state, ProcessState::Created);
        assert_ne!(process.address_space.entry_point, 0, "entry must be set");
        assert!(
            !process.address_space.segments.is_empty(),
            "PT_LOAD segments must be mapped"
        );
        assert_eq!(
            process.interp_base, None,
            "static busybox must not co-load an interpreter"
        );
    }

    // ── #527 `run` command parse/report ─────────────────────────────

    /// Bare `run` prints usage, not an exec attempt.
    #[test]
    fn run_without_args_prints_usage() {
        let out = run_command(&[]);
        assert!(out.contains("usage"), "missing usage: {out}");
        assert!(out.contains("run "), "usage must show the form: {out}");
    }

    /// `run` against a path with no File fact reports the exec error
    /// (host tests run with no SYSTEM state installed, so resolution
    /// fails as FileNotFound — same shape as an unknown binary).
    #[test]
    fn run_unknown_binary_reports_exec_failure() {
        let out = run_command(&["/bin/definitely-not-here"]);
        assert!(out.contains("exec"), "missing exec marker: {out}");
        assert!(
            out.contains("FileNotFound"),
            "missing FileNotFound detail: {out}"
        );
    }

    /// Applet-style invocation (`run ls /`) targets /bin/busybox with
    /// argv[0] = the applet name — the multi-call contract. The exec
    /// fails on host either way (no state → FileNotFound; state
    /// installed by a sibling test's `system::init` → SpawnFailed at
    /// the trampoline), and the report must name the busybox path it
    /// resolved. Entropy fixture covers the second shape's AT_RANDOM
    /// fill.
    #[test]
    fn run_applet_style_targets_busybox() {
        use super::super::process::tests::with_deterministic_entropy;
        with_deterministic_entropy([11u8; 32], || {
            let out = run_command(&["ls", "/"]);
            assert!(
                out.contains("/bin/busybox"),
                "applet form must resolve via /bin/busybox: {out}"
            );
        });
    }

    /// Spawning the prepared busybox on a HOST target walks the whole
    /// argv/envp/auxv stack build and stops at the trampoline (hosts
    /// have no ring 3 to drop into): state = SpawnFailed, stack
    /// preserved, argv recorded — the multi-call argv[0]="ls" shape the
    /// REPL will use for `run busybox ls /`.
    #[cfg(busybox_built)]
    #[test]
    fn spawn_busybox_ls_on_host_fails_at_trampoline_with_stack_built() {
        use super::super::process::tests::with_deterministic_entropy;
        use super::super::process::ProcessState;
        // spawn's AT_RANDOM fill reads the csprng — install the same
        // deterministic source the process.rs spawn tests use (shared
        // lock serializes all entropy-touching tests).
        with_deterministic_entropy([7u8; 32], || {
            let state = crate::system::seed_busybox_file_cells(Object::phi());
            let mut process = prepare_process_in("/bin/busybox", 1, &state)
                .expect("seeded busybox must prepare");
            let argv: &[&[u8]] = &[b"ls", b"/"];
            let err = process
                .spawn(argv, &[])
                .expect_err("host spawn must stop at the trampoline");
            let _ = err; // SpawnError detail is trampoline-target-specific.
            assert_eq!(process.state, ProcessState::SpawnFailed);
            assert!(
                process.initial_stack.is_some(),
                "failed spawn must preserve the built stack for inspection"
            );
            assert_eq!(process.argv, alloc::vec![b"ls".to_vec(), b"/".to_vec()]);
        });
    }
}

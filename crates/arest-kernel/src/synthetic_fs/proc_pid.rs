// crates/arest-kernel/src/synthetic_fs/proc_pid.rs
//
// `/proc/<pid>/*` per-process projection (#535, #475b). Each per-pid
// entry is a synthetic file whose bytes derive from the Process cell
// graph + the per-process kernel state (address-space segments, fd
// table, argv, exit status). Mirrors the Linux kernel's `procfs`
// `<pid>/{comm,stat,status,cmdline,maps,fd/}` projection — same
// formats userspace tools (ps, top, gdb's `info proc`, /sbin/init's
// boot logs) parse, so a Linux binary running on AREST can read these
// files transparently.
//
// Why a separate module rather than inlining into `proc.rs`
// ---------------------------------------------------------
// `proc.rs` (HHHHH's #534) is the dispatcher for the `/proc/` subtree.
// It already owns the kernel-wide `/proc/cpuinfo` + `/proc/meminfo`
// renders; per-pid projections are a different shape (parametric on
// pid rather than a fixed path) and want their own module to keep the
// dispatcher small. Same per-subtree pattern the synthetic-fs epic
// uses across the board (#536's `/sys/class/*` will be its own module,
// #537's `/dev/*` likewise).
//
// What this module ships
// ----------------------
// Six per-pid renderers, all keyed on a `ProcPidSnapshot` value type
// constructed from the live Process cell graph (the `from_process`
// constructor walks the process's pid / argv / address_space / fd
// table into the snapshot fields):
//
//   * `comm` — single-line process name (basename of argv[0], or the
//     empty string when no spawn has populated argv yet).
//   * `stat` — kernel-format space-separated stat line. Linux exposes
//     ~50 fields; tier-1 emits the format-correct shape with placeholder
//     zeros for fields the kernel doesn't compute yet (every modern
//     stat-parsing tool — ps, top, gdb, htop — tolerates zero values
//     but bails on a malformed line, so format > data correctness).
//   * `status` — human-readable `Key:\tvalue\n` lines. Same fields as
//     `stat` but in a self-describing layout for the convenience of
//     scripts that want named fields without an awk -F.
//   * `cmdline` — argv joined by NUL bytes per Linux convention. A
//     final NUL is appended after the last arg so the whole region is
//     a NUL-terminated `char**`-shaped buffer.
//   * `maps` — address-space layout. One line per loaded segment in
//     the format `addr-addr perms offset dev inode pathname`. Userspace
//     tools (gdb, perf) walk this for symbol resolution.
//   * `fd/<n>` — symlink-style projection of one open fd. For
//     synthetic-fs entries, returns the absolute path the fd was opened
//     against. For File-cell entries, returns the cell id with a
//     `/file/<id>` prefix (so the Linux convention "the symlink target
//     looks like a path" holds). For the seeded standard streams
//     (fd 0 / 1 / 2), returns `/dev/console` (the kernel's serial
//     console — same shape Linux uses for an early-boot init).
//
// Why a snapshot rather than reading cells live
// --------------------------------------------
// Same rationale `cpuinfo` + `meminfo` use: the renderer takes its
// data via a passable value type so a host `cargo test` can construct
// a fixture without touching the global `CURRENT_PROCESS` static. The
// production wiring (the `proc::render_proc_file` dispatcher) calls
// the live-Process accessor `from_current_process`, which folds the
// live state into a snapshot inside the lock and then renders outside
// — so the lock isn't held across the alloc-heavy render path.
//
// Format-correctness over data-correctness
// ----------------------------------------
// The Linux `/proc/<pid>/stat` format is a single space-separated line
// with positional fields. Tools that consume it (ps, top, htop, glibc's
// pidfs reader) parse it positionally — the field count + the NUL
// (paren-wrapped comm) escaping matters more than the actual numeric
// values. Tier-1 emits the placeholder zeros and lets the userspace
// tools draw their progress meters as "0 ticks of CPU consumed" — which
// is right anyway because the AREST kernel has no CPU accounting yet.
// As accounting subsystems land (#530 scheduler, #531 wait/exit, etc.)
// the snapshot grows fields and the renderer fills them in without a
// format change.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
#[cfg(all(test, target_os = "linux"))]
use alloc::vec;

// Reach process-side types through their submodule paths rather than
// the `pub use process::{...}` re-export at `crate::process::mod`
// because `mod.rs`'s re-export line is concurrently edited by other
// tracks (#544 futex, etc.) and racing on it re-introduces the
// CLAUDE.md flagged "shared staging index contamination" failure
// mode. The submodule paths bypass the shared re-export entirely.
use crate::process::address_space::{LoadedSegment, SegmentPerm};
use crate::process::fd_table::FdEntry as OpenFdEntry;
use crate::process::process::{
    current_process_mut, FdEntry, Process, ProcessState,
};

/// Renderable snapshot of a single process's state. Constructed via
/// `from_process` (against a borrowed `&Process`) or via the test
/// harness's `fixture` helper. Owns its data so the renderer doesn't
/// need to re-borrow the Process across the lock release.
#[derive(Debug, Clone)]
pub struct ProcPidSnapshot {
    /// Process id — the numeric pid the `<pid>` path component decoded
    /// to. Stored as `u32` to match `Process::pid`'s width.
    pub pid: u32,
    /// Parent process id — Linux convention is 0 for `init` (pid 1)
    /// and 1 for every direct child. Tier-1 has no fork() so every
    /// process effectively has ppid 0; future #531 wait/exit lands a
    /// real parent pointer.
    pub ppid: u32,
    /// argv strings as the spawn was invoked with. Empty `Vec` if the
    /// process hasn't been spawned yet (Created state). The basename
    /// of `argv[0]` projects as `/proc/<pid>/comm`; the NUL-joined
    /// list projects as `/proc/<pid>/cmdline`.
    pub argv: Vec<Vec<u8>>,
    /// Process state — drives the `R` / `S` / `Z` / `X` letter the
    /// `stat` format emits in field 3, and the `State:` line in the
    /// `status` format.
    pub state: ProcessState,
    /// Loaded segments from the process's address space. One entry per
    /// PT_LOAD segment; the `maps` renderer walks this for the
    /// `addr-addr perms offset dev inode pathname` projection.
    pub segments: Vec<SegmentSnapshot>,
    /// Per-fd snapshot — one entry per open fd. The `fd/<n>` renderer
    /// looks up entries by fd number; the `status` renderer reports
    /// the count as `FDSize:`.
    pub fds: Vec<FdSnapshot>,
    /// Exit status the process passed to `exit(2)` / `exit_group(2)`,
    /// or `None` if the process is still live. Reported as the `stat`
    /// format's `exit_code` field (#52).
    pub exit_status: Option<i32>,
}

/// One loaded segment as the `/proc/<pid>/maps` renderer needs it —
/// just the format-relevant fields (vaddr range + perms). The full
/// `LoadedSegment` carries a `NonNull<u8>` allocation pointer that
/// the snapshot doesn't need; flattening to this minimal value type
/// keeps the snapshot `Clone` (which `LoadedSegment` is not because
/// of the raw-pointer ownership).
#[derive(Debug, Clone, Copy)]
pub struct SegmentSnapshot {
    /// Start virtual address — the `addr-` half of the maps line.
    pub vaddr_start: u64,
    /// End virtual address (exclusive) — the `-addr` half of the maps
    /// line. Equals `vaddr_start + mem_size`.
    pub vaddr_end: u64,
    /// Permission shape — projects as the four-char `r-x-` / `rw--` /
    /// `r---` string (read / write / execute / private bits) the maps
    /// line emits.
    pub perm: SegmentPerm,
}

/// One fd-table entry as the `/proc/<pid>/fd/<n>` + `/proc/<pid>/status`
/// renderers need it. `kind` carries the variant; `target` is the
/// symlink-style projection (the path `/proc/<pid>/fd/<n>` would
/// resolve to if it were a real symlink).
#[derive(Debug, Clone)]
pub struct FdSnapshot {
    /// fd number — the `<n>` in `/proc/<pid>/fd/<n>`.
    pub fd: i32,
    /// Symlink-target projection. For a synthetic-fs fd, this is the
    /// absolute path. For a File-cell fd, this is `/file/<cell_id>`.
    /// For the seeded standard streams (fd 0 / 1 / 2 backed by
    /// `FdEntry::Serial`), this is `/dev/console` per Linux convention
    /// for early-init / `getty -s` style invocations.
    pub target: String,
}

impl ProcPidSnapshot {
    /// Build a snapshot from a borrowed Process. Pure projection — the
    /// Process is only read; the snapshot owns its data so the caller
    /// can release the Process borrow before the render runs. ppid
    /// defaults to 0 (tier-1: no fork() yet — every process is its own
    /// parent's child of init).
    pub fn from_process(p: &Process) -> Self {
        let segments = p
            .address_space
            .segments
            .iter()
            .map(SegmentSnapshot::from_loaded)
            .collect();
        let fds = collect_fd_snapshots(p);
        Self {
            pid: p.pid,
            ppid: 0,
            argv: p.argv.clone(),
            state: p.state,
            segments,
            fds,
            exit_status: p.exit_status,
        }
    }

    /// Build a snapshot from the kernel's currently-installed Process,
    /// or `None` if no process is installed. The closure runs inside
    /// the `current_process_mut` lock and folds the Process into a
    /// snapshot — the snapshot is returned out so the caller can render
    /// without holding the lock.
    pub fn from_current_process() -> Option<Self> {
        current_process_mut(|maybe_proc| maybe_proc.map(|p| Self::from_process(p)))
    }

    /// Test fixture helper — construct a synthetic snapshot with the
    /// supplied pid + argv for the unit tests. Used to exercise the
    /// renderers without standing up a full Process.
    #[cfg(test)]
    pub fn fixture(pid: u32, argv: &[&[u8]]) -> Self {
        let argv_owned = argv.iter().map(|a| a.to_vec()).collect();
        Self {
            pid,
            ppid: 0,
            argv: argv_owned,
            state: ProcessState::Running,
            segments: Vec::new(),
            fds: Vec::new(),
            exit_status: None,
        }
    }
}

impl SegmentSnapshot {
    /// Project a `LoadedSegment` into the format-relevant fields the
    /// maps line needs. `vaddr_end` is `vaddr + mem_size` — the
    /// half-open upper bound matches what Linux emits in `/proc/<pid>
    /// /maps`.
    pub fn from_loaded(seg: &LoadedSegment) -> Self {
        Self {
            vaddr_start: seg.vaddr,
            vaddr_end: seg.vaddr + seg.mem_size as u64,
            perm: seg.perm,
        }
    }
}

/// Walk the Process's seeded `Vec<FdEntry>` (fds 0 / 1 / 2 — the
/// standard streams backed by serial) and the richer `FdTable` (fds ≥
/// 3 — open()ed files / synthetic entries). Produces an ordered `Vec`
/// of `FdSnapshot` keyed by fd number for the per-fd renderer + for
/// the `status` page's `FDSize:` count.
fn collect_fd_snapshots(p: &Process) -> Vec<FdSnapshot> {
    let mut out = Vec::new();
    // Seeded standard streams — Vec<FdEntry> indexed by fd.
    for (fd, entry) in p.fd_table.iter().enumerate() {
        if matches!(entry, FdEntry::Closed) {
            continue;
        }
        let target = match entry {
            FdEntry::Serial => "/dev/console".to_string(),
            FdEntry::Closed => unreachable!("Closed elided above"),
        };
        out.push(FdSnapshot {
            fd: fd as i32,
            target,
        });
    }
    // Open fds from the richer FdTable — the openat handler populates
    // these with synthetic-fs paths or File-cell ids.
    //
    // The FdTable doesn't currently expose an iterator over its
    // entries; walking the [FIRST_USER_FD, FIRST_USER_FD + len()]
    // range works because allocate() returns the lowest free fd ≥
    // FIRST_USER_FD and the table walks compact in tests. A future
    // FdTable::iter() lands as part of the unification (post-#499).
    for fd in 3..(3 + p.open_fds.len() as i32 + 64) {
        if let Some(entry) = p.open_fds.lookup(fd) {
            let target = match entry {
                OpenFdEntry::Synthetic { path } => path.clone(),
                OpenFdEntry::File { cell_id } => format!("/file/{}", cell_id),
            };
            out.push(FdSnapshot { fd, target });
        }
    }
    out
}

/// Top-level dispatcher. Given a parsed `(pid, entry)` pair (the entry
/// is the path component after `/proc/<pid>/`), return the rendered
/// bytes for that entry, or `None` if `entry` doesn't name a known
/// projection. The pid resolution (numeric vs. `self`) happens upstream
/// in `proc::render_proc_file`; this module sees the resolved numeric
/// pid only.
///
/// Returns `None` for unknown entries so the upstream resolver can
/// pass through to the regular File-cell lookup chain (mirrors the
/// `proc::render_proc_file` convention).
pub fn render(pid: u32, entry: &str) -> Option<Vec<u8>> {
    let snapshot = ProcPidSnapshot::from_current_process()?;
    if snapshot.pid != pid {
        // The path's pid doesn't match the currently-installed
        // process. Tier-1 hosts at most one process at a time so any
        // mismatch is a stale lookup — return None so the resolver
        // falls through. Once #530 lands a multi-process scheduler,
        // this branch will walk the process table.
        return None;
    }
    render_with_snapshot(&snapshot, entry)
}

/// Pure render — given a snapshot + entry name, produce the bytes.
/// Separates the lock-bound snapshot construction from the alloc-heavy
/// render so unit tests can exercise the renderers without standing up
/// the global `CURRENT_PROCESS` singleton.
pub fn render_with_snapshot(snapshot: &ProcPidSnapshot, entry: &str) -> Option<Vec<u8>> {
    // Per-fd entries (`fd/<n>`) get a prefix-stripped lookup; everything
    // else is a fixed match arm. The split keeps the per-fd rendering
    // path small (one helper).
    if let Some(rest) = entry.strip_prefix("fd/") {
        return render_fd(snapshot, rest);
    }
    match entry {
        "comm" => Some(render_comm(snapshot)),
        "stat" => Some(render_stat(snapshot)),
        "status" => Some(render_status(snapshot)),
        "cmdline" => Some(render_cmdline(snapshot)),
        "maps" => Some(render_maps(snapshot)),
        _ => None,
    }
}

/// Render `/proc/<pid>/comm` — the basename of `argv[0]` followed by a
/// single newline (Linux convention; `cat /proc/$$/comm` emits one
/// line). Returns the empty string + newline when no argv has been
/// populated yet (no spawn has run).
///
/// Linux caps comm at 16 bytes (TASK_COMM_LEN); we mirror that cap so
/// a userspace tool that allocates a 16-byte buffer to read the whole
/// thing doesn't truncate unexpectedly.
pub fn render_comm(snapshot: &ProcPidSnapshot) -> Vec<u8> {
    let mut out = comm_basename(snapshot);
    // Cap at TASK_COMM_LEN - 1 = 15 bytes, then append newline. Linux
    // includes the NUL terminator inside the 16-byte cap; the rendered
    // file uses a newline rather than a NUL because the file is
    // ASCII-line-shaped (the in-kernel TASK_COMM_LEN region is C-string
    // shaped). Both shapes net 16 bytes for a maximum-length comm.
    if out.len() > 15 {
        out.truncate(15);
    }
    out.push(b'\n');
    out
}

/// Render `/proc/<pid>/stat` — the single-line space-separated fields
/// the kernel emits for procfs's stat projection. Format reference:
/// `man 5 proc`, section `/proc/[pid]/stat`. Tier-1 emits all 52
/// fields with placeholder zeros for everything the kernel doesn't
/// track yet (utime, stime, vsize, rss, etc.); userspace tools tolerate
/// zeros but bail on a malformed line.
///
/// The `comm` field (#2) is paren-wrapped per Linux convention — `(comm
/// content)` so a tool that splits on whitespace finds the right field
/// boundaries even when the comm itself contains spaces.
pub fn render_stat(snapshot: &ProcPidSnapshot) -> Vec<u8> {
    // Field 1: pid (numeric)
    // Field 2: comm (paren-wrapped, basename of argv[0])
    // Field 3: state (single character — see state_letter)
    // Field 4: ppid
    // Fields 5-52: the rest, mostly zero placeholders. We emit the
    //   exact 52-field count Linux's procfs does so positional parsers
    //   find the field they want at the right index.
    let comm = String::from_utf8_lossy(&comm_basename(snapshot)).to_string();
    let state = state_letter(snapshot.state);
    let exit_code = snapshot.exit_status.unwrap_or(0);
    // Build the 52-field line. The format string lists every field by
    // position so a future "fill in real value here" change is a one-
    // line edit at the right offset.
    let line = format!(
        "{pid} ({comm}) {state} {ppid} {pgid} {sid} {tty_nr} {tpgid} {flags} {minflt} {cminflt} {majflt} {cmajflt} {utime} {stime} {cutime} {cstime} {priority} {nice} {num_threads} {itrealvalue} {starttime} {vsize} {rss} {rsslim} {startcode} {endcode} {startstack} {kstkesp} {kstkeip} {signal} {blocked} {sigignore} {sigcatch} {wchan} {nswap} {cnswap} {exit_signal} {processor} {rt_priority} {policy} {delayacct_blkio_ticks} {guest_time} {cguest_time} {start_data} {end_data} {start_brk} {arg_start} {arg_end} {env_start} {env_end} {exit_code}\n",
        pid = snapshot.pid,
        comm = comm,
        state = state,
        ppid = snapshot.ppid,
        pgid = 0,
        sid = 0,
        tty_nr = 0,
        tpgid = -1i32,
        flags = 0u64,
        minflt = 0u64,
        cminflt = 0u64,
        majflt = 0u64,
        cmajflt = 0u64,
        utime = 0u64,
        stime = 0u64,
        cutime = 0i64,
        cstime = 0i64,
        priority = 20i64,
        nice = 0i64,
        num_threads = 1i64,
        itrealvalue = 0i64,
        starttime = 0u64,
        vsize = vsize_total(snapshot),
        rss = rss_pages(snapshot),
        rsslim = u64::MAX,
        startcode = code_start(snapshot),
        endcode = code_end(snapshot),
        startstack = 0u64,
        kstkesp = 0u64,
        kstkeip = 0u64,
        signal = 0u64,
        blocked = 0u64,
        sigignore = 0u64,
        sigcatch = 0u64,
        wchan = 0u64,
        nswap = 0u64,
        cnswap = 0u64,
        exit_signal = 17i32,
        processor = 0i32,
        rt_priority = 0u32,
        policy = 0u32,
        delayacct_blkio_ticks = 0u64,
        guest_time = 0u64,
        cguest_time = 0i64,
        start_data = 0u64,
        end_data = 0u64,
        start_brk = 0u64,
        arg_start = 0u64,
        arg_end = 0u64,
        env_start = 0u64,
        env_end = 0u64,
        exit_code = exit_code,
    );
    line.into_bytes()
}

/// Render `/proc/<pid>/status` — the human-readable per-key key:value
/// pairs Linux's procfs emits as a sibling of `stat`. Same data, line-
/// per-key layout for scripts that want named fields. Tier-1 emits
/// the headline fields (Name / State / Pid / PPid / FDSize / VmSize)
/// with the same placeholder-zero discipline as `stat`.
///
/// The Linux format uses tab-aligned columns (`Name:\tvalue\n`); we
/// reproduce that exactly — userspace tools (htop's status reader,
/// glibc's pidfs) split on `:\t` so the tab discipline must hold.
pub fn render_status(snapshot: &ProcPidSnapshot) -> Vec<u8> {
    let mut out = String::with_capacity(512);
    let comm = String::from_utf8_lossy(&comm_basename(snapshot)).to_string();
    let state_letter = state_letter(snapshot.state);
    let state_word = state_word(snapshot.state);
    push_kv(&mut out, "Name", &comm);
    push_kv(
        &mut out,
        "State",
        &format!("{} ({})", state_letter, state_word),
    );
    push_kv(&mut out, "Tgid", &format!("{}", snapshot.pid));
    push_kv(&mut out, "Pid", &format!("{}", snapshot.pid));
    push_kv(&mut out, "PPid", &format!("{}", snapshot.ppid));
    push_kv(&mut out, "TracerPid", "0");
    push_kv(&mut out, "Uid", "0\t0\t0\t0");
    push_kv(&mut out, "Gid", "0\t0\t0\t0");
    push_kv(&mut out, "FDSize", &format!("{}", snapshot.fds.len()));
    push_kv(&mut out, "VmSize", &format!("{} kB", vsize_total(snapshot) / 1024));
    push_kv(&mut out, "VmRSS", &format!("{} kB", rss_pages(snapshot) * 4));
    push_kv(&mut out, "Threads", "1");
    out.into_bytes()
}

/// Render `/proc/<pid>/cmdline` — argv joined by NUL bytes plus a
/// trailing NUL after the last arg. Returns the empty byte string when
/// the process hasn't been spawned yet (no argv populated).
///
/// Linux's `procfs` cmdline is the raw bytes of argv with a NUL
/// separator between args and a NUL terminator. Userspace tools (ps,
/// `cat /proc/$$/cmdline | tr '\0' ' '`) walk this format expecting
/// every string to be NUL-terminated; the trailing NUL keeps the
/// invariant for the last entry.
pub fn render_cmdline(snapshot: &ProcPidSnapshot) -> Vec<u8> {
    if snapshot.argv.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(64);
    for arg in &snapshot.argv {
        out.extend_from_slice(arg);
        out.push(0);
    }
    out
}

/// Render `/proc/<pid>/maps` — one line per loaded segment in the
/// format `addr-addr perms offset dev inode pathname`. Userspace tools
/// (gdb's `info proc mappings`, perf's symbolizer) walk this for
/// virtual-address → segment translation.
///
/// Tier-1 reports offset / dev / inode as `00000000 00:00 0` (no
/// underlying file system; the segments come from the in-memory ELF
/// load). The pathname is `[anon]` for the same reason — once #560's
/// VFS lands, the loader can record the source ELF path and project
/// it here.
pub fn render_maps(snapshot: &ProcPidSnapshot) -> Vec<u8> {
    let mut out = String::with_capacity(snapshot.segments.len() * 80);
    for seg in &snapshot.segments {
        let perms = perm_string(seg.perm);
        // Linux format: lower-case hex addresses with no `0x` prefix.
        // The trailing `[anon]` is the conventional pathname for an
        // anonymous (file-backed-but-loader-private) mapping.
        out.push_str(&format!(
            "{:x}-{:x} {} 00000000 00:00 0                          [anon]\n",
            seg.vaddr_start, seg.vaddr_end, perms
        ));
    }
    out.into_bytes()
}

/// Render `/proc/<pid>/fd/<n>` for a specific fd. Returns the symlink-
/// style target as the file's bytes — Linux exposes fd entries as
/// symlinks, but a `read()` on a symlink (without `O_PATH`) returns
/// the target string; we project the target as the file's contents
/// so a userspace tool that does `readlink /proc/<pid>/fd/<n>` finds
/// the same string by reading the file.
///
/// Returns `None` for unknown fds — the upstream resolver maps that
/// to the regular File-cell fallback (which will return `-ENOENT` for
/// any non-existent path).
pub fn render_fd(snapshot: &ProcPidSnapshot, fd_str: &str) -> Option<Vec<u8>> {
    let fd: i32 = fd_str.parse().ok()?;
    snapshot
        .fds
        .iter()
        .find(|f| f.fd == fd)
        .map(|f| f.target.as_bytes().to_vec())
}

// -- helpers -----------------------------------------------------------

/// Extract the basename of `argv[0]` — everything after the last `/`.
/// Returns the empty `Vec` when argv is empty (no spawn has run). Used
/// by `comm` (the file's bytes) + `stat` field 2 (the paren-wrapped
/// comm) + `status` Name: line.
fn comm_basename(snapshot: &ProcPidSnapshot) -> Vec<u8> {
    let arg0 = match snapshot.argv.first() {
        Some(a) => a,
        None => return Vec::new(),
    };
    // Walk backward to the last `/`; if none, the whole arg0 is the
    // basename. byte-level so non-UTF-8 paths project cleanly.
    match arg0.iter().rposition(|&b| b == b'/') {
        Some(idx) => arg0[idx + 1..].to_vec(),
        None => arg0.clone(),
    }
}

/// Map `ProcessState` to the single-character state letter Linux's
/// procfs emits in the `stat` format's field 3.
///
///   * R = Running (or runnable)
///   * S = Sleeping (interruptible wait)
///   * D = Disk sleep (uninterruptible wait — futex park is the only
///         tier-1 producer)
///   * Z = Zombie (exited but not yet reaped)
///   * X = Dead (the failed-spawn state — Linux uses X for tasks the
///         kernel killed before they could run)
fn state_letter(state: ProcessState) -> char {
    match state {
        ProcessState::Created => 'R',
        ProcessState::Running => 'R',
        ProcessState::SpawnFailed => 'X',
        ProcessState::Exited => 'Z',
        ProcessState::BlockedFutex(_) => 'D',
    }
}

/// Map `ProcessState` to the human-readable word Linux's procfs emits
/// inside the `State:` line of the `status` format. Mirrors
/// `state_letter` but spells the variant out.
fn state_word(state: ProcessState) -> &'static str {
    match state {
        ProcessState::Created => "running",
        ProcessState::Running => "running",
        ProcessState::SpawnFailed => "dead",
        ProcessState::Exited => "zombie",
        ProcessState::BlockedFutex(_) => "disk sleep",
    }
}

/// Render a `SegmentPerm` as the four-character `r-x-` / `rw--` /
/// `r---` string Linux's `/proc/<pid>/maps` format uses. The fourth
/// character is private/shared (`p` / `s`); tier-1 segments are all
/// process-private so we always emit `p`.
fn perm_string(perm: SegmentPerm) -> &'static str {
    match perm {
        SegmentPerm::Read => "r--p",
        SegmentPerm::ReadWrite => "rw-p",
        SegmentPerm::ReadExecute => "r-xp",
    }
}

/// Total virtual address-space size in bytes — sum of every segment's
/// `mem_size`. Reported as the `VmSize:` line in `status` (in kB) and
/// the `vsize` field in `stat` (in bytes).
fn vsize_total(snapshot: &ProcPidSnapshot) -> u64 {
    snapshot
        .segments
        .iter()
        .map(|s| s.vaddr_end - s.vaddr_start)
        .sum()
}

/// Resident-set size in 4 KiB pages — the page-resident portion of
/// vsize. Tier-1 has no page-out so every loaded segment is resident;
/// we approximate by walking the same segments as `vsize_total` and
/// dividing by PAGE_SIZE.
fn rss_pages(snapshot: &ProcPidSnapshot) -> u64 {
    vsize_total(snapshot) / 4096
}

/// First segment's `vaddr_start` — the conventional `startcode` value
/// the `stat` format expects (the start of the text segment). Returns
/// 0 when no segments exist.
fn code_start(snapshot: &ProcPidSnapshot) -> u64 {
    snapshot.segments.first().map(|s| s.vaddr_start).unwrap_or(0)
}

/// First segment's `vaddr_end` — paired with `code_start` for the
/// `endcode` field in `stat`. Approximation; once segment metadata
/// records the segment's role (text vs. data vs. bss), this can pick
/// the text segment specifically.
fn code_end(snapshot: &ProcPidSnapshot) -> u64 {
    snapshot.segments.first().map(|s| s.vaddr_end).unwrap_or(0)
}

/// Append a tab-aligned `Key:\tvalue\n` line to `out`. Linux's
/// procfs status format uses tab-alignment so `htop`'s parser (which
/// splits on `:\t`) finds the value half cleanly.
fn push_kv(out: &mut String, key: &str, value: &str) {
    out.push_str(key);
    out.push(':');
    out.push('\t');
    out.push_str(value);
    out.push('\n');
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
    fn render_comm_basename_strips_path() {
        let snap = ProcPidSnapshot::fixture(7, &[b"/usr/bin/sh", b"-i"]);
        let bytes = render_comm(&snap);
        assert_eq!(bytes, b"sh\n");
    }

    #[test]
    fn render_comm_no_path_separator_returns_arg0() {
        let snap = ProcPidSnapshot::fixture(7, &[b"true"]);
        let bytes = render_comm(&snap);
        assert_eq!(bytes, b"true\n");
    }

    #[test]
    fn render_comm_empty_argv_returns_just_newline() {
        let snap = ProcPidSnapshot::fixture(7, &[]);
        let bytes = render_comm(&snap);
        assert_eq!(bytes, b"\n");
    }

    #[test]
    fn render_comm_truncates_at_15_bytes() {
        let snap = ProcPidSnapshot::fixture(
            7,
            &[b"/bin/this_name_is_far_too_long_for_the_cap"],
        );
        let bytes = render_comm(&snap);
        // 15 bytes of name + 1 newline = 16 bytes total (TASK_COMM_LEN).
        assert_eq!(bytes.len(), 16);
        assert!(bytes.ends_with(b"\n"));
    }

    #[test]
    fn render_cmdline_joins_argv_with_nul() {
        let snap = ProcPidSnapshot::fixture(7, &[b"/bin/sh", b"-c", b"echo hi"]);
        let bytes = render_cmdline(&snap);
        // `/bin/sh\0-c\0echo hi\0`
        assert_eq!(bytes, b"/bin/sh\0-c\0echo hi\0");
    }

    #[test]
    fn render_cmdline_empty_argv_returns_empty_bytes() {
        let snap = ProcPidSnapshot::fixture(7, &[]);
        let bytes = render_cmdline(&snap);
        assert!(bytes.is_empty());
    }

    #[test]
    fn render_cmdline_single_arg_terminates_with_nul() {
        let snap = ProcPidSnapshot::fixture(7, &[b"/bin/true"]);
        let bytes = render_cmdline(&snap);
        assert_eq!(bytes, b"/bin/true\0");
    }

    #[test]
    fn render_stat_starts_with_pid_and_paren_wrapped_comm() {
        let snap = ProcPidSnapshot::fixture(42, &[b"/bin/sh"]);
        let bytes = render_stat(&snap);
        let s = core::str::from_utf8(&bytes).unwrap();
        assert!(s.starts_with("42 (sh) R "), "got: {}", s);
    }

    #[test]
    fn render_stat_emits_52_fields_separated_by_spaces() {
        let snap = ProcPidSnapshot::fixture(42, &[b"/bin/sh"]);
        let bytes = render_stat(&snap);
        let s = core::str::from_utf8(&bytes).unwrap();
        // Trim the trailing newline before counting fields.
        let trimmed = s.trim_end();
        let fields: Vec<&str> = trimmed.split_whitespace().collect();
        assert_eq!(
            fields.len(),
            52,
            "expected 52 stat fields, got {}: {}",
            fields.len(),
            trimmed
        );
    }

    #[test]
    fn render_stat_state_letter_reflects_state() {
        let mut snap = ProcPidSnapshot::fixture(1, &[b"sh"]);
        snap.state = ProcessState::Exited;
        let bytes = render_stat(&snap);
        let s = core::str::from_utf8(&bytes).unwrap();
        // Field 3 is the state letter — Z for zombie/exited.
        assert!(s.contains(" Z "), "got: {}", s);
    }

    #[test]
    fn render_status_emits_named_fields() {
        let snap = ProcPidSnapshot::fixture(42, &[b"/bin/sh", b"-i"]);
        let bytes = render_status(&snap);
        let s = core::str::from_utf8(&bytes).unwrap();
        assert!(s.contains("Name:\tsh\n"), "got: {}", s);
        assert!(s.contains("Pid:\t42\n"), "got: {}", s);
        assert!(s.contains("PPid:\t0\n"), "got: {}", s);
        assert!(s.contains("State:\tR (running)\n"), "got: {}", s);
        assert!(s.contains("Threads:\t1\n"), "got: {}", s);
    }

    #[test]
    fn render_maps_emits_per_segment_lines() {
        let mut snap = ProcPidSnapshot::fixture(1, &[b"sh"]);
        snap.segments.push(SegmentSnapshot {
            vaddr_start: 0x40_0000,
            vaddr_end: 0x40_1000,
            perm: SegmentPerm::ReadExecute,
        });
        snap.segments.push(SegmentSnapshot {
            vaddr_start: 0x60_0000,
            vaddr_end: 0x60_2000,
            perm: SegmentPerm::ReadWrite,
        });
        let bytes = render_maps(&snap);
        let s = core::str::from_utf8(&bytes).unwrap();
        assert!(
            s.contains("400000-401000 r-xp"),
            "got: {}",
            s
        );
        assert!(
            s.contains("600000-602000 rw-p"),
            "got: {}",
            s
        );
        assert!(s.contains("[anon]"));
        // One line per segment.
        assert_eq!(s.lines().count(), 2);
    }

    #[test]
    fn render_maps_empty_snapshot_renders_empty_bytes() {
        let snap = ProcPidSnapshot::fixture(1, &[b"sh"]);
        let bytes = render_maps(&snap);
        assert!(bytes.is_empty());
    }

    #[test]
    fn render_fd_synthetic_returns_path() {
        let mut snap = ProcPidSnapshot::fixture(1, &[b"sh"]);
        snap.fds.push(FdSnapshot {
            fd: 3,
            target: "/proc/cpuinfo".to_string(),
        });
        let bytes = render_fd(&snap, "3").unwrap();
        assert_eq!(bytes, b"/proc/cpuinfo");
    }

    #[test]
    fn render_fd_unknown_returns_none() {
        let snap = ProcPidSnapshot::fixture(1, &[b"sh"]);
        assert!(render_fd(&snap, "99").is_none());
        // Non-numeric is also `None` — the parse failure short-circuits.
        assert!(render_fd(&snap, "abc").is_none());
    }

    #[test]
    fn render_fd_seeded_serial_returns_dev_console() {
        let mut snap = ProcPidSnapshot::fixture(1, &[b"sh"]);
        snap.fds.push(FdSnapshot {
            fd: 1,
            target: "/dev/console".to_string(),
        });
        let bytes = render_fd(&snap, "1").unwrap();
        assert_eq!(bytes, b"/dev/console");
    }

    #[test]
    fn render_with_snapshot_dispatches_known_entries() {
        let snap = ProcPidSnapshot::fixture(7, &[b"/bin/sh"]);
        assert!(render_with_snapshot(&snap, "comm").is_some());
        assert!(render_with_snapshot(&snap, "stat").is_some());
        assert!(render_with_snapshot(&snap, "status").is_some());
        assert!(render_with_snapshot(&snap, "cmdline").is_some());
        assert!(render_with_snapshot(&snap, "maps").is_some());
        // Unknown entries return None so the upstream resolver can fall
        // through to the regular File-cell lookup.
        assert!(render_with_snapshot(&snap, "limits").is_none());
        assert!(render_with_snapshot(&snap, "").is_none());
    }

    #[test]
    fn from_process_projects_pid_and_argv() {
        use crate::process::address_space::AddressSpace;
        let address_space = AddressSpace::new(0x40_1000);
        let mut proc = Process::new(99, address_space);
        proc.argv = vec![b"/bin/sh".to_vec(), b"-c".to_vec()];
        let snap = ProcPidSnapshot::from_process(&proc);
        assert_eq!(snap.pid, 99);
        assert_eq!(snap.argv.len(), 2);
        assert_eq!(snap.argv[0], b"/bin/sh");
        assert_eq!(snap.argv[1], b"-c");
    }

    #[test]
    fn from_process_projects_segments() {
        use crate::process::address_space::{AddressSpace, SegmentPerm};
        let mut address_space = AddressSpace::new(0x40_1000);
        address_space
            .push_segment(0x40_1000, 0x10, SegmentPerm::ReadExecute, &[0x90; 8])
            .expect("text push");
        let proc = Process::new(99, address_space);
        let snap = ProcPidSnapshot::from_process(&proc);
        assert_eq!(snap.segments.len(), 1);
        assert_eq!(snap.segments[0].vaddr_start, 0x40_1000);
        assert_eq!(snap.segments[0].vaddr_end, 0x40_1000 + 0x10);
    }

    #[test]
    fn from_process_projects_seeded_fd_table_to_dev_console() {
        use crate::process::address_space::AddressSpace;
        let address_space = AddressSpace::new(0x40_1000);
        let proc = Process::new(99, address_space);
        let snap = ProcPidSnapshot::from_process(&proc);
        // Three seeded FdEntry::Serial entries → fds 0 / 1 / 2 all
        // project to /dev/console.
        assert_eq!(snap.fds.len(), 3);
        for (i, f) in snap.fds.iter().enumerate() {
            assert_eq!(f.fd, i as i32);
            assert_eq!(f.target, "/dev/console");
        }
    }
}

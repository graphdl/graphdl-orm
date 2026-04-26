// crates/arest-kernel/src/process/process.rs
//
// Process — the holding object for a freshly-spawned Linux binary.
// Owns its address space (the loaded PT_LOAD segments), its initial
// stack (argv/envp/auxv populated per System V AMD64 PSABI), and the
// per-process state (pid, fd table seed) the syscall surface (#473)
// will read from on the first syscall. The third leg of the #521
// spawn pipeline that the address-space loader (#519) and the stack
// builder (`process::stack`, this commit) feed.
//
// What `Process::new` does
// ------------------------
// Pure construction — takes an already-loaded `AddressSpace` plus a
// pid and produces a `Process` with a default fd_table (stdin/stdout/
// stderr seeded from the kernel's serial console) and `state =
// Created`. The actual stack allocation + entry-point invoke happens
// in `Process::spawn`, which composes the new constructor with the
// `StackBuilder` + `trampoline::invoke` calls.
//
// What `Process::spawn` does
// --------------------------
// Atomic: allocate the initial stack page, populate argv / envp / the
// minimum auxv set (AT_RANDOM / AT_PHDR / AT_PHENT / AT_PHNUM /
// AT_PAGESZ / AT_ENTRY / AT_NULL terminator), call `trampoline::invoke`
// to flip CPL bits and jump to e_entry. Returns the Process struct
// in `Running` state on success, drops it on `Err`.
//
// For tier-1, `trampoline::invoke` returns `NotYetImplemented` because
// the GDT/TSS scaffolding isn't yet there (#526) and there's no real
// page-table install (#527). The Process struct + the spawn
// orchestration land here so the next two slices can drop in without
// reshaping the call site.
//
// Why no Cell-recording on the Process
// ------------------------------------
// `AddressSpace::record_into_cells` already emits the `Process_has_*`
// cells the system::apply consumer wants; the Process struct itself
// is a kernel-side object (it owns hardware-backed state — heap
// pages, eventually a CR3 / TTBR0 value) that doesn't have a clean
// `Object` representation. The Process is what writes INTO the cell
// store via `AddressSpace::record_into_cells`; it's not itself a
// fact. Same shape as `crate::block::Disk` (kernel-side resource —
// no cell projection of its own).
//
// pid allocation
// --------------
// Tier-1 takes the pid as a constructor parameter — the caller
// (whoever wires `arest run <binary>` into the kernel REPL) picks
// the next free pid. A central pid allocator with O(1) reuse lands
// when the scheduler does (#530). For now any monotonically-
// increasing u32 works.

use alloc::format;
use alloc::vec::Vec;
use arest::ast::{cell_push, fact_from_pairs, Object};

use super::address_space::AddressSpace;
use super::elf::ELF64_PHENT_SIZE;
use super::fd_table::FdTable;
use super::stack::{AuxvEntry, AuxvType, InitialStack, StackBuilder, StackError};
use super::trampoline::{self, TrampolineError};

/// 16 bytes of CSPRNG output the auxv `AT_RANDOM` pointer references.
/// Tier-1 uses a deterministic placeholder — every spawn gets the same
/// 16 bytes — until #524's CSPRNG lands. libc tolerates any 16-byte
/// value as long as it's stable for the process's lifetime: the value
/// seeds the stack-canary / pointer-mangle, both of which are
/// per-process invariants. A real CSPRNG output (RDRAND on x86_64,
/// rndr on aarch64, or a TRNG-seeded ChaCha20 on armv7) lands in
/// `arch::random::fill` once that surface exists.
///
/// Placeholder bytes: ascii "AREST_TIER_1_RNG" — recognisable in a
/// memory dump, distinct from zero, exactly 16 bytes.
const PLACEHOLDER_AT_RANDOM: [u8; 16] = *b"AREST_TIER_1_RNG";

/// 4 KiB system page size — same value `AddressSpace::PAGE_SIZE`
/// publishes, exposed here as a `u64` for the auxv `AT_PAGESZ`
/// emission. C startup reads this via `sysconf(_SC_PAGESIZE)`.
const SYS_PAGESZ: u64 = 4096;

/// Per-process state machine. Tier-1 currently models construction
/// → spawn handoff (`Created` → `Running`) plus the userspace exit
/// path (`Running` → `Exited`, populated by the syscall surface in
/// #473a). Stop / Killed / Zombied transitions land alongside the
/// scheduler (#530) and waitpid surface (#531).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessState {
    /// Process struct constructed, address space live, stack not yet
    /// allocated. The state `Process::new` returns.
    Created,
    /// Stack populated, trampoline invoked. The state
    /// `Process::spawn` returns on success — though under tier-1 the
    /// trampoline currently errors before reaching this state because
    /// the GDT/TSS prerequisites are pending (#526/#527).
    Running,
    /// Spawn errored before reaching ring 3. `Process::spawn`
    /// transitions here on `Err`.
    SpawnFailed,
    /// Userspace called `exit(2)` or `exit_group(2)`. Set by the
    /// `crate::syscall::exit` handler (#473a) via the
    /// `current_process_mut` accessor below; the exit status is
    /// stashed in `Process::exit_status` for the future
    /// `waitpid`-like surface (#531) to consume.
    Exited,
    /// Userspace called `futex(uaddr, FUTEX_WAIT, val, ...)` and the
    /// memory-compare check passed (`*uaddr == val`), so the kernel
    /// parked the process on the per-uaddr wait queue
    /// (`process::futex_table::FUTEX_TABLE`). The carried `u64` is the
    /// userspace virtual address of the futex word — `FUTEX_WAKE`
    /// (#545) uses it to identify which queue to drain when a peer
    /// process posts a wake.
    ///
    /// Set by `crate::syscall::futex::handle` (#544) via the
    /// `current_process_mut` accessor below. The Process stays in
    /// this state until a peer's FUTEX_WAKE drains the queue and the
    /// scheduler (#530) transitions it back to `Running` — for tier-1
    /// (no scheduler yet) the state is observable but the kernel still
    /// returns to the trampoline doorstep, which keeps the surface
    /// honest about "the process asked to block" without requiring the
    /// full park-then-resume mechanism.
    BlockedFutex(u64),
}

/// File-descriptor table entry. Tier-1 shape — just a tag plus an
/// optional kernel-side handle (a serial-console handle for stdin
/// / stdout / stderr; future entries will be filesystem inodes,
/// virtio-blk regions, network sockets). The full `struct file`
/// equivalent (offset, refcount, fcntl flags) lands with the
/// `crate::vfs` epic (#560).
///
/// `Copy` so callers can stash + compare without lifetime hassles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FdEntry {
    /// File descriptor backed by the kernel's serial console.
    /// Reads block until a keystroke arrives; writes go to UART.
    /// Stdin / stdout / stderr are seeded with this for tier-1
    /// processes — a Linux binary that reads(2) stdin gets a
    /// UART scancode stream, and write(2) goes to the serial log.
    Serial,
    /// Closed slot. The fd table is sparse — `dup2(2)` and `close(2)`
    /// in the future syscall surface (#473) will return slots to this
    /// state for re-use.
    Closed,
}

/// Errors `Process::spawn` can return. Wraps the upstream stack
/// builder + trampoline error variants so a single call site can
/// branch by variant. Same shape as `process::elf::LoadOrParseError`
/// — one enum that flattens the multi-stage pipeline's errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpawnError {
    /// `StackBuilder::finalize` rejected the stack layout. Wraps
    /// `StackError`.
    Stack(StackError),
    /// `trampoline::invoke` rejected the ring-3 transition. Wraps
    /// `TrampolineError`. Tier-1 always returns this with
    /// `TrampolineError::NotYetImplemented` on x86_64 and
    /// `TrampolineError::UnsupportedArch` on aarch64 / armv7
    /// because the prerequisites for the actual ring-3 jump haven't
    /// landed (#526/#527).
    Trampoline(TrampolineError),
}

impl From<StackError> for SpawnError {
    fn from(e: StackError) -> Self {
        SpawnError::Stack(e)
    }
}

impl From<TrampolineError> for SpawnError {
    fn from(e: TrampolineError) -> Self {
        SpawnError::Trampoline(e)
    }
}

/// A live (or once-live) Linux process. Owns its address space (the
/// loaded PT_LOAD segments), its initial stack, and a small fd table
/// seeded with serial-console handles for stdin/stdout/stderr.
///
/// `Process` is NOT `Copy` — it carries a `Vec<FdEntry>` and (after
/// `spawn`) an `InitialStack` that owns its own page allocation. Drop
/// reclaims the storage via `AddressSpace`'s + `InitialStack`'s own
/// `Drop` impls.
pub struct Process {
    /// Process id. Tier-1 takes this from the constructor; #530
    /// brings a central allocator. Stays a `u32` because Linux
    /// `pid_t` is `i32` on every supported arch and tier-1 doesn't
    /// model negative-pid sentinels (`__WALL`, `__WCLONE`, etc.).
    pub pid: u32,
    /// Loaded PT_LOAD segments. The `AddressSpace::entry_point` is
    /// the rip the trampoline jumps to.
    pub address_space: AddressSpace,
    /// Per-process file-descriptor table. Indexed by Linux fd
    /// number — `fd_table[0]` is stdin, `fd_table[1]` is stdout,
    /// `fd_table[2]` is stderr. Sparse `Vec` so future `dup2(2)`
    /// can push past the current high-water mark; `Closed` slots
    /// are re-usable.
    pub fd_table: Vec<FdEntry>,
    /// Open-file table for fds ≥ 3 (the open()-side surface
    /// introduced by `openat` (#498) + `close` (#498)). The standard
    /// streams (fd 0 / 1 / 2) live on `fd_table` above for backwards
    /// compat with GGGGG's write handler; this richer table holds the
    /// per-fd backing entry (File-cell-backed or synthetic-fs-backed)
    /// for everything `openat` opens. The future `read` handler (#499)
    /// will look up entries here to source bytes; the future fd-table
    /// unification (post-#499) folds the legacy `Vec<FdEntry>` into
    /// this type.
    pub open_fds: FdTable,
    /// Construction / spawn / running / failed state. Drives the
    /// future scheduler's "is this process schedulable" check (#530).
    pub state: ProcessState,
    /// Owned initial stack. `None` until `spawn` allocates it; once
    /// populated, the stack outlives the Process and gets reclaimed
    /// on Drop. For tier-1 the trampoline currently fails before the
    /// jump, so a `Process::spawn` call leaves `initial_stack =
    /// Some(...)` and `state = SpawnFailed`. The stack is preserved
    /// (rather than dropped on the failed path) so the test harness
    /// can inspect the layout after a structural failure.
    pub initial_stack: Option<InitialStack>,
    /// Exit status the process passed to `exit(2)` / `exit_group(2)`.
    /// Populated by `crate::syscall::exit::handle` once the userspace
    /// syscall surface (#473a) is wired through to the process; `None`
    /// until then. `wait(2)` (#531) only reads the low 8 bits, but the
    /// kernel preserves the full i32 so a future signed-status check
    /// has the bits.
    pub exit_status: Option<i32>,
    /// Owned copy of the argv strings the spawn was launched with.
    /// Populated by `Process::spawn` (the borrowed `&[&[u8]]` argument
    /// is `to_vec()`-cloned into here so the strings outlive the spawn
    /// call's stack frame). Empty `Vec` until `spawn` runs — the
    /// `Process::new` doorstep doesn't take argv. Used by the
    /// `synthetic_fs::proc_pid` renderer to project `/proc/<pid>/cmdline`
    /// (NUL-joined argv) and `/proc/<pid>/comm` (the basename of
    /// `argv[0]`); future `prctl(PR_SET_NAME)` will mutate `argv[0]`
    /// shape via a separate `comm_override` field.
    pub argv: Vec<Vec<u8>>,
}

impl Process {
    /// Construct a fresh Process around an already-loaded
    /// `AddressSpace`. State starts at `Created`; the fd table
    /// seeds stdin / stdout / stderr to the kernel's serial console.
    /// Caller picks the pid (tier-1 — see module docstring).
    pub fn new(pid: u32, address_space: AddressSpace) -> Self {
        // Seed stdin / stdout / stderr with serial. The fd table is
        // a `Vec` so the future syscall surface (#473) can grow it
        // via `dup2(2)` / `open(2)` without reshaping the type.
        let mut fd_table = Vec::with_capacity(3);
        fd_table.push(FdEntry::Serial); // stdin
        fd_table.push(FdEntry::Serial); // stdout
        fd_table.push(FdEntry::Serial); // stderr

        Self {
            pid,
            address_space,
            fd_table,
            open_fds: FdTable::new(),
            state: ProcessState::Created,
            initial_stack: None,
            exit_status: None,
            argv: Vec::new(),
        }
    }

    /// Spawn the process — allocate the initial stack page,
    /// populate argv / envp / the minimum auxv set per System V
    /// AMD64 PSABI, and invoke the trampoline to transition to
    /// ring 3 + jump to `e_entry`.
    ///
    /// Tier-1 limitation: the trampoline currently returns
    /// `NotYetImplemented` (x86_64) / `UnsupportedArch` (aarch64 /
    /// armv7) because the GDT/TSS scaffolding (#526) and
    /// page-table install (#527) haven't landed. The stack is
    /// allocated + populated + checked, then a `SpawnError` is
    /// returned. The Process retains the populated stack so the
    /// caller can introspect it (useful for the unit tests that
    /// assert the layout took the right shape).
    ///
    /// `argv` and `envp` are `&[&[u8]]` slices because the System V
    /// ABI is byte-string-typed (no UTF-8 promise — Linux file
    /// paths can be arbitrary bytes). Convention: `argv[0]` is the
    /// program path; the caller is responsible for picking it.
    pub fn spawn(&mut self, argv: &[&[u8]], envp: &[&[u8]]) -> Result<(), SpawnError> {
        // Step 0: stash an owned copy of the argv strings so the
        // /proc/<pid>/cmdline + /proc/<pid>/comm renderers
        // (synthetic_fs::proc_pid) can project them after the spawn
        // call returns. The borrowed `&[&[u8]]` strings would otherwise
        // dangle once the caller's stack frame unwinds.
        self.argv = argv.iter().map(|a| a.to_vec()).collect();

        // Step 1: build the auxv set. Tier-1 emits the minimum the
        // System V AMD64 PSABI requires for a static binary's
        // _start to find what it needs without making syscalls.
        //
        // Note on AT_RANDOM: the auxv value is the ADDRESS of 16
        // CSPRNG bytes in the process address space — not the bytes
        // themselves. We can't yet point at a userspace VA because
        // the page-table install (#527) hasn't landed; for tier-1
        // we point at the placeholder bytes' kernel-space address,
        // which (under UEFI's identity mapping) coincides with the
        // userspace VA the future page-table install will use.
        // Same identity-mapping rationale `AddressSpace`'s
        // PhysAddr re-derivation uses (process/address_space.rs:42).
        let phdr_count = self.address_space.segments.len() as u64;
        let phdr_addr = self
            .address_space
            .segments
            .first()
            .map(|s| s.vaddr)
            .unwrap_or(0);
        let entry = self.address_space.entry_point;
        let random_addr = PLACEHOLDER_AT_RANDOM.as_ptr() as u64;
        let auxv: [AuxvEntry; 7] = [
            AuxvEntry::new(AuxvType::Phdr, phdr_addr),
            AuxvEntry::new(AuxvType::Phent, ELF64_PHENT_SIZE as u64),
            AuxvEntry::new(AuxvType::Phnum, phdr_count),
            AuxvEntry::new(AuxvType::Pagesz, SYS_PAGESZ),
            AuxvEntry::new(AuxvType::Entry, entry),
            AuxvEntry::new(AuxvType::Random, random_addr),
            // AT_NULL is appended by `StackBuilder::finalize` — do
            // NOT emit it explicitly (the builder's contract is to
            // own the terminator).
            // Sentinel hint to the reader: this is the LAST real
            // entry; the trailing AT_NULL terminator is implicit.
            AuxvEntry::new(AuxvType::Null, 0),
        ];

        // Step 2: build the stack. Walk argv / envp / auxv in order;
        // `StackBuilder::finalize` allocates + populates the stack
        // page in one shot.
        let mut builder = StackBuilder::new();
        for arg in argv {
            builder = builder.push_argv(arg);
        }
        for var in envp {
            builder = builder.push_envp(var);
        }
        // Skip the trailing AT_NULL sentinel — `StackBuilder::finalize`
        // owns the terminator. We slice the array to drop the last
        // entry (index 6 = AT_NULL placeholder) before pushing.
        for entry in &auxv[..auxv.len() - 1] {
            builder = builder.push_auxv(*entry);
        }
        let stack = builder.finalize().map_err(SpawnError::from)?;

        // Step 3: invoke the trampoline. Diverges (returns `!`) on
        // success; returns `Err(...)` if the prerequisites aren't met.
        // For tier-1 this always returns `NotYetImplemented` (x86_64)
        // or `UnsupportedArch` (aarch64 / armv7) — the populated
        // stack is preserved so the caller can introspect.
        let invoke_result = trampoline::invoke(&self.address_space, &stack);

        // Step 4: store the stack on the Process regardless of
        // invoke success/failure. On a SpawnFailed path the caller
        // can still inspect the layout; on a successful jump the
        // trampoline diverges so this assignment is never reached
        // (but is harmless — the trampoline's `!` return type makes
        // the rest of the function dead in that branch).
        self.initial_stack = Some(stack);

        // Step 5: state transition + error propagation.
        match invoke_result {
            Ok(_) => {
                // The trampoline returned `Ok(Infallible)` — which is
                // structurally impossible because `Infallible` has no
                // inhabitants. If we ever reach here the trampoline
                // is misimplemented; mark Running for completeness.
                self.state = ProcessState::Running;
                Ok(())
            }
            Err(e) => {
                self.state = ProcessState::SpawnFailed;
                Err(SpawnError::from(e))
            }
        }
    }

    /// Compose this Process's facts onto `state` and return the
    /// new state. Same shape as `AddressSpace::record_into_cells`
    /// — pure function, caller decides whether to commit via
    /// `system::apply` (production wiring) or to inspect the
    /// returned Object (test harness).
    ///
    /// Cells emitted (one fact each):
    ///   * `Process_has_Pid` — (Process, Pid) where Pid = "<pid>"
    ///   * `Process_has_State` — (Process, State) where State =
    ///       "Created" / "Running" / "SpawnFailed"
    ///   * `Process_has_FdTable` — (Process, Fd, Backend) one fact
    ///       per non-Closed fd (sparse table — Closed slots elide).
    /// Plus all the cells `AddressSpace::record_into_cells` emits
    /// (Process_has_EntryPoint / Process_has_Segment / Segment_has_Layout).
    ///
    /// `process_id` is the atom the caller picks — typically the
    /// process's hex pid (`format!("{:x}", self.pid)`) or a hash of
    /// the ELF blob.
    pub fn record_into_cells(&self, process_id: &str, state: &Object) -> Object {
        let pid_atom = format!("{}", self.pid);
        let mut s = cell_push(
            "Process_has_Pid",
            fact_from_pairs(&[("Process", process_id), ("Pid", &pid_atom)]),
            state,
        );
        // BlockedFutex is rendered as "BlockedFutex" without the
        // uaddr — the cell shape stays a single string for forward-
        // compat with the existing Process_has_State consumers (the
        // uaddr is recorded separately in the future #545's
        // Futex_has_Waiter cell once that handler lands).
        let state_atom = match self.state {
            ProcessState::Created => "Created",
            ProcessState::Running => "Running",
            ProcessState::SpawnFailed => "SpawnFailed",
            ProcessState::Exited => "Exited",
            ProcessState::BlockedFutex(_) => "BlockedFutex",
        };
        s = cell_push(
            "Process_has_State",
            fact_from_pairs(&[("Process", process_id), ("State", state_atom)]),
            &s,
        );
        for (fd, entry) in self.fd_table.iter().enumerate() {
            if matches!(entry, FdEntry::Closed) {
                continue;
            }
            let fd_atom = format!("{}", fd);
            let backend_atom = match entry {
                FdEntry::Serial => "Serial",
                FdEntry::Closed => unreachable!("Closed elided above"),
            };
            s = cell_push(
                "Process_has_FdTable",
                fact_from_pairs(&[
                    ("Process", process_id),
                    ("Fd", &fd_atom),
                    ("Backend", backend_atom),
                ]),
                &s,
            );
        }
        // Compose the address-space cells last so a debugger / cell
        // inspector sees them as children of the Process_has_State
        // / Process_has_Pid facts.
        self.address_space.record_into_cells(process_id, &s)
    }
}

// -- current_process accessor (#473a) -----------------------------------
//
// The syscall surface (`crate::syscall::dispatch::dispatch`) is a fixed
// `(rax, rdi, rsi, rdx, r10, r8, r9) -> i64` signature — no Process
// reference threads through. Per-syscall handlers (`syscall::write`,
// `syscall::exit`) reach the calling Process via this kernel-wide
// accessor: `current_process_mut(|maybe_proc| ...)` runs the closure
// against an `Option<&mut Process>`, returning `None` when no process
// is currently registered (the kernel boots with no process; the
// future #552 ring-3 gate will install one before flipping to ring 3).
//
// Tier-1: single-threaded model
// -----------------------------
// The kernel runs at most one Linux process at a time today (no
// scheduler — #530). A `spin::Mutex<Option<Process>>` static carries
// the registered process; install / uninstall transitions are
// explicit. Once the scheduler lands, this accessor will switch to a
// per-CPU `current_task` lookup (matching Linux's `current` macro
// shape) — but the call-site shape (closure receives an
// `Option<&mut Process>`) stays the same so the syscall handlers
// don't need re-shaping.
//
// Why a closure rather than a `static mut Option<&'static mut Process>`
// ---------------------------------------------------------------------
// The closure shape lets the static stay private — callers can't
// stash the `&mut Process` past the `with` call's borrow lifetime.
// This is the same shape the kernel already uses for every other
// global mutable singleton (`arch::uefi::memory::with_page_table`,
// `arch::uefi::memory::with_frame_allocator`). Consistency matters
// for the same reason the other singletons use this pattern: the
// borrow-checker enforces "you can't keep a reference past the
// lock's release" without runtime overhead.
//
// Why install/uninstall rather than `set(Option<Process>)`
// --------------------------------------------------------
// `install(Process)` makes the "the kernel just took ownership of
// this process" intent explicit at the call site; `uninstall()`
// makes the "the kernel just dropped it" intent equally explicit.
// A combined `set(Option<Process>)` would muddy both — the test
// suite uses both to set up + tear down per-test, and named
// transitions read better in a test diff.
//
// Why no `Send` bound contortion
// ------------------------------
// `spin::Mutex` doesn't require `Send` of its payload — the lock
// guards access; the kernel is single-threaded so there's no actual
// cross-thread share happening. `Process` carries `AddressSpace`
// which holds `LoadedSegment` (raw pointers); the existing
// `unsafe impl Send` on `LoadedSegment` (process/address_space.rs)
// already says "the kernel will keep this single-owner per the
// scheduler invariant" — same invariant applies here.

/// Singleton holding the Linux process the kernel is currently
/// hosting. `None` before the future #552 ring-3 gate installs one;
/// `Some(...)` while the process is live (Created / Running). After
/// the process exits (`crate::syscall::exit::handle` transitions to
/// `Exited`) the static stays populated so `wait`-like callers can
/// still read the exit status — `uninstall` is the explicit
/// "scheduler reaped this process" transition.
///
/// `spin::Mutex` rather than `RefCell` so a future SMP path doesn't
/// have to retrofit the lock; the cost is minimal (single-CPU lock
/// = no contention) and the API matches the rest of the kernel's
/// global mutable singletons.
static CURRENT_PROCESS: spin::Mutex<Option<Process>> = spin::Mutex::new(None);

/// Run a closure against the currently-installed Process, returning
/// the closure's result. The closure receives `Option<&mut Process>`
/// — `Some` if a process is installed (the post-#552 production
/// path), `None` if not (kernel boot before any spawn, or the test
/// suite's "uninstall fired between tests" state).
///
/// Returns whatever the closure returns — typed `R` so the call site
/// can extract values out of the locked region without ferrying them
/// through a `mem::take`-style dance.
///
/// Holds the singleton's `spin::Mutex` for the duration of the
/// closure. Don't park / await inside the closure — the lock is
/// released only when the closure returns. (No async in the kernel
/// today; this is a "don't grow one" reminder for the future.)
pub fn current_process_mut<F, R>(f: F) -> R
where
    F: FnOnce(Option<&mut Process>) -> R,
{
    let mut guard = CURRENT_PROCESS.lock();
    f(guard.as_mut())
}

/// Install `proc` as the kernel's current process. Replaces any
/// previously-installed process — caller is responsible for
/// `uninstall`-ing first if that's not the intended semantic.
///
/// The future #552 ring-3 gate calls this once per spawn, just before
/// flipping to ring 3; the trampoline returns control to the kernel
/// only when the process exits or faults, at which point a future
/// scheduler (#530) calls `uninstall` and picks the next runnable
/// process.
pub fn current_process_install(proc: Process) {
    *CURRENT_PROCESS.lock() = Some(proc);
}

/// Drop the kernel's current process, returning it to the caller.
/// Returns `None` if no process was installed. Used by the test
/// harness to clean up between tests, and by the future scheduler
/// (#530) to reap exited processes.
pub fn current_process_uninstall() -> Option<Process> {
    CURRENT_PROCESS.lock().take()
}

/// Read-only accessor returning the currently-installed process's pid,
/// or `None` if no process is installed. Sibling of
/// `current_process_mut` — same lock discipline, but cheaper because
/// it copies the `u32` pid out of the locked region instead of handing
/// the closure a `&mut Process` borrow.
///
/// Used by the `synthetic_fs::proc` resolver to translate `/proc/self/*`
/// path lookups into the calling process's pid (Linux convention: the
/// `self` symlink in /proc resolves to the calling thread's pid). The
/// resolver doesn't need to mutate the Process, just to know which pid
/// to look up — `current_process_mut` would over-grant the lock for
/// the read-only use case.
pub fn current_process_id() -> Option<u32> {
    CURRENT_PROCESS.lock().as_ref().map(|p| p.pid)
}

/// Run a closure against the currently-installed Process's open-fd
/// table (the richer `FdTable` introduced by openat + close, #498).
/// Sibling of `current_process_mut` — same closure shape, same lock
/// discipline, scoped to the per-process fd table so the openat /
/// close / read handlers don't have to ferry an `Option<&mut Process>`
/// through their bodies just to reach the table.
///
/// The closure receives `Option<&mut FdTable>` — `Some` when a
/// process is installed, `None` when not (kernel boot before any
/// spawn, or the test suite's "uninstall fired between tests" state).
/// The caller is responsible for handling `None` — typically by
/// returning `-EBADF` or `-ENOSYS` to userspace per Linux's
/// "syscall called before any process is live" convention.
///
/// Returns whatever the closure returns — typed `R` so the call
/// site can extract values out of the locked region without a
/// `mem::take`-style dance.
///
/// Holds the singleton's `spin::Mutex` for the duration of the
/// closure. Don't park / await inside the closure — same constraint
/// as `current_process_mut`. The lock is released when the closure
/// returns; no async in the kernel today.
pub fn current_process_fd_table<F, R>(f: F) -> R
where
    F: FnOnce(Option<&mut FdTable>) -> R,
{
    let mut guard = CURRENT_PROCESS.lock();
    f(guard.as_mut().map(|p| &mut p.open_fds))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::address_space::SegmentPerm;

    /// `Process::new` produces a `Created`-state process with the
    /// fd table seeded for stdin / stdout / stderr.
    #[test]
    fn new_seeds_fd_table_and_state() {
        let mut address_space = AddressSpace::new(0x40_1000);
        address_space
            .push_segment(0x40_1000, 0x10, SegmentPerm::ReadExecute, &[0x90; 8])
            .expect(".text push");
        let proc = Process::new(42, address_space);
        assert_eq!(proc.pid, 42);
        assert_eq!(proc.state, ProcessState::Created);
        assert_eq!(proc.fd_table.len(), 3);
        assert_eq!(proc.fd_table[0], FdEntry::Serial);
        assert_eq!(proc.fd_table[1], FdEntry::Serial);
        assert_eq!(proc.fd_table[2], FdEntry::Serial);
        assert!(proc.initial_stack.is_none());
    }

    /// `Process::new` preserves the address space's entry point.
    /// Used by `spawn` to populate AT_ENTRY in the auxv.
    #[test]
    fn new_preserves_entry_point() {
        let address_space = AddressSpace::new(0xDEAD_BEEF);
        let proc = Process::new(1, address_space);
        assert_eq!(proc.address_space.entry_point, 0xDEAD_BEEF);
    }

    /// `Process::spawn` populates the initial stack and transitions
    /// state to `SpawnFailed` (tier-1 — the trampoline's
    /// prerequisites haven't landed). The stack itself is preserved
    /// so the caller can introspect.
    #[test]
    fn spawn_populates_stack_and_marks_failed_under_tier_1() {
        let mut address_space = AddressSpace::new(0x40_1000);
        address_space
            .push_segment(0x40_1000, 0x10, SegmentPerm::ReadExecute, &[0x90; 8])
            .expect(".text push");
        let mut proc = Process::new(1, address_space);
        let argv: &[&[u8]] = &[b"/bin/true"];
        let envp: &[&[u8]] = &[b"PATH=/usr/bin"];
        let err = proc.spawn(argv, envp).unwrap_err();
        // Wrapped trampoline error — variant depends on target arch.
        assert!(matches!(err, SpawnError::Trampoline(_)));
        // State reflects the failure.
        assert_eq!(proc.state, ProcessState::SpawnFailed);
        // Stack is preserved.
        assert!(proc.initial_stack.is_some());
        let stack = proc.initial_stack.as_ref().unwrap();
        // argc lives at sp; argv had one entry.
        assert_eq!(stack.read_argc(), 1);
        // SP is 16-aligned per System V ABI.
        assert_eq!(stack.sp() % 16, 0);
    }

    /// `Process::spawn` walks the argv list correctly — the populated
    /// stack reports the correct argc.
    #[test]
    fn spawn_argc_matches_argv_count() {
        let mut address_space = AddressSpace::new(0x40_1000);
        address_space
            .push_segment(0x40_1000, 0x10, SegmentPerm::ReadExecute, &[0x90; 8])
            .expect(".text push");
        let mut proc = Process::new(1, address_space);
        let argv: &[&[u8]] = &[b"/bin/sh", b"-c", b"echo hi"];
        let envp: &[&[u8]] = &[];
        let _ = proc.spawn(argv, envp); // expected to fail under tier-1
        let stack = proc.initial_stack.as_ref().unwrap();
        assert_eq!(stack.read_argc(), 3);
    }

    /// `Process::spawn` emits the auxv entries in the order the spawn
    /// builds them. This is structural — the test reads the populated
    /// region and confirms the entries land where the layout
    /// constants predict (after argv NULL, envp NULL).
    #[test]
    fn spawn_auxv_layout_matches_spec() {
        let mut address_space = AddressSpace::new(0x40_1000);
        address_space
            .push_segment(0x40_1000, 0x10, SegmentPerm::ReadExecute, &[0x90; 8])
            .expect(".text push");
        let mut proc = Process::new(1, address_space);
        let _ = proc.spawn(&[], &[]); // expected to fail under tier-1
        let stack = proc.initial_stack.as_ref().unwrap();
        let pop = stack.populated();
        // Layout for empty argv + empty envp:
        //   argc(8) + argv NULL(8) + envp NULL(8) = 24 bytes header,
        //   then auxv entries starting at offset 24.
        // First auxv entry should be AT_PHDR (a_type = 3).
        let auxv_base = 24;
        let mut buf = [0u8; 8];
        buf.copy_from_slice(&pop[auxv_base..auxv_base + 8]);
        assert_eq!(u64::from_le_bytes(buf), AuxvType::Phdr as u64);
    }

    /// `Process::spawn` populates AT_PHNUM with the segment count.
    /// Used by libc to walk the loaded program headers.
    #[test]
    fn spawn_at_phnum_reflects_segment_count() {
        let mut address_space = AddressSpace::new(0x40_1000);
        address_space
            .push_segment(0x40_1000, 0x10, SegmentPerm::ReadExecute, &[0x90; 8])
            .expect(".text push");
        address_space
            .push_segment(0x40_2000, 0x20, SegmentPerm::ReadWrite, &[0x42; 16])
            .expect(".data push");
        let mut proc = Process::new(1, address_space);
        let _ = proc.spawn(&[], &[]);
        let stack = proc.initial_stack.as_ref().unwrap();
        let pop = stack.populated();
        // Layout: argc(8) + argv NULL(8) + envp NULL(8) = 24, then
        // auxv. AT_PHDR (offset 24..40), AT_PHENT (offset 40..56),
        // AT_PHNUM (offset 56..72). The AT_PHNUM value is at offset
        // 64..72 (the val half of the third auxv pair).
        let mut buf = [0u8; 8];
        buf.copy_from_slice(&pop[64..72]);
        assert_eq!(u64::from_le_bytes(buf), 2);
    }

    /// `Process::spawn` populates AT_PAGESZ with 4096.
    #[test]
    fn spawn_at_pagesz_is_4096() {
        let mut address_space = AddressSpace::new(0x40_1000);
        address_space
            .push_segment(0x40_1000, 0x10, SegmentPerm::ReadExecute, &[0x90; 8])
            .expect(".text push");
        let mut proc = Process::new(1, address_space);
        let _ = proc.spawn(&[], &[]);
        let stack = proc.initial_stack.as_ref().unwrap();
        let pop = stack.populated();
        // AT_PHDR (24..40), AT_PHENT (40..56), AT_PHNUM (56..72),
        // AT_PAGESZ (72..88). Value at offset 80..88.
        let mut buf = [0u8; 8];
        buf.copy_from_slice(&pop[80..88]);
        assert_eq!(u64::from_le_bytes(buf), 4096);
    }

    /// `Process::spawn` populates AT_ENTRY with the address space's
    /// entry point. Mirrors the trampoline's iretq RIP value.
    #[test]
    fn spawn_at_entry_matches_address_space() {
        let mut address_space = AddressSpace::new(0xCAFE_BABE);
        address_space
            .push_segment(0xCAFE_BABE, 0x10, SegmentPerm::ReadExecute, &[0x90; 8])
            .expect(".text push");
        let mut proc = Process::new(1, address_space);
        let _ = proc.spawn(&[], &[]);
        let stack = proc.initial_stack.as_ref().unwrap();
        let pop = stack.populated();
        // AT_PHDR (24..40), AT_PHENT (40..56), AT_PHNUM (56..72),
        // AT_PAGESZ (72..88), AT_ENTRY (88..104). Value at 96..104.
        let mut buf = [0u8; 8];
        buf.copy_from_slice(&pop[96..104]);
        assert_eq!(u64::from_le_bytes(buf), 0xCAFE_BABE);
    }

    /// `Process::spawn` populates AT_RANDOM with a non-zero address.
    /// The address points at the placeholder 16-byte CSPRNG seed.
    #[test]
    fn spawn_at_random_is_non_zero() {
        let mut address_space = AddressSpace::new(0x40_1000);
        address_space
            .push_segment(0x40_1000, 0x10, SegmentPerm::ReadExecute, &[0x90; 8])
            .expect(".text push");
        let mut proc = Process::new(1, address_space);
        let _ = proc.spawn(&[], &[]);
        let stack = proc.initial_stack.as_ref().unwrap();
        let pop = stack.populated();
        // AT_RANDOM at offset 104..120 (sixth auxv pair). Value at
        // 112..120.
        let mut buf = [0u8; 8];
        buf.copy_from_slice(&pop[112..120]);
        assert_ne!(u64::from_le_bytes(buf), 0, "AT_RANDOM must be non-zero");
    }

    /// `Process::spawn` appends AT_NULL terminator to the auxv.
    /// Comes after the seven explicit entries.
    #[test]
    fn spawn_auxv_terminated_with_at_null() {
        let mut address_space = AddressSpace::new(0x40_1000);
        address_space
            .push_segment(0x40_1000, 0x10, SegmentPerm::ReadExecute, &[0x90; 8])
            .expect(".text push");
        let mut proc = Process::new(1, address_space);
        let _ = proc.spawn(&[], &[]);
        let stack = proc.initial_stack.as_ref().unwrap();
        let pop = stack.populated();
        // Six explicit auxv entries (AT_PHDR / AT_PHENT / AT_PHNUM /
        // AT_PAGESZ / AT_ENTRY / AT_RANDOM) × 16 bytes each = 96
        // bytes, starting at offset 24. AT_NULL terminator at
        // offset 24 + 96 = 120, value 0.
        let mut buf = [0u8; 8];
        buf.copy_from_slice(&pop[120..128]);
        assert_eq!(u64::from_le_bytes(buf), AuxvType::Null as u64);
    }

    /// `From<StackError>` flows through to `SpawnError::Stack`.
    #[test]
    fn spawn_error_from_stack_error() {
        let err: SpawnError = StackError::OutOfMemory.into();
        assert_eq!(err, SpawnError::Stack(StackError::OutOfMemory));
    }

    /// `From<TrampolineError>` flows through to `SpawnError::Trampoline`.
    #[test]
    fn spawn_error_from_trampoline_error() {
        let err: SpawnError = TrampolineError::NullEntry.into();
        assert_eq!(err, SpawnError::Trampoline(TrampolineError::NullEntry));
    }

    /// `record_into_cells` emits the expected per-Process facts.
    #[test]
    fn record_into_cells_emits_pid_state_and_fd_facts() {
        let mut address_space = AddressSpace::new(0x40_1000);
        address_space
            .push_segment(0x40_1000, 0x10, SegmentPerm::ReadExecute, &[0x90; 8])
            .expect(".text push");
        let proc = Process::new(42, address_space);
        let recorded = proc.record_into_cells("test_proc", &Object::phi());
        let serialised = format!("{:?}", recorded);
        assert!(serialised.contains("Process_has_Pid"));
        assert!(serialised.contains("Process_has_State"));
        assert!(serialised.contains("Process_has_FdTable"));
        // Underlying address-space cells should also be present.
        assert!(serialised.contains("Process_has_EntryPoint"));
        assert!(serialised.contains("Created"));
        assert!(serialised.contains("Serial"));
    }

    /// `record_into_cells` elides Closed fd entries — the table is
    /// sparse and Closed slots don't deserve a fact.
    #[test]
    fn record_into_cells_elides_closed_fd_entries() {
        let address_space = AddressSpace::new(0x40_1000);
        let mut proc = Process::new(1, address_space);
        // Manually punch fd 1 to Closed.
        proc.fd_table[1] = FdEntry::Closed;
        let recorded = proc.record_into_cells("test_proc", &Object::phi());
        let serialised = format!("{:?}", recorded);
        // Two Process_has_FdTable facts (fd 0 + fd 2), not three.
        let count = serialised.matches("Process_has_FdTable").count();
        assert_eq!(count, 2, "Closed fd 1 must elide");
    }

    // -- Integration: SPAWN_ELF end-to-end ---------------------------
    //
    // The real proof of life for #521: parse + load + spawn against
    // the SPAWN_ELF fixture (a minimal static binary with x86_64
    // instructions for write+exit_group). Validates that the four
    // pipeline stages compose without panicking and produce the
    // expected outputs at each boundary.

    use crate::process::elf::{load_segments, parse};
    use crate::process::test_fixtures::SPAWN_ELF;
    use crate::process::trampoline::{setup_x86_64, IretqFrame};

    /// SPAWN_ELF parses + loads + spawns end-to-end. Spawn fails at
    /// the trampoline doorstep (per tier-1 — see Process::spawn
    /// docstring) but every stage before the ring-3 jump completes
    /// cleanly: parsed binary has the expected headline fields, the
    /// loaded address space carries one segment with the
    /// instruction bytes, and the populated stack reports the
    /// expected argc.
    #[test]
    fn spawn_elf_end_to_end() {
        let parsed = parse(SPAWN_ELF).expect("SPAWN_ELF must parse");
        assert_eq!(parsed.entry, 0x40_1000);
        assert_eq!(parsed.program_headers.len(), 2);

        let address_space =
            load_segments(&parsed, SPAWN_ELF).expect("load must succeed");
        assert_eq!(address_space.entry_point, 0x40_1000);
        assert_eq!(address_space.segments.len(), 1);
        let segment = &address_space.segments[0];
        // Verify the loaded instruction bytes match the fixture's
        // PT_LOAD payload — first 5 bytes are the `mov eax, 1`
        // opcode (b8 01 00 00 00).
        let view = segment.pages_view();
        assert_eq!(&view[..5], &[0xb8, 0x01, 0x00, 0x00, 0x00]);

        let mut proc = Process::new(7, address_space);
        let argv: &[&[u8]] = &[b"/bin/spawn"];
        let envp: &[&[u8]] = &[b"PATH=/usr/bin"];
        // Spawn errors at the trampoline doorstep on every arch
        // (tier-1 limitation); the structural pipeline still runs.
        let err = proc.spawn(argv, envp).unwrap_err();
        assert!(matches!(err, SpawnError::Trampoline(_)));
        assert_eq!(proc.state, ProcessState::SpawnFailed);

        // Stack populated correctly.
        let stack = proc.initial_stack.as_ref().unwrap();
        assert_eq!(stack.read_argc(), 1);
        assert_eq!(stack.sp() % 16, 0);
    }

    /// Trampoline `setup_x86_64` produces an IretqFrame with rip =
    /// SPAWN_ELF's e_entry. The frame is the data the (currently
    /// stubbed) ring-3 jump will consume once #526's GDT/TSS lands.
    #[test]
    fn spawn_elf_setup_produces_iretq_frame() {
        let parsed = parse(SPAWN_ELF).expect("parse");
        let address_space = load_segments(&parsed, SPAWN_ELF).expect("load");
        let stack = StackBuilder::new()
            .push_argv(b"/bin/spawn")
            .finalize()
            .expect("stack");
        let frame: IretqFrame =
            setup_x86_64(&address_space, &stack).expect("setup");
        assert_eq!(frame.rip, 0x40_1000);
        assert_eq!(frame.rsp, stack.sp());
        // CS / SS / RFLAGS come from the placeholder constants until
        // #526; verify the values match for forward-compatibility.
        assert_eq!(
            frame.cs & 0b11,
            3,
            "CS RPL must be 3 — userspace selector"
        );
        assert_eq!(
            frame.ss & 0b11,
            3,
            "SS RPL must be 3 — userspace selector"
        );
        assert_eq!(
            frame.rflags & (1 << 9),
            1 << 9,
            "RFLAGS must have IF set"
        );
    }
}

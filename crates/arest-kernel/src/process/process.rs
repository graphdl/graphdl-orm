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

use alloc::boxed::Box;
use alloc::format;
use alloc::vec::Vec;
use arest::ast::{cell_push, fact_from_pairs, Object};

use super::address_space::AddressSpace;
use super::elf::{DynamicImage, ELF64_PHENT_SIZE};
use super::fd_table::FdTable;
use super::signal::{SigInfo, SignalDelivery, SignalState, SIGCHLD, SIGSEGV};
use super::stack::{AuxvEntry, AuxvType, InitialStack, StackBuilder, StackError};
use super::trampoline::{self, TrampolineError};

/// Width (in bytes) of the AT_RANDOM auxv buffer, per the ELF System V
/// PSABI: libc's `_dl_setup_stack_chk_guard` and the pointer-guard
/// initialiser both consume EXACTLY 16 bytes from the address auxv's
/// AT_RANDOM points at. Smaller buffers leak adjacent stack bytes;
/// larger buffers waste kernel heap and confuse anyone reading the
/// spec. Held as a named constant so the test that asserts
/// "AT_RANDOM is 16 bytes wide" reads as enforcing the spec rather
/// than a magic literal.
pub(crate) const AT_RANDOM_WIDTH: usize = 16;

/// 4 KiB system page size — same value `AddressSpace::PAGE_SIZE`
/// publishes, exposed here as a `u64` for the auxv `AT_PAGESZ`
/// emission. C startup reads this via `sysconf(_SC_PAGESIZE)`.
const SYS_PAGESZ: u64 = 4096;

/// Per-process state machine. Tier-1 models construction → spawn
/// handoff (`Created` → `Running`), the userspace exit path
/// (`Running` → `Exited`, populated by the syscall surface in #473a),
/// and signal-driven termination (`Killed`, #549 — set when a fatal
/// signal's default action fires). Stop (job control) + Zombied
/// (reaping) transitions land alongside the scheduler (#530) and
/// waitpid surface (#531).
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
    /// A fatal signal terminated the process (#549). The carried `i32`
    /// is the terminating signal number — Linux's `WTERMSIG`, what a
    /// future `wait(2)` (#531) reports as the cause of death (distinct
    /// from `Exited`'s voluntary `exit_status`). Set by
    /// `Process::deliver_signal` when the delivery decision is
    /// `Terminate` (SIGTERM / SIGKILL / any default-Term signal) or
    /// `CoreDump` (SIGSEGV / SIGABRT / … — #550 writes the core file;
    /// the process still dies). SIGKILL reaches here un-catchably: the
    /// delivery decision honours no handler parked against it.
    Killed(i32),
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
    /// 16 bytes of CSPRNG output the auxv `AT_RANDOM` pointer
    /// references — sourced from `arest::crypto::random_bytes` which
    /// delegates to the seeded ChaCha20 csprng installed by the
    /// entropy framework (#578 funnel; host CLI hardware seed via
    /// `cli::entropy_host::HostEntropySource` per #574, kernel x86_64
    /// RDSEED/RDRAND seed per #569, UEFI EFI_RNG_PROTOCOL fallback
    /// per #571).
    ///
    /// Held in a `Box` so the address is stable across moves of the
    /// `Process` struct (in particular, the move into
    /// `CURRENT_PROCESS`'s `spin::Mutex<Option<Process>>` post-spawn).
    /// The auxv `AT_RANDOM` value records `at_random.as_ptr() as u64`
    /// at spawn time; if the bytes lived inline on the struct, that
    /// pointer would dangle the moment the Process moves.
    ///
    /// Allocated zero in `Process::new`; filled with real CSPRNG
    /// output in `Process::spawn` immediately before computing the
    /// auxv pointer. libc consumes the bytes once at startup
    /// (`__libc_setup_tls` reads them for stack-canary +
    /// pointer-mangle initialisation), so the value need only stay
    /// alive between `spawn` and the C startup's first read — which,
    /// for a `Process` that survives that far, is the entire process
    /// lifetime.
    pub at_random: Box<[u8; AT_RANDOM_WIDTH]>,
    /// Base address of the FS segment register — the per-thread
    /// pointer musl and glibc install via `arch_prctl(ARCH_SET_FS, …)`
    /// in `_start`'s first instructions (#501 syscall 158). Everything
    /// that relies on TLS (errno, `pthread_self`, stack guard, pointer
    /// canaries) reads through this base: `FS:0x0` is the `pthread`
    /// struct; `FS:0x28` is the stack canary.
    ///
    /// Initialised to 0 in `Process::new` (FS base is undefined until
    /// `arch_prctl(ARCH_SET_FS)` runs — accessing TLS before that is
    /// UB on the x86_64 ABI). On the real x86_64-UEFI target the
    /// `arch_prctl` handler also programs the IA32_FS_BASE MSR
    /// (0xC0000100) so the CPU's FS.base reads this value; unit tests
    /// verify only that the field is stored correctly (no real MSR in
    /// test). `ARCH_GET_FS` reads it back symmetrically.
    pub fs_base: u64,
    /// Current program-break — the first byte ABOVE the process heap.
    /// Set by the `brk(2)` syscall (SYS_BRK = 12, #509). Linux brk
    /// semantics: `brk(0)` returns the current break; `brk(addr ≥
    /// heap_start)` extends (or shrinks) the heap to `addr` and
    /// returns the new break; an invalid or too-low address leaves the
    /// break unchanged and returns the current value.
    ///
    /// Initialised to 0 in `Process::new`. A break of 0 is the
    /// "uninitialised / no heap yet" sentinel: `brk(0)` returns 0 and
    /// `brk(non_zero_addr)` installs the first real break. The actual
    /// page-table install (mapping the [heap_start, new_break) region)
    /// is the boot-integration half (real UEFI target); unit tests
    /// verify only the bookkeeping + validation logic here, mirroring
    /// how `fs_base` + `arch_prctl` gate the MSR write behind
    /// `#[cfg(all(target_os = "uefi", target_arch = "x86_64"))]`.
    pub heap_break: u64,
    /// Monotonic bump pointer for anonymous `mmap(2)` allocations.
    /// SYS_MMAP (9) with MAP_ANONYMOUS advances this pointer forward by
    /// `len` rounded up to PAGE_SIZE (4096) and returns the pre-advance
    /// value as the allocated base address (#497). This is a pure
    /// bookkeeping field — the same "no real page-table manipulation on
    /// this foundation slice" rationale that `heap_break` uses applies
    /// here: the real page-frame allocation + PTE install is deferred to
    /// the boot-integration track (UEFI target, future #497 child tasks).
    ///
    /// Initialised to `MMAP_BASE` (0x7000_0000_0000) in `Process::new`,
    /// a canonical start of the mmap region on Linux x86_64 (below the
    /// 128 TiB user address-space limit, above any typical heap). Two
    /// consecutive mmaps return non-overlapping regions by construction.
    ///
    /// `munmap` does not advance or retreat this pointer (no per-mapping
    /// free list in tier-1 — documented no-op). A real allocator that
    /// tracks individual mappings is a future child task of #497.
    pub mmap_bump: u64,
    /// Per-process signal state — the disposition table, the thread
    /// signal mask, and the `rt_sigreturn` saved-context slot (#548).
    /// Populated by `rt_sigaction` (install/replace a handler) and
    /// `rt_sigprocmask` (block/unblock signals); the saved-context slot
    /// is parked by the (future #549+) delivery path and consumed by
    /// `rt_sigreturn`. Initialised to `SignalState::new()` in
    /// `Process::new`: every signal at SIG_DFL, empty mask, no handler
    /// executing — the disposition Linux boots a process with.
    ///
    /// Lives inline on the Process (like `fd_table` / `fs_base`) rather
    /// than in a global table because all signal state is strictly
    /// per-process; nothing here is shared kernel-wide (contrast the
    /// `futex_table`, which is a kernel-wide rendezvous).
    pub signals: SignalState,
    /// Parent process id — the pid `fork(2)` / `clone(2)` (the future
    /// #530 process-creation surface) records on the child. `None` for
    /// a process with no parent: the initial process the kernel
    /// hand-spawns (tier-1's single resident process) and, on Linux,
    /// any process re-parented to init after its parent dies. The
    /// SIGCHLD path (#551) reads it to decide whom to notify when this
    /// process exits; the process-table lookup that turns the pid into
    /// the live parent Process rides the scheduler (#530).
    pub parent_pid: Option<u32>,
    /// Userspace VA of the thread's robust-futex list head — the
    /// `struct robust_list_head *` registered via `set_robust_list(2)`
    /// (SYS_SET_ROBUST_LIST = 273, #546). `0` means no list is
    /// registered (the default; `get_robust_list` reports it back as a
    /// null head). On thread death the kernel walks this list and stamps
    /// `FUTEX_OWNER_DIED` on every robust mutex the dying thread still
    /// holds so the next acquirer runs recovery
    /// (`syscall::robust_list::walk_on_death`).
    ///
    /// Per-thread on real Linux (each `task_struct` has its own
    /// `robust_list`); tier-1's single-thread model collapses thread and
    /// process, so it lives on the Process like `fs_base` / `heap_break`.
    pub robust_list_head: u64,
    /// Byte length the thread passed to `set_robust_list(2)` alongside
    /// `robust_list_head`. Linux requires it equal
    /// `sizeof(struct robust_list_head)` (24 on LP64) and reports it back
    /// verbatim from `get_robust_list(2)`; the kernel itself only uses
    /// the head pointer + the in-band `futex_offset` to walk. `0` until
    /// `set_robust_list` runs.
    pub robust_list_len: u64,
    /// For a dynamically-linked image (#522): the load base of the
    /// program interpreter (ld-musl), published to userspace as auxv
    /// `AT_BASE`. `None` for a statically-linked image, which omits
    /// AT_BASE. Set by `Process::from_dynamic_image` from the
    /// `DynamicImage` the loader produced; `spawn` emits the AT_BASE row
    /// only when it is `Some`.
    pub interp_base: Option<u64>,
    /// For a dynamically-linked image (#522): the *program's* own entry
    /// point (auxv `AT_ENTRY`), which differs from
    /// `address_space.entry_point` — the latter is the *interpreter's*
    /// entry, where the kernel actually starts execution. `None` for a
    /// static image, where AT_ENTRY is just `entry_point`. Set by
    /// `Process::from_dynamic_image`.
    pub program_entry: Option<u64>,
}

/// Assemble the auxiliary vector `Process::spawn` pushes onto the
/// initial stack. The AT_NULL terminator is appended by
/// `StackBuilder::finalize`, so it is NOT included here.
///
/// For a statically-linked image `interp_base` / `program_entry` are
/// `None` and the result is the System V AMD64 minimum (AT_PHDR /
/// PHENT / PHNUM / PAGESZ / ENTRY / RANDOM). For a dynamically-linked
/// image (#522), `interp_base` is `Some(base)` and AT_BASE is emitted
/// (the interpreter's load base, which the interpreter relocates
/// itself against), and `program_entry` is `Some(e)` so AT_ENTRY
/// carries the PROGRAM's own entry rather than the jump target
/// (`entry_point`, which for a dynamic image is the *interpreter's*
/// entry). Order follows the static fast path with AT_BASE slotted in
/// before AT_ENTRY.
fn build_auxv(
    phdr_addr: u64,
    phdr_count: u64,
    random_addr: u64,
    entry_point: u64,
    interp_base: Option<u64>,
    program_entry: Option<u64>,
) -> Vec<AuxvEntry> {
    let mut auxv = Vec::with_capacity(8);
    auxv.push(AuxvEntry::new(AuxvType::Phdr, phdr_addr));
    auxv.push(AuxvEntry::new(AuxvType::Phent, ELF64_PHENT_SIZE as u64));
    auxv.push(AuxvEntry::new(AuxvType::Phnum, phdr_count));
    auxv.push(AuxvEntry::new(AuxvType::Pagesz, SYS_PAGESZ));
    // AT_BASE only for a dynamically-linked image — the interpreter
    // relocates itself relative to this base. Slotted before AT_ENTRY.
    if let Some(base) = interp_base {
        auxv.push(AuxvEntry::new(AuxvType::Base, base));
    }
    // AT_ENTRY is the PROGRAM's entry. For a static image that's the
    // jump target (`entry_point`); for a dynamic image the jump target
    // is the interpreter's entry, so the program entry is supplied
    // separately via `program_entry`.
    auxv.push(AuxvEntry::new(
        AuxvType::Entry,
        program_entry.unwrap_or(entry_point),
    ));
    auxv.push(AuxvEntry::new(AuxvType::Random, random_addr));
    auxv
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
            // Allocate the AT_RANDOM buffer up-front (zero-filled) so
            // the address is stable from `new` onward; `spawn` overwrites
            // the contents with CSPRNG bytes before recording the
            // pointer in the auxv. The Box keeps the bytes pinned even
            // when the Process moves into CURRENT_PROCESS post-spawn.
            at_random: Box::new([0u8; AT_RANDOM_WIDTH]),
            // FS base starts undefined (0). musl's `_start` calls
            // `arch_prctl(ARCH_SET_FS, tp)` in its very first
            // instructions (#501); until that syscall fires, any TLS
            // access (errno, stack canary, pthread_self) would read
            // through FS:0 — undefined behaviour on the x86_64 ABI.
            // The handler writes the real pointer here and, on the
            // x86_64-UEFI target, also programs the IA32_FS_BASE MSR.
            fs_base: 0,
            // Heap break starts at 0 — no heap yet. The first
            // `brk(non_zero_addr)` from userspace installs the initial
            // break. `brk(0)` before any real brk call returns 0,
            // which is the conventional "heap not yet mapped" sentinel
            // libc uses to discover the initial break on startup
            // (e.g., musl's `__brk` init path). The handler in
            // syscall::brk (#509) updates this field; the real
            // page-table mapping of the heap region is deferred to
            // the boot-integration track (UEFI target only).
            heap_break: 0,
            // mmap bump pointer starts at the canonical mmap base for
            // Linux x86_64 (0x7000_0000_0000). The first anonymous
            // mmap returns this address; each subsequent call advances
            // the pointer by len-rounded-up-to-4096. No real PTE
            // install happens on this foundation slice — same
            // rationale as heap_break above.
            mmap_bump: crate::syscall::mmap::MMAP_BASE,
            // Signal state starts at the Linux boot disposition: every
            // signal at SIG_DFL, empty thread mask, no handler running.
            // `rt_sigaction` / `rt_sigprocmask` (#548) mutate it; the
            // delivery path (#549+) parks the saved context that
            // `rt_sigreturn` restores.
            signals: SignalState::new(),
            // No parent until fork(2)/clone(2) (#530) records one. The
            // initial hand-spawned process is parentless; #551 reads
            // this to decide whom SIGCHLD wakes on exit.
            parent_pid: None,
            // No robust-futex list until `set_robust_list(2)` (#546)
            // registers one. `0` is the "no list" sentinel: the
            // exit-time walk no-ops and `get_robust_list` reports a null
            // head. glibc/musl call `set_robust_list` from their thread
            // bring-up (`__pthread_init` / `__init_tp`), so a real
            // pthreads binary populates this early.
            robust_list_head: 0,
            robust_list_len: 0,
            // Static image by default — no interpreter. `from_dynamic_image`
            // sets these for a dynamically-linked program (#522).
            interp_base: None,
            program_entry: None,
        }
    }

    /// Build a Process around a `DynamicImage` (a dynamically-linked
    /// program loaded together with its interpreter — #522). The
    /// address space is the combined program + interpreter image (its
    /// `entry_point` is the interpreter's entry, where the kernel starts
    /// execution); `interp_base` (auxv `AT_BASE`) and `program_entry`
    /// (auxv `AT_ENTRY`) are carried so `spawn` publishes them to the C
    /// startup. Everything else matches `new`.
    pub fn from_dynamic_image(pid: u32, image: DynamicImage) -> Self {
        let mut proc = Self::new(pid, image.address_space);
        proc.interp_base = Some(image.interp_base);
        proc.program_entry = Some(image.program_entry);
        proc
    }

    /// True when this process is a child of `parent_pid` — the
    /// predicate the SIGCHLD path (#551) uses to confirm a candidate
    /// parent before delivering the signal. False for a parentless
    /// process (the initial / re-parented-to-init case).
    pub fn is_child_of(&self, parent_pid: u32) -> bool {
        self.parent_pid == Some(parent_pid)
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
        // we point at the random bytes' kernel-space address, which
        // (under UEFI's identity mapping) coincides with the userspace
        // VA the future page-table install will use. Same identity-
        // mapping rationale `AddressSpace`'s PhysAddr re-derivation
        // uses (process/address_space.rs:42).
        //
        // The bytes are sourced from `arest::csprng::random_bytes`
        // — the process-wide ChaCha20 csprng. (The host-side public
        // entry point is `arest::crypto::random_bytes` per #578, but
        // that module is `cfg(not(feature = "no_std"))`-gated, so the
        // kernel reaches the same delegate target directly.) The
        // csprng is seeded once at boot from the installed
        // `entropy::EntropySource` —
        // `cli::entropy_host::HostEntropySource` for the host CLI
        // (#574), `arch::uefi::x86_64::entropy::HardwareEntropySource`
        // for the UEFI x86_64 kernel (#569 RDSEED/RDRAND, #571 UEFI
        // EFI_RNG_PROTOCOL fallback). Tests pin the source to a
        // `DeterministicSource` so consecutive spawns produce
        // predictable-yet-distinct seeds.
        //
        // We fill INTO `*self.at_random` (the Boxed buffer the Process
        // owns) rather than a stack array because the auxv records
        // the address of those bytes — a stack-local would dangle the
        // moment `spawn` returns and the trampoline reads the auxv.
        arest::csprng::random_bytes(self.at_random.as_mut_slice());
        let phdr_count = self.address_space.segments.len() as u64;
        let phdr_addr = self
            .address_space
            .segments
            .first()
            .map(|s| s.vaddr)
            .unwrap_or(0);
        let entry = self.address_space.entry_point;
        let random_addr = self.at_random.as_ptr() as u64;
        // For a static image `interp_base` / `program_entry` are `None`
        // (the System V minimum, AT_ENTRY == entry); for a dynamically-
        // linked image (#522) they carry AT_BASE + the program-entry
        // AT_ENTRY. `build_auxv` does NOT emit AT_NULL — `finalize`
        // owns the terminator.
        let auxv = build_auxv(
            phdr_addr,
            phdr_count,
            random_addr,
            entry,
            self.interp_base,
            self.program_entry,
        );

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
        // `build_auxv` returns the real entries only (no AT_NULL —
        // `StackBuilder::finalize` owns the terminator), so push them all.
        for aux in &auxv {
            builder = builder.push_auxv(*aux);
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
        // BlockedFutex / Killed render as the bare variant name without
        // their payload — the cell shape stays a single string for
        // forward-compat with the existing Process_has_State consumers
        // (BlockedFutex's uaddr lands in #545's Futex_has_Waiter cell;
        // Killed's terminating signal stays on the state variant for
        // the future wait(2) surface, #531).
        let state_atom = match self.state {
            ProcessState::Created => "Created",
            ProcessState::Running => "Running",
            ProcessState::SpawnFailed => "SpawnFailed",
            ProcessState::Exited => "Exited",
            ProcessState::BlockedFutex(_) => "BlockedFutex",
            ProcessState::Killed(_) => "Killed",
        };
        s = cell_push(
            "Process_has_State",
            fact_from_pairs(&[("Process", process_id), ("State", state_atom)]),
            &s,
        );
        // Parent linkage (#551) — sparse: a parentless process (the
        // initial process / re-parented-to-init) earns no fact.
        if let Some(ppid) = self.parent_pid {
            let parent_atom = format!("{}", ppid);
            s = cell_push(
                "Process_has_Parent",
                fact_from_pairs(&[("Process", process_id), ("Parent", &parent_atom)]),
                &s,
            );
        }
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
        // Compose the per-process signal state's facts (the blocked
        // mask + any non-default dispositions + the handler-active
        // flag). See `SignalState::record_into_cells` (#548).
        s = self.signals.record_into_cells(process_id, &s);
        // Compose the address-space cells last so a debugger / cell
        // inspector sees them as children of the Process_has_State
        // / Process_has_Pid facts.
        self.address_space.record_into_cells(process_id, &s)
    }

    /// Deliver signal `signum` to this process and apply the part #549
    /// owns. Computes the pure delivery decision
    /// (`SignalState::delivery_decision`) against this process's own
    /// disposition table, then enacts the termination transition: a
    /// `Terminate` or `CoreDump` outcome moves the process to
    /// `ProcessState::Killed(signum)`, carrying the terminating signal
    /// for the future `wait(2)` surface (#531).
    ///
    /// Returns the decision so the caller can drive the outcomes #549
    /// does NOT yet enact: the ring-3 handler redirect for
    /// `RunHandler` (#549 follow-up), the core-file write for
    /// `CoreDump` (#550 — the state transition here already kills the
    /// process; #550 only adds the dump), and job-control suspend /
    /// resume for `Stop` / `Continue` (#530). `Ignore` and an
    /// out-of-range signal (`None`) leave the state untouched.
    ///
    /// The un-catchable invariant lives in `delivery_decision`: SIGKILL
    /// resolves to `Terminate` even with a handler in the table, so
    /// this method terminates regardless — there is no "catch SIGKILL"
    /// branch to forget here.
    pub fn deliver_signal(&mut self, signum: i32) -> Option<SignalDelivery> {
        let decision = self.signals.delivery_decision(signum)?;
        if matches!(
            decision,
            SignalDelivery::Terminate | SignalDelivery::CoreDump
        ) {
            self.state = ProcessState::Killed(signum);
            // A thread killed by a fatal signal while holding robust
            // mutexes must still run owner-death recovery — Linux reaches
            // `exit_robust_list` from `do_exit`, which BOTH a clean
            // exit(2) and a fatal-signal death funnel through. Walk the
            // registered robust list here so `FUTEX_OWNER_DIED` is
            // stamped on the kill path too, not just the exit syscall
            // (#546). No-op when no list is registered (head == 0), which
            // is the case for every process that never called
            // `set_robust_list` — so the signal unit tests are
            // unaffected.
            crate::syscall::robust_list::walk_on_death(
                self.robust_list_head,
                self.robust_list_len,
                self.pid,
            );
        }
        Some(decision)
    }

    /// Raise SIGSEGV against this process for a page fault at
    /// `fault_addr` — the #550 fault-delivery path. `present` is the
    /// #PF error-code present-bit (`true` ⇒ a mapped-but-protected
    /// access → `SEGV_ACCERR`; `false` ⇒ an unmapped address →
    /// `SEGV_MAPERR`).
    ///
    /// Builds the `SigInfo` a SA_SIGINFO handler reads (`si_addr` =
    /// the fault address) and drives delivery through
    /// `deliver_signal(SIGSEGV)`, so the default disposition (no
    /// handler) dumps core + terminates (`CoreDump` →
    /// `Killed(SIGSEGV)`, #549) while an installed handler is reported
    /// as `RunHandler` WITHOUT terminating — letting a JIT or language
    /// runtime recover from a speculative fault. Returns the
    /// `(decision, siginfo)` pair: the caller — the x86_64 #PF handler,
    /// gated on the ring-3 descent (#552) — drives the ring-3 handler
    /// redirect for `RunHandler` and the core-file write for
    /// `CoreDump`.
    pub fn raise_segv(&mut self, fault_addr: u64, present: bool) -> (SignalDelivery, SigInfo) {
        let info = SigInfo::segv(fault_addr, present);
        // SIGSEGV is always a valid signum, so `deliver_signal` is
        // `Some`; the CoreDump/Terminate → Killed transition lives
        // there (#549), so a defaulted SIGSEGV terminates here.
        let decision = self
            .deliver_signal(SIGSEGV)
            .expect("SIGSEGV is a valid signal number");
        (decision, info)
    }

    /// Notify this process's parent that the process exited, by
    /// delivering SIGCHLD to `parent` — the #551 child-reaping path.
    /// `parent` is the candidate parent the kernel resolved from
    /// `self.parent_pid` (the process-table lookup that finds it rides
    /// the scheduler #530); the method guards on `is_child_of`, so a
    /// mismatched parent — or a parentless process — signals no one and
    /// returns `None`. On a match it returns the parent's SIGCHLD
    /// delivery decision: `Ignore` under the POSIX default (no handler
    /// installed — the parent's state is untouched, SIGCHLD's default
    /// being Ignore), or `RunHandler` when the parent registered a
    /// SIGCHLD handler (the shell / service-supervisor reap path that
    /// wait()s the child).
    ///
    /// The wait() / waitpid() wakeup that unblocks a parent sleeping on
    /// its child is the #531 surface; this handles the signal half (the
    /// asynchronous notification) only.
    pub fn notify_parent_exit(&self, parent: &mut Process) -> Option<SignalDelivery> {
        if !self.is_child_of(parent.pid) {
            return None;
        }
        parent.deliver_signal(SIGCHLD)
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

/// Test-only serialisation lock for the process-global
/// `CURRENT_PROCESS` singleton. `cargo test` runs tests in parallel by
/// default; every test that calls `current_process_install` /
/// `current_process_uninstall` (the syscall handler test suites in
/// `syscall::openat`, `syscall::close`, `syscall::exit`, `syscall::futex`)
/// touches the same `spin::Mutex<Option<Process>>` slot above.
/// Without serialisation, two tests racing on install/uninstall clobber
/// each other's process — e.g. `syscall::openat::tests::
/// sequential_opens_allocate_increasing_fds` expects fds to land at 3
/// then 4, but if a sibling test installs a fresh process between the
/// two `handle()` calls the fd-table state resets and the second
/// handle returns 3 instead of 4.
///
/// All syscall test modules that touch `CURRENT_PROCESS` acquire this
/// lock at the top of their bodies. Same shape as the
/// `TEST_ENTROPY_LOCK` and `TEST_NET_LOCK` patterns (#658) — the lock
/// is per-resource, not per-module, so concurrent tests that don't
/// touch the process global still parallelise.
#[cfg(test)]
pub(crate) static CURRENT_PROCESS_TEST_LOCK: spin::Mutex<()> = spin::Mutex::new(());

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

/// Run a closure against the currently-installed Process's signal
/// state (the per-process `SignalState` introduced by the signal
/// plumbing, #548). Sibling of `current_process_fd_table` — same
/// closure shape, same lock discipline, scoped to the signal state so
/// the `rt_sigaction` / `rt_sigprocmask` / `rt_sigreturn` handlers
/// don't have to ferry an `Option<&mut Process>` through their bodies
/// just to reach the table + mask + saved-context slot.
///
/// The closure receives `Option<&mut SignalState>` — `Some` when a
/// process is installed, `None` when not (kernel boot before any
/// spawn, or test teardown). The caller is responsible for handling
/// `None` — the signal handlers map it to `-ESRCH` (the Linux errno a
/// signal syscall returns when there's no addressable task), keeping
/// the surface honest about "called before any process is live".
///
/// Holds the singleton's `spin::Mutex` for the duration of the
/// closure — same constraint as `current_process_mut`: don't park /
/// await inside (no async in the kernel today).
pub fn current_process_signals<F, R>(f: F) -> R
where
    F: FnOnce(Option<&mut SignalState>) -> R,
{
    let mut guard = CURRENT_PROCESS.lock();
    f(guard.as_mut().map(|p| &mut p.signals))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::process::address_space::SegmentPerm;
    use arest::entropy::{self, DeterministicSource};

    /// Test fixture: install a deterministic entropy source seeded with
    /// `seed`, force a CSPRNG reseed so subsequent `random_bytes` calls
    /// derive from `seed`, run the body, then uninstall + reseed so
    /// the next test starts clean.
    ///
    /// Mirrors `arest::crypto::tests::with_deterministic_entropy` and
    /// `arest::csprng::tests::with_deterministic_csprng`. Required
    /// because `Process::spawn` now calls `arest::crypto::random_bytes`
    /// (#575) which panics if no entropy source is installed — every
    /// spawn-touching test needs to either install a source or skip
    /// the spawn step.
    ///
    /// The cross-module `arest::entropy::TEST_LOCK` is `pub(crate)` to
    /// `arest`, so we can't reach it from arest-kernel; we serialise
    /// kernel-side via `cargo test`'s default per-test ordering plus
    /// the fact that no other kernel test touches `entropy`. If a
    /// future kernel test races against this fixture, lift the lock
    /// to `pub` in `arest::entropy`.
    ///
    /// `cargo test` runs tests in parallel by default. The
    /// `entropy::install` / `entropy::uninstall` pair touches a
    /// process-global slot (`arest::entropy::GLOBAL_SOURCE`), and the
    /// csprng reseed/random_bytes path reads it. Without
    /// serialization, two `with_deterministic_entropy` calls running
    /// concurrently can race: thread A installs source A and reseeds,
    /// then thread B installs source B (overwriting A) and reseeds,
    /// then thread A's body calls random_bytes against B's source
    /// (or worse — between A's uninstall and A's body's next
    /// random_bytes, the global is `None` and seed_from_entropy
    /// panics).
    ///
    /// `TEST_ENTROPY_LOCK` serializes the entire install / body /
    /// uninstall sequence so each test sees a clean entropy world for
    /// the duration of its body. Cost is parallelism within the
    /// entropy-touching subset of tests; everything else still runs
    /// in parallel.
    static TEST_ENTROPY_LOCK: spin::Mutex<()> = spin::Mutex::new(());

    // pub(crate): shared with sibling test modules (process::exec) so
    // every entropy-touching test serializes on the same lock.
    pub(crate) fn with_deterministic_entropy<F: FnOnce()>(seed: [u8; 32], body: F) {
        let _guard = TEST_ENTROPY_LOCK.lock();
        entropy::install(alloc::boxed::Box::new(DeterministicSource::new(seed)));
        arest::csprng::reseed();
        body();
        entropy::uninstall();
        arest::csprng::reseed();
    }

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
        with_deterministic_entropy([1u8; 32], || {
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
        });
    }

    /// `Process::spawn` walks the argv list correctly — the populated
    /// stack reports the correct argc.
    #[test]
    fn spawn_argc_matches_argv_count() {
        with_deterministic_entropy([2u8; 32], || {
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
        });
    }

    /// `Process::spawn` emits the auxv entries in the order the spawn
    /// builds them. This is structural — the test reads the populated
    /// region and confirms the entries land where the layout
    /// constants predict (after argv NULL, envp NULL).
    #[test]
    fn spawn_auxv_layout_matches_spec() {
        with_deterministic_entropy([3u8; 32], || {
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
        });
    }

    /// `Process::spawn` populates AT_PHNUM with the segment count.
    /// Used by libc to walk the loaded program headers.
    #[test]
    fn spawn_at_phnum_reflects_segment_count() {
        with_deterministic_entropy([4u8; 32], || {
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
        });
    }

    /// `Process::spawn` populates AT_PAGESZ with 4096.
    #[test]
    fn spawn_at_pagesz_is_4096() {
        with_deterministic_entropy([5u8; 32], || {
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
        });
    }

    /// `Process::spawn` populates AT_ENTRY with the address space's
    /// entry point. Mirrors the trampoline's iretq RIP value.
    #[test]
    fn spawn_at_entry_matches_address_space() {
        with_deterministic_entropy([6u8; 32], || {
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
        });
    }

    /// `Process::spawn` populates AT_RANDOM with a non-zero address.
    /// The address points at the per-process 16-byte CSPRNG buffer
    /// the spawn just filled from `arest::crypto::random_bytes`.
    #[test]
    fn spawn_at_random_is_non_zero() {
        with_deterministic_entropy([7u8; 32], || {
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
            // The recorded address must point at the per-process
            // at_random buffer the spawn just filled.
            assert_eq!(
                u64::from_le_bytes(buf),
                proc.at_random.as_ptr() as u64,
                "AT_RANDOM auxv value must match Process.at_random buffer address"
            );
        });
    }

    /// #575 / Rand-C1: the AT_RANDOM bytes must come from
    /// `arest::crypto::random_bytes` (which delegates to the seeded
    /// ChaCha20 CSPRNG installed by the entropy framework), NOT a
    /// hardcoded literal. Two consecutive spawns under DIFFERENT
    /// deterministic seeds must produce DIFFERENT 16-byte buffers —
    /// proves the bytes track the entropy source rather than coming
    /// from rodata.
    #[test]
    fn at_random_is_random() {
        let mut bytes_seed_a = [0u8; AT_RANDOM_WIDTH];
        let mut bytes_seed_b = [0u8; AT_RANDOM_WIDTH];
        with_deterministic_entropy([0xAAu8; 32], || {
            let mut address_space = AddressSpace::new(0x40_1000);
            address_space
                .push_segment(0x40_1000, 0x10, SegmentPerm::ReadExecute, &[0x90; 8])
                .expect(".text push");
            let mut proc = Process::new(1, address_space);
            let _ = proc.spawn(&[], &[]);
            bytes_seed_a.copy_from_slice(proc.at_random.as_ref());
        });
        with_deterministic_entropy([0xBBu8; 32], || {
            let mut address_space = AddressSpace::new(0x40_1000);
            address_space
                .push_segment(0x40_1000, 0x10, SegmentPerm::ReadExecute, &[0x90; 8])
                .expect(".text push");
            let mut proc = Process::new(1, address_space);
            let _ = proc.spawn(&[], &[]);
            bytes_seed_b.copy_from_slice(proc.at_random.as_ref());
        });
        assert_ne!(
            bytes_seed_a, bytes_seed_b,
            "AT_RANDOM bytes must differ when the entropy seed differs — \
             proves the bytes flow from arest::crypto::random_bytes rather \
             than a hardcoded rodata literal"
        );
        // Defensive: neither buffer should be the legacy
        // b"AREST_TIER_1_RNG" placeholder.
        assert_ne!(&bytes_seed_a, b"AREST_TIER_1_RNG");
        assert_ne!(&bytes_seed_b, b"AREST_TIER_1_RNG");
    }

    /// #575 / Rand-C1: AT_RANDOM must be exactly 16 bytes per the
    /// ELF AUX_RANDOM spec. libc's stack-canary / pointer-mangle
    /// initialiser reads exactly 16 bytes from the address auxv
    /// records — a smaller buffer would leak adjacent kernel/user
    /// memory, a larger one wastes the heap allocation.
    #[test]
    fn at_random_is_16_bytes() {
        with_deterministic_entropy([0xCCu8; 32], || {
            let address_space = AddressSpace::new(0x40_1000);
            let proc = Process::new(1, address_space);
            // The Box payload's runtime length must be exactly 16.
            assert_eq!(
                proc.at_random.len(),
                AT_RANDOM_WIDTH,
                "AT_RANDOM buffer must be exactly 16 bytes (ELF AUX_RANDOM spec)"
            );
            // Type-level check: AT_RANDOM_WIDTH equals 16, the named
            // constant guards against a careless copy-paste shrinking
            // the buffer width.
            assert_eq!(AT_RANDOM_WIDTH, 16);
        });
    }

    /// `Process::spawn` appends AT_NULL terminator to the auxv.
    /// Comes after the seven explicit entries.
    #[test]
    fn spawn_auxv_terminated_with_at_null() {
        with_deterministic_entropy([8u8; 32], || {
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
        });
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
        // Count fact-shape `Backend` pair occurrences — that pair is
        // unique to Process_has_FdTable facts (Pid/State/EntryPoint
        // facts don't carry it). The cell-name `Process_has_FdTable`
        // itself appears once regardless of fact count, so we can't
        // count by that string. Two open fds (0 = Serial, 2 = Serial)
        // → two `Backend` pairs in the recorded Object.
        let count = serialised.matches("Backend").count();
        assert_eq!(count, 2, "Closed fd 1 must elide; expected one Backend pair per remaining fd (0 + 2)");
    }

    // -- #549: signal-driven process termination ---------------------

    use crate::process::signal::{
        SigAction, SigInfo, SignalDelivery, SEGV_ACCERR, SEGV_MAPERR, SIGCHLD, SIGKILL, SIGSEGV,
        SIGTERM,
    };

    /// Delivering SIGTERM (default disposition, no handler installed)
    /// transitions the process to `Killed(SIGTERM)` and reports the
    /// Terminate outcome — the catchable-but-uncaught path.
    #[test]
    fn deliver_sigterm_transitions_to_killed() {
        let address_space = AddressSpace::new(0x40_1000);
        let mut proc = Process::new(7, address_space);
        let outcome = proc.deliver_signal(SIGTERM);
        assert_eq!(outcome, Some(SignalDelivery::Terminate));
        assert_eq!(proc.state, ProcessState::Killed(SIGTERM));
    }

    /// SIGKILL is uncatchable end-to-end: even with a userspace handler
    /// forced into the process's signal table, delivering SIGKILL still
    /// terminates the process. The #549 headline through the Process
    /// surface.
    #[test]
    fn deliver_sigkill_uncatchable_terminates() {
        let address_space = AddressSpace::new(0x40_1000);
        let mut proc = Process::new(9, address_space);
        proc.signals
            .set_action(
                SIGKILL,
                SigAction { handler: 0x4444_0000, flags: 0, restorer: 0, mask: 0 },
            )
            .unwrap();
        let outcome = proc.deliver_signal(SIGKILL);
        assert_eq!(outcome, Some(SignalDelivery::Terminate));
        assert_eq!(proc.state, ProcessState::Killed(SIGKILL));
    }

    /// SIGTERM with a handler installed is *caught*: delivery reports
    /// RunHandler and the process is NOT terminated (the handler-run
    /// ring-3 redirect is the #549 follow-up track, but the termination
    /// transition must not fire for a caught signal).
    #[test]
    fn deliver_sigterm_with_handler_does_not_kill() {
        let address_space = AddressSpace::new(0x40_1000);
        let mut proc = Process::new(15, address_space);
        proc.signals
            .set_action(
                SIGTERM,
                SigAction { handler: 0x5555_0000, flags: 0, restorer: 0, mask: 0 },
            )
            .unwrap();
        let outcome = proc.deliver_signal(SIGTERM);
        assert_eq!(outcome, Some(SignalDelivery::RunHandler(0x5555_0000)));
        assert_eq!(
            proc.state,
            ProcessState::Created,
            "a caught signal must not terminate the process"
        );
    }

    /// A default-Ignore signal (SIGCHLD) delivered to a process with no
    /// handler is a silent no-op: Ignore outcome, state unchanged.
    #[test]
    fn deliver_ignored_signal_leaves_state() {
        let address_space = AddressSpace::new(0x40_1000);
        let mut proc = Process::new(17, address_space);
        let outcome = proc.deliver_signal(SIGCHLD);
        assert_eq!(outcome, Some(SignalDelivery::Ignore));
        assert_eq!(proc.state, ProcessState::Created);
    }

    /// A `Killed` process projects its state cell as "Killed". The
    /// terminating signal stays on the state variant for the future
    /// wait(2) surface; the cell renders the name, matching the
    /// `BlockedFutex` precedent.
    #[test]
    fn killed_state_records_into_state_cell() {
        let address_space = AddressSpace::new(0x40_1000);
        let mut proc = Process::new(15, address_space);
        proc.deliver_signal(SIGTERM);
        let recorded = proc.record_into_cells("proc15", &Object::phi());
        let serialised = format!("{:?}", recorded);
        assert!(serialised.contains("Process_has_State"));
        assert!(serialised.contains("Killed"), "state cell must render Killed");
    }

    // -- #550: SIGSEGV from a page fault -----------------------------

    /// A page fault at an unmapped address with no SIGSEGV handler:
    /// `raise_segv` reports CoreDump (default action = core + terminate),
    /// transitions the process to `Killed(SIGSEGV)`, and the returned
    /// siginfo carries the fault address + `SEGV_MAPERR`.
    #[test]
    fn raise_segv_default_dumps_core_and_kills() {
        let address_space = AddressSpace::new(0x40_1000);
        let mut proc = Process::new(11, address_space);
        let (delivery, info): (SignalDelivery, SigInfo) = proc.raise_segv(0xdead_0000, false);
        assert_eq!(delivery, SignalDelivery::CoreDump);
        assert_eq!(proc.state, ProcessState::Killed(SIGSEGV));
        assert_eq!(info.signo, SIGSEGV);
        assert_eq!(info.addr, 0xdead_0000);
        assert_eq!(info.code, SEGV_MAPERR);
    }

    /// A page fault when the process installed a SIGSEGV handler (the
    /// JIT / language-runtime recovery case): `raise_segv` reports
    /// RunHandler, does NOT terminate the process, and the siginfo
    /// carries the fault address + `SEGV_ACCERR` (mapped-but-protected)
    /// for the handler to inspect via `si_addr`.
    #[test]
    fn raise_segv_with_handler_runs_handler_without_killing() {
        let address_space = AddressSpace::new(0x40_1000);
        let mut proc = Process::new(11, address_space);
        proc.signals
            .set_action(
                SIGSEGV,
                SigAction { handler: 0x6000_0000, flags: 0, restorer: 0, mask: 0 },
            )
            .unwrap();
        let (delivery, info): (SignalDelivery, SigInfo) = proc.raise_segv(0x4020_0000, true);
        assert_eq!(delivery, SignalDelivery::RunHandler(0x6000_0000));
        assert_eq!(
            proc.state,
            ProcessState::Created,
            "a handled SIGSEGV must not terminate the process"
        );
        assert_eq!(info.addr, 0x4020_0000);
        assert_eq!(info.code, SEGV_ACCERR);
    }

    // -- #551: SIGCHLD parent linkage --------------------------------

    /// A fresh process has no parent — `parent_pid` defaults to None
    /// (the initial process the kernel hand-spawns; fork(2) sets one
    /// on real children).
    #[test]
    fn new_process_has_no_parent() {
        let address_space = AddressSpace::new(0x40_1000);
        let proc = Process::new(2, address_space);
        assert_eq!(proc.parent_pid, None);
        assert!(!proc.is_child_of(1));
    }

    /// Setting a parent pid makes `is_child_of` true for that pid and
    /// false for any other.
    #[test]
    fn is_child_of_reflects_parent_pid() {
        let address_space = AddressSpace::new(0x40_1000);
        let mut proc = Process::new(7, address_space);
        proc.parent_pid = Some(1);
        assert!(proc.is_child_of(1));
        assert!(!proc.is_child_of(2));
    }

    /// `record_into_cells` emits Process_has_Parent when a parent is
    /// set and elides it for a parentless process.
    #[test]
    fn record_into_cells_emits_parent_when_set() {
        let address_space = AddressSpace::new(0x40_1000);
        let mut child = Process::new(7, address_space);
        child.parent_pid = Some(1);
        let with_parent = format!("{:?}", child.record_into_cells("p7", &Object::phi()));
        assert!(with_parent.contains("Process_has_Parent"));

        let orphan_space = AddressSpace::new(0x40_1000);
        let orphan = Process::new(1, orphan_space);
        let without = format!("{:?}", orphan.record_into_cells("p1", &Object::phi()));
        assert!(!without.contains("Process_has_Parent"));
    }

    /// When a child exits, notifying its parent delivers SIGCHLD. With
    /// no handler installed the parent's default disposition is Ignore
    /// (SIGCHLD's POSIX default) — the delivery is reported but the
    /// parent's state is untouched.
    #[test]
    fn notify_parent_exit_delivers_sigchld_default_ignore() {
        let child_space = AddressSpace::new(0x40_1000);
        let mut child = Process::new(7, child_space);
        child.parent_pid = Some(1);
        let parent_space = AddressSpace::new(0x40_1000);
        let mut parent = Process::new(1, parent_space);
        let outcome = child.notify_parent_exit(&mut parent);
        assert_eq!(outcome, Some(SignalDelivery::Ignore));
        assert_eq!(parent.state, ProcessState::Created);
    }

    /// A parent that installed a SIGCHLD handler gets RunHandler — the
    /// shell / service-supervisor reaping path (the handler runs to
    /// wait() the child).
    #[test]
    fn notify_parent_exit_runs_parent_handler() {
        let child_space = AddressSpace::new(0x40_1000);
        let mut child = Process::new(7, child_space);
        child.parent_pid = Some(1);
        let parent_space = AddressSpace::new(0x40_1000);
        let mut parent = Process::new(1, parent_space);
        parent
            .signals
            .set_action(
                SIGCHLD,
                SigAction { handler: 0x7000_0000, flags: 0, restorer: 0, mask: 0 },
            )
            .unwrap();
        assert_eq!(
            child.notify_parent_exit(&mut parent),
            Some(SignalDelivery::RunHandler(0x7000_0000))
        );
    }

    /// Notifying a process that is NOT the child's parent delivers
    /// nothing and leaves it untouched — the kernel signals only the
    /// real parent.
    #[test]
    fn notify_parent_exit_wrong_parent_is_none() {
        let child_space = AddressSpace::new(0x40_1000);
        let mut child = Process::new(7, child_space);
        child.parent_pid = Some(1);
        let other_space = AddressSpace::new(0x40_1000);
        let mut other = Process::new(99, other_space);
        let before = other.signals.action(SIGCHLD);
        assert_eq!(child.notify_parent_exit(&mut other), None);
        assert_eq!(other.signals.action(SIGCHLD), before);
    }

    /// A parentless (orphan / init) child notifies no one — None.
    #[test]
    fn notify_parent_exit_orphan_is_none() {
        let child_space = AddressSpace::new(0x40_1000);
        let child = Process::new(1, child_space);
        let other_space = AddressSpace::new(0x40_1000);
        let mut other = Process::new(2, other_space);
        assert_eq!(child.notify_parent_exit(&mut other), None);
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
        with_deterministic_entropy([9u8; 32], || {
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
        });
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

    /// `build_auxv` for a static image (no interpreter) emits the System
    /// V minimum with AT_ENTRY == the entry point and NO AT_BASE — and
    /// no AT_NULL (the stack builder owns the terminator).
    #[test]
    fn build_auxv_static_omits_base_entry_is_entry_point() {
        let auxv = build_auxv(0x1000, 2, 0x2000, 0x0040_1000, None, None);
        assert!(
            !auxv.iter().any(|e| e.a_type == AuxvType::Base as u64),
            "a static image omits AT_BASE"
        );
        assert!(
            !auxv.iter().any(|e| e.a_type == AuxvType::Null as u64),
            "build_auxv omits AT_NULL (finalize owns it)"
        );
        let at_entry = auxv
            .iter()
            .find(|e| e.a_type == AuxvType::Entry as u64)
            .expect("AT_ENTRY present");
        assert_eq!(at_entry.a_val, 0x0040_1000);
    }

    /// `build_auxv` for a dynamic image (#522) emits AT_BASE = the
    /// interpreter load base, and AT_ENTRY = the PROGRAM's entry — NOT
    /// the jump target (`entry_point`, which is the interpreter's entry).
    #[test]
    fn build_auxv_dynamic_emits_base_and_program_entry() {
        let interp_entry = 0x0000_1000_0040_1000; // jump target = interp entry
        let prog_entry = 0x0040_1000;
        let base = 0x0000_1000_0000_0000;
        let auxv = build_auxv(0x1000, 2, 0x2000, interp_entry, Some(base), Some(prog_entry));
        let at_base = auxv
            .iter()
            .find(|e| e.a_type == AuxvType::Base as u64)
            .expect("AT_BASE present for a dynamic image");
        assert_eq!(at_base.a_val, base);
        let at_entry = auxv
            .iter()
            .find(|e| e.a_type == AuxvType::Entry as u64)
            .expect("AT_ENTRY present");
        assert_eq!(
            at_entry.a_val, prog_entry,
            "AT_ENTRY is the program entry, not the interpreter jump target"
        );
    }

    /// `Process::from_dynamic_image` carries the `DynamicImage`'s
    /// AT_BASE / AT_ENTRY onto the Process and adopts the combined
    /// program+interpreter address space (whose entry_point is the
    /// interpreter's entry — the kernel's jump target).
    #[test]
    fn from_dynamic_image_carries_base_and_program_entry() {
        let space = AddressSpace::new(0xdead_0000);
        let img = DynamicImage {
            address_space: space,
            interp_base: 0x7f00_0000,
            program_entry: 0x0040_1000,
        };
        let proc = Process::from_dynamic_image(9, img);
        assert_eq!(proc.interp_base, Some(0x7f00_0000));
        assert_eq!(proc.program_entry, Some(0x0040_1000));
        assert_eq!(proc.address_space.entry_point, 0xdead_0000);
        // A from_dynamic_image process starts Created, like new().
        assert_eq!(proc.state, ProcessState::Created);
    }

    /// End-to-end (#522): spawning a Process built from a DynamicImage
    /// lands AT_BASE (= the interpreter load base) AND AT_ENTRY (= the
    /// PROGRAM entry, not the interpreter jump target) on the actual
    /// initial stack. This verifies the seam the unit tests don't:
    /// from_dynamic_image → build_auxv → spawn push → StackBuilder
    /// finalize all compose so the dynamic auxv reaches userspace.
    #[test]
    fn spawn_dynamic_image_lands_at_base_on_stack() {
        with_deterministic_entropy([7u8; 32], || {
            // entry_point is the interpreter's entry (the jump target);
            // interp_base / program_entry are the dynamic auxv values.
            let mut address_space = AddressSpace::new(0x8000_4000);
            address_space
                .push_segment(0x0040_1000, 0x10, SegmentPerm::ReadExecute, &[0x90; 8])
                .expect("program .text push");
            let img = DynamicImage {
                address_space,
                interp_base: 0x8000_0000,
                program_entry: 0x0040_1000,
            };
            let mut proc = Process::from_dynamic_image(1, img);
            let _ = proc.spawn(&[], &[]); // tier-1: trampoline fails after stack build
            let stack = proc.initial_stack.as_ref().unwrap();
            let pop = stack.populated();
            let rd = |off: usize| {
                let mut b = [0u8; 8];
                b.copy_from_slice(&pop[off..off + 8]);
                u64::from_le_bytes(b)
            };
            // Empty argv/envp → header is argc(8)+argvNULL(8)+envpNULL(8)
            // = 24. Dynamic auxv order: PHDR, PHENT, PHNUM, PAGESZ, BASE,
            // ENTRY, RANDOM. AT_BASE is the 5th entry → 24 + 4*16 = 88.
            assert_eq!(rd(88), AuxvType::Base as u64, "AT_BASE key present for dynamic image");
            assert_eq!(rd(96), 0x8000_0000, "AT_BASE value == interpreter load base");
            // AT_ENTRY is the 6th entry → 24 + 5*16 = 104.
            assert_eq!(rd(104), AuxvType::Entry as u64, "AT_ENTRY key follows AT_BASE");
            assert_eq!(
                rd(112),
                0x0040_1000,
                "AT_ENTRY value == program entry, NOT the interpreter jump target (0x8000_4000)"
            );
        });
    }
}
